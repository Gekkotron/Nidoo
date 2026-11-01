//! Helpers shared by the mappers: the message being applied, config lookup (re-exported from
//! [`crate::config`]), templated reads and payload parsing.

use serde_json::{Map, Value};

use super::{Applied, StateError};
use crate::template::{TemplateError, Templates};

/// What reading a topic of the entity yielded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Read {
    /// The topic is not the one named by the config key.
    Other,
    /// The message carries nothing for this entity: the template read a key the payload lacks,
    /// or rendered an empty string.
    Missing,
    /// The template rendered `None`: the device reports the value as unknown.
    None,
    /// The value, trimmed when it came through a template.
    Value(String),
}

/// One MQTT message seen through an entity's config.
#[derive(Clone, Copy)]
pub(crate) struct Message<'a> {
    pub(crate) templates: &'a Templates,
    pub(crate) config: &'a Value,
    pub(crate) topic: &'a str,
    pub(crate) payload: &'a str,
}

impl Message<'_> {
    /// True when `config[key]` names this message's topic.
    pub(crate) fn is_topic(&self, key: &str) -> bool {
        self.config.get(key).and_then(Value::as_str) == Some(self.topic)
    }

    /// Renders `template` against the payload; `Ok(None)` when it reads a key the payload lacks.
    pub(crate) fn render(&self, template: &str) -> Result<Option<String>, StateError> {
        match self.templates.render(template, self.payload) {
            Ok(rendered) => Ok(Some(rendered)),
            Err(TemplateError::Undefined(_)) => Ok(None),
            Err(err) => Err(StateError::Template(err)),
        }
    }

    /// Reads the topic named by `config[topic_key]` through the first template present among
    /// `template_keys` (the raw payload when there is none).
    pub(crate) fn read(&self, topic_key: &str, template_keys: &[&str]) -> Result<Read, StateError> {
        if !self.is_topic(topic_key) {
            return Ok(Read::Other);
        }
        let template = template_keys
            .iter()
            .find_map(|key| self.config.get(key).and_then(Value::as_str));
        let value = match template {
            Some(template) => match self.render(template)? {
                Some(rendered) => rendered.trim().to_owned(),
                None => return Ok(Read::Missing),
            },
            None => self.payload.to_owned(),
        };
        Ok(if value.is_empty() {
            Read::Missing
        } else if value.eq_ignore_ascii_case("none") {
            Read::None
        } else {
            Read::Value(value)
        })
    }

    /// Reads the topic at `topic_key` into a numeric `slot` (`template_key`, then
    /// `value_template`), clearing it on a rendered `None`; `applied` records the outcome.
    pub(crate) fn number_field(
        &self,
        topic_key: &str,
        template_key: &str,
        applied: &mut Applied,
        slot: &mut Option<f64>,
    ) -> Result<(), StateError> {
        match self.read(topic_key, &[template_key, "value_template"])? {
            Read::Other => {}
            Read::Value(text) => match parse_number(&text) {
                Some(number) => {
                    *slot = Some(number);
                    *applied = Applied::Changed;
                }
                None => *applied = strongest(*applied, Applied::Unchanged),
            },
            Read::None => {
                *slot = None;
                *applied = Applied::Changed;
            }
            Read::Missing => *applied = strongest(*applied, Applied::Unchanged),
        }
        Ok(())
    }

    /// Like [`number_field`](Self::number_field) for a text `slot`.
    pub(crate) fn text_field(
        &self,
        topic_key: &str,
        template_key: &str,
        applied: &mut Applied,
        slot: &mut Option<String>,
    ) -> Result<(), StateError> {
        match self.read(topic_key, &[template_key, "value_template"])? {
            Read::Other => {}
            Read::Value(text) => {
                *slot = Some(text);
                *applied = Applied::Changed;
            }
            Read::None => {
                *slot = None;
                *applied = Applied::Changed;
            }
            Read::Missing => *applied = strongest(*applied, Applied::Unchanged),
        }
        Ok(())
    }
}

pub(crate) use crate::config::{config_f64, config_string, config_string_or, parse_number};

/// The payload as a JSON object.
pub(crate) fn parse_object(payload: &str) -> Option<Map<String, Value>> {
    serde_json::from_str(payload).ok()
}

/// `Some(true)` when `text` is `on`, `Some(false)` when `off` (ASCII case-insensitive, since
/// rendered booleans may come out as `True`/`False`), `None` otherwise.
pub(crate) fn on_off(text: &str, on: &str, off: &str) -> Option<bool> {
    if text.eq_ignore_ascii_case(on) {
        Some(true)
    } else if text.eq_ignore_ascii_case(off) {
        Some(false)
    } else {
        None
    }
}

/// A finite number rounded and clamped into `0..=255`.
pub(crate) fn to_u8_255(value: f64) -> Option<u8> {
    value
        .is_finite()
        .then(|| value.round().clamp(0.0, 255.0) as u8)
}

/// A finite number rounded and clamped into `0..=100`.
pub(crate) fn to_u8_100(value: f64) -> Option<u8> {
    value
        .is_finite()
        .then(|| value.round().clamp(0.0, 100.0) as u8)
}

/// The stronger of two outcomes: `Changed` over `Unchanged` over `NotForThisEntity`.
pub(crate) fn strongest(a: Applied, b: Applied) -> Applied {
    match (a, b) {
        (Applied::Changed, _) | (_, Applied::Changed) => Applied::Changed,
        (Applied::Unchanged, _) | (_, Applied::Unchanged) => Applied::Unchanged,
        _ => Applied::NotForThisEntity,
    }
}
