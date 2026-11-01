//! Binary sensor: `state_topic` through `value_template`, matched against `payload_on` /
//! `payload_off` (defaults `ON` / `OFF`); any other value is ignored, as in Home Assistant.

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
    let config = &entity.bindings.config;
    let on = util::config_string_or(config, "payload_on", "ON");
    let off = util::config_string_or(config, "payload_off", "OFF");
    let message = Message {
        templates,
        config,
        topic,
        payload,
    };
    apply_on_off(&message, state, &on, &off)
}

/// Shared by switches: reads `state_topic` and maps the value onto `OnOff`.
pub(super) fn apply_on_off(
    message: &Message<'_>,
    state: &mut EntityState,
    on: &str,
    off: &str,
) -> Result<Applied, StateError> {
    Ok(match message.read("state_topic", &["value_template"])? {
        Read::Other => Applied::NotForThisEntity,
        Read::Missing => Applied::Unchanged,
        Read::None => set_value(state, StateValue::Unknown),
        Read::Value(text) => match util::on_off(&text, on, off) {
            Some(is_on) => set_value(state, StateValue::OnOff(is_on)),
            None => Applied::Unchanged,
        },
    })
}
