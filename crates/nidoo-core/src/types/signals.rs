//! Signals: the named events a remote press, an `event` entity or a tag scan carries.

use serde_json::Value;

use crate::model::Entity;
use crate::template::Templates;
use crate::types::{Placement, TypeDef};

/// Returns the signal name a message carries, if it is a signal from `entity`.
///
/// A `device_automation` remote signals the payload of the action whose topic and payload match;
/// an `event` signals its (templated) `event_type`; a `tag` signals the (templated) tag id.
pub fn signal_of(
    entity: &Entity,
    def: &TypeDef,
    topic: &str,
    payload: &[u8],
    templates: &Templates,
) -> Option<String> {
    if def.placement != Placement::Signal {
        return None;
    }
    let text = std::str::from_utf8(payload).ok()?;
    let cfg = &entity.bindings.config;
    let signal = match entity.component.as_str() {
        "device_automation" => {
            return cfg
                .get("actions")
                .and_then(Value::as_array)?
                .iter()
                .find(|action| {
                    action.get("topic").and_then(Value::as_str) == Some(topic)
                        && action.get("payload").and_then(Value::as_str) == Some(text)
                })
                .map(|_| text.to_owned());
        }
        "event" => {
            if cfg.get("state_topic").and_then(Value::as_str) != Some(topic) {
                return None;
            }
            match cfg.get("value_template").and_then(Value::as_str) {
                Some(template) => templates.render(template, text).ok()?,
                None => serde_json::from_str::<Value>(text)
                    .ok()?
                    .get("event_type")?
                    .as_str()?
                    .to_owned(),
            }
        }
        "tag" => {
            if cfg.get("topic").and_then(Value::as_str) != Some(topic) {
                return None;
            }
            match cfg.get("value_template").and_then(Value::as_str) {
                Some(template) => templates.render(template, text).ok()?,
                None => text.to_owned(),
            }
        }
        _ => return None,
    };
    let signal = signal.trim();
    (!signal.is_empty()).then(|| signal.to_owned())
}
