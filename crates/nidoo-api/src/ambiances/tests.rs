use super::*;
use nidoo_core::model::{ClimateState, CoverState, EntityState, LightState, VacuumState};
use serde_json::json;

/// Spec Appendix A.2, without the comment. The command is the real `select_option`
/// shape (the spec's `select` was a placeholder).
fn spec_example() -> Value {
    json!({
      "id": "bonne_nuit",
      "name": "Bonne nuit",
      "subtitle": "La maison s'endort",
      "icon": "moon",
      "pinned": true,
      "order": 1,
      "actions": [
        { "type": "category", "what": "lights", "to": "off", "area": null, "except": [] },
        { "type": "category", "what": "lights", "to": "on", "brightness": 30, "area": "living_room" },
        { "type": "category", "what": "plugs", "to": "off" },
        { "type": "category", "what": "covers", "to": "close" },
        { "type": "device", "entity": "select.house_mode", "command": { "type": "select_option", "option": "night" } },
        { "type": "climate", "zone": null, "setpoint": 16.0, "mode": null, "hours": 12 },
        { "type": "climate", "zone": null, "clear": true },
        { "type": "flow", "flow": "away_lights", "do": "enable" }
      ]
    })
}

/// Null and absent are the same on the wire: optional fields are not serialized when
/// empty, so the expected value drops its nulls and empty `except` lists.
fn without_empty(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(k, v)| !v.is_null() && !(k.as_str() == "except" && *v == &json!([])))
                .map(|(k, v)| (k.clone(), without_empty(v)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(without_empty).collect()),
        other => other.clone(),
    }
}

fn ambiance(actions: Value) -> Ambiance {
    serde_json::from_value(json!({"id": "a", "name": "A", "actions": actions})).unwrap()
}

#[test]
fn spec_example_round_trips() {
    let value = spec_example();
    let a: Ambiance = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(a.actions.len(), 8);
    assert_eq!(serde_json::to_value(&a).unwrap(), without_empty(&value));
}

#[test]
fn unknown_top_level_fields_survive() {
    let value = json!({"id": "a", "name": "A", "pinned": false, "order": 0, "actions": [], "color": "#fff"});
    let a: Ambiance = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(a.extra["color"], "#fff");
    assert_eq!(serde_json::to_value(&a).unwrap(), value);
}

#[test]
fn minimal_ambiance_defaults() {
    let a: Ambiance = serde_json::from_value(json!({"id": "a", "name": "A"})).unwrap();
    assert!(!a.pinned);
    assert_eq!(a.order, 0);
    assert!(a.actions.is_empty());
    let no_id: Ambiance = serde_json::from_value(json!({"name": "A"})).unwrap();
    assert_eq!(no_id.id, "");
}

#[test]
fn validate_accepts_spec_example() {
    let a: Ambiance = serde_json::from_value(spec_example()).unwrap();
    assert_eq!(validate_ambiance(&a), Ok(()));
}

