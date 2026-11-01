//! Expansion of a raw discovery config: abbreviated keys and the `~` base topic.

use serde_json::{Map, Value};

use super::abbreviations::{Table, rename, tables};

/// Replaces abbreviated keys at every nesting level (top level, `availability[]`, `device`,
/// `origin` and each `components` entry) and resolves a leading or trailing `~` in the string
/// values with the `~` key's value; the `~` key itself is dropped.
pub fn expand(raw: Value) -> Value {
    match raw {
        Value::Object(object) => Value::Object(expand_object(object)),
        other => other,
    }
}

/// [`expand`] for a config already known to be an object.
pub(super) fn expand_object(object: Map<String, Value>) -> Map<String, Value> {
    let mut object = expand_keys(object);
    let base = take_base(&mut object);
    if let Some(base) = &base {
        resolve_tilde(&mut object, base);
    }
    if let Some(Value::Object(components)) = object.get_mut("components") {
        for entry in components.values_mut().filter_map(Value::as_object_mut) {
            if let Some(own) = take_base(entry).or_else(|| base.clone()) {
                resolve_tilde(entry, &own);
            }
        }
    }
    object
}

/// Renames the keys of one entity config and of the objects nested in it.
fn expand_keys(object: Map<String, Value>) -> Map<String, Value> {
    let tables = tables();
    let mut object = rename(object, &tables.payload);
    for (key, value) in object.iter_mut() {
        match key.as_str() {
            "device" => rename_in_place(value, &tables.device),
            "origin" => rename_in_place(value, &tables.origin),
            "availability" => {
                if let Value::Array(entries) = value {
                    for entry in entries.iter_mut() {
                        rename_in_place(entry, &tables.payload);
                    }
                }
            }
            "components" => {
                if let Value::Object(components) = value {
                    for config in components.values_mut().filter_map(Value::as_object_mut) {
                        *config = expand_keys(std::mem::take(config));
                    }
                }
            }
            _ => {}
        }
    }
    object
}

fn rename_in_place(value: &mut Value, table: &Table) {
    if let Value::Object(object) = value {
        *object = rename(std::mem::take(object), table);
    }
}

/// Removes the `~` key and returns its string value.
fn take_base(object: &mut Map<String, Value>) -> Option<String> {
    match object.remove("~") {
        Some(Value::String(base)) => Some(base),
        _ => None,
    }
}

/// Resolves `~` in the top-level string values and in each `availability[]` entry.
fn resolve_tilde(object: &mut Map<String, Value>, base: &str) {
    for (key, value) in object.iter_mut() {
        match value {
            Value::String(text) => resolve_in(text, base),
            Value::Array(entries) if key == "availability" => {
                for entry in entries.iter_mut().filter_map(Value::as_object_mut) {
                    for text in entry.values_mut().filter_map(string_mut) {
                        resolve_in(text, base);
                    }
                }
            }
            _ => {}
        }
    }
}

fn resolve_in(text: &mut String, base: &str) {
    if let Some(rest) = text.strip_prefix('~') {
        *text = format!("{base}{rest}");
    } else if let Some(rest) = text.strip_suffix('~') {
        *text = format!("{rest}{base}");
    }
}

fn string_mut(value: &mut Value) -> Option<&mut String> {
    match value {
        Value::String(text) => Some(text),
        _ => None,
    }
}
