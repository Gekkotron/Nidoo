use nidoo_core::flow::{
    Edge, Node, NodeKind, TriggerManualParams, TriggerScheduleParams, TriggerStateParams,
};
use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, StateValue, UiOverride};
use serde_json::json;

use super::*;

fn ent(kind: Kind) -> EntityView {
    EntityView {
        id: "e".into(),
        device_id: None,
        name: "E".into(),
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
        area: "a".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState::default(),
    }
}

#[test]
fn upstream_trigger_field_follows_the_one_state_trigger() {
    let battery = node(
        "t",
        NodeKind::TriggerState(TriggerStateParams {
            entity: "robo".into(),
            field: Some("battery".into()),
            ..Default::default()
        }),
    );
    let flow = |nodes: Vec<Node>| Flow {
        edges: nodes
            .iter()
            .filter(|n| n.id != "check")
            .map(|n| Edge {
                from: n.id.clone(),
                to: "check".into(),
                port: None,
            })
            .collect(),
        nodes,
        ..Flow::decode(br#"{"id":"f","name":"F"}"#).unwrap()
    };
    let one = flow(vec![battery.clone(), plain("check")]);
    assert_eq!(
        upstream_trigger_field(&one, "check").as_deref(),
        Some("battery")
    );
    let plain_state = flow(vec![state("t", "robo"), plain("check")]);
    assert_eq!(upstream_trigger_field(&plain_state, "check"), None);
    let mixed = flow(vec![battery, state("u", "robo"), plain("check")]);
    assert_eq!(upstream_trigger_field(&mixed, "check"), None);
}

fn vacuum() -> EntityView {
    let mut e = ent(Kind::Vacuum);
    e.id = "robo".into();
    e.name = "Roborock".into();
    e.device_id = Some("robo_dev".into());
    e.capabilities.fan_speeds = vec!["Silent".into(), "Turbo".into()];
    e.state.value = StateValue::Vacuum(nidoo_core::model::VacuumState {
        state: Some("cleaning".into()),
        battery: Some(62),
        fan_speed: Some("Turbo".into()),
    });
    e
}

fn house(list: Vec<EntityView>) -> std::collections::BTreeMap<String, EntityView> {
    list.into_iter().map(|e| (e.id.clone(), e)).collect()
}

#[test]
fn vacuum_value_of_lists_fields_then_siblings() {
    let e = vacuum();
    let entities = house(vec![e.clone()]);
    let choices = value_of_choices(Lang::En, &entities, &e);
    let of: Vec<ValueOf> = choices.iter().map(|c| c.of.clone()).collect();
    assert_eq!(
        of,
        [
            ValueOf::Headline,
            ValueOf::Field("battery".into()),
            ValueOf::Field("fan_speed".into()),
        ]
    );
    assert_eq!(choices[1].label, "Battery (62\u{a0}%)");
    let fr = value_of_choices(Lang::Fr, &entities, &e);
    assert_eq!(fr[1].label, "Batterie (62\u{a0}%)");
}

#[test]
fn zigbee_value_of_lists_diagnostic_siblings() {
    let mut smoke = ent(Kind::BinarySensor);
    smoke.id = "0xabc_smoke".into();
    smoke.name = "Smoke".into();
    smoke.device_id = Some("0xabc".into());
    let mut battery = ent(Kind::Sensor);
    battery.id = "0xabc_battery".into();
    battery.name = "Battery".into();
    battery.device_id = Some("0xabc".into());
    battery.unit = Some("%".into());
    battery.category = EntityCategory::Diagnostic;
    battery.state.value = StateValue::Number(80.0);
    // Zigbee2MQTT announces link quality disabled by default: listed, after the others.
    let mut link = ent(Kind::Sensor);
    link.id = "0xabc_linkquality".into();
    link.name = "Linkquality".into();
    link.device_id = Some("0xabc".into());
    link.category = EntityCategory::Diagnostic;
    link.enabled_by_default = false;
    let mut other = ent(Kind::Sensor);
    other.id = "0xdef_battery".into();
    other.device_id = Some("0xdef".into());
    let entities = house(vec![smoke.clone(), link, battery, other]);
    let choices = value_of_choices(Lang::En, &entities, &smoke);
    let of: Vec<ValueOf> = choices.iter().map(|c| c.of.clone()).collect();
    assert_eq!(
        of,
        [
            ValueOf::Headline,
            ValueOf::Sibling("0xabc_battery".into()),
            ValueOf::Sibling("0xabc_linkquality".into()),
        ]
    );
    assert_eq!(choices[1].label, "Battery (80\u{a0}%)");
}

#[test]
fn entity_without_device_has_no_siblings() {
    let mut lone = ent(Kind::Sensor);
    lone.id = "lone".into();
    let mut other = ent(Kind::Sensor);
    other.id = "other".into();
    let entities = house(vec![lone.clone(), other]);
    let choices = value_of_choices(Lang::En, &entities, &lone);
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].of, ValueOf::Headline);
}

