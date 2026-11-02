//! The retained discovery config of a network watch: a connectivity binary sensor.

use nidoo_api::NetworkWatchSpec;
use serde_json::{Value, json};

use super::topics::{availability_topic, state_topic, unique_id};

/// The discovery config for watch `id`; `device_block` is the `device` the entity attaches to.
pub fn build_config(prefix: &str, id: &str, spec: &NetworkWatchSpec, device_block: Value) -> Value {
    json!({
        "name": spec.name,
        "unique_id": unique_id(id),
        "state_topic": state_topic(prefix, id),
        "availability_topic": availability_topic(prefix, id),
        "payload_on": "ON",
        "payload_off": "OFF",
        "payload_available": "online",
        "payload_not_available": "offline",
        "device_class": "connectivity",
        "device": device_block,
    })
}

/// The device block of a watch that is not attached to a device.
pub fn standalone_device(id: &str, name: &str) -> Value {
    json!({ "identifiers": [format!("nidoo_network_{id}")], "name": name })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_is_a_connectivity_binary_sensor() {
        let spec = NetworkWatchSpec {
            name: "NAS".into(),
            host: "nas.local".into(),
            interval_s: 30,
            enabled: true,
            area: None,
            device: None,
        };
        let config = build_config("nidoo", "nas", &spec, standalone_device("nas", "NAS"));
        assert_eq!(config["name"], "NAS");
        assert_eq!(config["unique_id"], "network_nas");
        assert_eq!(config["state_topic"], "nidoo/network/nas");
        assert_eq!(config["availability_topic"], "nidoo/network/nas/avail");
        assert_eq!(config["payload_on"], "ON");
        assert_eq!(config["payload_off"], "OFF");
        assert_eq!(config["payload_available"], "online");
        assert_eq!(config["payload_not_available"], "offline");
        assert_eq!(config["device_class"], "connectivity");
        assert_eq!(config["device"]["identifiers"][0], "nidoo_network_nas");
    }
}
