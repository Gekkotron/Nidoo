//! The solar forecast device: pure builders for the retained discovery configs and the state
//! payloads.
//!
//! Topics, for the first discovery prefix `P`: the config is `P/sensor/solar/<key>/config`, the
//! state `P/solar/<key>` (a plain number, or an RFC 3339 UTC time), the availability
//! `P/solar/availability` and the retained forecast cache `P/solar/cache`. The `solar` sub-tree
//! is reserved, so these topics are never read as discovery.

use chrono::{DateTime, NaiveDate, SecondsFormat};
use nidoo_api::SolarView;
use serde_json::{Value, json};

/// The entity keys, in the order of the spec.
pub const KEYS: [&str; 10] = [
    "forecast_today",
    "forecast_today_low",
    "forecast_today_high",
    "forecast_remaining_today",
    "forecast_tomorrow",
    "forecast_tomorrow_low",
    "forecast_tomorrow_high",
    "forecast_power_now",
    "forecast_peak_today",
    "forecast_peak_time_today",
];

const DEVICE_ID: &str = "nidoo_solar";

/// Name and unit per key (the peak time has no unit).
fn describe(key: &str) -> (&'static str, Option<&'static str>) {
    match key {
        "forecast_today" => ("Solar forecast today", Some("kWh")),
        "forecast_today_low" => ("Solar forecast today (low)", Some("kWh")),
        "forecast_today_high" => ("Solar forecast today (high)", Some("kWh")),
        "forecast_remaining_today" => ("Solar forecast remaining today", Some("kWh")),
        "forecast_tomorrow" => ("Solar forecast tomorrow", Some("kWh")),
        "forecast_tomorrow_low" => ("Solar forecast tomorrow (low)", Some("kWh")),
        "forecast_tomorrow_high" => ("Solar forecast tomorrow (high)", Some("kWh")),
        "forecast_power_now" => ("Solar forecast power now", Some("kW")),
        "forecast_peak_today" => ("Solar forecast peak today", Some("kW")),
        "forecast_peak_time_today" => ("Solar forecast peak time today", None),
        other => unreachable!("unknown solar key `{other}`"),
    }
}

/// The retained discovery config topic of `key`.
pub fn config_topic(prefix: &str, key: &str) -> String {
    format!("{prefix}/sensor/solar/{key}/config")
}

/// The retained state topic of `key`.
pub fn state_topic(prefix: &str, key: &str) -> String {
    format!("{prefix}/solar/{key}")
}

/// The retained `online` / `offline` topic of the whole device.
pub fn availability_topic(prefix: &str) -> String {
    format!("{prefix}/solar/availability")
}

/// The retained topic holding the last forecast, so a restart needs no call.
pub fn cache_topic(prefix: &str) -> String {
    format!("{prefix}/solar/cache")
}

fn config(prefix: &str, key: &str) -> Value {
    let (name, unit) = describe(key);
    let mut config = json!({
        "name": name,
        "object_id": format!("solar_{key}"),
        "unique_id": format!("solar_{key}"),
        "state_topic": state_topic(prefix, key),
        "availability_topic": availability_topic(prefix),
        "icon": "mdi:solar-power",
        "device": {
            "identifiers": [DEVICE_ID],
            "name": "Solar forecast",
            "manufacturer": "Nidoo",
        },
    });
    if let Some(unit) = unit {
        config["unit_of_measurement"] = json!(unit);
    }
    match key {
        "forecast_power_now" => config["state_class"] = json!("measurement"),
        "forecast_peak_time_today" => config["device_class"] = json!("timestamp"),
        _ => {}
    }
    config
}

/// Every entity's (config topic, config).
pub fn configs(prefix: &str) -> Vec<(String, Value)> {
    KEYS.iter()
        .map(|key| (config_topic(prefix, key), config(prefix, key)))
        .collect()
}

/// One (key, payload) per key that has a value.
pub fn states(view: &SolarView) -> Vec<(&'static str, String)> {
    // Tomorrow is the day after today's date, not merely the first listed day.
    let tomorrow = view
        .today
        .date
        .parse::<NaiveDate>()
        .ok()
        .and_then(|today| today.succ_opt())
        .map(|date| date.format("%Y-%m-%d").to_string())
        .and_then(|date| view.days.iter().find(|d| d.date == date));
    let numbers = |v: Option<f64>| v.map(|v| v.to_string());
    let peak_time = view.today.peak.as_ref().and_then(|p| {
        let at = DateTime::from_timestamp(i64::try_from(p.at).ok()?, 0)?;
        Some(at.to_rfc3339_opts(SecondsFormat::Secs, true))
    });
    let values: [Option<String>; 10] = [
        numbers(view.today.estimate),
        numbers(view.today.low),
        numbers(view.today.high),
        numbers(view.now.remaining_today),
        numbers(tomorrow.map(|d| d.estimate)),
        numbers(tomorrow.map(|d| d.low)),
        numbers(tomorrow.map(|d| d.high)),
        numbers(view.now.power),
        numbers(view.today.peak.as_ref().map(|p| p.power)),
        peak_time,
    ];
    KEYS.iter()
        .zip(values)
        .filter_map(|(key, value)| value.map(|value| (*key, value)))
        .collect()
}