#[test]
fn battery_field_input_is_a_percent_number() {
    let e = vacuum();
    assert_eq!(
        value_input_of(Lang::En, Some(&e), Some("battery")),
        ValueInput::Number {
            unit: Some("%".into())
        }
    );
    assert!(matches!(
        value_input_of(Lang::En, Some(&e), Some("fan_speed")),
        ValueInput::List(list) if list.len() == 2
    ));
    assert_eq!(
        value_input_of(Lang::En, Some(&e), None),
        value_input(Lang::En, Some(&e))
    );
}

#[test]
fn average_only_on_recorded_fields() {
    let e = vacuum();
    let climate = ent(Kind::Climate);
    assert!(average_allowed(Some(&e), None));
    assert!(!average_allowed(Some(&e), Some("battery")));
    assert!(average_allowed(Some(&climate), Some("current")));
    assert!(!average_allowed(Some(&climate), Some("target")));
}

fn node(id: &str, kind: NodeKind) -> Node {
    Node {
        id: id.into(),
        name: None,
        position: None,
        kind,
        extra: Default::default(),
    }
}

fn state(id: &str, entity: &str) -> Node {
    node(
        id,
        NodeKind::TriggerState(TriggerStateParams {
            entity: entity.into(),
            ..Default::default()
        }),
    )
}

fn schedule(id: &str) -> Node {
    node(
        id,
        NodeKind::TriggerSchedule(TriggerScheduleParams {
            cron: "0 * * * *".into(),
            ..Default::default()
        }),
    )
}

fn plain(id: &str) -> Node {
    node(
        id,
        NodeKind::Delay(nidoo_core::flow::DelayParams {
            for_: "1m".into(),
            ..Default::default()
        }),
    )
}

fn edge(from: &str, to: &str) -> Edge {
    Edge {
        from: from.into(),
        to: to.into(),
        port: None,
    }
}

fn flow(nodes: Vec<Node>, edges: Vec<Edge>) -> Flow {
    Flow {
        nodes,
        edges,
        ..Default::default()
    }
}

fn list(input: ValueInput) -> Vec<Choice> {
    match input {
        ValueInput::List(l) => l,
        other => panic!("not a list: {other:?}"),
    }
}

#[test]
fn cover_lists_core_words_in_french() {
    let l = list(value_input(Lang::Fr, Some(&ent(Kind::Cover))));
    assert_eq!(l[1].value, json!("closed"));
    assert_eq!(l[1].label, "Fermé");
}

#[test]
fn number_pick_clears_only_a_truly_empty_box() {
    assert_eq!(number_pick("21.5", false), Some(Some(json!(21.5))));
    assert_eq!(number_pick("", false), Some(None));
    assert_eq!(number_pick("  ", false), Some(None));
    // A typo the browser cannot parse reads "" with bad input: keep the saved value.
    assert_eq!(number_pick("", true), None);
    assert_eq!(number_pick("abc", false), None);
}

#[test]
fn a_saved_value_that_is_not_a_number_is_not_shown_in_a_number_box() {
    assert!(fits_number(None));
    assert!(fits_number(Some(&json!(21.5))));
    assert!(!fits_number(Some(&json!("unavailable"))));
    assert!(!fits_number(Some(&json!("21"))));
}

