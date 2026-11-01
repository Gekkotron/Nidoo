//! Number and select: `state_topic` through `value_template`; a number yields `Number`
//! (text when it does not parse), a select always `Text`.

use super::{sensor::number_or_text, set_value};
use crate::model::{Entity, EntityState, Kind, StateValue};
use crate::state::util::{Message, Read};
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
        Read::Value(text) if entity.protocol_kind == Kind::Number => {
            set_value(state, number_or_text(text))
        }
        Read::Value(text) => set_value(state, StateValue::Text(text)),
    })
}
