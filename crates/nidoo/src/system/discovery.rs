//! The retained discovery configs of the system sensors, all on one "Nidoo hub" device. For
//! the first discovery prefix `P`: the config is `P/sensor/nidoo_hub/<id>/config`
//! (`P/binary_sensor/…` for the three services), the state `P/system/<id>` and the
//! availability `P/system/<id>/avail`.

use nidoo_api::system::ids;
use serde_json::{Value, json};

/// The device every system sensor belongs to.
const DEVICE_ID: &str = "nidoo_hub";
const DEVICE_NAME: &str = "Nidoo hub";

/// Whether `id` is one of the three service connectivity sensors.
pub fn is_binary(id: &str) -> bool {
    matches!(id, ids::BROKER | ids::ZIGBEE2MQTT | ids::DATABASE)
}

/// The retained discovery config topic of sensor `id`.
pub fn config_topic(prefix: &str, id: &str) -> String {
    let component = if is_binary(id) {
        "binary_sensor"
    } else {
        "sensor"
    };
    format!("{prefix}/{component}/{DEVICE_ID}/{id}/config")
}

/// The retained state topic of sensor `id`.
pub fn state_topic(prefix: &str, id: &str) -> String {
    format!("{prefix}/system/{id}")
}

/// The retained availability topic of sensor `id`, `online` or `offline`.
pub fn availability_topic(prefix: &str, id: &str) -> String {
    format!("{prefix}/system/{id}/avail")
}

/// The name, then the unit, device class, state class and display precision of sensor `id`.
fn describe(
    id: &str,
) -> (
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
    Option<u8>,
) {
    match id {
        ids::CPU => ("CPU", Some("%"), None, Some("measurement"), Some(1)),
        ids::CPU_HUB => ("Hub CPU", Some("%"), None, Some("measurement"), Some(1)),
        ids::RAM_USED => (
            "RAM used",
            Some("MB"),
            Some("data_size"),
            Some("measurement"),
            Some(0),
        ),
        ids::RAM_HUB => (
            "Hub RAM",
            Some("MB"),
            Some("data_size"),
            Some("measurement"),
            Some(0),
        ),
        ids::DISK_USED => (
            "Disk used",
            Some("GB"),
            Some("data_size"),
            Some("measurement"),
            Some(2),
        ),
        ids::DB_SIZE => (
            "Database size",
            Some("GB"),
            Some("data_size"),
            Some("measurement"),
            Some(2),
        ),
        ids::TEMPERATURE => (
            "Temperature",
            Some("°C"),
            Some("temperature"),
            Some("measurement"),
            Some(1),
        ),
        ids::DEVICES => ("Devices", None, None, Some("measurement"), None),
        ids::ENTITIES => ("Entities", None, None, Some("measurement"), None),
        ids::RETAINED => ("Retained messages", None, None, Some("measurement"), None),
        ids::COMMANDS => ("Commands today", None, None, Some("total_increasing"), None),
        ids::STARTED => ("Started", None, Some("timestamp"), None, None),
        ids::BROKER => ("Broker", None, Some("connectivity"), None, None),
        ids::ZIGBEE2MQTT => ("Zigbee2MQTT", None, Some("connectivity"), None, None),
        ids::DATABASE => ("Database", None, Some("connectivity"), None, None),
        ids::LAST_BACKUP => ("Last backup", None, Some("timestamp"), None, None),
        _ => ("", None, None, None, None),
    }
}

/// The discovery config of sensor `id`.
pub fn config(prefix: &str, id: &str) -> Value {
    let (name, unit, class, state_class, precision) = describe(id);
    let mut config = json!({
        "name": name,
        "unique_id": id,
        "state_topic": state_topic(prefix, id),
        "availability_topic": availability_topic(prefix, id),
        "payload_available": "online",
        "payload_not_available": "offline",
        "entity_category": "diagnostic",
        "device": {"identifiers": [DEVICE_ID], "name": DEVICE_NAME},
    });
    let fields = [
        ("unit_of_measurement", unit.map(Value::from)),
        ("device_class", class.map(Value::from)),
        ("state_class", state_class.map(Value::from)),
        ("suggested_display_precision", precision.map(Value::from)),
    ];
    for (key, value) in fields {
        if let Some(value) = value {
            config[key] = value;
        }
    }
    if is_binary(id) {
        config["payload_on"] = "ON".into();
        config["payload_off"] = "OFF".into();
    }
    config
}

