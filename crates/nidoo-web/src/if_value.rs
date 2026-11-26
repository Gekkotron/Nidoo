//! The values an If card can compare an entity with, as core's [`nidoo_core::vocabulary`]
//! lists them from discovery, worded for the screen; plus the rules the card follows when the
//! value is typed by hand or the entity changes.

use std::collections::{BTreeMap, BTreeSet};

use nidoo_api::EntityView;
use nidoo_core::flow::{AlarmModeFilter, Flow, NodeKind, Scalar, field_scalar, fields};
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

/// The `<option>` value of "Other value…" in a value list.
pub const OTHER_OPTION: &str = "other";

/// What a pick in the value list means.
#[derive(Clone, Debug, PartialEq)]
pub enum ListPick {
    /// The "no filter" first option ("Any change").
    Any,
    /// "Other value…": switch to a typed box.
    Other,
    Value(Value),
}

/// The meaning of the `<option>` value `picked`: "" is [`ListPick::Any`] when `any` is offered,
/// [`OTHER_OPTION`] is [`ListPick::Other`], an index is that choice's value; else `None`.
pub fn list_pick(picked: &str, choices: &[Choice], any: bool) -> Option<ListPick> {
    match picked {
        "" if any => Some(ListPick::Any),
        OTHER_OPTION => Some(ListPick::Other),
        _ => picked
            .parse::<usize>()
            .ok()
            .and_then(|i| choices.get(i))
            .map(|c| ListPick::Value(c.value.clone())),
    }
}

/// What leaving a number box means: `Some(Some(n))` writes the number, `Some(None)` clears
/// the filter (a truly empty box), `None` keeps the saved value (a typo, which a browser
/// reports as "" with `bad_input`).
pub fn number_pick(text: &str, bad_input: bool) -> Option<Option<Value>> {
    let text = text.trim();
    if text.is_empty() {
        return (!bad_input).then_some(None);
    }
    parse_decimal(text).map(|n| Some(Scalar::Number(n).to_json()))
}

/// Whether a number box can show `value`: nothing, or a number. Anything else (an old
/// `"unavailable"`) needs a text box, so it stays visible.
pub fn fits_number(value: Option<&Value>) -> bool {
    value.is_none_or(Value::is_number)
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
        Kind::MediaPlayer => StateCtx::Media,
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
                    | NodeKind::TriggerWakeUp(_)
                    | NodeKind::TriggerManual(_)
                    | NodeKind::TriggerSignal(_)
                    | NodeKind::TriggerAlarm(_)),
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

/// The input for comparing `e`, or `upstream` (what a Manual or Alarm card upstream carries)
/// when there is no entity.
pub fn compared_input(
    lang: Lang,
    e: Option<&EntityView>,
    upstream: Option<ValueInput>,
) -> ValueInput {
    match (e, upstream) {
        (None, Some(input)) => input,
        _ => value_input(lang, e),
    }
}

/// The states the alarm reports, as a list to pick from.
fn alarm_value_input(lang: Lang) -> ValueInput {
    ValueInput::List(
        AlarmModeFilter::ALL
            .into_iter()
            .filter(|m| *m != AlarmModeFilter::Armed)
            .map(|m| Choice {
                value: Value::String(m.as_str().to_owned()),
                label: state_word(lang, StateCtx::Alarm, m.as_str()),
            })
            .collect(),
    )
}

/// The input for the value a run carries to `node_id`: the Manual card's kind of value, or the
/// alarm states when every path back from it starts at an Alarm card.
pub fn upstream_value_input(lang: Lang, flow: &Flow, node_id: &str) -> Option<ValueInput> {
    if let Some(value) = upstream_manual_value(flow, node_id) {
        return Some(manual_value_input(lang, &value));
    }
    let triggers = upstream_triggers(flow, node_id);
    (!triggers.is_empty()
        && triggers
            .iter()
            .all(|k| matches!(k, NodeKind::TriggerAlarm(_))))
    .then(|| alarm_value_input(lang))
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
mod tests;
