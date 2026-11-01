//! The current state of an entity, as the app's `StateValue`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::command::Hs;

/// Whether the entity (or the bridge) is reachable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Online,
    Offline,
    #[default]
    Unknown,
}

/// A light's state; `brightness` is normalised to 0..=255.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct LightState {
    pub on: bool,
    pub brightness: Option<u8>,
    pub hs: Option<Hs>,
    pub color_temp: Option<u32>,
    pub effect: Option<String>,
}

/// A cover's state; `position` is normalised to 0 (closed) ..= 100 (open).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CoverState {
    pub state: Option<String>,
    pub position: Option<u8>,
}

/// A climate's state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ClimateState {
    pub current: Option<f64>,
    pub target: Option<f64>,
    pub mode: Option<String>,
    pub fan_mode: Option<String>,
    pub action: Option<String>,
}

/// A vacuum's state; `battery` is 0..=100.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct VacuumState {
    pub state: Option<String>,
    pub battery: Option<u8>,
    pub fan_speed: Option<String>,
}

/// A water heater's state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct WaterHeaterState {
    pub mode: Option<String>,
    pub current: Option<f64>,
    pub target: Option<f64>,
}

/// One named value of a generic entity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum GenericValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

/// A generic entity's state: the fields of its type that have been reported so far.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct GenericState {
    pub fields: BTreeMap<String, GenericValue>,
}

/// The typed value of an entity, by kind.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum StateValue {
    OnOff(bool),
    Number(f64),
    Text(String),
    Light(LightState),
    Cover(CoverState),
    Climate(ClimateState),
    Alarm(String),
    Vacuum(VacuumState),
    WaterHeater(WaterHeaterState),
    Generic(GenericState),
    /// Nothing received yet, or the payload could not be read.
    #[default]
    Unknown,
}

/// Everything known about an entity's current state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct EntityState {
    pub value: StateValue,
    /// Merged from `json_attributes_topic`.
    pub attributes: Map<String, Value>,
    pub availability: Availability,
    /// Unix time in seconds of the last state message, when any.
    pub last_update: Option<u64>,
}

#[cfg(test)]
mod generic_tests {
    use super::*;

    #[test]
    fn generic_state_round_trips_mixed_fields() {
        let mut fields = BTreeMap::new();
        fields.insert("a".to_owned(), GenericValue::Bool(true));
        fields.insert("b".to_owned(), GenericValue::Number(2.5));
        fields.insert("c".to_owned(), GenericValue::Text("x".into()));
        let value = StateValue::Generic(GenericState { fields });
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(
            json,
            r#"{"type":"generic","value":{"fields":{"a":true,"b":2.5,"c":"x"}}}"#
        );
        assert_eq!(serde_json::from_str::<StateValue>(&json).unwrap(), value);
    }
}
