//! What an entity can do, read from its expanded discovery config.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::model::{AlarmMode, Capabilities, CodeFormat, Kind};

/// Home Assistant's default climate modes when `modes` is not published.
const DEFAULT_CLIMATE_MODES: &[&str] = &["auto", "off", "cool", "heat", "dry", "fan_only"];
/// Home Assistant's default climate fan modes when `fan_modes` is not published.
const DEFAULT_FAN_MODES: &[&str] = &["auto", "low", "medium", "high"];
/// Home Assistant's default water heater modes when `modes` is not published.
const DEFAULT_WATER_HEATER_MODES: &[&str] = &[
    "off",
    "eco",
    "electric",
    "gas",
    "heat_pump",
    "high_demand",
    "performance",
];
/// Home Assistant's default vacuum `supported_features`.
const DEFAULT_VACUUM_FEATURES: &[&str] = &[
    "start",
    "stop",
    "return_home",
    "status",
    "battery",
    "clean_spot",
];
/// Vacuum feature → action, in the order the app shows them.
const VACUUM_ACTIONS: &[(&str, &str)] = &[
    ("start", "start"),
    ("pause", "pause"),
    ("stop", "stop"),
    ("return_home", "return_to_base"),
    ("locate", "locate"),
];
/// Alarm `supported_features` entry → mode.
const ALARM_FEATURES: &[(&str, AlarmMode)] = &[
    ("arm_home", AlarmMode::Home),
    ("arm_away", AlarmMode::Away),
    ("arm_night", AlarmMode::Night),
    ("arm_vacation", AlarmMode::Vacation),
    ("arm_custom_bypass", AlarmMode::CustomBypass),
];

/// Reads the capabilities of a `kind` entity from its expanded config.
pub fn from_config(kind: Kind, cfg: &Value) -> Capabilities {
    let mut caps = by_kind(kind, cfg);
    caps.z2m_group = cfg
        .get("group")
        .and_then(Value::as_array)
        .is_some_and(|members| !members.is_empty());
    caps
}

fn by_kind(kind: Kind, cfg: &Value) -> Capabilities {
    match kind {
        Kind::Light => light(cfg),
        Kind::Cover => Capabilities {
            position: cfg.get("set_position_topic").is_some_and(Value::is_string),
            open_close: has_str(cfg, "command_topic"),
            // Home Assistant turns Stop off when `payload_stop` is published as null.
            stop: has_str(cfg, "command_topic")
                && !cfg.get("payload_stop").is_some_and(Value::is_null),
            ..Capabilities::default()
        },
        Kind::Climate => Capabilities {
            hvac_modes: str_list(cfg, "modes").unwrap_or_else(|| owned(DEFAULT_CLIMATE_MODES)),
            fan_modes: if has_str(cfg, "fan_mode_command_topic") {
                str_list(cfg, "fan_modes").unwrap_or_else(|| owned(DEFAULT_FAN_MODES))
            } else {
                Vec::new()
            },
            min_temp: number(cfg, "min_temp"),
            max_temp: number(cfg, "max_temp"),
            temp_step: number(cfg, "temp_step"),
            ..Capabilities::default()
        },
        Kind::WaterHeater => Capabilities {
            hvac_modes: str_list(cfg, "modes").unwrap_or_else(|| owned(DEFAULT_WATER_HEATER_MODES)),
            min_temp: number(cfg, "min_temp"),
            max_temp: number(cfg, "max_temp"),
            temp_step: number(cfg, "precision"),
            ..Capabilities::default()
        },
        // The generic caps are set by `parse` from the type table.
        Kind::Generic => Capabilities::default(),
        Kind::Number => Capabilities {
            min: number(cfg, "min"),
            max: number(cfg, "max"),
            step: number(cfg, "step"),
            ..Capabilities::default()
        },
        Kind::Select => Capabilities {
            options: str_list(cfg, "options").unwrap_or_default(),
            ..Capabilities::default()
        },
        Kind::Vacuum => vacuum(cfg),
        Kind::AlarmPanel => alarm(cfg),
        Kind::Switch | Kind::Sensor | Kind::BinarySensor | Kind::Button => Capabilities::default(),
    }
}

