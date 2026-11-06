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
mod tests {
    use super::*;
    use nidoo_core::model::{Capabilities, EntityCategory, EntityState, UiOverride};

    fn member(id: &str, kind: Kind, caps: Capabilities) -> EntityView {
        EntityView {
            id: id.to_owned(),
            device_id: None,
            name: id.to_owned(),
            device_name: None,
            label: None,
            kind,
            component: kind.component().to_string(),
            protocol_kind: kind,
            device_class: None,
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: String::new(),
            part_of: None,
            capabilities: caps,
            ui: UiOverride::default(),
            state: EntityState::default(),
        }
    }

    fn spec(kind: Kind) -> GroupSpec {
        GroupSpec {
            kind,
            name: "Living".to_owned(),
            members: vec![],
            hide_members: true,
        }
    }

    fn light(brightness: bool, colour: bool) -> EntityView {
        let mut caps = Capabilities {
            brightness,
            ..Default::default()
        };
        if colour {
            caps.color_modes.insert("hs".to_owned());
        }
        member("l", Kind::Light, caps)
    }

    #[test]
    fn common_keys() {
        let config = build_config("nidoo", "lights", &spec(Kind::Switch), &[]);
        assert_eq!(config["name"], "Living");
        assert_eq!(config["unique_id"], "group_lights");
        assert_eq!(
            config["device"],
            json!({ "identifiers": ["nidoo_group"], "name": "Groups" })
        );
        assert_eq!(config["availability_topic"], "nidoo/group/lights/avail");
        assert_eq!(config["payload_available"], "online");
        assert_eq!(config["retain"], true);
        assert_eq!(config["command_topic"], "nidoo/group/lights");
        assert_eq!(config["state_topic"], "nidoo/group/lights");
    }

    #[test]
    fn light_brightness_needs_every_member() {
        let both = [light(true, false), light(true, false)];
        let config = build_config("nidoo", "g", &spec(Kind::Light), &both);
        assert_eq!(config["brightness"], true);
        assert_eq!(config["brightness_scale"], 255);
        let mixed = [light(true, false), light(false, false)];
        let config = build_config("nidoo", "g", &spec(Kind::Light), &mixed);
        assert!(config.get("brightness").is_none());
        assert!(config.get("brightness_command_topic").is_none());
    }

    #[test]
    fn light_colour_needs_every_member() {
        let one = [light(true, true), light(true, false)];
        let config = build_config("nidoo", "g", &spec(Kind::Light), &one);
        assert!(config.get("hs_command_topic").is_none());
        let all = [light(true, true), light(true, true)];
        let config = build_config("nidoo", "g", &spec(Kind::Light), &all);
        assert_eq!(config["hs_state_topic"], "nidoo/group/g");
    }

    #[test]
    fn cover_position_needs_every_member() {
        let with = |position| {
            member(
                "c",
                Kind::Cover,
                Capabilities {
                    position,
                    ..Default::default()
                },
            )
        };
        let config = build_config("nidoo", "g", &spec(Kind::Cover), &[with(true), with(true)]);
        assert_eq!(config["position_topic"], "nidoo/group/g");
        let config = build_config("nidoo", "g", &spec(Kind::Cover), &[with(true), with(false)]);
        assert!(config.get("position_topic").is_none());
    }

    #[test]
    fn climate_intersects_ranges_and_modes() {
        let climate = |min, max, step, modes: &[&str]| {
            member(
                "c",
                Kind::Climate,
                Capabilities {
                    min_temp: Some(min),
                    max_temp: Some(max),
                    temp_step: Some(step),
                    hvac_modes: modes.iter().map(|m| (*m).to_owned()).collect(),
                    ..Default::default()
                },
            )
        };
        let members = [
            climate(5.0, 30.0, 0.5, &["off", "heat", "auto"]),
            climate(7.0, 28.0, 1.0, &["off", "heat"]),
        ];
        let config = build_config("nidoo", "g", &spec(Kind::Climate), &members);
        assert_eq!(config["min_temp"], 7.0);
        assert_eq!(config["max_temp"], 28.0);
        assert_eq!(config["temp_step"], 1.0);
        assert_eq!(config["modes"], json!(["off", "heat"]));
    }

    #[test]
    fn button_has_no_state_and_no_retain() {
        let config = build_config("nidoo", "g", &spec(Kind::Button), &[]);
        assert_eq!(config["payload_press"], r#"{"type":"press"}"#);
        assert!(config.get("state_topic").is_none());
        assert!(config.get("retain").is_none());
        assert_eq!(config["availability_topic"], "nidoo/group/g/avail");
    }

    #[test]
    fn select_and_vacuum_intersect() {
        let select = |options: &[&str]| {
            member(
                "s",
                Kind::Select,
                Capabilities {
                    options: options.iter().map(|o| (*o).to_owned()).collect(),
                    ..Default::default()
                },
            )
        };
        let config = build_config(
            "nidoo",
            "g",
            &spec(Kind::Select),
            &[select(&["a", "b", "c"]), select(&["c", "a"])],
        );
        assert_eq!(config["options"], json!(["a", "c"]));

        let vacuum = |actions: &[&str]| {
            member(
                "v",
                Kind::Vacuum,
                Capabilities {
                    vacuum_actions: actions.iter().map(|a| (*a).to_owned()).collect(),
                    ..Default::default()
                },
            )
        };
        let config = build_config(
            "nidoo",
            "g",
            &spec(Kind::Vacuum),
            &[
                vacuum(&["start", "stop", "locate"]),
                vacuum(&["start", "stop"]),
            ],
        );
        assert_eq!(
            config["payload_start"],
            r#"{"type":"vacuum_action","action":"start"}"#
        );
        assert!(config.get("payload_locate").is_none());
    }

    #[test]
    fn number_intersects_bounds_and_unit() {
        let number = |min, max, step, unit: &str| {
            let mut view = member(
                "n",
                Kind::Number,
                Capabilities {
                    min: Some(min),
                    max: Some(max),
                    step: Some(step),
                    ..Default::default()
                },
            );
            view.unit = Some(unit.to_owned());
            view
        };
        let config = build_config(
            "nidoo",
            "g",
            &spec(Kind::Number),
            &[number(0.0, 100.0, 1.0, "%"), number(10.0, 90.0, 5.0, "%")],
        );
        assert_eq!(
            (config["min"].as_f64(), config["max"].as_f64()),
            (Some(10.0), Some(90.0))
        );
        assert_eq!(config["step"], 5.0);
        assert_eq!(config["unit_of_measurement"], "%");
        let config = build_config(
            "nidoo",
            "g",
            &spec(Kind::Number),
            &[number(0.0, 100.0, 1.0, "%"), number(0.0, 100.0, 1.0, "W")],
        );
        assert!(config.get("unit_of_measurement").is_none());
    }
}
