use super::*;
use chrono::NaiveDate;
use nidoo_core::energy::{OffPeakDevice, OffPeakWindow};
use nidoo_core::registry::RegistryConfig;

#[test]
fn wake_up_off_only_on_the_on_to_off_edge() {
    let with = |on| HomeSettings {
        wake_up: Some(on),
        ..HomeSettings::default()
    };
    assert!(wake_up_turned_off(&with(true), &with(false)));
    assert!(!wake_up_turned_off(&with(false), &with(false)));
    assert!(!wake_up_turned_off(&with(false), &with(true)));
    assert!(!wake_up_turned_off(&with(true), &with(true)));
}

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../nidoo-core/tests/fixtures/");

fn registry() -> Registry {
    let mut reg = Registry::new(RegistryConfig {
        discovery_prefixes: vec!["nidoo".to_owned()],
        ..RegistryConfig::default()
    });
    for (topic, file) in [
        (
            "nidoo/sensor/0x00124b0008fd6568/temperature_1/config",
            "sensor__0x00124b0008fd6568__temperature_1.json",
        ),
        (
            "nidoo/sensor/0x00124b0008fd6568/humidity_1/config",
            "sensor__0x00124b0008fd6568__humidity_1.json",
        ),
        (
            "nidoo/select/0x000d6ffffe9d44e4/indicator_mode/config",
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
fn a_document_without_wake_up_keeps_the_stored_value() {
    let on = HomeSettings {
        wake_up: Some(true),
        ..HomeSettings::default()
    };
    // A client that does not know the field sends none: Wake-up stays on, no alarm goes off.
    let next = next_document(&on, HomeSettings::default(), day("2026-11-20"));
    assert_eq!(next.wake_up, Some(true));
    assert!(!wake_up_turned_off(&on, &next));
    // An explicit off still turns it off.
    let off = HomeSettings {
        wake_up: Some(false),
        ..HomeSettings::default()
    };
    let next = next_document(&on, off, day("2026-11-20"));
    assert!(wake_up_turned_off(&on, &next));
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
    assert_eq!(topic("nidoo/ui"), "nidoo/ui/_home");
}