/// Whether a discovery payload is one of the solar device's own configs.
pub fn is_own_config(payload: &[u8]) -> bool {
    serde_json::from_slice::<Value>(payload)
        .ok()
        .and_then(|config| {
            config["device"]["identifiers"]
                .as_array()
                .map(|ids| ids.iter().any(|id| id == DEVICE_ID))
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solar::view::build_view;
    use chrono::{DateTime, Utc};
    use nidoo_solcast::{Ledger, merge, parse_forecasts};

    fn fixture_view() -> SolarView {
        let periods = parse_forecasts(include_str!(
            "../../../nidoo-solcast/tests/fixtures/forecasts.json"
        ))
        .unwrap();
        let now: DateTime<Utc> = "2026-10-23T10:00:00Z".parse().unwrap();
        build_view(
            &merge(vec![periods], 0),
            &Ledger::default(),
            chrono_tz::Europe::Paris,
            now,
            None,
            10,
            false,
        )
    }

    #[test]
    fn configs_have_no_energy_or_power_class() {
        let all = configs("nidoo");
        assert_eq!(all.len(), 10);
        for (_, config) in &all {
            assert_ne!(config["device_class"], json!("energy"));
            assert_ne!(config["device_class"], json!("power"));
            assert_ne!(config["state_class"], json!("total_increasing"));
            assert_eq!(config["icon"], json!("mdi:solar-power"));
            assert_eq!(config["device"]["identifiers"], json!(["nidoo_solar"]));
            assert_eq!(config["device"]["name"], json!("Solar forecast"));
        }
        let by_key = |k: &str| &all[KEYS.iter().position(|x| *x == k).unwrap()].1;
        assert_eq!(
            by_key("forecast_peak_time_today")["device_class"],
            json!("timestamp")
        );
        assert!(by_key("forecast_peak_time_today")["unit_of_measurement"].is_null());
        assert_eq!(
            by_key("forecast_power_now")["state_class"],
            json!("measurement")
        );
        assert_eq!(
            by_key("forecast_power_now")["unit_of_measurement"],
            json!("kW")
        );
        assert_eq!(
            by_key("forecast_today")["unit_of_measurement"],
            json!("kWh")
        );
        assert!(by_key("forecast_today")["state_class"].is_null());
    }

    #[test]
    fn configs_topics_and_ids() {
        let all = configs("nidoo");
        assert_eq!(all[0].0, "nidoo/sensor/solar/forecast_today/config");
        assert_eq!(all[0].1["unique_id"], json!("solar_forecast_today"));
        assert_eq!(all[0].1["state_topic"], json!("nidoo/solar/forecast_today"));
        assert_eq!(
            all[0].1["availability_topic"],
            json!("nidoo/solar/availability")
        );
        assert_eq!(cache_topic("nidoo"), "nidoo/solar/cache");
        assert_eq!(
            state_topic("nidoo", "forecast_power_now"),
            "nidoo/solar/forecast_power_now"
        );
    }

    #[test]
    fn states_omit_missing_values() {
        let full = states(&fixture_view());
        assert!(full.contains(&("forecast_today", "12.4".to_owned())));
        assert!(full.contains(&("forecast_peak_today", "2.4".to_owned())));
        assert!(full.contains(&(
            "forecast_peak_time_today",
            "2026-10-23T11:15:00Z".to_owned()
        )));
        // The fixture holds one day only: no tomorrow figures.
        let keys: Vec<&str> = full.iter().map(|(k, _)| *k).collect();
        assert!(!keys.contains(&"forecast_tomorrow"));
        assert!(!keys.contains(&"forecast_tomorrow_low"));
        assert!(!keys.contains(&"forecast_tomorrow_high"));
        let mut empty = fixture_view();
        empty.today.estimate = None;
        empty.today.peak = None;
        let keys: Vec<&str> = states(&empty).iter().map(|(k, _)| *k).collect();
        assert!(!keys.contains(&"forecast_today"));
        assert!(!keys.contains(&"forecast_peak_today"));
        assert!(!keys.contains(&"forecast_peak_time_today"));
    }

    #[test]
    fn tomorrow_entities_use_the_day_after_today() {
        let day = |date: &str, estimate: f64| nidoo_api::SolarDay {
            date: date.to_owned(),
            estimate,
            low: estimate - 1.0,
            high: estimate + 1.0,
        };
        let mut view = fixture_view();
        assert_eq!(view.today.date, "2026-10-23");
        // The first listed day is two days out: no tomorrow figures.
        view.days = vec![day("2026-10-25", 9.0)];
        let keys: Vec<&str> = states(&view).iter().map(|(k, _)| *k).collect();
        assert!(!keys.contains(&"forecast_tomorrow"));
        // Tomorrow is found wherever it sits in the list.
        view.days = vec![day("2026-10-25", 9.0), day("2026-10-24", 7.5)];
        let full = states(&view);
        assert!(full.contains(&("forecast_tomorrow", "7.5".to_owned())));
        assert!(full.contains(&("forecast_tomorrow_low", "6.5".to_owned())));
        assert!(full.contains(&("forecast_tomorrow_high", "8.5".to_owned())));
    }

    #[test]
    fn own_config_recognised() {
        let own = configs("nidoo")[0].1.to_string();
        assert!(is_own_config(own.as_bytes()));
        assert!(!is_own_config(
            br#"{"device":{"identifiers":["nidoo_weather"]}}"#
        ));
        assert!(!is_own_config(b"garbage"));
    }
}