fn light(cfg: &Value) -> Capabilities {
    let json_schema = cfg.get("schema").and_then(Value::as_str) == Some("json");
    if json_schema {
        Capabilities {
            brightness: boolean(cfg, "brightness", false),
            color_modes: str_list(cfg, "supported_color_modes")
                .unwrap_or_default()
                .into_iter()
                .collect(),
            effects: if boolean(cfg, "effect", false) {
                str_list(cfg, "effect_list").unwrap_or_default()
            } else {
                Vec::new()
            },
            ..Capabilities::default()
        }
    } else {
        Capabilities {
            brightness: has_str(cfg, "brightness_command_topic"),
            color_modes: if has_str(cfg, "rgb_command_topic") {
                BTreeSet::from(["rgb".to_owned()])
            } else {
                BTreeSet::new()
            },
            effects: if has_str(cfg, "effect_command_topic") {
                str_list(cfg, "effect_list").unwrap_or_default()
            } else {
                Vec::new()
            },
            ..Capabilities::default()
        }
    }
}

fn vacuum(cfg: &Value) -> Capabilities {
    let features =
        str_list(cfg, "supported_features").unwrap_or_else(|| owned(DEFAULT_VACUUM_FEATURES));
    let has = |feature: &str| features.iter().any(|f| f == feature);
    Capabilities {
        vacuum_actions: VACUUM_ACTIONS
            .iter()
            .filter(|(feature, _)| has(feature))
            .map(|(_, action)| (*action).to_owned())
            .collect(),
        fan_speeds: if has("fan_speed") && has_str(cfg, "set_fan_speed_topic") {
            str_list(cfg, "fan_speed_list").unwrap_or_default()
        } else {
            Vec::new()
        },
        ..Capabilities::default()
    }
}

fn alarm(cfg: &Value) -> Capabilities {
    let alarm_modes = match str_list(cfg, "supported_features") {
        Some(features) => ALARM_FEATURES
            .iter()
            .filter(|(feature, _)| features.iter().any(|f| f == feature))
            .map(|(_, mode)| *mode)
            .collect(),
        None => ALARM_FEATURES.iter().map(|(_, mode)| *mode).collect(),
    };
    let code = cfg.get("code").and_then(Value::as_str);
    Capabilities {
        alarm_modes,
        code_arm_required: code.is_some() && boolean(cfg, "code_arm_required", true),
        code_disarm_required: code.is_some() && boolean(cfg, "code_disarm_required", true),
        code_format: code_format(code),
        ..Capabilities::default()
    }
}

/// `REMOTE_CODE_TEXT` or a literal code with a non-digit gets a text field; anything else the keypad.
fn code_format(code: Option<&str>) -> CodeFormat {
    match code {
        None | Some("REMOTE_CODE") => CodeFormat::Number,
        Some("REMOTE_CODE_TEXT") => CodeFormat::Text,
        Some(literal) if literal.chars().all(|c| c.is_ascii_digit()) => CodeFormat::Number,
        Some(_) => CodeFormat::Text,
    }
}

fn owned(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn has_str(cfg: &Value, key: &str) -> bool {
    cfg.get(key).is_some_and(Value::is_string)
}

fn boolean(cfg: &Value, key: &str, default: bool) -> bool {
    cfg.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn number(cfg: &Value, key: &str) -> Option<f64> {
    cfg.get(key).and_then(Value::as_f64)
}

/// The string entries of an array value; `None` when the key is absent or not an array.
fn str_list(cfg: &Value, key: &str) -> Option<Vec<String>> {
    let items = cfg.get(key)?.as_array()?;
    Some(
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn cover_stop_follows_payload_stop() {
        let with_null = json!({"command_topic": "c/set", "payload_stop": null});
        let caps = from_config(Kind::Cover, &with_null);
        assert!(caps.open_close);
        assert!(!caps.stop);
        let plain = from_config(Kind::Cover, &json!({"command_topic": "c/set"}));
        assert!(plain.open_close);
        assert!(plain.stop);
        let read_only = from_config(Kind::Cover, &json!({"state_topic": "c"}));
        assert!(!read_only.open_close);
        assert!(!read_only.stop);
    }
}