#[test]
fn validate_rejects() {
    let cat = |v: Value| ambiance(json!([v]));
    let climate = |v: Value| {
        let mut m = json!({"type": "climate"});
        m.as_object_mut()
            .unwrap()
            .extend(v.as_object().unwrap().clone());
        ambiance(json!([m]))
    };
    let base = || ambiance(json!([]));
    let mut cases: Vec<(&str, Ambiance)> = Vec::new();
    let mut bad = base();
    bad.id = "Bonne Nuit".into();
    cases.push(("id with caps and space", bad));
    let mut bad = base();
    bad.id = "a".repeat(49);
    cases.push(("id too long", bad));
    let mut bad = base();
    bad.name = "  ".into();
    cases.push(("blank name", bad));
    let mut bad = base();
    bad.name = "n".repeat(61);
    cases.push(("name too long", bad));
    let mut bad = base();
    bad.subtitle = Some("s".repeat(81));
    cases.push(("subtitle too long", bad));
    let mut bad = base();
    bad.icon = Some("rocket".into());
    cases.push(("unknown icon", bad));
    cases.push((
        "lights open",
        cat(json!({"type": "category", "what": "lights", "to": "open"})),
    ));
    cases.push((
        "covers off",
        cat(json!({"type": "category", "what": "covers", "to": "off"})),
    ));
    cases.push((
        "plugs brightness",
        cat(json!({"type": "category", "what": "plugs", "to": "on", "brightness": 50})),
    ));
    cases.push((
        "lights off brightness",
        cat(json!({"type": "category", "what": "lights", "to": "off", "brightness": 50})),
    ));
    for b in [0, 101] {
        cases.push((
            "brightness out of range",
            cat(json!({"type": "category", "what": "lights", "to": "on", "brightness": b})),
        ));
    }
    cases.push((
        "clear with setpoint",
        climate(json!({"clear": true, "setpoint": 16})),
    ));
    cases.push((
        "clear with hours",
        climate(json!({"clear": true, "hours": 2})),
    ));
    cases.push(("empty climate", climate(json!({"zone": "z"}))));
    cases.push(("setpoint without hours", climate(json!({"setpoint": 16}))));
    for h in [0, 721] {
        cases.push((
            "hours out of range",
            climate(json!({"setpoint": 16, "hours": h})),
        ));
    }
    cases.push((
        "bad mode",
        climate(json!({"mode": "dry", "setpoint": 16, "hours": 2})),
    ));
    cases.push((
        "bad setpoint",
        climate(json!({"setpoint": "warm", "hours": 2})),
    ));
    for (label, a) in &cases {
        assert!(validate_ambiance(a).is_err(), "{label} should be refused");
    }
}

#[test]
fn action_errors_name_the_index() {
    let a = ambiance(json!([
        {"type": "category", "what": "plugs", "to": "off"},
        {"type": "category", "what": "covers", "to": "off"},
    ]));
    assert_eq!(
        validate_ambiance(&a),
        Err("action 1: covers take open or close".to_string())
    );
}

fn entity(id: &str, kind: Kind) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: id.into(),
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
        area: "living".into(),
        part_of: None,
        capabilities: Default::default(),
        ui: Default::default(),
        state: Default::default(),
    }
}

fn switch_as(id: &str, show_as: Option<&str>) -> EntityView {
    let mut e = entity(id, Kind::Switch);
    e.ui.show_as = show_as.map(Into::into);
    e
}

fn act(what: Category, to: Target) -> CategoryAction {
    CategoryAction {
        what,
        to,
        brightness: None,
        area: None,
        except: vec![],
    }
}

fn ids(v: Vec<&EntityView>) -> Vec<&str> {
    v.into_iter().map(|e| e.id.as_str()).collect()
}

