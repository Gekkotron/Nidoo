//! Availability from `availability_topic` or the `availability[]` list, combined per
//! `availability_mode`.

use serde_json::{Map, Value};

use super::util::{self, Message};
use super::{Applied, StateError};
use crate::model::{Availability, EntityState};

/// Reserved attribute holding the last word of each availability topic (`online`/`offline`)
/// when an entity has several and combines them with `all` or `any`.
pub const AVAILABILITY_ATTRIBUTE: &str = "__availability";

const ONLINE: &str = "online";
const OFFLINE: &str = "offline";

/// One availability topic with its words and template.
struct Source {
    topic: String,
    template: Option<String>,
    available: String,
    not_available: String,
}

/// The availability topics of a config: the `availability[]` list when present, else
/// `availability_topic`.
fn sources(config: &Value) -> Vec<Source> {
    let list: Vec<Source> = config
        .get("availability")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    Some(Source {
                        topic: entry.get("topic")?.as_str()?.to_owned(),
                        template: entry
                            .get("value_template")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        available: util::config_string_or(entry, "payload_available", ONLINE),
                        not_available: util::config_string_or(
                            entry,
                            "payload_not_available",
                            OFFLINE,
                        ),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if !list.is_empty() {
        return list;
    }
    config
        .get("availability_topic")
        .and_then(Value::as_str)
        .map(|topic| Source {
            topic: topic.to_owned(),
            template: config
                .get("availability_template")
                .and_then(Value::as_str)
                .map(str::to_owned),
            available: util::config_string_or(config, "payload_available", ONLINE),
            not_available: util::config_string_or(config, "payload_not_available", OFFLINE),
        })
        .into_iter()
        .collect()
}

/// Applies an availability message; `NotForThisEntity` when the topic is none of the entity's
/// availability topics.
pub(super) fn apply(message: &Message<'_>, state: &mut EntityState) -> Result<Applied, StateError> {
    let config = message.config;
    let sources = sources(config);
    let Some(source) = sources.iter().find(|source| source.topic == message.topic) else {
        return Ok(Applied::NotForThisEntity);
    };
    let word = match &source.template {
        Some(template) => match message.render(template)? {
            Some(rendered) => rendered.trim().to_owned(),
            None => return Ok(Applied::Unchanged),
        },
        None => message.payload.to_owned(),
    };
    let Some(online) = util::on_off(&word, &source.available, &source.not_available) else {
        return Ok(Applied::Unchanged);
    };
    let mode = config
        .get("availability_mode")
        .and_then(Value::as_str)
        .unwrap_or("latest");
    let combined = if sources.len() > 1 && matches!(mode, "all" | "any") {
        let record = record(state, message.topic, online);
        let seen =
            |source: &Source| record.get(&source.topic).and_then(Value::as_str) == Some(ONLINE);
        if mode == "all" {
            sources.iter().all(seen)
        } else {
            sources.iter().any(seen)
        }
    } else {
        online
    };
    let next = if combined {
        Availability::Online
    } else {
        Availability::Offline
    };
    if state.availability == next {
        return Ok(Applied::Unchanged);
    }
    state.availability = next;
    Ok(Applied::Changed)
}

/// Records `topic`'s latest word under [`AVAILABILITY_ATTRIBUTE`] and returns the record.
fn record(state: &mut EntityState, topic: &str, online: bool) -> Map<String, Value> {
    let mut map = match state.attributes.remove(AVAILABILITY_ATTRIBUTE) {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    let word = if online { ONLINE } else { OFFLINE };
    map.insert(topic.to_owned(), Value::from(word));
    state.attributes.insert(
        AVAILABILITY_ATTRIBUTE.to_owned(),
        Value::Object(map.clone()),
    );
    map
}
