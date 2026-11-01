//! The home settings document: the few values the home header needs from the hub (the eco
//! setpoint, the house mode select and the indoor temperature sensor). It is retained on
//! `<ui_prefix>/_home`, next to `_areas` and `_types`, and intercepted by the hub before the
//! registry so it never becomes an override.

use chrono::NaiveDate;
use chrono::{DateTime, TimeZone, Timelike};
use chrono_tz::Tz;
use nidoo_api::{HomeSettings, WsEvent};
use nidoo_core::command::Publish;
use nidoo_core::energy::{
    EnergyMeters, GridCounters, OffPeakEntry, device_off_peak, next_boundary, record_change,
    resolve_off_peak, validate_windows, windows_for,
};
use nidoo_core::model::Kind;
use nidoo_core::registry::Registry;
use std::sync::MutexGuard;

use crate::hub::{Hub, HubError};

/// The lowest accepted eco setpoint, in °C.
const ECO_MIN: f64 = 5.0;
/// The highest accepted eco setpoint, in °C.
const ECO_MAX: f64 = 30.0;
/// The lowest accepted cold or hot threshold, in °C.
const THRESHOLD_MIN: f64 = 5.0;
/// The highest accepted cold or hot threshold, in °C.
const THRESHOLD_MAX: f64 = 35.0;

/// The topic the document is retained on.
pub fn topic(ui_prefix: &str) -> String {
    format!("{ui_prefix}/_home")
}

/// The document to store for `incoming`: the client's `off_peak_history` is replaced by the
/// hub's, with an entry for `today` when the windows in force change.
pub fn next_document(
    current: &HomeSettings,
    incoming: HomeSettings,
    today: NaiveDate,
) -> HomeSettings {
    let off_peak_history = record_change(&current.off_peak_history, &incoming.off_peak, today);
    HomeSettings {
        off_peak_history,
        ..incoming
    }
}

/// The dated windows to split days with: the stored history, or the current windows from
/// `today` (and so, as the oldest entry, every earlier day) for a document saved before the
/// hub kept a history.
pub fn effective_history(settings: &HomeSettings, today: NaiveDate) -> Vec<OffPeakEntry> {
    if settings.off_peak_history.is_empty() {
        record_change(&[], &settings.off_peak, today)
    } else {
        settings.off_peak_history.clone()
    }
}

/// Checks the document against the house: the setpoint is a sane temperature, the mode is a
/// select, the indoor entity is a temperature sensor and the cold threshold is below the hot one.
/// The message names the field.
pub fn validate(settings: &HomeSettings, reg: &Registry) -> Result<(), String> {
    validate_windows(&settings.off_peak)?;
    if let Some(eco) = settings.eco_setpoint
        && !(eco.is_finite() && (ECO_MIN..=ECO_MAX).contains(&eco))
    {
        return Err(format!(
            "eco_setpoint must be between {ECO_MIN} and {ECO_MAX} °C"
        ));
    }
    for (name, value) in [
        ("cold_below", settings.cold_below),
        ("hot_above", settings.hot_above),
    ] {
        if let Some(v) = value
            && !(v.is_finite() && (THRESHOLD_MIN..=THRESHOLD_MAX).contains(&v))
        {
            return Err(format!(
                "{name} must be between {THRESHOLD_MIN} and {THRESHOLD_MAX} °C"
            ));
        }
    }
    if let (Some(cold), Some(hot)) = (settings.cold_below, settings.hot_above)
        && cold >= hot
    {
        return Err("cold_below must be lower than hot_above".to_owned());
    }
    if let Some(id) = &settings.mode_entity {
        match reg.entity(id) {
            Some(entity) if entity.kind == Kind::Select => {}
            Some(_) => return Err(format!("mode_entity `{id}` is not a select")),
            None => return Err(format!("mode_entity `{id}` is not an entity")),
        }
    }
    if let Some(id) = &settings.indoor_entity {
        match reg.entity(id) {
            Some(entity)
                if entity.kind == Kind::Sensor
                    && entity.device_class.as_deref() == Some("temperature") => {}
            Some(_) => return Err(format!("indoor_entity `{id}` is not a temperature sensor")),
            None => return Err(format!("indoor_entity `{id}` is not an entity")),
        }
    }
    if let Some(device) = &settings.off_peak_device {
        if device.entity.is_empty() {
            return Err("off_peak_device needs an entity".to_owned());
        }
        if reg.entity(&device.entity).is_none() {
            return Err(format!(
                "off_peak_device `{}` is not an entity",
                device.entity
            ));
        }
        if device.values.is_empty() {
            return Err("off_peak_device needs at least one off-peak value".to_owned());
        }
    }
    Ok(())
}