fn light_state(on: bool, brightness: Option<u8>) -> EntityState {
    EntityState {
        value: StateValue::Light(LightState {
            on,
            brightness,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn with_state(mut e: EntityView, value: StateValue) -> EntityView {
    e.state.value = value;
    e
}

#[test]
fn plugs_never_include_door_siren_chime() {
    let all = [
        switch_as("door", Some("door")),
        switch_as("siren", Some("siren")),
        switch_as("chime", Some("chime")),
        switch_as("plain", None),
        switch_as("plug", Some("plug")),
    ];
    let a = act(Category::Plugs, Target::Off);
    assert_eq!(ids(category_members(&a, &all)), ["plug"]);
}

#[test]
fn untyped_switch_is_a_switch_not_a_plug() {
    let all = [switch_as("relay", None), switch_as("plug", Some("plug"))];
    let a = act(Category::Plugs, Target::Off);
    assert_eq!(ids(category_members(&a, &all)), ["plug"]);
}

#[test]
fn switch_shown_as_light_is_a_light_not_a_plug() {
    let mut lamp = switch_as("lamp", Some("light"));
    lamp.kind = Kind::Light;
    let all = [lamp];
    assert!(category_members(&act(Category::Plugs, Target::Off), &all).is_empty());
    assert_eq!(
        ids(category_members(&act(Category::Lights, Target::Off), &all)),
        ["lamp"]
    );
}

#[test]
fn groups_are_not_category_members() {
    let all = [entity("group_x", Kind::Light), entity("lamp", Kind::Light)];
    let a = act(Category::Lights, Target::Off);
    assert_eq!(ids(category_members(&a, &all)), ["lamp"]);
}

#[test]
fn virtual_entities_and_the_z2m_bridge_are_never_plugs() {
    let mut permit_join = switch_as(
        "bridge_0x00124b0022983b3e_permit_join_zigbee2mqtt",
        Some("plug"),
    );
    permit_join.device_id = Some("zigbee2mqtt_bridge_0x00124b0022983b3e".into());
    let all = [
        switch_as("virtual_guest_mode", Some("plug")),
        permit_join,
        switch_as("plug", Some("plug")),
    ];
    let a = act(Category::Plugs, Target::Off);
    assert_eq!(ids(category_members(&a, &all)), ["plug"]);
}

#[test]
fn z2m_groups_are_not_category_members() {
    let mut group = entity("1_light_zigbee2mqtt", Kind::Light);
    group.capabilities.z2m_group = true;
    let all = [group, entity("lamp", Kind::Light)];
    let a = act(Category::Lights, Target::Off);
    assert_eq!(ids(category_members(&a, &all)), ["lamp"]);
}

#[test]
fn covers_are_window_coverings_only() {
    let cover = |id: &str, class: Option<&str>| {
        let mut e = entity(id, Kind::Cover);
        e.device_class = class.map(Into::into);
        e
    };
    let all = [
        cover("shutter", Some("shutter")),
        cover("blind", Some("blind")),
        cover("curtain", Some("curtain")),
        cover("shade", Some("shade")),
        cover("awning", Some("awning")),
        cover("window", Some("window")),
        cover("garage", Some("garage")),
        cover("gate", Some("gate")),
        cover("door", Some("door")),
        cover("damper", Some("damper")),
    ];
    let a = act(Category::Covers, Target::Close);
    assert_eq!(
        ids(category_members(&a, &all)),
        ["shutter", "blind", "curtain", "shade", "awning", "window"]
    );
}

fn cover_marked(id: &str, class: Option<&str>, mark: bool) -> EntityView {
    let mut e = entity(id, Kind::Cover);
    e.device_class = class.map(Into::into);
    e.ui.show_as = mark.then(|| "shutter".to_owned());
    e
}

#[test]
fn unclassed_cover_is_left_out() {
    let all = [cover_marked("plain", None, false)];
    let a = act(Category::Covers, Target::Close);
    assert!(category_members(&a, &all).is_empty());
}

#[test]
fn unclassed_cover_with_the_mark_is_included() {
    let all = [cover_marked("plain", None, true)];
    let a = act(Category::Covers, Target::Close);
    assert_eq!(ids(category_members(&a, &all)), ["plain"]);
}

#[test]
fn garage_class_with_the_mark_is_left_out() {
    let all = [cover_marked("garage", Some("garage"), true)];
    let a = act(Category::Covers, Target::Close);
    assert!(category_members(&a, &all).is_empty());
}

#[test]
fn unknown_show_as_is_not_a_plug() {
    let all = [
        switch_as("fan", Some("fan")),
        switch_as("plug", Some("plug")),
    ];
    let a = act(Category::Plugs, Target::Off);
    assert_eq!(ids(category_members(&a, &all)), ["plug"]);
}

#[test]
fn non_primary_and_folded_are_excluded_hidden_is_kept() {
    let mut diag = entity("diag", Kind::Light);
    diag.category = EntityCategory::Diagnostic;
    let mut off_by_default = entity("dis", Kind::Light);
    off_by_default.enabled_by_default = false;
    let mut folded = entity("folded", Kind::Light);
    folded.part_of = Some("other".into());
    let mut hidden = entity("hidden", Kind::Light);
    hidden.ui.hidden = true;
    let all = [diag, off_by_default, folded, hidden];
    let a = act(Category::Lights, Target::Off);
    assert_eq!(ids(category_members(&a, &all)), ["hidden"]);
}

#[test]
fn area_and_except_filter() {
    let mut kitchen = entity("k", Kind::Light);
    kitchen.area = "kitchen".into();
    let all = [entity("a", Kind::Light), entity("b", Kind::Light), kitchen];
    let mut a = act(Category::Lights, Target::Off);
    a.area = Some("living".into());
    a.except = vec!["b".into()];
    assert_eq!(ids(category_members(&a, &all)), ["a"]);
}

#[test]
fn brightness_pct_maps_to_255_scale_only_when_dimmable() {
    let mut dimmable = entity("dim", Kind::Light);
    dimmable.capabilities.brightness = true;
    let plain = entity("plain", Kind::Light);
    let mut a = act(Category::Lights, Target::On);
    a.brightness = Some(30);
    let plan = plan_category(&a, &[dimmable, plain]);
    let on = |brightness| Command::TurnOn {
        brightness,
        color: None,
        effect: None,
    };
    assert_eq!(
        plan.send,
        [("dim".into(), on(Some(77))), ("plain".into(), on(None))]
    );
}

#[test]
fn plan_splits_offline_already_and_send() {
    let mut off = entity("off", Kind::Light);
    off.state = light_state(false, None);
    let mut on = entity("on", Kind::Light);
    on.state = light_state(true, Some(200));
    let mut gone = entity("gone", Kind::Light);
    gone.state = light_state(true, None);
    gone.state.availability = Availability::Offline;
    let plan = plan_category(&act(Category::Lights, Target::Off), &[off, on, gone]);
    assert_eq!(plan.send, [("on".to_string(), Command::TurnOff)]);
    assert_eq!(plan.already, ["off"]);
    assert_eq!(plan.offline, ["gone"]);
}

#[test]
fn already_there_cases() {
    let on = |brightness| Command::TurnOn {
        brightness,
        color: None,
        effect: None,
    };
    let light = |on, b| {
        let mut e = entity("l", Kind::Light);
        e.state = light_state(on, b);
        e
    };
    let plug = |v| with_state(entity("p", Kind::Switch), StateValue::OnOff(v));
    let cover = |state: Option<&str>, position| {
        with_state(
            entity("c", Kind::Cover),
            StateValue::Cover(CoverState {
                state: state.map(Into::into),
                position,
            }),
        )
    };
    let climate = with_state(
        entity("t", Kind::Climate),
        StateValue::Climate(ClimateState {
            target: Some(20.0),
            mode: Some("heat".into()),
            ..Default::default()
        }),
    );

    let vacuum = |state: &str| {
        with_state(
            entity("v", Kind::Vacuum),
            StateValue::Vacuum(VacuumState {
                state: Some(state.into()),
                ..Default::default()
            }),
        )
    };
    let action = |a: &str| Command::VacuumAction { action: a.into() };
    assert!(already_there(&vacuum("cleaning"), &action("start")));
    assert!(!already_there(&vacuum("docked"), &action("start")));
    assert!(already_there(
        &vacuum("returning"),
        &action("return_to_base")
    ));
    assert!(already_there(&vacuum("docked"), &action("stop")));
    assert!(!already_there(&vacuum("cleaning"), &action("pause")));
    assert!(!already_there(&vacuum("docked"), &Command::Press));
    assert!(already_there(&plug(false), &Command::TurnOff));
    assert!(!already_there(&plug(true), &Command::TurnOff));
    assert!(already_there(&light(false, None), &Command::TurnOff));
    assert!(already_there(&plug(true), &on(None)));
    assert!(already_there(&light(true, Some(10)), &on(None)));
    assert!(!already_there(&light(false, None), &on(None)));
    assert!(already_there(&light(true, Some(250)), &on(Some(255))));
    assert!(!already_there(&light(true, Some(240)), &on(Some(255))));
    assert!(!already_there(&light(false, Some(255)), &on(Some(255))));
    let red = Hs { h: 0.0, s: 100.0 };
    let coloured = |on, hs| {
        let mut e = light(on, None);
        if let StateValue::Light(l) = &mut e.state.value {
            l.hs = hs;
        }
        e
    };
    let on_red = Command::TurnOn {
        brightness: None,
        color: Some(red),
        effect: None,
    };
    assert!(already_there(
        &coloured(true, Some(Hs { h: 358.0, s: 98.0 })),
        &on_red
    ));
    assert!(!already_there(
        &coloured(true, Some(Hs { h: 225.0, s: 100.0 })),
        &on_red
    ));
    assert!(!already_there(&coloured(true, None), &on_red));
    assert!(!already_there(&coloured(false, Some(red)), &on_red));
    assert!(!already_there(&plug(true), &on_red));
    assert!(already_there(&cover(None, Some(0)), &Command::Close));
    assert!(already_there(&cover(Some("closed"), None), &Command::Close));
    assert!(!already_there(&cover(None, Some(50)), &Command::Close));
    assert!(already_there(&cover(None, Some(100)), &Command::Open));
    assert!(already_there(&cover(Some("open"), None), &Command::Open));
    // A known position wins over the open/closed word: half-open is neither.
    assert!(!already_there(
        &cover(Some("open"), Some(30)),
        &Command::Open
    ));
    assert!(!already_there(
        &cover(Some("closed"), Some(30)),
        &Command::Close
    ));
    assert!(already_there(
        &cover(Some("open"), Some(100)),
        &Command::Open
    ));
    assert!(already_there(
        &cover(None, Some(40)),
        &Command::SetPosition { position: 40 }
    ));
    assert!(!already_there(
        &cover(None, Some(41)),
        &Command::SetPosition { position: 40 }
    ));
    let sel = with_state(entity("s", Kind::Select), StateValue::Text("night".into()));
    assert!(already_there(
        &sel,
        &Command::SelectOption {
            option: "night".into()
        }
    ));
    assert!(already_there(
        &climate,
        &Command::SetTemperature { temperature: 20.04 }
    ));
    assert!(!already_there(
        &climate,
        &Command::SetTemperature { temperature: 20.1 }
    ));
    assert!(already_there(
        &climate,
        &Command::SetHvacMode {
            mode: "heat".into()
        }
    ));
    assert!(!already_there(
        &climate,
        &Command::SetHvacMode {
            mode: "cool".into()
        }
    ));
    assert!(!already_there(&entity("u", Kind::Light), &Command::TurnOff));
    assert!(!already_there(&plug(true), &Command::Toggle));
}

#[test]
fn already_there_for_media() {
    let player = |state: &str| {
        let mut e = entity("m", Kind::MediaPlayer);
        e.state.value = StateValue::MediaPlayer(nidoo_core::model::MediaPlayerState {
            state: Some(state.into()),
            ..Default::default()
        });
        e
    };
    let action = |a: &str| Command::MediaAction { action: a.into() };
    assert!(already_there(&player("paused"), &action("pause")));
    assert!(!already_there(&player("playing"), &action("pause")));
    assert!(already_there(&player("playing"), &action("play")));
    assert!(!already_there(&player("idle"), &action("play")));
    assert!(!already_there(&player("playing"), &action("next")));
    assert!(!already_there(
        &player("playing"),
        &Command::PlayMedia { query: "x".into() }
    ));
}
