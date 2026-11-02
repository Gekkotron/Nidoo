//! Topic layout of a network watch, for the first discovery prefix `P`: the definition is
//! `P/network/<id>/definition`, the state `P/network/<id>`, the availability
//! `P/network/<id>/avail` and the discovery config `P/binary_sensor/network/<id>/config`.

use nidoo_api::NetworkWatchSpec;

use crate::virtual_entities;

const MAX_ID_LEN: usize = 48;
const MIN_INTERVAL_S: u32 = 10;
const MAX_INTERVAL_S: u32 = 600;

/// The retained definition topic.
pub fn definition_topic(prefix: &str, id: &str) -> String {
    format!("{prefix}/network/{id}/definition")
}

/// The retained discovery config topic.
pub fn config_topic(prefix: &str, id: &str) -> String {
    format!("{prefix}/binary_sensor/network/{id}/config")
}

/// The retained state topic, `ON` or `OFF`.
pub fn state_topic(prefix: &str, id: &str) -> String {
    format!("{prefix}/network/{id}")
}

/// The retained availability topic.
pub fn availability_topic(prefix: &str, id: &str) -> String {
    format!("{prefix}/network/{id}/avail")
}

/// The entity's unique id.
pub fn unique_id(id: &str) -> String {
    format!("network_{id}")
}

/// A watch id derived from a device id: lowercase, every character outside `[a-z0-9_]`
/// becomes `_`. `None` when the result is empty or too long.
pub fn id_for_device(device_id: &str) -> Option<String> {
    let id: String = device_id
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    (!id.is_empty() && id.len() <= MAX_ID_LEN && virtual_entities::validate_id(&id).is_ok())
        .then_some(id)
}

/// Checks a watch: a name, a host without whitespace, and an interval of 10 to 600 seconds.
pub fn validate_spec(spec: &NetworkWatchSpec) -> Result<(), String> {
    if spec.name.trim().is_empty() {
        return Err("the watch needs a name".to_owned());
    }
    let host = spec.host.trim();
    if host.is_empty() || host.chars().any(char::is_whitespace) {
        return Err("the host must be a name or an address without spaces".to_owned());
    }
    if !(MIN_INTERVAL_S..=MAX_INTERVAL_S).contains(&spec.interval_s) {
        return Err(format!(
            "the interval must be {MIN_INTERVAL_S} to {MAX_INTERVAL_S} seconds"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(host: &str, interval_s: u32) -> NetworkWatchSpec {
        NetworkWatchSpec {
            name: "NAS".into(),
            host: host.into(),
            interval_s,
            enabled: true,
            area: None,
            device: None,
        }
    }

    #[test]
    fn topics_live_under_the_network_subtree() {
        assert_eq!(
            definition_topic("nidoo", "nas"),
            "nidoo/network/nas/definition"
        );
        assert_eq!(
            config_topic("nidoo", "nas"),
            "nidoo/binary_sensor/network/nas/config"
        );
        assert_eq!(state_topic("nidoo", "nas"), "nidoo/network/nas");
        assert_eq!(
            availability_topic("nidoo", "nas"),
            "nidoo/network/nas/avail"
        );
    }

    #[test]
    fn id_for_device_sanitizes() {
        assert_eq!(
            id_for_device("roborock_40313ca8b964").as_deref(),
            Some("roborock_40313ca8b964")
        );
        assert_eq!(id_for_device("Living-TV.1").as_deref(), Some("living_tv_1"));
        assert_eq!(id_for_device(&"a".repeat(49)), None);
        assert_eq!(id_for_device(""), None);
    }

    #[test]
    fn interval_bounds() {
        assert!(validate_spec(&spec("nas.local", 9)).is_err());
        assert!(validate_spec(&spec("nas.local", 601)).is_err());
        assert!(validate_spec(&spec("nas.local", 10)).is_ok());
        assert!(validate_spec(&spec("nas.local", 600)).is_ok());
    }

    #[test]
    fn blank_host_rejected() {
        assert!(validate_spec(&spec("  ", 30)).is_err());
        assert!(validate_spec(&spec("nas local", 30)).is_err());
    }
}
