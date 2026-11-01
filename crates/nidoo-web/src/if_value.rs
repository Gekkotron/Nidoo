//! The values an If card can compare an entity with, as core's [`nidoo_core::vocabulary`]
//! lists them from discovery, worded for the screen; plus the rules the card follows when the
//! value is typed by hand or the entity changes.

use std::collections::{BTreeMap, BTreeSet};

use nidoo_api::EntityView;
use nidoo_core::flow::{Flow, NodeKind, Scalar, field_scalar, fields};
use nidoo_core::model::{Kind, StateValue};
use nidoo_core::types::{Placement, type_def};
use nidoo_core::vocabulary::{Reports, reports};
use serde_json::Value;

use crate::format::{entity_value, shown_name, type_word};
use crate::i18n::Lang;
use crate::i18n::flows::field_label;
use crate::i18n::format::{number, parse_decimal, with_unit};
use crate::i18n::states::{StateCtx, on_off, state_word};

/// One entry of the value list: what is stored and what the user reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub value: Value,
    pub label: String,
}

/// How the value is picked.
#[derive(Clone, Debug, PartialEq)]
pub enum ValueInput {
    List(Vec<Choice>),
    Number { unit: Option<String> },
    Text,
}

/// The input for comparing `e`; a text box when the entity is unknown.
pub fn value_input(lang: Lang, e: Option<&EntityView>) -> ValueInput {
    let Some(e) = e else {
        return ValueInput::Text;
    };
    input_for(
        lang,
        e,
        reports(e.kind, &e.component, &e.capabilities, e.unit.as_deref()),
    )
}

/// The input for comparing `field` of `e` (its headline value when `None`); a text box when
/// the entity is unknown or does not report the field.
pub fn value_input_of(lang: Lang, e: Option<&EntityView>, field: Option<&str>) -> ValueInput {
    let (Some(e), Some(field)) = (e, field) else {
        return value_input(lang, e);
    };
    match entity_fields(e).into_iter().find(|f| f.name == field) {
        Some(info) => input_for(lang, e, info.reports),
        None => ValueInput::Text,
    }
}

/// Whether a card on `field` of `e` may average: always on the headline value, else only on
/// the field the history records for the entity.
pub fn average_allowed(e: Option<&EntityView>, field: Option<&str>) -> bool {
    let Some(field) = field else {
        return true;
    };
    e.is_some_and(|e| {
        entity_fields(e)
            .iter()
            .any(|info| info.name == field && info.recorded)
    })
}

fn entity_fields(e: &EntityView) -> Vec<nidoo_core::flow::FieldInfo> {
    fields(e.kind, &e.component, &e.capabilities, e.unit.as_deref())
}

/// The input for a value of `e` that reports `what`.
fn input_for(lang: Lang, e: &EntityView, what: Reports) -> ValueInput {
    match what {
        Reports::Values { values } => ValueInput::List(
            values
                .iter()
                .map(|v| Choice {
                    value: v.to_json(),
                    label: label(lang, e, &v.to_json()),
                })
                .collect(),
        ),
        Reports::Number { unit, .. } => ValueInput::Number { unit },
        // A reading that is a number is compared as one, whatever discovery said.
        Reports::Text | Reports::Nothing => match e.state.value {
            StateValue::Number(_) => ValueInput::Number {
                unit: e.unit.clone(),
            },
            _ => ValueInput::Text,
        },
    }
}

/// What a flow card reads from a device: the entity's headline value, a field of its state, or
/// another entity of the same device.
#[derive(Clone, Debug, PartialEq)]
pub enum ValueOf {
    Headline,
    Field(String),
    Sibling(String),
}

/// One entry of the "Value of" menu.
#[derive(Clone, Debug, PartialEq)]
pub struct ValueOfChoice {
    pub of: ValueOf,
    pub label: String,
}

/// "Battery (62 %)": a label with the current value, the bare label while it is unknown.
fn labelled_now(label: String, current: Option<String>) -> String {
    match current.filter(|v| v != "—") {
        Some(v) => format!("{label} ({v})"),
        None => label,
    }
}

