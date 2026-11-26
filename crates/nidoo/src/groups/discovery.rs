//! The retained discovery config of a group (Appendix A.3). A capability is offered only
//! when every member has it, so a command never reaches a member that cannot take it.

use nidoo_api::{EntityView, GroupSpec};
use nidoo_core::model::{Capabilities, Kind};
use serde_json::{Map, Value, json};

use super::topics::{availability_topic, state_topic, unique_id};

const COLOUR_MODES: [&str; 5] = ["hs", "xy", "rgb", "rgbw", "rgbww"];

/// True when there is at least one member and each has the capability.
fn shared(members: &[EntityView], has: impl Fn(&Capabilities) -> bool) -> bool {
    !members.is_empty() && members.iter().all(|m| has(&m.capabilities))
}

fn largest(members: &[EntityView], get: impl Fn(&Capabilities) -> Option<f64>) -> Option<f64> {
    members
        .iter()
        .filter_map(|m| get(&m.capabilities))
        .reduce(f64::max)
}

fn smallest(members: &[EntityView], get: impl Fn(&Capabilities) -> Option<f64>) -> Option<f64> {
    members
        .iter()
        .filter_map(|m| get(&m.capabilities))
        .reduce(f64::min)
}

/// The entries of the first member's list that every member lists, in that order.
fn common(members: &[EntityView], list: impl Fn(&Capabilities) -> &Vec<String>) -> Vec<String> {
    let Some((first, rest)) = members.split_first() else {
        return Vec::new();
    };
    list(&first.capabilities)
        .iter()
        .filter(|item| rest.iter().all(|m| list(&m.capabilities).contains(item)))
        .cloned()
        .collect()
}

fn set(config: &mut Map<String, Value>, entries: &[(&str, Value)]) {
    for (key, value) in entries {
        config.insert((*key).to_owned(), value.clone());
    }
}

