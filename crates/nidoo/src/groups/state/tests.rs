use super::*;
use nidoo_core::model::{
    Capabilities, ClimateState, CoverState, EntityCategory, EntityState, LightState, UiOverride,
    VacuumState,
};
use serde_json::{Value, json};

fn member(kind: Kind, value: StateValue, caps: Capabilities) -> EntityView {
    EntityView {
        id: "m".to_owned(),
        device_id: None,
        name: "m".to_owned(),
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
        state: EntityState {
            value,
            ..Default::default()
        },
    }
}

fn encoded(kind: Kind, members: &[EntityView]) -> Value {
    let state = combine(kind, members).expect("a state");
    serde_json::from_str(&encode(&state)).unwrap()
}

fn switch(on: bool) -> EntityView {
    member(Kind::Switch, StateValue::OnOff(on), Capabilities::default())
}

fn cover(state: &str) -> EntityView {
    member(
        Kind::Cover,
        StateValue::Cover(CoverState {
            state: Some(state.to_owned()),
            position: None,
        }),
        Capabilities::default(),
    )
}

fn climate(mode: &str, target: f64) -> EntityView {
    member(
        Kind::Climate,
        StateValue::Climate(ClimateState {
            mode: Some(mode.to_owned()),
            target: Some(target),
            ..Default::default()
        }),
        Capabilities::default(),
    )
}

#[test]
fn switch_is_on_when_any_member_is_on() {
    assert_eq!(
        encoded(Kind::Switch, &[switch(true), switch(false)]),
        json!({"state":"ON"})
    );
    assert_eq!(
        encoded(Kind::Switch, &[switch(false), switch(false)]),
        json!({"state":"OFF"})
    );
}

#[test]
fn nothing_is_published_without_a_reported_member() {
    assert!(combine(Kind::Switch, &[]).is_none());
    let unknown = member(Kind::Switch, StateValue::Unknown, Capabilities::default());
    assert!(combine(Kind::Switch, &[unknown]).is_none());
    assert!(combine(Kind::Button, &[switch(true)]).is_none());
}

#[test]
fn light_means_cover_the_members_that_are_on() {
    let caps = Capabilities {
        brightness: true,
        ..Default::default()
    };
    let light = |on, brightness| {
        member(
            Kind::Light,
            StateValue::Light(LightState {
                on,
                brightness: Some(brightness),
                ..Default::default()
            }),
            caps.clone(),
        )
    };
    let state = encoded(
        Kind::Light,
        &[light(true, 100), light(true, 201), light(false, 0)],
    );
    assert_eq!(state, json!({"state":"ON","brightness":151}));
    let off = encoded(Kind::Light, &[light(false, 0)]);
    assert_eq!(off, json!({"state":"OFF"}));
}

#[test]
fn cover_opening_wins_and_has_text() {
    let state = encoded(
        Kind::Cover,
        &[cover("open"), cover("opening"), cover("closing")],
    );
    assert_eq!(state["state"], "opening");
    assert_eq!(state["text"], "Opening…");
    let state = encoded(Kind::Cover, &[cover("closed"), cover("closing")]);
    assert_eq!(
        (state["state"].as_str(), state["text"].as_str()),
        (Some("closing"), Some("Closing…"))
    );
}

#[test]
fn cover_texts_for_settled_members() {
    let all_open = encoded(Kind::Cover, &[cover("open"), cover("open")]);
    assert_eq!(
        (all_open["state"].as_str(), all_open["text"].as_str()),
        (Some("open"), Some("All open"))
    );
    let all_closed = encoded(Kind::Cover, &[cover("closed")]);
    assert_eq!(
        (all_closed["state"].as_str(), all_closed["text"].as_str()),
        (Some("closed"), Some("All closed"))
    );
    let split = encoded(
        Kind::Cover,
        &[cover("open"), cover("closed"), cover("closed")],
    );
    assert_eq!(
        (split["state"].as_str(), split["text"].as_str()),
        (Some("open"), Some("1 open · 2 closed"))
    );
}

#[test]
fn cover_position_is_the_rounded_mean_when_shared() {
    let at = |position| {
        member(
            Kind::Cover,
            StateValue::Cover(CoverState {
                state: Some("open".to_owned()),
                position: Some(position),
            }),
            Capabilities {
                position: true,
                ..Default::default()
            },
        )
    };
    assert_eq!(encoded(Kind::Cover, &[at(10), at(25)])["position"], 18);
}

#[test]
fn climate_common_mode_and_range() {
    let state = encoded(
        Kind::Climate,
        &[climate("heat", 19.0), climate("heat", 21.0)],
    );
    assert_eq!(state["mode"], "heat");
    assert_eq!(state["target"], 20.0);
    assert_eq!(state["range"], json!([19.0, 21.0]));
    let same = encoded(
        Kind::Climate,
        &[climate("heat", 20.0), climate("heat", 20.0)],
    );
    assert!(same.get("range").is_none());
}

#[test]
fn climate_mixed_modes_encode_null() {
    let state = encoded(
        Kind::Climate,
        &[climate("heat", 20.0), climate("off", 20.0)],
    );
    assert_eq!(state["mode"], Value::Null);
}

#[test]
fn climate_target_is_rounded_to_the_step_and_clamped() {
    let caps = Capabilities {
        min_temp: Some(5.0),
        max_temp: Some(21.0),
        temp_step: Some(0.5),
        ..Default::default()
    };
    let with = |mut view: EntityView| {
        view.capabilities = caps.clone();
        view
    };
    let state = encoded(
        Kind::Climate,
        &[with(climate("heat", 20.0)), with(climate("heat", 23.0))],
    );
    assert_eq!(state["target"], 21.0);
}

#[test]
fn number_and_select_share_a_common_value() {
    let number = |v| member(Kind::Number, StateValue::Number(v), Capabilities::default());
    assert_eq!(
        encoded(Kind::Number, &[number(3.0), number(3.0)]),
        json!({"value":3.0})
    );
    assert_eq!(
        encoded(Kind::Number, &[number(1.0), number(2.0)]),
        json!({"value":1.5})
    );
    let select = |o: &str| {
        member(
            Kind::Select,
            StateValue::Text(o.to_owned()),
            Capabilities::default(),
        )
    };
    assert_eq!(
        encoded(Kind::Select, &[select("a"), select("a")]),
        json!({"option":"a"})
    );
    assert_eq!(
        encoded(Kind::Select, &[select("a"), select("b")]),
        json!({"option":null})
    );
}

#[test]
fn vacuum_precedence() {
    let vacuum = |s: &str| {
        member(
            Kind::Vacuum,
            StateValue::Vacuum(VacuumState {
                state: Some(s.to_owned()),
                ..Default::default()
            }),
            Capabilities::default(),
        )
    };
    assert_eq!(
        encoded(Kind::Vacuum, &[vacuum("docked"), vacuum("cleaning")])["state"],
        "cleaning"
    );
    assert_eq!(
        encoded(Kind::Vacuum, &[vacuum("docked"), vacuum("returning")])["state"],
        "returning"
    );
    assert_eq!(
        encoded(Kind::Vacuum, &[vacuum("docked"), vacuum("docked")])["state"],
        "docked"
    );
    assert_eq!(
        encoded(Kind::Vacuum, &[vacuum("paused"), vacuum("docked")])["state"],
        "paused"
    );
}
