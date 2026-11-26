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
/// The bounds of the assistant's no-question range, in °C.
const ASSIST_MIN: f64 = 5.0;
const ASSIST_MAX: f64 = 35.0;

/// Whether Wake-up went from on to off between two documents.
fn wake_up_turned_off(before: &HomeSettings, after: &HomeSettings) -> bool {
    before.wake_up_on() && !after.wake_up_on()
}

/// The topic the document is retained on.
pub fn topic(ui_prefix: &str) -> String {
    format!("{ui_prefix}/_home")
}

/// The document to store for `incoming`: the client's `off_peak_history` is replaced by the
/// hub's, with an entry for `today` when the windows in force change; a document without
/// `wake_up` keeps the stored one (a client that does not know it must not turn it off).
pub fn next_document(
    current: &HomeSettings,
    incoming: HomeSettings,
    today: NaiveDate,
) -> HomeSettings {
    let off_peak_history = record_change(&current.off_peak_history, &incoming.off_peak, today);
    HomeSettings {
        off_peak_history,
        wake_up: incoming.wake_up.or(current.wake_up),
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
    for (name, value) in [
        ("assist_min_c", settings.assist_min_c),
        ("assist_max_c", settings.assist_max_c),
    ] {
        if let Some(v) = value
            && !(v.is_finite() && (ASSIST_MIN..=ASSIST_MAX).contains(&v))
        {
            return Err(format!(
                "{name} must be between {ASSIST_MIN} and {ASSIST_MAX} °C"
            ));
        }
    }
    let (min, max) = settings.assist_range();
    if min >= max {
        return Err("assist_min_c must be lower than assist_max_c".to_owned());
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

    /// Saves the settings like [`Hub::set_home_settings`], then, when Wake-up went from on to
    /// off and the `wake-up` label exists, turns off every flow carrying it. A failure of that
    /// switch is logged: the settings are saved either way.
    pub async fn apply_home_settings(
        &self,
        settings: HomeSettings,
    ) -> Result<HomeSettings, HubError> {
        let before = self.home_settings();
        let stored = self.set_home_settings(settings)?;
        if wake_up_turned_off(&before, &stored)
            && self
                .flow_label_catalog()
                .label(nidoo_core::flow::wake_up::LABEL)
                .is_some()
            && let Err(err) = self
                .switch_flow_label(
                    nidoo_core::flow::wake_up::LABEL,
                    nidoo_core::flow::LabelAction::Off,
                    None,
                )
                .await
        {
            tracing::warn!(%err, "could not turn off the wake-up flows");
        }
        Ok(stored)
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
mod tests;
