//! Entity kinds and the Home Assistant components they come from.

use serde::{Deserialize, Serialize};

/// The entity kinds Nidoo supports, one per Home Assistant MQTT component.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Light,
    Switch,
    Cover,
    Climate,
    Sensor,
    BinarySensor,
    AlarmPanel,
    Number,
    Select,
    Button,
    Vacuum,
    WaterHeater,
    MediaPlayer,
    /// Any other component, described by the type table; the component name is in `Entity.component`.
    Generic,
}

/// Home Assistant components that are recognised as discovery but never become entities.
pub const IGNORED_COMPONENTS: &[&str] = &["device"];

impl Kind {
    /// Every kind, in declaration order.
    pub const ALL: &[Kind] = &[
        Kind::Light,
        Kind::Switch,
        Kind::Cover,
        Kind::Climate,
        Kind::Sensor,
        Kind::BinarySensor,
        Kind::AlarmPanel,
        Kind::Number,
        Kind::Select,
        Kind::Button,
        Kind::Vacuum,
        Kind::WaterHeater,
        Kind::MediaPlayer,
        Kind::Generic,
    ];

    /// The kind for a Home Assistant component name (`alarm_control_panel` → [`Kind::AlarmPanel`]).
    pub fn from_component(component: &str) -> Option<Kind> {
        Kind::ALL
            .iter()
            .copied()
            .find(|kind| kind.component() == component)
    }

    /// The Home Assistant component name this kind is discovered under.
    pub fn component(self) -> &'static str {
        match self {
            Kind::Light => "light",
            Kind::Switch => "switch",
            Kind::Cover => "cover",
            Kind::Climate => "climate",
            Kind::Sensor => "sensor",
            Kind::BinarySensor => "binary_sensor",
            Kind::AlarmPanel => "alarm_control_panel",
            Kind::Number => "number",
            Kind::Select => "select",
            Kind::Button => "button",
            Kind::Vacuum => "vacuum",
            Kind::WaterHeater => "water_heater",
            Kind::MediaPlayer => "media_player",
            // A formal placeholder: the real component lives in `Entity.component`.
            Kind::Generic => "generic",
        }
    }
}
