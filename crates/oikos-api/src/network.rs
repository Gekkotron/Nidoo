//! Network presence: a device watched by ping, its spec and its live status.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A watch's definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NetworkWatchSpec {
    /// The display name.
    pub name: String,
    /// The host or IP address to ping.
    pub host: String,
    /// Seconds between pings, 10 to 600.
    #[serde(rename = "intervalS", default = "default_interval")]
    pub interval_s: u32,
    /// Whether the watch pings; a paused watch keeps its definition.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// The area the entity belongs to.
    #[serde(default)]
    pub area: Option<String>,
    /// The device the entity attaches to.
    #[serde(default)]
    pub device: Option<String>,
}

/// The default of `intervalS`.
pub fn default_interval() -> u32 {
    30
}

/// The default of `enabled`.
pub fn default_enabled() -> bool {
    true
}

/// Where a watch stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WatchState {
    /// No verdict yet.
    Waiting,
    Connected,
    Disconnected,
    Paused,
    /// The hub cannot send pings.
    CannotPing,
}

/// A watch's live status; times are Unix seconds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WatchStatus {
    pub state: WatchState,
    /// When the connected or disconnected verdict last changed.
    pub since: Option<u64>,
    /// When the last ping was answered.
    pub last_reply: Option<u64>,
    /// The round trip of the last answer, in milliseconds.
    pub rtt_ms: Option<f64>,
}

/// A stored watch with its entity and status.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NetworkWatchView {
    pub id: String,
    pub spec: NetworkWatchSpec,
    /// The watch's entity id, `network_<id>`.
    pub entity: String,
    pub status: WatchStatus,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn spec_defaults_interval_and_enabled() {
        let spec: NetworkWatchSpec =
            serde_json::from_value(json!({"name": "NAS", "host": "nas.local"})).unwrap();
        assert_eq!(spec.interval_s, 30);
        assert!(spec.enabled);
        assert_eq!(spec.area, None);
        assert_eq!(spec.device, None);
    }

    #[test]
    fn spec_rejects_unknown_fields() {
        let spec =
            serde_json::from_value::<NetworkWatchSpec>(json!({"name": "a", "host": "b", "x": 1}));
        assert!(spec.is_err());
    }

    #[test]
    fn view_serializes_camel_case() {
        let view = NetworkWatchView {
            id: "nas".into(),
            spec: NetworkWatchSpec {
                name: "NAS".into(),
                host: "192.168.1.42".into(),
                interval_s: 30,
                enabled: true,
                area: None,
                device: None,
            },
            entity: "network_nas".into(),
            status: WatchStatus {
                state: WatchState::CannotPing,
                since: Some(1),
                last_reply: Some(2),
                rtt_ms: Some(4.0),
            },
        };
        let value = serde_json::to_value(&view).unwrap();
        assert_eq!(value["spec"]["intervalS"], 30);
        assert_eq!(value["status"]["lastReply"], 2);
        assert_eq!(value["status"]["rttMs"], 4.0);
        assert_eq!(value["status"]["state"], "cannot_ping");
    }
}
