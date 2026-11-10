//! WebSocket events and REST bodies.

use nidoo_core::model::EntityState;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::alerts::AlertView;
use crate::ambiances::{AmbianceView, ApplyReport};
use crate::flows::{FlowView, RunView};
use crate::home::HomeSettings;
use crate::snapshot::Snapshot;
use crate::views::{AreaView, BridgeView, DeviceView, EntityView};

/// The hub's link to the MQTT broker.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConnectionStatus {
    #[default]
    Connecting,
    Online,
    Offline {
        reason: String,
    },
}

/// One message pushed to a WebSocket client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(
    clippy::large_enum_variant,
    reason = "the variants are the wire shape; an event is built and serialized once"
)]
pub enum WsEvent {
    Snapshot(Snapshot),
    EntityUpserted(EntityView),
    EntityRemoved {
        id: String,
    },
    StateChanged {
        id: String,
        state: EntityState,
    },
    AreasChanged {
        areas: Vec<AreaView>,
    },
    DevicesChanged {
        devices: Vec<DeviceView>,
    },
    AlertsChanged {
        alerts: Vec<AlertView>,
    },
    FlowsChanged {
        flows: Vec<FlowView>,
    },
    FlowRunChanged {
        flow_id: String,
        run: RunView,
    },
    BridgeChanged(BridgeView),
    ConnectionChanged(ConnectionStatus),
    CommandFailed {
        id: String,
        reason: String,
    },
    /// A virtual or discovered button was pressed (a non-retained message on its command
    /// topic). An event, not a state: nothing is stored.
    ButtonPressed {
        id: String,
    },
    /// A remote, event entity or tag scanner sent a signal (a non-retained message). An event,
    /// not a state: a repeated signal is a new one.
    SignalReceived {
        id: String,
        signal: String,
    },
    /// A climate document changed: one zone's, or the house settings when `zone` is `None`.
    ClimateChanged {
        zone: Option<String>,
    },
    /// The alarm's settings, state or warnings changed: refetch `GET /api/v1/alarm`.
    AlarmChanged,
    /// The outdoor weather was refreshed: refetch `GET /api/v1/weather`.
    WeatherChanged,
    /// The solar forecast was refreshed: refetch `GET /api/v1/solar`.
    SolarChanged,
    /// The home settings document changed.
    HomeSettingsChanged {
        settings: HomeSettings,
    },
    /// The flow label catalog changed. `counts` is as of this change; clients count a
    /// flow's labels from the flow list as it changes.
    FlowLabelsChanged {
        labels: crate::FlowLabelsView,
    },
    /// The ambiances list changed.
    AmbiancesChanged {
        ambiances: Vec<AmbianceView>,
    },
    /// An ambiance was applied; the report says what each action did.
    AmbianceApplied {
        report: ApplyReport,
    },
    /// The assistant answered a turn, whoever spoke (a page, a satellite, a flow): refetch
    /// `GET /api/v1/assist/turns`.
    AssistTurnAdded {
        id: String,
    },
}

/// A partial override: the keys to set (`null` clears one).
pub type OverridePatch = serde_json::Map<String, serde_json::Value>;

/// The reply to a command request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CommandResponse {
    pub accepted: bool,
    pub reason: Option<String>,
    pub state: Option<EntityState>,
}

/// The body of an error reply.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ApiError {
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_list_event_serializes() {
        let cases = [
            (
                WsEvent::AreasChanged { areas: vec![] },
                json!({"type": "areas_changed", "areas": []}),
            ),
            (
                WsEvent::DevicesChanged { devices: vec![] },
                json!({"type": "devices_changed", "devices": []}),
            ),
            (
                WsEvent::AlertsChanged { alerts: vec![] },
                json!({"type": "alerts_changed", "alerts": []}),
            ),
            (
                WsEvent::FlowsChanged { flows: vec![] },
                json!({"type": "flows_changed", "flows": []}),
            ),
            (
                WsEvent::AssistTurnAdded { id: "t1".into() },
                json!({"type": "assist_turn_added", "id": "t1"}),
            ),
        ];
        for (event, expected) in cases {
            let value = serde_json::to_value(&event).expect("serializes");
            assert_eq!(value, expected);
            let back: WsEvent = serde_json::from_value(value).expect("round-trips");
            assert_eq!(back, event);
        }
    }

    #[test]
    fn new_events_serialize() {
        assert_eq!(
            serde_json::to_value(WsEvent::WeatherChanged).unwrap(),
            json!({"type": "weather_changed"})
        );
    }

    #[test]
    fn solar_changed_serializes() {
        assert_eq!(
            serde_json::to_value(WsEvent::SolarChanged).unwrap(),
            json!({"type": "solar_changed"})
        );
    }

    #[test]
    fn home_settings_changed_serializes() {
        let e = WsEvent::HomeSettingsChanged {
            settings: HomeSettings::default(),
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], "home_settings_changed");
        assert_eq!(serde_json::from_value::<WsEvent>(v).unwrap(), e);
    }
}
