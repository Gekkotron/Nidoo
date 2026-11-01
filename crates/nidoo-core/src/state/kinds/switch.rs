//! Switch: like a binary sensor, with `state_on` / `state_off` falling back to
//! `payload_on` / `payload_off`, swapped when the override says the switch is inverted.

use super::binary_sensor::apply_on_off;
use crate::model::{Entity, EntityState};
use crate::state::util::{self, Message};
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
    let on = util::config_string(config, "state_on")
        .unwrap_or_else(|| util::config_string_or(config, "payload_on", "ON"));
    let off = util::config_string(config, "state_off")
        .unwrap_or_else(|| util::config_string_or(config, "payload_off", "OFF"));
    let message = Message {
        templates,
        config,
        topic,
        payload,
    };
    if entity.ui.invert {
        apply_on_off(&message, state, &off, &on)
    } else {
        apply_on_off(&message, state, &on, &off)
    }
}
