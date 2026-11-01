//! State mapping: from an MQTT message to an [`EntityState`], following the Home Assistant
//! MQTT field names an entity was discovered with.
//!
//! [`apply_message`] handles the entity's availability topics and `json_attributes_topic`
//! first, then hands the message to the mapper of the entity's `protocol_kind`. Nothing here
//! panics on a hostile payload: a value that cannot be read leaves the state alone.

pub mod color;

mod availability;
mod kinds;
pub(crate) mod util;

use crate::model::{Entity, EntityState};
use crate::template::{TemplateError, Templates};

pub use availability::AVAILABILITY_ATTRIBUTE;
pub(crate) use kinds::{ALARM_STATES, COVER_STATES, VACUUM_STATES};

/// What a message did to an entity's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Applied {
    /// A value, attribute set or availability was read from the message and stored, so
    /// `last_update` moved. A value equal to the previous one still counts: the device spoke.
    /// Clearing a value to `Unknown` (a rendered `None`) counts only when there was a value,
    /// and an availability message counts only when the availability flips.
    Changed,
    /// The topic belongs to this entity but the message carried nothing it could use: the
    /// payload lacks this entity's key on a shared topic, or its value is not one the config
    /// recognises.
    Unchanged,
    /// The topic is not one this entity reads.
    NotForThisEntity,
}

/// Why a message could not be mapped at all.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StateError {
    /// A template of this entity is broken (syntax, unknown filter) or failed in a way that is
    /// not a missing key: the caller marks the entity unknown and reports it once.
    #[error("template: {0}")]
    Template(TemplateError),
}

/// What each reader of a message did, so the registry can tell a value stored by the kind
/// mapper from an availability or attribute update on the same message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Parts {
    pub(crate) availability: Applied,
    pub(crate) attributes: Applied,
    /// The kind mapper's outcome: `Changed` when it stored a value.
    pub(crate) value: Applied,
}

impl Parts {
    /// The outcome of the message as a whole: the strongest of the three.
    pub(crate) fn overall(self) -> Applied {
        util::strongest(
            util::strongest(self.availability, self.attributes),
            self.value,
        )
    }
}

/// Applies the message published on `topic` to `state`, with `at` as the caller's timestamp
/// for `last_update` when something was stored (a value equal to the previous one included:
/// the device spoke).
///
/// On `Err`, whatever was read before the failing template (availability, attributes) stays
/// applied; the value is untouched.
pub fn apply_message(
    entity: &Entity,
    state: &mut EntityState,
    topic: &str,
    payload: &str,
    at: u64,
    templates: &Templates,
) -> Result<Applied, StateError> {
    apply_parts(entity, state, topic, payload, at, templates).map(Parts::overall)
}

/// [`apply_message`] with the outcome of each reader kept apart.
pub(crate) fn apply_parts(
    entity: &Entity,
    state: &mut EntityState,
    topic: &str,
    payload: &str,
    at: u64,
    templates: &Templates,
) -> Result<Parts, StateError> {
    let message = util::Message {
        templates,
        config: &entity.bindings.config,
        topic,
        payload,
    };
    let parts = Parts {
        availability: availability::apply(&message, state)?,
        attributes: apply_attributes(&message, state)?,
        value: kinds::apply(entity, state, topic, payload, templates)?,
    };
    if parts.overall() == Applied::Changed {
        state.last_update = Some(at);
    }
    Ok(parts)
}

/// Merges the JSON object published on `json_attributes_topic` (after
/// `json_attributes_template`) into the attributes; keys overwrite, reserved `__` keys are
/// kept.
fn apply_attributes(
    message: &util::Message<'_>,
    state: &mut EntityState,
) -> Result<Applied, StateError> {
    let rendered = match message.read("json_attributes_topic", &["json_attributes_template"])? {
        util::Read::Other => return Ok(Applied::NotForThisEntity),
        util::Read::Value(rendered) => rendered,
        util::Read::Missing | util::Read::None => return Ok(Applied::Unchanged),
    };
    let Some(attributes) = util::parse_object(&rendered) else {
        return Ok(Applied::Unchanged);
    };
    for (key, value) in attributes {
        if !key.starts_with("__") {
            state.attributes.insert(key, value);
        }
    }
    Ok(Applied::Changed)
}
