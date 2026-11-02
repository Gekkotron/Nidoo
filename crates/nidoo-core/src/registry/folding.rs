//! `part_of`: which entities fold into another one's card, the way the app computes it.
//!
//! - A device with exactly one light folds its numbers and selects into that light (a strip's
//!   "Effect speed" belongs in the light's sheet). Two lights make the owner ambiguous: no fold.
//! - Zigbee2MQTT publishes a cover and a separate "Moving" sensor off the same state payload: a
//!   sensor folds into the cover of its device when both read the same `state_topic` and the
//!   sensor's `value_template` reads exactly one JSON field that the cover's template also reads.
//!   A binary sensor with `device_class: moving` on a device with exactly one cover folds too.
//! - Hub groups never fold: they all share the hub's one `nidoo_group` device, so a single light
//!   group would otherwise swallow every number or select group.
//!
//! Recomputed from scratch after every change, so the result is the same whatever order the
//! cover and its sensor arrived in, and clears when either goes away. The cover rule wins a
//! conflict: it is applied last.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::model::{Entity, Kind};

/// The device every hub group is published on (`groups/discovery.rs` in nidoo).
const GROUPS_DEVICE: &str = "nidoo_group";

/// `unique_id → owner unique_id` for every folded entity.
pub(super) fn compute(entities: &BTreeMap<String, Entity>) -> BTreeMap<String, String> {
    let mut part_of = BTreeMap::new();
    let light_of = single_per_device(entities, Kind::Light);
    for entity in entities.values().filter(|e| folds(e)) {
        if !matches!(entity.kind, Kind::Number | Kind::Select) {
            continue;
        }
        if let Some(light) = entity.device_id.as_deref().and_then(|d| light_of.get(d)) {
            part_of.insert(entity.unique_id.clone(), (*light).to_owned());
        }
    }
    let cover_of = single_per_device(entities, Kind::Cover);
    let covers: Vec<&Entity> = entities
        .values()
        .filter(|e| e.kind == Kind::Cover)
        .collect();
    for entity in entities.values().filter(|e| folds(e)) {
        let Some(device_id) = entity.device_id.as_deref() else {
            continue;
        };
        let owner = match entity.kind {
            Kind::Sensor => covers
                .iter()
                .find(|cover| reads_a_field_of(entity, cover, device_id))
                .map(|cover| cover.unique_id.as_str()),
            Kind::BinarySensor if entity.device_class.as_deref() == Some("moving") => {
                cover_of.get(device_id).copied()
            }
            _ => None,
        };
        if let Some(owner) = owner {
            part_of.insert(entity.unique_id.clone(), owner.to_owned());
        }
    }
    part_of
}

/// False for the entities that never fold: the hub's groups. Only an entity of the same device
/// folds into an owner, so skipping them here also keeps group owners out.
fn folds(entity: &Entity) -> bool {
    entity.device_id.as_deref() != Some(GROUPS_DEVICE)
}

/// `device_id → unique_id` for the devices holding exactly one entity of `kind`.
fn single_per_device(entities: &BTreeMap<String, Entity>, kind: Kind) -> BTreeMap<&str, &str> {
    let mut per_device: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for entity in entities.values().filter(|e| e.kind == kind) {
        if let Some(device_id) = entity.device_id.as_deref() {
            per_device
                .entry(device_id)
                .or_default()
                .push(&entity.unique_id);
        }
    }
    per_device
        .into_iter()
        .filter_map(|(device, ids)| match ids.as_slice() {
            [single] => Some((device, *single)),
            _ => None,
        })
        .collect()
}

/// True when `sensor` and `cover` share `device_id` and `state_topic`, and the sensor's
/// `value_template` reads exactly one field that the cover's template also reads.
fn reads_a_field_of(sensor: &Entity, cover: &Entity, device_id: &str) -> bool {
    if cover.device_id.as_deref() != Some(device_id) {
        return false;
    }
    let (Some(sensor_topic), Some(cover_topic)) = (
        config_str(sensor, "state_topic"),
        config_str(cover, "state_topic"),
    ) else {
        return false;
    };
    if sensor_topic != cover_topic {
        return false;
    }
    let Some(field) = config_str(sensor, "value_template").and_then(sole_field) else {
        return false;
    };
    config_str(cover, "value_template").is_some_and(|template| references_field(template, field))
}

fn config_str<'a>(entity: &'a Entity, key: &str) -> Option<&'a str> {
    entity.bindings.config.get(key).and_then(Value::as_str)
}

/// The field name when `template` is exactly `{{ value_json.<name> }}` or
/// `{{ value_json["<name>"] }}` (single quotes accepted), with any spacing.
fn sole_field(template: &str) -> Option<&str> {
    let inner = template
        .trim()
        .strip_prefix("{{")?
        .strip_suffix("}}")?
        .trim();
    let rest = inner.strip_prefix("value_json")?;
    let name = match rest.strip_prefix('.') {
        Some(name) => name,
        None => quoted(rest.strip_prefix('[')?.strip_suffix(']')?.trim())?,
    };
    is_word(name).then_some(name)
}

/// True when `template` reads `value_json.<field>` or `value_json["<field>"]` anywhere.
fn references_field(template: &str, field: &str) -> bool {
    template.match_indices("value_json").any(|(at, _)| {
        let rest = &template[at + "value_json".len()..];
        if let Some(after) = rest.strip_prefix('.').and_then(|r| r.strip_prefix(field)) {
            return !after.starts_with(is_word_char);
        }
        rest.strip_prefix('[')
            .map(str::trim_start)
            .and_then(quoted_prefix)
            .is_some_and(|(name, after)| name == field && after.trim_start().starts_with(']'))
    })
}

/// `"name"` or `'name'` → `name`.
fn quoted(text: &str) -> Option<&str> {
    let (name, rest) = quoted_prefix(text)?;
    rest.is_empty().then_some(name)
}

/// A leading quoted string and what follows it.
fn quoted_prefix(text: &str) -> Option<(&str, &str)> {
    let quote = text.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let body = &text[quote.len_utf8()..];
    let end = body.find(quote)?;
    Some((&body[..end], &body[end + quote.len_utf8()..]))
}

fn is_word(text: &str) -> bool {
    !text.is_empty() && text.chars().all(is_word_char)
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}