fn light(config: &mut Map<String, Value>, topic: &str, members: &[EntityView]) {
    set(
        config,
        &[
            ("state_value_template", json!("{{ value_json.state }}")),
            ("state_on", json!("ON")),
            ("state_off", json!("OFF")),
            ("payload_on", json!(r#"{"type":"turn_on"}"#)),
            ("payload_off", json!(r#"{"type":"turn_off"}"#)),
        ],
    );
    if shared(members, |c| c.brightness) {
        set(
            config,
            &[
                ("brightness", json!(true)),
                ("brightness_state_topic", json!(topic)),
                ("brightness_command_topic", json!(topic)),
                (
                    "brightness_value_template",
                    json!("{{ value_json.brightness }}"),
                ),
                ("brightness_scale", json!(255)),
                (
                    "brightness_command_template",
                    json!(r#"{"type":"turn_on","brightness":{{ value }}}"#),
                ),
            ],
        );
    }
    if shared(members, |c| {
        COLOUR_MODES.iter().any(|m| c.color_modes.contains(*m))
    }) {
        set(
            config,
            &[
                ("hs_state_topic", json!(topic)),
                ("hs_command_topic", json!(topic)),
                ("hs_value_template", json!("{{ value_json.hs }}")),
                (
                    "hs_command_template",
                    json!(r#"{"type":"turn_on","color":{"h":{{ hue }},"s":{{ sat }}}}"#),
                ),
            ],
        );
    }
}

fn switch(config: &mut Map<String, Value>) {
    set(
        config,
        &[
            ("value_template", json!("{{ value_json.state }}")),
            ("state_on", json!("ON")),
            ("state_off", json!("OFF")),
            ("payload_on", json!(r#"{"type":"turn_on"}"#)),
            ("payload_off", json!(r#"{"type":"turn_off"}"#)),
        ],
    );
}

fn cover(config: &mut Map<String, Value>, topic: &str, members: &[EntityView]) {
    set(
        config,
        &[
            ("payload_open", json!(r#"{"type":"open"}"#)),
            ("payload_close", json!(r#"{"type":"close"}"#)),
            ("payload_stop", json!(r#"{"type":"stop"}"#)),
            ("value_template", json!("{{ value_json.state }}")),
        ],
    );
    if shared(members, |c| c.position) {
        set(
            config,
            &[
                ("position_topic", json!(topic)),
                ("position_template", json!("{{ value_json.position }}")),
                ("set_position_topic", json!(topic)),
                (
                    "set_position_template",
                    json!(r#"{"type":"set_position","position":{{ position }}}"#),
                ),
            ],
        );
    }
}

/// Climate and water heater share their keys.
fn thermostat(config: &mut Map<String, Value>, topic: &str, members: &[EntityView]) {
    set(
        config,
        &[
            ("temperature_command_topic", json!(topic)),
            (
                "temperature_command_template",
                json!(r#"{"type":"set_temperature","temperature":{{ value }}}"#),
            ),
            ("temperature_state_topic", json!(topic)),
            (
                "temperature_state_template",
                json!("{{ value_json.target }}"),
            ),
            ("mode_command_topic", json!(topic)),
            (
                "mode_command_template",
                json!(r#"{"type":"set_hvac_mode","mode":"{{ value }}"}"#),
            ),
            ("mode_state_topic", json!(topic)),
            ("mode_state_template", json!("{{ value_json.mode }}")),
            // Lets the UI read the members' target `range` from the same payload.
            ("json_attributes_topic", json!(topic)),
            ("modes", json!(common(members, |c| &c.hvac_modes))),
        ],
    );
    if let Some(step) = largest(members, |c| c.temp_step) {
        config.insert("temp_step".to_owned(), json!(step));
    }
    if let Some(min) = largest(members, |c| c.min_temp) {
        config.insert("min_temp".to_owned(), json!(min));
    }
    if let Some(max) = smallest(members, |c| c.max_temp) {
        config.insert("max_temp".to_owned(), json!(max));
    }
}

fn number(config: &mut Map<String, Value>, topic: &str, members: &[EntityView]) {
    set(
        config,
        &[
            ("command_topic", json!(topic)),
            (
                "command_template",
                json!(r#"{"type":"set_number","value":{{ value }}}"#),
            ),
            ("value_template", json!("{{ value_json.value }}")),
        ],
    );
    if let Some(min) = largest(members, |c| c.min) {
        config.insert("min".to_owned(), json!(min));
    }
    if let Some(max) = smallest(members, |c| c.max) {
        config.insert("max".to_owned(), json!(max));
    }
    if let Some(step) = largest(members, |c| c.step) {
        config.insert("step".to_owned(), json!(step));
    }
    if let Some(unit) = members.first().and_then(|m| m.unit.as_deref())
        && members.iter().all(|m| m.unit.as_deref() == Some(unit))
    {
        config.insert("unit_of_measurement".to_owned(), json!(unit));
    }
}

fn select(config: &mut Map<String, Value>, topic: &str, members: &[EntityView]) {
    set(
        config,
        &[
            ("command_topic", json!(topic)),
            (
                "command_template",
                json!(r#"{"type":"select_option","option":"{{ value }}"}"#),
            ),
            ("value_template", json!("{{ value_json.option }}")),
            ("options", json!(common(members, |c| &c.options))),
        ],
    );
}

fn vacuum(config: &mut Map<String, Value>, topic: &str, members: &[EntityView]) {
    set(
        config,
        &[
            ("schema", json!("state")),
            ("command_topic", json!(topic)),
            ("state_value_template", json!("{{ value_json.state }}")),
        ],
    );
    for action in common(members, |c| &c.vacuum_actions) {
        config.insert(
            format!("payload_{action}"),
            json!(format!(r#"{{"type":"vacuum_action","action":"{action}"}}"#)),
        );
    }
}

/// The discovery config for group `id`; `members` are its present, matching entities.
/// Colour temperature is not offered: the command set has no way to set it.
pub fn build_config(prefix: &str, id: &str, spec: &GroupSpec, members: &[EntityView]) -> Value {
    let topic = state_topic(prefix, id);
    let mut config = Map::new();
    set(
        &mut config,
        &[
            ("name", json!(spec.name)),
            ("unique_id", json!(unique_id(id))),
            (
                "device",
                json!({ "identifiers": ["nidoo_group"], "name": "Groups" }),
            ),
            ("availability_topic", json!(availability_topic(prefix, id))),
            ("payload_available", json!("online")),
            ("payload_not_available", json!("offline")),
            ("command_topic", json!(topic)),
        ],
    );
    if spec.kind != Kind::Button {
        set(&mut config, &[("retain", json!(true))]);
    }
    if !matches!(spec.kind, Kind::Button | Kind::Number | Kind::Select) {
        set(&mut config, &[("state_topic", json!(topic))]);
    }
    match spec.kind {
        Kind::Light => light(&mut config, &topic, members),
        Kind::Switch => switch(&mut config),
        Kind::Cover => cover(&mut config, &topic, members),
        Kind::Climate | Kind::WaterHeater => {
            config.remove("command_topic");
            thermostat(&mut config, &topic, members);
        }
        Kind::Number => {
            config.insert("state_topic".to_owned(), json!(topic));
            number(&mut config, &topic, members);
        }
        Kind::Select => {
            config.insert("state_topic".to_owned(), json!(topic));
            select(&mut config, &topic, members);
        }
        Kind::Button => set(
            &mut config,
            &[("payload_press", json!(r#"{"type":"press"}"#))],
        ),
        Kind::Vacuum => vacuum(&mut config, &topic, members),
        Kind::Sensor
        | Kind::BinarySensor
        | Kind::AlarmPanel
        | Kind::MediaPlayer
        | Kind::Generic => {}
    }
    Value::Object(config)
}
#[cfg(test)]
mod tests;