#[test]
fn list_pick_reads_any_other_and_indexes() {
    let choices = vec![
        Choice {
            value: json!(true),
            label: "Open".into(),
        },
        Choice {
            value: json!(false),
            label: "Closed".into(),
        },
    ];
    assert_eq!(list_pick("", &choices, true), Some(ListPick::Any));
    assert_eq!(list_pick("", &choices, false), None);
    assert_eq!(list_pick("other", &choices, true), Some(ListPick::Other));
    assert_eq!(
        list_pick("1", &choices, true),
        Some(ListPick::Value(json!(false)))
    );
    assert_eq!(list_pick("9", &choices, true), None);
}

#[test]
fn a_saved_unlisted_trigger_value_stays_in_the_list() {
    let mut door = ent(Kind::BinarySensor);
    door.device_class = Some("door".into());
    let l = list(with_current(
        value_input(Lang::En, Some(&door)),
        Some(&json!("ON")),
        Lang::En,
        Some(&door),
    ));
    assert_eq!(l[0].value, json!("ON"));
}

#[test]
fn door_sensor_is_worded_by_device_class() {
    let mut door = ent(Kind::BinarySensor);
    door.device_class = Some("door".into());
    let l = list(value_input(Lang::En, Some(&door)));
    assert_eq!(
        (l[0].value.clone(), l[0].label.as_str()),
        (json!(true), "Open")
    );
}

#[test]
fn numeric_sensor_is_a_number_with_its_unit() {
    let mut t = ent(Kind::Sensor);
    t.unit = Some("°C".into());
    assert_eq!(
        value_input(Lang::En, Some(&t)),
        ValueInput::Number {
            unit: Some("°C".into())
        }
    );
}

#[test]
fn text_sensor_with_a_numeric_state_gets_a_number_box() {
    let mut s = ent(Kind::Sensor);
    s.state.value = StateValue::Number(3.0);
    assert_eq!(
        value_input(Lang::En, Some(&s)),
        ValueInput::Number { unit: None }
    );
    assert_eq!(
        value_input(Lang::En, Some(&ent(Kind::Sensor))),
        ValueInput::Text
    );
    assert_eq!(value_input(Lang::En, None), ValueInput::Text);
}

#[test]
fn unlisted_current_value_is_kept_first() {
    let sw = ent(Kind::Switch);
    let l = list(with_current(
        value_input(Lang::En, Some(&sw)),
        Some(&json!("closed")),
        Lang::En,
        Some(&sw),
    ));
    assert_eq!((l[0].value.clone(), l.len()), (json!("closed"), 3));
    let kept = list(with_current(
        value_input(Lang::En, Some(&sw)),
        Some(&json!(true)),
        Lang::En,
        Some(&sw),
    ));
    assert_eq!(kept.len(), 2);
}

#[test]
fn other_value_parses_numbers_bools_and_text() {
    assert_eq!(parse_other("21,5"), Some(json!(21.5)));
    assert_eq!(parse_other("true"), Some(json!(true)));
    assert_eq!(parse_other(" on "), Some(json!("on")));
    assert_eq!(parse_other("  "), None);
}

#[test]
fn entity_change_keeps_an_offered_value_else_resets() {
    let cover = value_input(Lang::En, Some(&ent(Kind::Cover)));
    assert_eq!(
        after_entity_change(Some(&json!("closed")), &cover),
        Some(json!("closed"))
    );
    assert_eq!(
        after_entity_change(Some(&json!(true)), &cover),
        Some(json!("open"))
    );
    let number = ValueInput::Number { unit: None };
    assert_eq!(
        after_entity_change(Some(&json!(3)), &number),
        Some(json!(3))
    );
    assert_eq!(after_entity_change(Some(&json!("x")), &number), None);
}

#[test]
fn upstream_entity_follows_the_feeding_state_trigger() {
    let f = flow(
        vec![
            state("t1", "a"),
            state("t2", "b"),
            plain("d0"),
            plain("if1"),
            plain("d1"),
        ],
        vec![edge("t1", "d0"), edge("d0", "if1"), edge("t2", "d1")],
    );
    assert_eq!(upstream_trigger_entity(&f, "if1"), Some("a".into()));
}