/// The moment the wall clock reads `ahead` minutes after `now`'s minute: on a DST day the
/// windows still start and end at their local times. A time the clock skips (spring forward)
/// is the first valid one after it; a time it reads twice (fall back) is the first of the two.
pub fn boundary_at(now: DateTime<Tz>, ahead: u16) -> DateTime<Tz> {
    let zone = now.timezone();
    let mut local = now
        .date_naive()
        .and_hms_opt(now.hour(), now.minute(), 0)
        .unwrap_or_default()
        + chrono::Duration::minutes(i64::from(ahead));
    // A skipped hour is at most a few, so this stops quickly.
    for _ in 0..16 {
        if let Some(at) = zone.from_local_datetime(&local).earliest() {
            return at;
        }
        local += chrono::Duration::minutes(15);
    }
    now + chrono::Duration::minutes(i64::from(ahead))
}

/// Whether it is off-peak at a moment, and what said so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OffPeakNow {
    pub off_peak: bool,
    /// The device entity when its reading decided; `None` when the windows did.
    pub device: Option<String>,
    /// The next window start or end; `None` without windows.
    pub next_boundary: Option<DateTime<Tz>>,
}

impl Hub {
    /// Off-peak at `now`: the off-peak device's reading when usable, else the windows in force
    /// that day.
    pub fn off_peak_now(&self, now: DateTime<Tz>) -> OffPeakNow {
        let settings = self.home_settings();
        let reading = settings.off_peak_device.as_ref().and_then(|device| {
            let reg = self.read();
            let state = reg.state(&device.entity)?;
            device_off_peak(device, &state.value, state.availability)
        });
        let today = now.date_naive();
        let history = effective_history(&settings, today);
        let windows = windows_for(&history, today);
        let minute = (now.hour() * 60 + now.minute()) as u16;
        let next_boundary = next_boundary(windows, minute).map(|ahead| boundary_at(now, ahead));
        OffPeakNow {
            off_peak: resolve_off_peak(reading, windows, minute),
            device: reading
                .and(settings.off_peak_device)
                .map(|device| device.entity),
            next_boundary,
        }
    }

    fn home_slot(&self) -> MutexGuard<'_, HomeSettings> {
        self.home_settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The home settings as last applied (defaults until one is retained).
    pub fn home_settings(&self) -> HomeSettings {
        self.home_slot().clone()
    }

    /// Today's date in the house zone.
    pub fn house_today(&self) -> NaiveDate {
        crate::history::query::local_date(self.zone(), crate::clock::now_micros())
    }

    /// The dated off-peak windows the grid day splits with.
    pub fn off_peak_history(&self) -> Vec<OffPeakEntry> {
        effective_history(&self.home_slot(), self.house_today())
    }

    /// The grid meter's bought and sold counters, from the entities' energy roles.
    pub fn grid_counters(&self) -> GridCounters {
        let entities = self.snapshot().entities;
        EnergyMeters::grid_counters(&nidoo_api::energy_entities(&entities))
    }

    /// Validates, publishes the document retained and applies it. Returns the stored document.
    pub fn set_home_settings(&self, settings: HomeSettings) -> Result<HomeSettings, HubError> {
        validate(&settings, &self.read()).map_err(HubError::Rejected)?;
        let today =
            chrono::DateTime::<chrono::Utc>::from_timestamp_micros(crate::clock::now_micros())
                .unwrap_or_default()
                .with_timezone(&self.zone())
                .date_naive();
        let settings = next_document(&self.home_settings(), settings, today);
        let payload = serde_json::to_string(&settings)
            .map_err(|err| HubError::Rejected(format!("home settings: {err}")))?;
        self.publish(Publish {
            topic: topic(&self.ui_prefix),
            payload: payload.clone(),
            retain: true,
        })?;
        self.ingest_home(payload.as_bytes());
        Ok(self.home_settings())
    }