/// Every value the device of `e` reports: `e`'s headline value, its fields, then the device's
/// other entities (diagnostic ones included, those announced disabled by default such as link
/// quality last, signal ones left out), each labelled with its current value.
pub fn value_of_choices(
    lang: Lang,
    entities: &BTreeMap<String, EntityView>,
    e: &EntityView,
) -> Vec<ValueOfChoice> {
    let mut list = vec![ValueOfChoice {
        of: ValueOf::Headline,
        label: labelled_now(type_word(lang, e), Some(entity_value(lang, e))),
    }];
    for info in entity_fields(e) {
        let unit = match &info.reports {
            Reports::Number { unit, .. } => unit.clone(),
            _ => None,
        };
        let current = field_scalar(&e.state.value, &info.name).map(|v| match v {
            Scalar::Number(n) => with_unit(number(lang, n), unit.as_deref()),
            other => label(lang, e, &other.to_json()),
        });
        list.push(ValueOfChoice {
            label: labelled_now(field_label(lang, &info.name), current),
            of: ValueOf::Field(info.name),
        });
    }
    let Some(device) = e.device_id.as_deref() else {
        return list;
    };
    let mut siblings: Vec<(bool, ValueOfChoice)> = entities
        .values()
        .filter(|s| s.id != e.id && s.device_id.as_deref() == Some(device))
        .filter(|s| {
            type_def(&s.component).is_none_or(|def| def.placement != Placement::Signal)
                && reports(s.kind, &s.component, &s.capabilities, s.unit.as_deref())
                    != Reports::Nothing
        })
        .map(|s| {
            let choice = ValueOfChoice {
                of: ValueOf::Sibling(s.id.clone()),
                label: labelled_now(shown_name(lang, s).to_owned(), Some(entity_value(lang, s))),
            };
            (!s.enabled_by_default, choice)
        })
        .collect();
    siblings.sort_by(|(a_off, a), (b_off, b)| (a_off, &a.label).cmp(&(b_off, &b.label)));
    list.extend(siblings.into_iter().map(|(_, choice)| choice));
    list
}

/// The word for a value of `e`: on/off worded by its kind, a state word in its vocabulary.
fn label(lang: Lang, e: &EntityView, value: &Value) -> String {
    match value {
        Value::Bool(on) => match e.kind {
            Kind::BinarySensor => state_word(
                lang,
                StateCtx::BinarySensor(e.device_class.clone()),
                if *on { "on" } else { "off" },
            ),
            kind => on_off(lang, kind == Kind::Light, *on).to_owned(),
        },
        Value::String(word) => state_word(lang, word_context(e.kind), word),
        other => other.to_string(),
    }
}

/// Which vocabulary translates a kind's state words; only the wording depends on it.
fn word_context(kind: Kind) -> StateCtx {
    match kind {
        Kind::Cover => StateCtx::Cover,
        Kind::AlarmPanel => StateCtx::Alarm,
        Kind::Climate | Kind::WaterHeater => StateCtx::Hvac,
        Kind::Vacuum => StateCtx::Vacuum,
        Kind::Generic => StateCtx::Choice,
        _ => StateCtx::Other,
    }
}

/// `input` with `current` first when it is a list that does not hold it, so a saved value
/// stays selected rather than replaced.
pub fn with_current(
    input: ValueInput,
    current: Option<&Value>,
    lang: Lang,
    e: Option<&EntityView>,
) -> ValueInput {
    match (input, current) {
        (ValueInput::List(mut list), Some(current))
            if !list.iter().any(|c| &c.value == current) =>
        {
            let label = match e {
                Some(e) => label(lang, e, current),
                None => current.to_string(),
            };
            list.insert(
                0,
                Choice {
                    value: current.clone(),
                    label,
                },
            );
            ValueInput::List(list)
        }
        (input, _) => input,
    }
}

/// A value typed by hand: a number (decimal comma accepted), `true` / `false`, else text.
/// Blank is nothing.
pub fn parse_other(text: &str) -> Option<Value> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(match (text, parse_decimal(text)) {
        (_, Some(n)) => Scalar::Number(n).to_json(),
        ("true", _) => Value::Bool(true),
        ("false", _) => Value::Bool(false),
        _ => Value::String(text.to_owned()),
    })
}

/// The value after the user picks another entity: kept when the new input can hold it, else
/// the new list's first value, else nothing.
pub fn after_entity_change(current: Option<&Value>, input: &ValueInput) -> Option<Value> {
    match input {
        ValueInput::List(list) => match current {
            Some(v) if list.iter().any(|c| &c.value == v) => Some(v.clone()),
            _ => list.first().map(|c| c.value.clone()),
        },
        ValueInput::Number { .. } => current.filter(|v| v.is_number()).cloned(),
        ValueInput::Text => current.filter(|v| v.is_string()).cloned(),
    }
}

