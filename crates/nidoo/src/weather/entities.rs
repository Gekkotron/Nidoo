//! The weather device: pure builders for the retained discovery configs and the state payloads.
//!
//! Topics, for the first discovery prefix `P`: the config is
//! `P/<sensor|binary_sensor>/weather/<key>/config`, the state `P/weather/<key>` (a plain number,
//! enum text or `ON`/`OFF`) and the availability `P/weather/availability`. The `weather`
//! sub-tree is reserved, so the state topics are never read as discovery.

use nidoo_api::{WEATHER_DEVICE_NAME, WeatherCondition, WeatherView, weather_entity_name};
use serde_json::{Value, json};

/// The entity keys, in the order of the spec.
pub const KEYS: [&str; 26] = [
    "temperature",
    "apparent_temperature",
    "humidity",
    "condition",
    "daylight",
    "cloud_cover",
    "precipitation",
    "wind_speed",
    "wind_gusts",
    "wind_direction",
    "pressure",
    "uv_index",
    "today_min",
    "today_max",
    "today_precipitation",
    "today_precipitation_probability",
    "today_wind_gusts_max",
    "rain_next_hour",
    "tonight_min",
    "tomorrow_condition",
    "tomorrow_min",
    "tomorrow_max",
    "tomorrow_precipitation",
    "tomorrow_precipitation_probability",
    "tomorrow_wind_gusts_max",
    "tomorrow_uv_index_max",
];

const DEVICE_ID: &str = "nidoo_weather";

/// Unit, device class and whether it is a "now" measurement, per key.
fn describe(key: &str) -> (Option<&'static str>, Option<&'static str>, bool) {
    match key {
        "temperature" | "apparent_temperature" => (Some("°C"), Some("temperature"), true),
        "humidity" => (Some("%"), Some("humidity"), true),
        "condition" | "tomorrow_condition" => (None, Some("enum"), false),
        "daylight" | "tomorrow_uv_index_max" => (None, None, false),
        "cloud_cover" => (Some("%"), None, true),
        "precipitation" => (Some("mm"), Some("precipitation"), true),
        "wind_speed" | "wind_gusts" => (Some("km/h"), Some("wind_speed"), true),
        "wind_direction" => (Some("°"), None, true),
        "pressure" => (Some("hPa"), Some("atmospheric_pressure"), true),
        "uv_index" => (None, None, true),
        "today_min" | "today_max" | "tonight_min" | "tomorrow_min" | "tomorrow_max" => {
            (Some("°C"), Some("temperature"), false)
        }
        "today_precipitation" | "rain_next_hour" | "tomorrow_precipitation" => {
            (Some("mm"), Some("precipitation"), false)
        }
        "today_precipitation_probability" | "tomorrow_precipitation_probability" => {
            (Some("%"), None, false)
        }
        "today_wind_gusts_max" | "tomorrow_wind_gusts_max" => {
            (Some("km/h"), Some("wind_speed"), false)
        }
        other => unreachable!("unknown weather key `{other}`"),
    }
}

fn component(key: &str) -> &'static str {
    if key == "daylight" {
        "binary_sensor"
    } else {
        "sensor"
    }
}

/// The retained discovery config topic of `key`.
pub fn config_topic(prefix: &str, key: &str) -> String {
    format!("{prefix}/{}/weather/{key}/config", component(key))
}

/// The retained state topic of `key`.
pub fn state_topic(prefix: &str, key: &str) -> String {
    format!("{prefix}/weather/{key}")
}

/// The retained `online` / `offline` topic of the whole device.
pub fn availability_topic(prefix: &str) -> String {
    format!("{prefix}/weather/availability")
}