    /// Applies the retained document: empty means defaults, an undecodable payload keeps the
    /// previous document. A change is broadcast.
    pub(crate) fn ingest_home(&self, payload: &[u8]) {
        let next = if payload.is_empty() {
            HomeSettings::default()
        } else {
            match serde_json::from_slice::<HomeSettings>(payload) {
                Ok(settings) => settings,
                Err(err) => {
                    tracing::warn!(%err, "ignoring an undecodable home settings document");
                    return;
                }
            }
        };
        {
            let mut slot = self.home_slot();
            if *slot == next {
                return;
            }
            *slot = next.clone();
        }
        self.broadcast(WsEvent::HomeSettingsChanged { settings: next });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use nidoo_core::energy::{OffPeakDevice, OffPeakWindow};
    use nidoo_core::registry::RegistryConfig;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../nidoo-core/tests/fixtures/");

    fn registry() -> Registry {
        let mut reg = Registry::new(RegistryConfig {
            discovery_prefixes: vec!["oikos".to_owned()],
            ..RegistryConfig::default()
        });
        for (topic, file) in [
            (
                "oikos/sensor/0x00124b0008fd6568/temperature_1/config",
                "sensor__0x00124b0008fd6568__temperature_1.json",
            ),
            (
                "oikos/sensor/0x00124b0008fd6568/humidity_1/config",
                "sensor__0x00124b0008fd6568__humidity_1.json",
            ),
            (
                "oikos/select/0x000d6ffffe9d44e4/indicator_mode/config",
                "select__0x000d6ffffe9d44e4__indicator_mode.json",
            ),
        ] {
            let payload = std::fs::read(format!("{FIXTURES}discovery/{file}")).expect("fixture");
            let inbound = reg.route(topic, payload, 0);
            reg.apply(inbound);
        }
        reg
    }

    /// The id of the first entity matching `pred`.
    fn id_where(reg: &Registry, pred: impl Fn(&nidoo_core::model::Entity) -> bool) -> String {
        reg.entities()
            .find(|e| pred(e))
            .expect("fixture entity")
            .unique_id
            .clone()
    }

    fn eco(value: Option<f64>) -> HomeSettings {
        HomeSettings {
            eco_setpoint: value,
            ..HomeSettings::default()
        }
    }

    #[test]
    fn eco_setpoint_bounds() {
        let reg = registry();
        assert!(validate(&eco(Some(5.0)), &reg).is_ok());
        assert!(validate(&eco(Some(30.0)), &reg).is_ok());
        assert!(validate(&eco(None), &reg).is_ok());
        for bad in [4.9, 30.1, f64::NAN, f64::INFINITY] {
            let err = validate(&eco(Some(bad)), &reg).expect_err("out of range");
            assert!(err.contains("eco_setpoint"), "{err}");
        }
    }

    #[test]
    fn thresholds_bounds_and_order() {
        let reg = registry();
        let cold = |v: Option<f64>| HomeSettings {
            cold_below: v,
            ..HomeSettings::default()
        };
        let hot = |v: Option<f64>| HomeSettings {
            hot_above: v,
            ..HomeSettings::default()
        };
        for ok in [5.0, 35.0] {
            assert!(validate(&cold(Some(ok)), &reg).is_ok());
            assert!(validate(&hot(Some(ok)), &reg).is_ok());
        }
        for bad in [4.9, 35.1, f64::NAN] {
            let err = validate(&cold(Some(bad)), &reg).expect_err("out of range");
            assert!(err.contains("cold_below"), "{err}");
            let err = validate(&hot(Some(bad)), &reg).expect_err("out of range");
            assert!(err.contains("hot_above"), "{err}");
        }
        let both = |c: f64, h: f64| HomeSettings {
            cold_below: Some(c),
            hot_above: Some(h),
            ..HomeSettings::default()
        };
        let err = validate(&both(20.0, 20.0), &reg).expect_err("not ordered");
        assert!(err.contains("cold_below"), "{err}");
        assert!(validate(&both(15.0, 26.0), &reg).is_ok());
    }

    #[test]
    fn mode_entity_must_be_a_select() {
        let reg = registry();
        let select = id_where(&reg, |e| e.kind == Kind::Select);
        let sensor = id_where(&reg, |e| e.kind == Kind::Sensor);
        let with = |id: &str| HomeSettings {
            mode_entity: Some(id.to_owned()),
            ..HomeSettings::default()
        };
        assert!(validate(&with(&select), &reg).is_ok());
        for bad in [sensor.as_str(), "nope"] {
            let err = validate(&with(bad), &reg).expect_err("not a select");
            assert!(err.contains("mode_entity"), "{err}");
        }
    }

    #[test]
    fn indoor_entity_must_be_a_temperature_sensor() {
        let reg = registry();
        let class = |c: &str| {
            id_where(&reg, |e| {
                e.kind == Kind::Sensor && e.device_class.as_deref() == Some(c)
            })
        };
        let with = |id: &str| HomeSettings {
            indoor_entity: Some(id.to_owned()),
            ..HomeSettings::default()
        };
        assert!(validate(&with(&class("temperature")), &reg).is_ok());
        for bad in [class("humidity"), "nope".to_owned()] {
            let err = validate(&with(&bad), &reg).expect_err("not a temperature sensor");
            assert!(err.contains("indoor_entity"), "{err}");
        }
    }

    fn win(start: &str, end: &str) -> OffPeakWindow {
        OffPeakWindow {
            start: start.to_owned(),
            end: end.to_owned(),
        }
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn boundary_follows_the_wall_clock_across_dst() {
        use chrono::TimeZone;
        use chrono_tz::Europe::Paris;
        // Spring forward: 2026-03-29 02:00 CET jumps to 03:00 CEST.
        let now = Paris.with_ymd_and_hms(2026, 3, 29, 0, 30, 0).unwrap();
        let at = boundary_at(now, 6 * 60 - 30);
        assert_eq!(at, Paris.with_ymd_and_hms(2026, 3, 29, 6, 0, 0).unwrap());
        // A boundary inside the gap is the first valid time after it.
        let at = boundary_at(now, 2 * 60);
        assert_eq!(at, Paris.with_ymd_and_hms(2026, 3, 29, 3, 0, 0).unwrap());
        // Fall back: 2026-10-25 03:00 CEST goes back to 02:00 CET.
        let now = Paris.with_ymd_and_hms(2026, 10, 25, 0, 30, 0).unwrap();
        let at = boundary_at(now, 6 * 60 - 30);
        assert_eq!(at, Paris.with_ymd_and_hms(2026, 10, 25, 6, 0, 0).unwrap());
        // Past midnight.
        let now = Paris.with_ymd_and_hms(2026, 1, 1, 23, 0, 30).unwrap();
        let at = boundary_at(now, 7 * 60);
        assert_eq!(at, Paris.with_ymd_and_hms(2026, 1, 2, 6, 0, 0).unwrap());
    }

    #[test]
    fn off_peak_device_must_name_an_entity_and_values() {
        let reg = registry();
        let sensor = id_where(&reg, |e| e.unique_id.contains("temperature"));
        let with = |entity: &str, values: &[&str]| HomeSettings {
            off_peak_device: Some(OffPeakDevice {
                entity: entity.to_owned(),
                values: values.iter().map(|v| (*v).to_owned()).collect(),
            }),
            ..HomeSettings::default()
        };
        for bad in [
            with("sensor.nope", &["OFF"]),
            with("", &["OFF"]),
            with(&sensor, &[]),
        ] {
            let err = validate(&bad, &reg).expect_err("refused");
            assert!(err.starts_with("off_peak_device"), "{err}");
        }
        // A device alone is enough: the windows are optional.
        let ok = with(&sensor, &["OFF"]);
        assert!(ok.off_peak.is_empty());
        assert_eq!(validate(&ok, &reg), Ok(()));
    }

    #[test]
    fn validate_refuses_overlapping_windows() {
        let reg = registry();
        let doc = HomeSettings {
            off_peak: vec![win("22:00", "06:00"), win("05:00", "07:00")],
            ..HomeSettings::default()
        };
        let err = validate(&doc, &reg).expect_err("overlap");
        assert!(err.starts_with("off_peak windows 1 and 2 overlap"), "{err}");
    }

    #[test]
    fn set_records_window_change() {
        let incoming = HomeSettings {
            off_peak: vec![win("01:00", "07:00")],
            ..HomeSettings::default()
        };
        let next = next_document(&HomeSettings::default(), incoming, day("2026-10-24"));
        assert_eq!(next.off_peak_history.len(), 1);
        assert_eq!(next.off_peak_history[0].since, "2026-10-24");
        assert_eq!(
            next.off_peak_history[0].windows,
            vec![win("01:00", "07:00")]
        );
    }

    #[test]
    fn unchanged_windows_keep_history() {
        let first = next_document(
            &HomeSettings::default(),
            HomeSettings {
                off_peak: vec![win("01:00", "07:00")],
                ..HomeSettings::default()
            },
            day("2026-10-20"),
        );
        let again = HomeSettings {
            eco_setpoint: Some(16.0),
            ..first.clone()
        };
        let next = next_document(&first, again, day("2026-10-24"));
        assert_eq!(next.off_peak_history, first.off_peak_history);
        assert_eq!(next.eco_setpoint, Some(16.0));
    }

    #[test]
    fn client_history_is_ignored() {
        let forged = HomeSettings {
            off_peak_history: vec![OffPeakEntry {
                since: "2020-01-01".to_owned(),
                windows: vec![win("00:00", "12:00")],
            }],
            ..HomeSettings::default()
        };
        let next = next_document(&HomeSettings::default(), forged, day("2026-10-24"));
        assert!(next.off_peak_history.is_empty());
    }

    #[test]
    fn history_falls_back_to_current_windows() {
        let doc = HomeSettings {
            off_peak: vec![win("01:00", "07:00")],
            ..HomeSettings::default()
        };
        let h = effective_history(&doc, day("2026-10-24"));
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].windows, doc.off_peak);
        assert!(effective_history(&HomeSettings::default(), day("2026-10-24")).is_empty());
    }

    #[test]
    fn topic_sits_beside_areas() {
        assert_eq!(topic("oikos/ui"), "oikos/ui/_home");
    }
}
