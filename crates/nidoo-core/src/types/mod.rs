//! The generic entity type table: which Home Assistant components become which kind of tile,
//! described as data (fields, controls, alerts, history) in an embedded JSON file.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::model::{GenericState, GenericValue};

pub mod caps;
pub mod command;
pub mod signals;
pub mod state;
pub use caps::{GenericCaps, Range};

/// The table, embedded verbatim; the hub also publishes it as-is.
const TABLE_JSON: &str = include_str!("table.json");

/// The whole table: component name (`lock`, `fan`, ...) to its definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TypeTable {
    pub version: u32,
    pub types: BTreeMap<String, TypeDef>,
}

/// One component type: where it shows, what it reads and what it can do.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TypeDef {
    pub placement: Placement,
    /// Glyph name known to the web icon set.
    pub icon: String,
    /// Field shown as the card value.
    pub primary: Option<String>,
    #[serde(default)]
    pub fields: BTreeMap<String, FieldDef>,
    #[serde(default)]
    pub controls: BTreeMap<String, ControlDef>,
    /// Default main control, in preference order.
    #[serde(default)]
    pub main: Vec<String>,
    #[serde(default)]
    pub alerts: Vec<AlertDef>,
    pub history: Option<HistoryDef>,
    /// Reject configs without the primary field's topic.
    #[serde(default)]
    pub requires_state: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    Card,
    Signal,
    Target,
    Update,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    OnOff,
    Number,
    Choice,
    Text,
    Picture,
    Version,
}

/// A value the entity reports.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct FieldDef {
    pub shape: Shape,
    /// Config key of the topic, e.g. `state_topic`.
    pub topic: String,
    /// Config key of the template, e.g. `value_template`.
    pub template: Option<String>,
    /// Payload word to canonical value.
    #[serde(default)]
    pub values: Vec<WordDef>,
    /// Config key of a list, e.g. `preset_modes`.
    pub options: Option<String>,
    pub range: Option<RangeDef>,
    /// OnOff: canonical value meaning "on" (e.g. `locked`).
    pub on: Option<String>,
    /// For a text/version shape: when the rendered template produces a JSON object, read
    /// the value at this top-level key instead of using the whole blob as the field value.
    /// Lets a Z2M `update` entity expose `installed_version` and `latest_version` from one
    /// `value_template`.
    #[serde(default)]
    pub json_path: Option<String>,
}

/// A payload word: its canonical value, the config key that may override it, and the default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct WordDef {
    pub value: String,
    pub key: Option<String>,
    pub default: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RangeDef {
    pub min_key: Option<String>,
    pub max_key: Option<String>,
    pub step_key: Option<String>,
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

/// Something the user can do to the entity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ControlDef {
    pub shape: Shape,
    /// Config key, e.g. `command_topic`.
    pub topic: String,
    /// Config key, e.g. `command_template`.
    pub template: Option<String>,
    /// Field this control writes (echo and current value).
    pub field: Option<String>,
    /// A momentary action with no value.
    #[serde(default)]
    pub press: bool,
    /// Press payload.
    pub payload: Option<WordDef>,
    pub payload_on: Option<WordDef>,
    pub payload_off: Option<WordDef>,
    /// Send `{json_key: value}` instead of the bare value.
    pub json_key: Option<String>,
    /// Extra config key that must be present to announce the control.
    pub requires: Option<String>,
    /// Card button words for OnOff.
    pub labels: Option<ActionLabels>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ActionLabels {
    pub on: String,
    pub off: String,
}

/// Raise an alert when `field` takes one of these values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AlertDef {
    pub field: String,
    pub any_of: Vec<String>,
}

/// Which field is charted; an empty map means the number as-is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct HistoryDef {
    pub field: String,
    #[serde(default)]
    pub map: BTreeMap<String, f64>,
}

static TABLE: LazyLock<TypeTable> = LazyLock::new(|| {
    serde_json::from_str(TABLE_JSON).expect("types/table.json is a valid type table")
});

/// The parsed type table.
pub fn table() -> &'static TypeTable {
    &TABLE
}

/// The definition for a Home Assistant component, if the table has one.
pub fn type_def(component: &str) -> Option<&'static TypeDef> {
    TABLE.types.get(component)
}

