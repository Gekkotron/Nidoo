//! Sensor: `state_topic` through `value_template`; a number when it parses, text otherwise.

use super::set_value;
use crate::model::{Entity, EntityState, StateValue};
use crate::state::util::{self, Message, Read};
use crate::state::{Applied, StateError};
use crate::template::Templates;

pub(super) fn apply(
    entity: &Entity,
    state: &mut EntityState,
    topic: &str,
    payload: &str,
    templates: &Templates,
) -> Result<Applied, StateError> {
    let message = Message {
        templates,
        config: &entity.bindings.config,
        topic,
        payload,
    };
    Ok(match message.read("state_topic", &["value_template"])? {
        Read::Other => Applied::NotForThisEntity,
        Read::Missing => Applied::Unchanged,
        Read::None => set_value(state, StateValue::Unknown),
        Read::Value(text) => set_value(state, number_or_text(text)),
    })
}

/// `Number` when the text is a finite number, else `Text`.
pub(super) fn number_or_text(text: String) -> StateValue {
    match util::parse_number(&text) {
        Some(number) => StateValue::Number(number),
        None => StateValue::Text(text),
    }
}
