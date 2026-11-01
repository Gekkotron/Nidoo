//! Commands for generic entities: one `Command::Control` carries out any control the type
//! table describes, with the payload words, templates and ranges of the entity's config.

use serde_json::{Value, json};

use crate::command::{CommandError, Mapper, Publish, in_range, listed};
use crate::config::config_string_or;
use crate::model::{Command, Entity};
use crate::types::{ControlDef, Shape, TypeDef, WordDef};

/// The word of a payload definition: the config's override or the Home Assistant default.
fn word(m: &Mapper<'_>, def: &WordDef) -> String {
    match def.key.as_deref() {
        Some(key) => config_string_or(m.config, key, &def.default),
        None => def.default.clone(),
    }
}

fn out_of_range(what: &str) -> CommandError {
    CommandError::OutOfRange(what.to_owned())
}

/// Maps a `Command::Control` of an entity of type `def` to its publishes.
pub(crate) fn map(m: &Mapper<'_>, def: &TypeDef) -> Result<Vec<Publish>, CommandError> {
    let Command::Control { control, value } = m.command else {
        return Err(m.unsupported());
    };
    let ctl = def.controls.get(control).ok_or_else(|| m.unsupported())?;
    let caps = m.entity.capabilities.generic.as_ref();
    if !caps.is_some_and(|g| g.controls.iter().any(|c| c == control)) {
        return Err(m.unsupported());
    }
    let payload = payload(m, ctl, control, value.as_ref())?;
    let payload = match &ctl.json_key {
        Some(key) => serde_json::to_string(&json!({ key: payload }))
            .map_err(|e| CommandError::Template(e.to_string()))?,
        None => payload_text(&payload),
    };
    Ok(vec![m.publish(
        &ctl.topic,
        ctl.template.as_deref(),
        &payload,
        &[],
    )?])
}

fn payload_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The checked payload value (a JSON value, so `json_key` can wrap it as it is).
fn payload(
    m: &Mapper<'_>,
    ctl: &ControlDef,
    control: &str,
    value: Option<&Value>,
) -> Result<Value, CommandError> {
    let caps = m.entity.capabilities.generic.as_ref();
    if ctl.press {
        if value.is_some_and(|v| !v.is_null()) {
            return Err(out_of_range("press takes no value"));
        }
        let def = ctl.payload.as_ref().ok_or_else(|| m.unsupported())?;
        return Ok(Value::String(word(m, def)));
    }
    match ctl.shape {
        Shape::OnOff => {
            let Some(Value::Bool(on)) = value else {
                return Err(out_of_range("expected true or false"));
            };
            let def = if *on {
                &ctl.payload_on
            } else {
                &ctl.payload_off
            };
            let def = def.as_ref().ok_or_else(|| m.unsupported())?;
            Ok(Value::String(word(m, def)))
        }
        Shape::Number => {
            let Some(Value::Number(n)) = value else {
                return Err(out_of_range("expected a number"));
            };
            let number = n.as_f64().ok_or_else(|| out_of_range("not a number"))?;
            let range = ctl
                .field
                .as_deref()
                .and_then(|f| caps.and_then(|g| g.ranges.get(f)));
            in_range(number, range.map(|r| r.min), range.map(|r| r.max), control)?;
            Ok(Value::Number(n.clone()))
        }
        Shape::Choice => {
            let Some(Value::String(s)) = value else {
                return Err(out_of_range("expected a string"));
            };
            let field_options = ctl
                .field
                .as_deref()
                .and_then(|f| caps.and_then(|g| g.options.get(f)))
                .cloned();
            let options = field_options.unwrap_or_else(|| {
                ctl.requires
                    .as_deref()
                    .and_then(|k| m.config.get(k))
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default()
            });
            listed(s, &options, control)?;
            Ok(Value::String(s.clone()))
        }
        Shape::Text => {
            let Some(Value::String(s)) = value else {
                return Err(out_of_range("expected a string"));
            };
            let len = s.chars().count() as f64;
            let bound = |key: &str| m.config.get(key).and_then(Value::as_f64);
            in_range(len, bound("min"), bound("max"), "text length")?;
            Ok(Value::String(s.clone()))
        }
        Shape::Picture | Shape::Version => Err(m.unsupported()),
    }
}

/// True when carrying out the control is followed by a state report to wait for: the control
/// writes a field whose state topic the entity has.
pub fn expects_echo(def: &TypeDef, entity: &Entity, control_name: &str) -> bool {
    let Some(field) = def
        .controls
        .get(control_name)
        .and_then(|ctl| ctl.field.as_deref())
        .and_then(|f| def.fields.get(f))
    else {
        return false;
    };
    entity
        .bindings
        .config
        .get(field.topic.as_str())
        .is_some_and(Value::is_string)
}
