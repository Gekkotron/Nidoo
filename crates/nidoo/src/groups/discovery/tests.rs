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