/// The raw embedded JSON.
pub fn table_json() -> &'static str {
    TABLE_JSON
}

/// The control a card leads with: the user's choice if announced (`"none"` means no main
/// control), else the first announced entry of the type's `main` list.
pub fn main_control<'a>(
    def: &'a TypeDef,
    caps: &'a GenericCaps,
    chosen: Option<&'a str>,
) -> Option<&'a str> {
    let announced = |name: &str| {
        caps.controls
            .iter()
            .find(|c| *c == name)
            .map(String::as_str)
    };
    if chosen == Some("none") {
        return None;
    }
    if let Some(found) = chosen.and_then(announced) {
        return Some(found);
    }
    def.main.iter().find_map(|m| announced(m))
}

/// True when an alert of the type fires for the state.
pub fn alerting(def: &TypeDef, state: &GenericState) -> bool {
    def.alerts
        .iter()
        .any(|alert| match state.fields.get(&alert.field) {
            Some(GenericValue::Text(s)) => alert.any_of.contains(s),
            Some(GenericValue::Bool(b)) => {
                let word = if *b { "on" } else { "off" };
                alert.any_of.iter().any(|a| a == word)
            }
            _ => false,
        })
}

/// The number charted for the state, following the type's history definition.
pub fn history_num(def: &TypeDef, state: &GenericState) -> Option<f64> {
    let h = def.history.as_ref()?;
    let n = match state.fields.get(&h.field)? {
        GenericValue::Number(n) if h.map.is_empty() => *n,
        GenericValue::Number(_) => return None,
        GenericValue::Bool(b) => *h.map.get(if *b { "on" } else { "off" })?,
        GenericValue::Text(s) => *h.map.get(s)?,
    };
    n.is_finite().then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_table_parses_and_is_version_one() {
        assert_eq!(table().version, 1);
    }

    #[test]
    fn every_reference_in_the_table_resolves() {
        for (name, def) in &table().types {
            if let Some(p) = &def.primary {
                assert!(def.fields.contains_key(p), "{name}: primary {p}");
            }
            for m in &def.main {
                assert!(def.controls.contains_key(m), "{name}: main {m}");
            }
            for a in &def.alerts {
                assert!(
                    def.fields.contains_key(&a.field),
                    "{name}: alert {}",
                    a.field
                );
            }
            if let Some(h) = &def.history {
                assert!(def.fields.contains_key(&h.field), "{name}: history");
            }
            for (c, ctl) in &def.controls {
                if let Some(f) = &ctl.field {
                    assert!(def.fields.contains_key(f), "{name}.{c}: field {f}");
                }
                if ctl.press {
                    assert!(ctl.payload.is_some(), "{name}.{c}: press needs payload");
                }
                if ctl.shape == Shape::OnOff && !ctl.press {
                    assert!(
                        ctl.payload_on.is_some() && ctl.payload_off.is_some(),
                        "{name}.{c}"
                    );
                }
            }
        }
    }

    fn main_def() -> TypeDef {
        serde_json::from_value(serde_json::json!({
            "placement": "card", "icon": "x", "main": ["power", "speed"]
        }))
        .unwrap()
    }

    fn caps_with(controls: &[&str]) -> GenericCaps {
        GenericCaps {
            controls: controls.iter().map(|s| s.to_string()).collect(),
            ..GenericCaps::default()
        }
    }

    #[test]
    fn main_control_prefers_override_then_table_order() {
        let d = main_def();
        let caps = caps_with(&["power", "speed"]);
        assert_eq!(main_control(&d, &caps, Some("speed")), Some("speed"));
        assert_eq!(main_control(&d, &caps, None), Some("power"));
        assert_eq!(main_control(&d, &caps, Some("none")), None);
    }

    #[test]
    fn main_control_falls_back_when_not_announced() {
        let d = main_def();
        let caps = caps_with(&["power"]);
        assert_eq!(main_control(&d, &caps, Some("speed")), Some("power"));
    }

    #[test]
    fn unknown_component_has_no_type() {
        assert!(type_def("light").is_none());
        assert!(type_def("no_such_component").is_none());
    }
}