/// The trigger nodes every path back from `node_id` starts at.
fn upstream_triggers<'a>(flow: &'a Flow, node_id: &str) -> Vec<&'a NodeKind> {
    let mut seen = BTreeSet::from([node_id.to_owned()]);
    let mut queue = vec![node_id.to_owned()];
    let mut triggers = Vec::new();
    while let Some(id) = queue.pop() {
        for edge in flow.edges.iter().filter(|e| e.to == id) {
            if !seen.insert(edge.from.clone()) {
                continue;
            }
            match flow.node(&edge.from).map(|n| &n.kind) {
                Some(
                    kind @ (NodeKind::TriggerState(_)
                    | NodeKind::TriggerSchedule(_)
                    | NodeKind::TriggerSun(_)
                    | NodeKind::TriggerManual(_)
                    | NodeKind::TriggerSignal(_)),
                ) => triggers.push(kind),
                _ => queue.push(edge.from.clone()),
            }
        }
    }
    triggers
}

/// The entity whose value reaches `node_id`: the one state trigger every path back from the
/// node starts at. `None` when the paths start at several entities or at another trigger.
pub fn upstream_trigger_entity(flow: &Flow, node_id: &str) -> Option<String> {
    let entities: BTreeSet<Option<String>> = upstream_triggers(flow, node_id)
        .into_iter()
        .map(|kind| match kind {
            NodeKind::TriggerState(t) => Some(t.entity.clone()),
            _ => None,
        })
        .collect();
    match entities.len() {
        1 => entities.into_iter().next().flatten(),
        _ => None,
    }
}

/// The field of the state trigger whose value reaches `node_id`: set when every path back from
/// the node starts at state triggers reading the same field of the same entity.
pub fn upstream_trigger_field(flow: &Flow, node_id: &str) -> Option<String> {
    let reads: BTreeSet<Option<(String, Option<String>)>> = upstream_triggers(flow, node_id)
        .into_iter()
        .map(|kind| match kind {
            NodeKind::TriggerState(t) => Some((t.entity.clone(), t.field.clone())),
            _ => None,
        })
        .collect();
    match reads.len() {
        1 => reads
            .into_iter()
            .next()
            .flatten()
            .and_then(|(_, field)| field),
        _ => None,
    }
}

/// The value a run carries to `node_id` when every path back from it starts at a Manual card
/// with a value of the same kind (on/off, number or text): the first card's value. `None` when a
/// path starts at another trigger, or at a card with no value or another kind of value.
pub fn upstream_manual_value(flow: &Flow, node_id: &str) -> Option<Scalar> {
    let mut values = Vec::new();
    for kind in upstream_triggers(flow, node_id) {
        match kind {
            NodeKind::TriggerManual(p) => values.push(p.value.clone()?),
            _ => return None,
        }
    }
    let first = values.first()?.clone();
    values
        .iter()
        .all(|v| std::mem::discriminant(v) == std::mem::discriminant(&first))
        .then_some(first)
}

/// The input for comparing `e`, or for the value a Manual card carries when there is no entity.
pub fn compared_input(lang: Lang, e: Option<&EntityView>, manual: Option<&Scalar>) -> ValueInput {
    match (e, manual) {
        (None, Some(m)) => manual_value_input(lang, m),
        _ => value_input(lang, e),
    }
}

/// The input for a value that comes from a Manual card: on/off as for a binary sensor, a number
/// box, or a text box.
pub fn manual_value_input(lang: Lang, value: &Scalar) -> ValueInput {
    match value {
        Scalar::Bool(_) => ValueInput::List(
            [true, false]
                .into_iter()
                .map(|on| Choice {
                    value: Value::Bool(on),
                    label: state_word(
                        lang,
                        StateCtx::BinarySensor(None),
                        if on { "on" } else { "off" },
                    ),
                })
                .collect(),
        ),
        Scalar::Number(_) => ValueInput::Number { unit: None },
        Scalar::Text(_) => ValueInput::Text,
    }
}

#[cfg(test)]
mod tests {
    use nidoo_core::flow::{
        Edge, Node, NodeKind, TriggerManualParams, TriggerScheduleParams, TriggerStateParams,
    };
    use nidoo_core::model::{
        Capabilities, EntityCategory, EntityState, Kind, StateValue, UiOverride,
    };
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
}