#[test]
fn upstream_entity_is_none_for_mixed_triggers() {
    let mixed = flow(
        vec![state("t1", "a"), schedule("t2"), plain("if1")],
        vec![edge("t1", "if1"), edge("t2", "if1")],
    );
    assert_eq!(upstream_trigger_entity(&mixed, "if1"), None);
    let two = flow(
        vec![state("t1", "a"), state("t2", "b"), plain("if1")],
        vec![edge("t1", "if1"), edge("t2", "if1")],
    );
    assert_eq!(upstream_trigger_entity(&two, "if1"), None);
    let same = flow(
        vec![state("t1", "a"), state("t2", "a"), plain("if1")],
        vec![edge("t1", "if1"), edge("t2", "if1")],
    );
    assert_eq!(upstream_trigger_entity(&same, "if1"), Some("a".into()));
}

#[test]
fn upstream_walk_survives_a_cycle() {
    let looped = flow(
        vec![state("t1", "a"), plain("x"), plain("if1")],
        vec![edge("t1", "x"), edge("x", "if1"), edge("if1", "x")],
    );
    assert_eq!(upstream_trigger_entity(&looped, "if1"), Some("a".into()));
}

fn manual(id: &str, value: Option<Scalar>) -> Node {
    node(
        id,
        NodeKind::TriggerManual(TriggerManualParams {
            value,
            ..Default::default()
        }),
    )
}

#[test]
fn manual_value_reaches_an_if_behind_one_card() {
    let f = flow(
        vec![manual("m", Some(Scalar::Bool(true))), plain("i")],
        vec![edge("m", "i")],
    );
    assert_eq!(upstream_manual_value(&f, "i"), Some(Scalar::Bool(true)));
    assert_eq!(upstream_trigger_entity(&f, "i"), None);
}

#[test]
fn manual_value_needs_one_card_with_a_value() {
    let valueless = flow(vec![manual("m", None), plain("i")], vec![edge("m", "i")]);
    assert_eq!(upstream_manual_value(&valueless, "i"), None);
    let mixed = flow(
        vec![
            manual("m", Some(Scalar::Bool(true))),
            state("s", "e"),
            plain("i"),
        ],
        vec![edge("m", "i"), edge("s", "i")],
    );
    assert_eq!(upstream_manual_value(&mixed, "i"), None);
    let two = flow(
        vec![
            manual("a", Some(Scalar::Bool(true))),
            manual("b", Some(Scalar::Number(21.5))),
            plain("i"),
        ],
        vec![edge("a", "i"), edge("b", "i")],
    );
    assert_eq!(upstream_manual_value(&two, "i"), None);
    let same = flow(
        vec![
            manual("a", Some(Scalar::Number(1.0))),
            manual("b", Some(Scalar::Number(2.0))),
            plain("i"),
        ],
        vec![edge("a", "i"), edge("b", "i")],
    );
    assert!(upstream_manual_value(&same, "i").is_some());
}

#[test]
fn alarm_trigger_offers_the_alarm_states() {
    use nidoo_core::flow::TriggerAlarmParams;
    let alarm = flow(
        vec![
            node("t", NodeKind::TriggerAlarm(TriggerAlarmParams::default())),
            plain("i"),
        ],
        vec![edge("t", "i")],
    );
    let l = list(upstream_value_input(Lang::Fr, &alarm, "i").unwrap());
    assert_eq!(l.len(), 7);
    assert_eq!(l[0].value, json!("disarmed"));
    assert_eq!(l[0].label, "Désactivée");
    assert!(l.iter().all(|c| c.value != json!("armed")));
    let other = flow(vec![schedule("t"), plain("i")], vec![edge("t", "i")]);
    assert_eq!(upstream_value_input(Lang::En, &other, "i"), None);
}

#[test]
fn manual_value_input_follows_the_value_kind() {
    assert_eq!(
        manual_value_input(Lang::En, &Scalar::Number(1.0)),
        ValueInput::Number { unit: None }
    );
    assert_eq!(
        manual_value_input(Lang::En, &Scalar::Text("x".into())),
        ValueInput::Text
    );
    let l = list(manual_value_input(Lang::En, &Scalar::Bool(true)));
    assert_eq!(l.len(), 2);
    assert_eq!(
        (l[0].value.clone(), l[1].value.clone()),
        (json!(true), json!(false))
    );
}
