//! Reads MQTT messages into a [`GenericState`] following the type table.

use serde_json::Value;

use crate::model::{Entity, EntityState, GenericState, GenericValue, StateValue};
use crate::state::util::{self, Message, Read};
use crate::state::{Applied, StateError};
use crate::template::{TemplateError, Templates};
use crate::types::{FieldDef, Shape, TypeDef, WordDef};

/// The word `word` stands for in this config: the overriding config value, else the default.
fn word_text<'a>(cfg: &'a Value, word: &'a WordDef) -> &'a str {
    word.key
        .as_deref()
        .and_then(|key| cfg.get(key))
        .and_then(Value::as_str)
        .unwrap_or(&word.default)
}

/// What one field does with a value read from its topic.
enum Update {
    Set(GenericValue),
    Remove,
    Nothing,
}

/// The string at the field's `json_path` when the rendered value is a JSON object with it.
fn json_key(field: &FieldDef, value: &str) -> Option<String> {
    let key = field.json_path.as_deref()?;
    let Value::Object(map) = serde_json::from_str::<Value>(value).ok()? else {
        return None;
    };
    match map.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

fn read_field(cfg: &Value, field: &FieldDef, value: &str) -> Update {
    match field.shape {
        Shape::OnOff => {
            let on = field.on.as_deref();
            field
                .values
                .iter()
                .find(|w| word_text(cfg, w).eq_ignore_ascii_case(value))
                .map_or(Update::Nothing, |w| {
                    Update::Set(GenericValue::Bool(Some(w.value.as_str()) == on))
                })
        }
        Shape::Choice => {
            let canonical = field
                .values
                .iter()
                .find(|w| word_text(cfg, w) == value)
                .map_or(value, |w| w.value.as_str());
            Update::Set(GenericValue::Text(canonical.to_owned()))
        }
        Shape::Number => match util::parse_number(value) {
            Some(n) if n.is_finite() => Update::Set(GenericValue::Number(n)),
            Some(_) => Update::Remove,
            None => Update::Nothing,
        },
        Shape::Text | Shape::Version => Update::Set(GenericValue::Text(
            json_key(field, value).unwrap_or_else(|| value.to_owned()),
        )),
        // The picture bytes are the hub's business.
        Shape::Picture => Update::Nothing,
    }
}

/// Applies the message to every field of the type whose topic it is, merging into the
/// entity's [`GenericState`].
pub fn apply(
    entity: &Entity,
    def: &TypeDef,
    state: &mut EntityState,
    topic: &str,
    payload: &str,
    templates: &Templates,
) -> Result<Applied, StateError> {
    let cfg = &entity.bindings.config;
    let message = Message {
        templates,
        config: cfg,
        topic,
        payload,
    };
    let mut applied = Applied::NotForThisEntity;
    let mut updates = Vec::new();
    for (name, field) in &def.fields {
        let template: &[&str] = match field.template.as_deref() {
            Some(key) => &[key],
            None => &[],
        };
        let read = match message.read(&field.topic, template) {
            // A template that fails on this payload (a key it lacks, an operation on it) says
            // nothing about this field; only a syntax error is the entity's own fault.
            Err(StateError::Template(TemplateError::Other(_))) => Ok(Read::Missing),
            other => other,
        };
        let update = match read? {
            Read::Other => continue,
            Read::Missing => Update::Nothing,
            Read::None => Update::Remove,
            Read::Value(value) => read_field(cfg, field, &value),
        };
        applied = util::strongest(applied, Applied::Unchanged);
        updates.push((name, update));
    }
    for (name, update) in updates {
        if !matches!(state.value, StateValue::Generic(_)) && matches!(update, Update::Set(_)) {
            state.value = StateValue::Generic(GenericState::default());
        }
        let StateValue::Generic(generic) = &mut state.value else {
            continue;
        };
        match update {
            Update::Set(value) => {
                generic.fields.insert(name.clone(), value);
                applied = Applied::Changed;
            }
            Update::Remove => {
                if generic.fields.remove(name).is_some() {
                    applied = Applied::Changed;
                }
            }
            Update::Nothing => {}
        }
    }
    Ok(applied)
}

/// The field shown as the card value, when it has been reported.
pub fn primary<'a>(def: &TypeDef, state: &'a GenericState) -> Option<&'a GenericValue> {
    def.primary
        .as_deref()
        .and_then(|name| state.fields.get(name))
}
