//! Commands a client can send to an entity, as the app's `Command`.

use serde::{Deserialize, Serialize};

/// A light colour without brightness: `h` in degrees 0..<360, `s` in percent 0..=100.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Hs {
    pub h: f64,
    pub s: f64,
}

/// Alarm panel arming modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum AlarmMode {
    Home,
    Away,
    Night,
    Vacation,
    CustomBypass,
}

/// What a client asks an entity to do; the hub maps it to MQTT publishes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// `brightness` is 0..=255; `color` carries no brightness; `effect` is one of the light's effects.
    TurnOn {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        brightness: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<Hs>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effect: Option<String>,
    },
    TurnOff,
    Toggle,
    /// `position` is 0 (closed) ..= 100 (open).
    SetPosition {
        position: u8,
    },
    Open,
    Close,
    Stop,
    SetTemperature {
        temperature: f64,
    },
    SetHvacMode {
        mode: String,
    },
    SetFanMode {
        mode: String,
    },
    SetNumber {
        value: f64,
    },
    SelectOption {
        option: String,
    },
    Press,
    Arm {
        mode: AlarmMode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
    Disarm {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
    /// `action` is one of start, pause, stop, return_to_base, locate.
    VacuumAction {
        action: String,
    },
    SetFanSpeed {
        speed: String,
    },
    /// A control a generic entity announces (`capabilities.generic.controls`); `value` follows
    /// the control's shape: absent for a press, a bool, a number or a string.
    Control {
        control: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<serde_json::Value>,
    },
}
