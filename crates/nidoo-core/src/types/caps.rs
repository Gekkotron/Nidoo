//! What a generic entity actually announces, derived from its discovery config and its type.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{Shape, TypeDef};

/// A resolved numeric range.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Range {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

/// The controls, options and ranges one entity announces, stored in `Capabilities.generic`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct GenericCaps {
    /// Announced control names, in table order.
    pub controls: Vec<String>,
    /// Choice options per field, from the config list else the field's values.
    pub options: BTreeMap<String, Vec<String>>,
    /// Resolved range per number field.
    pub ranges: BTreeMap<String, Range>,
    /// Signal names (filled by the signal wave).
    pub signals: Vec<String>,
    /// Text mode is a password.
    pub secret: bool,
}

fn number(cfg: &serde_json::Value, key: Option<&String>) -> Option<f64> {
    key.and_then(|k| cfg.get(k.as_str()))
        .and_then(|v| v.as_f64())
}

impl GenericCaps {
    /// Derive the announced surface of an entity of type `def` from its discovery `cfg`.
    pub fn from_config(def: &TypeDef, cfg: &serde_json::Value) -> GenericCaps {
        let controls = def
            .controls
            .iter()
            .filter(|(_, ctl)| {
                cfg.get(ctl.topic.as_str()).is_some_and(|v| v.is_string())
                    && ctl
                        .requires
                        .as_ref()
                        .is_none_or(|k| cfg.get(k.as_str()).is_some())
            })
            .map(|(name, _)| name.clone())
            .collect();

        let mut options = BTreeMap::new();
        let mut ranges = BTreeMap::new();
        for (name, field) in &def.fields {
            match field.shape {
                Shape::Choice | Shape::Text => {
                    let listed = field
                        .options
                        .as_ref()
                        .and_then(|k| cfg.get(k.as_str()))
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(str::to_owned))
                                .collect::<Vec<_>>()
                        });
                    if let Some(list) = listed {
                        options.insert(name.clone(), list);
                    } else if field.shape == Shape::Choice && !field.values.is_empty() {
                        options.insert(
                            name.clone(),
                            field.values.iter().map(|w| w.value.clone()).collect(),
                        );
                    }
                }
                Shape::Number => {
                    if let Some(r) = &field.range {
                        ranges.insert(
                            name.clone(),
                            Range {
                                min: number(cfg, r.min_key.as_ref()).unwrap_or(r.min),
                                max: number(cfg, r.max_key.as_ref()).unwrap_or(r.max),
                                step: number(cfg, r.step_key.as_ref()).unwrap_or(r.step),
                            },
                        );
                    }
                }
                _ => {}
            }
        }

        GenericCaps {
            controls,
            options,
            ranges,
            signals: Vec::new(),
            secret: cfg.get("mode").and_then(|v| v.as_str()) == Some("password"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn def(src: serde_json::Value) -> TypeDef {
        serde_json::from_value(src).expect("test type def")
    }

    #[test]
    fn announced_controls_follow_config_topics() {
        let d = def(json!({
            "placement": "card", "icon": "x",
            "controls": {
                "power": {"shape": "on_off", "topic": "command_topic"},
                "speed": {"shape": "number", "topic": "percentage_command_topic"}
            }
        }));
        let caps = GenericCaps::from_config(&d, &json!({"command_topic": "a/set"}));
        assert_eq!(caps.controls, vec!["power".to_string()]);
    }

    #[test]
    fn requires_key_gates_a_control() {
        let d = def(json!({
            "placement": "card", "icon": "x",
            "controls": {
                "open": {"shape": "on_off", "topic": "command_topic", "requires": "payload_open"}
            }
        }));
        let without = GenericCaps::from_config(&d, &json!({"command_topic": "a/set"}));
        assert!(without.controls.is_empty());
        let with = GenericCaps::from_config(
            &d,
            &json!({"command_topic": "a/set", "payload_open": "OPEN"}),
        );
        assert_eq!(with.controls, vec!["open".to_string()]);
    }

    #[test]
    fn options_come_from_config_list_then_values() {
        let d = def(json!({
            "placement": "card", "icon": "x",
            "fields": {
                "preset": {"shape": "choice", "topic": "state_topic", "options": "preset_modes"},
                "state": {"shape": "choice", "topic": "state_topic", "values": [
                    {"value": "locked", "default": "LOCKED"},
                    {"value": "unlocked", "default": "UNLOCKED"}
                ]}
            }
        }));
        let caps = GenericCaps::from_config(&d, &json!({"preset_modes": ["eco", "boost"]}));
        assert_eq!(caps.options["preset"], vec!["eco", "boost"]);
        assert_eq!(caps.options["state"], vec!["locked", "unlocked"]);
    }

    #[test]
    fn password_mode_is_secret() {
        let d = def(json!({"placement": "card", "icon": "x"}));
        assert!(GenericCaps::from_config(&d, &json!({"mode": "password"})).secret);
        assert!(!GenericCaps::from_config(&d, &json!({"mode": "text"})).secret);
    }

    #[test]
    fn ranges_resolve_config_over_defaults() {
        let d = def(json!({
            "placement": "card", "icon": "x",
            "fields": {"level": {"shape": "number", "topic": "state_topic", "range": {
                "min_key": "min", "max_key": "max", "min": 0.0, "max": 100.0, "step": 1.0
            }}}
        }));
        let caps = GenericCaps::from_config(&d, &json!({"min": 5, "max": 50.5}));
        assert_eq!(
            caps.ranges["level"],
            Range {
                min: 5.0,
                max: 50.5,
                step: 1.0
            }
        );
    }
}