/// One `(config topic, config)` per system sensor, in [`ids::ALL`] order.
pub fn configs(prefix: &str) -> Vec<(String, Value)> {
    ids::ALL
        .iter()
        .map(|id| (config_topic(prefix, id), config(prefix, id)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_of(id: &str) -> Value {
        configs("nidoo")
            .into_iter()
            .find(|(_, c)| c["unique_id"] == id)
            .map(|(_, c)| c)
            .unwrap_or_else(|| panic!("no config for {id}"))
    }

    #[test]
    fn configs_cover_every_id() {
        let all = configs("nidoo");
        assert_eq!(all.len(), 16);
        for ((topic, config), id) in all.iter().zip(ids::ALL) {
            assert_eq!(config["unique_id"], id);
            assert_eq!(config["entity_category"], "diagnostic", "{id}");
            assert_eq!(config["device"]["identifiers"][0], "nidoo_hub", "{id}");
            assert_eq!(config["device"]["name"], "Nidoo hub", "{id}");
            assert_eq!(config["state_topic"], format!("nidoo/system/{id}"));
            assert_eq!(
                config["availability_topic"],
                format!("nidoo/system/{id}/avail")
            );
            assert_eq!(config["payload_available"], "online");
            assert_eq!(config["payload_not_available"], "offline");
            assert_eq!(topic, &config_topic("nidoo", id));
        }
        let disk = config_of(ids::DISK_USED);
        assert_eq!(disk["unit_of_measurement"], "GB");
        assert_eq!(disk["device_class"], "data_size");
        let broker = config_of(ids::BROKER);
        assert_eq!(broker["device_class"], "connectivity");
        assert_eq!(broker["payload_on"], "ON");
        assert_eq!(broker["payload_off"], "OFF");
    }

    #[test]
    fn units_and_classes() {
        for id in [ids::CPU, ids::CPU_HUB] {
            assert_eq!(config_of(id)["unit_of_measurement"], "%");
        }
        for id in [ids::RAM_USED, ids::RAM_HUB] {
            assert_eq!(config_of(id)["unit_of_measurement"], "MB");
            assert_eq!(config_of(id)["device_class"], "data_size");
        }
        let db = config_of(ids::DB_SIZE);
        assert_eq!(db["unit_of_measurement"], "GB");
        assert_eq!(db["suggested_display_precision"], 2);
        let temperature = config_of(ids::TEMPERATURE);
        assert_eq!(temperature["unit_of_measurement"], "°C");
        assert_eq!(temperature["device_class"], "temperature");
        for id in [ids::DEVICES, ids::ENTITIES, ids::RETAINED, ids::COMMANDS] {
            assert!(config_of(id).get("unit_of_measurement").is_none(), "{id}");
        }
        assert_eq!(config_of(ids::COMMANDS)["state_class"], "total_increasing");
        assert_eq!(config_of(ids::STARTED)["device_class"], "timestamp");
        let last_backup = config_of(ids::LAST_BACKUP);
        assert_eq!(last_backup["name"], "Last backup");
        assert_eq!(last_backup["device_class"], "timestamp");
        assert!(last_backup.get("state_class").is_none());
    }

    #[test]
    fn services_are_binary_sensors() {
        for id in [ids::BROKER, ids::ZIGBEE2MQTT, ids::DATABASE] {
            assert_eq!(
                config_topic("nidoo", id),
                format!("nidoo/binary_sensor/nidoo_hub/{id}/config")
            );
        }
        assert_eq!(
            config_topic("nidoo", ids::CPU),
            "nidoo/sensor/nidoo_hub/system_cpu/config"
        );
        assert_eq!(state_topic("nidoo", ids::CPU), "nidoo/system/system_cpu");
        assert_eq!(
            availability_topic("nidoo", ids::CPU),
            "nidoo/system/system_cpu/avail"
        );
    }
}