fn config(prefix: &str, key: &str) -> Value {
    let (unit, class, measurement) = describe(key);
    let [name, _] = weather_entity_name(key).expect("every key has a name");
    let mut config = json!({
        "name": name,
        "unique_id": format!("weather_{key}"),
        "state_topic": state_topic(prefix, key),
        "availability_topic": availability_topic(prefix),
        "device": { "identifiers": [DEVICE_ID], "name": WEATHER_DEVICE_NAME[0] },
    });
    if let Some(unit) = unit {
        config["unit_of_measurement"] = json!(unit);
    }
    if let Some(class) = class {
        config["device_class"] = json!(class);
    }
    if measurement {
        config["state_class"] = json!("measurement");
    }
    match key {
        "condition" | "tomorrow_condition" => {
            config["options"] = json!(WeatherCondition::ALL.map(WeatherCondition::as_str));
        }
        "daylight" => {
            config["payload_on"] = json!("ON");
            config["payload_off"] = json!("OFF");
        }
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
pub fn states(view: &WeatherView) -> Vec<(&'static str, String)> {
    let c = &view.current;
    let t = &view.today;
    let numbers = |v: Option<f64>| v.map(|v| v.to_string());
    let tomorrow = view.days.first();
    let values: [Option<String>; 26] = [
        numbers(c.temperature),
        numbers(c.apparent_temperature),
        numbers(c.humidity),
        c.condition.map(|c| c.as_str().to_owned()),
        c.daylight
            .map(|up| if up { "ON" } else { "OFF" }.to_owned()),
        numbers(c.cloud_cover),
        numbers(c.precipitation),
        numbers(c.wind_speed),
        numbers(c.wind_gusts),
        numbers(c.wind_direction),
        numbers(c.pressure),
        numbers(c.uv_index),
        numbers(t.min),
        numbers(t.max),
        numbers(t.precipitation),
        numbers(t.precipitation_probability),
        numbers(t.wind_gusts_max),
        numbers(view.rain_next_hour),
        numbers(view.tonight_min),
        tomorrow.and_then(|d| d.condition.map(|c| c.as_str().to_owned())),
        tomorrow.and_then(|d| numbers(d.min)),
        tomorrow.and_then(|d| numbers(d.max)),
        tomorrow.and_then(|d| numbers(d.precipitation)),
        tomorrow.and_then(|d| numbers(d.precipitation_probability)),
        tomorrow.and_then(|d| numbers(d.wind_gusts_max)),
        tomorrow.and_then(|d| numbers(d.uv_index_max)),
    ];
    KEYS.iter()
        .zip(values)
        .filter_map(|(key, value)| value.map(|value| (*key, value)))
        .collect()
}

/// Whether a discovery payload is one of the weather device's own configs.
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
    use crate::weather::{is_expired, is_stale};
    use nidoo_api::{WeatherCurrent, WeatherDay, WeatherPlace, WeatherToday};

    fn view() -> WeatherView {
        WeatherView {
            place: WeatherPlace {
                name: "Paris".into(),
                admin: None,
                country: None,
            },
            fetched_at: 0,
            stale: false,
            current: WeatherCurrent {
                temperature: Some(13.4),
                apparent_temperature: None,
                humidity: Some(81.0),
                cloud_cover: None,
                precipitation: None,
                wind_speed: None,
                wind_gusts: None,
                wind_direction: None,
                pressure: None,
                uv_index: None,
                condition: Some(WeatherCondition::Rain),
                daylight: Some(true),
            },
            today: WeatherToday {
                min: None,
                max: None,
                precipitation: None,
                precipitation_probability: None,
                wind_gusts_max: None,
            },
            rain_next_hour: None,
            tonight_min: None,
            days: vec![],
        }
    }

    fn day() -> WeatherDay {
        WeatherDay {
            date: "2026-10-24".into(),
            condition: Some(WeatherCondition::Snow),
            min: Some(-1.5),
            max: Some(4.0),
            precipitation_probability: Some(60.0),
            apparent_max: None,
            precipitation: Some(3.2),
            wind_speed_max: None,
            wind_gusts_max: Some(55.0),
            wind_direction: None,
            humidity: None,
            pressure: None,
            cloud_cover: None,
            uv_index_max: Some(0.5),
        }
    }

    #[test]
    fn there_are_twenty_six_configs_with_own_device() {
        let all = configs("nidoo");
        assert_eq!(all.len(), 26);
        for ((_, config), key) in all.iter().zip(KEYS) {
            assert_eq!(config["device"]["identifiers"], json!(["nidoo_weather"]));
            assert_eq!(config["unique_id"], json!(format!("weather_{key}")));
            assert_eq!(
                config["availability_topic"],
                json!("nidoo/weather/availability")
            );
            assert_eq!(config["state_topic"], json!(format!("nidoo/weather/{key}")));
        }
    }

    #[test]
    fn config_topics_use_the_component() {
        assert_eq!(
            config_topic("nidoo", "daylight"),
            "nidoo/binary_sensor/weather/daylight/config"
        );
        assert_eq!(
            config_topic("nidoo", "temperature"),
            "nidoo/sensor/weather/temperature/config"
        );
        let all = configs("nidoo");
        let daylight = &all[4].1;
        assert_eq!(daylight["payload_on"], json!("ON"));
        assert_eq!(daylight["payload_off"], json!("OFF"));
    }

    #[test]
    fn condition_is_an_enum_sensor() {
        let all = configs("nidoo");
        let condition = &all[3].1;
        assert_eq!(condition["device_class"], json!("enum"));
        let options: Vec<&str> = WeatherCondition::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(condition["options"], json!(options));
        assert_eq!(options.len(), 10);
    }

    #[test]
    fn tomorrow_values_come_from_the_first_day() {
        let mut v = view();
        let mut later = day();
        later.min = Some(99.0);
        v.days = vec![day(), later];
        let states = states(&v);
        assert!(states.contains(&("tomorrow_condition", "snow".to_owned())));
        assert!(states.contains(&("tomorrow_min", "-1.5".to_owned())));
        assert!(states.contains(&("tomorrow_max", "4".to_owned())));
        assert!(states.contains(&("tomorrow_precipitation", "3.2".to_owned())));
        assert!(states.contains(&("tomorrow_precipitation_probability", "60".to_owned())));
        assert!(states.contains(&("tomorrow_wind_gusts_max", "55".to_owned())));
        assert!(states.contains(&("tomorrow_uv_index_max", "0.5".to_owned())));
    }

    #[test]
    fn tomorrow_condition_is_an_enum_sensor() {
        let all = configs("nidoo");
        let index = KEYS
            .iter()
            .position(|k| *k == "tomorrow_condition")
            .unwrap();
        let tomorrow = &all[index].1;
        assert_eq!(tomorrow["device_class"], json!("enum"));
        assert_eq!(tomorrow["options"], all[3].1["options"]);
        assert!(tomorrow.get("state_class").is_none());
    }

    #[test]
    fn states_skip_missing_values() {
        let states = states(&view());
        assert!(states.iter().all(|(key, _)| *key != "uv_index"));
        assert!(states.contains(&("daylight", "ON".to_owned())));
        assert!(states.contains(&("condition", "rain".to_owned())));
        assert!(states.contains(&("temperature", "13.4".to_owned())));
        assert!(states.contains(&("humidity", "81".to_owned())));
        assert_eq!(states.len(), 4);
    }

    #[test]
    fn full_view_yields_every_key_in_order() {
        let mut full = view();
        let c = &mut full.current;
        c.apparent_temperature = Some(1.0);
        c.cloud_cover = Some(1.0);
        c.precipitation = Some(1.0);
        c.wind_speed = Some(1.0);
        c.wind_gusts = Some(1.0);
        c.wind_direction = Some(1.0);
        c.pressure = Some(1.0);
        c.uv_index = Some(1.0);
        full.today = WeatherToday {
            min: Some(1.0),
            max: Some(1.0),
            precipitation: Some(1.0),
            precipitation_probability: Some(1.0),
            wind_gusts_max: Some(1.0),
        };
        full.rain_next_hour = Some(1.0);
        full.tonight_min = Some(1.0);
        full.days = vec![day()];
        let keys: Vec<&str> = states(&full).iter().map(|(key, _)| *key).collect();
        assert_eq!(keys, KEYS);
    }

    #[test]
    fn own_config_is_recognised() {
        let own = configs("nidoo")[0].1.to_string();
        assert!(is_own_config(own.as_bytes()));
        assert!(!is_own_config(br#"{"device":{"identifiers":["0x1234"]}}"#));
        assert!(!is_own_config(b"garbage"));
    }

    #[test]
    fn freshness_rules() {
        assert!(!is_stale(0, 1800, 15));
        assert!(is_stale(0, 1801, 15));
        assert!(!is_expired(0, 3600));
        assert!(is_expired(0, 3601));
    }
}
