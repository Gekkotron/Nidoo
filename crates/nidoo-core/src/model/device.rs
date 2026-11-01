//! Zigbee2MQTT devices and bridge status, from `zigbee2mqtt/bridge/{devices,info,state}`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::state::Availability;

/// One Zigbee device from `bridge/devices`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Device {
    /// `zigbee2mqtt_<ieee>`, the HA discovery `device.identifiers` value.
    pub id: String,
    pub ieee: String,
    pub friendly_name: String,
    /// `Router` or `EndDevice`.
    pub device_type: String,
    pub model: Option<String>,
    pub vendor: Option<String>,
    pub description: Option<String>,
    pub supported: bool,
    pub disabled: bool,
    pub interview_state: String,
    pub power_source: Option<String>,
    /// `definition.exposes`, kept as JSON.
    pub exposes: Value,
}

/// The `bridge/devices` entry as Zigbee2MQTT publishes it; unknown fields are ignored.
#[derive(Deserialize)]
struct RawDevice {
    ieee_address: String,
    friendly_name: String,
    #[serde(rename = "type")]
    device_type: String,
    #[serde(default)]
    supported: bool,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    interview_state: String,
    #[serde(default)]
    power_source: Option<String>,
    #[serde(default)]
    model_id: Option<String>,
    #[serde(default)]
    manufacturer: Option<String>,
    #[serde(default)]
    definition: Option<RawDefinition>,
}

#[derive(Default, Deserialize)]
struct RawDefinition {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    vendor: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    exposes: Value,
}

impl From<RawDevice> for Device {
    fn from(raw: RawDevice) -> Self {
        let definition = raw.definition.unwrap_or_default();
        Device {
            id: format!("zigbee2mqtt_{}", raw.ieee_address),
            ieee: raw.ieee_address,
            friendly_name: raw.friendly_name,
            device_type: raw.device_type,
            model: definition.model.or(raw.model_id),
            vendor: definition.vendor.or(raw.manufacturer),
            description: definition.description,
            supported: raw.supported,
            disabled: raw.disabled,
            interview_state: raw.interview_state,
            power_source: raw.power_source,
            exposes: definition.exposes,
        }
    }
}

impl Device {
    /// Parses the `bridge/devices` array, leaving out the coordinator.
    pub fn parse_list(payload: &[u8]) -> Result<Vec<Device>, serde_json::Error> {
        let raw: Vec<RawDevice> = serde_json::from_slice(payload)?;
        Ok(raw
            .into_iter()
            .filter(|device| device.device_type != "Coordinator")
            .map(Device::from)
            .collect())
    }
}

/// Zigbee2MQTT bridge status.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BridgeInfo {
    pub version: Option<String>,
    /// `coordinator.type`, e.g. `ZStack3x0`.
    pub coordinator: Option<String>,
    pub permit_join: bool,
    pub state: Availability,
}

impl BridgeInfo {
    /// Reads `version`, `coordinator.type` and `permit_join` from a `bridge/info` payload.
    /// An unreadable payload changes nothing.
    pub fn apply_info(&mut self, payload: &[u8]) {
        let Ok(info) = serde_json::from_slice::<Value>(payload) else {
            return;
        };
        if let Some(version) = info.get("version").and_then(Value::as_str) {
            self.version = Some(version.to_owned());
        }
        if let Some(coordinator) = info
            .get("coordinator")
            .and_then(|c| c.get("type"))
            .and_then(Value::as_str)
        {
            self.coordinator = Some(coordinator.to_owned());
        }
        if let Some(permit_join) = info.get("permit_join").and_then(Value::as_bool) {
            self.permit_join = permit_join;
        }
    }

    /// Reads a `bridge/state` payload: `{"state":"online"|"offline"}`, or the legacy bare
    /// `online`/`offline` string. Anything else reads as unknown.
    pub fn apply_state(&mut self, payload: &[u8]) {
        let state = match serde_json::from_slice::<Value>(payload) {
            Ok(value) => value
                .get("state")
                .and_then(Value::as_str)
                .or(value.as_str())
                .map(str::to_owned),
            Err(_) => std::str::from_utf8(payload)
                .ok()
                .map(|s| s.trim().to_owned()),
        };
        self.state = match state.as_deref() {
            Some("online") => Availability::Online,
            Some("offline") => Availability::Offline,
            _ => Availability::Unknown,
        };
    }
}
