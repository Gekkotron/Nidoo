//! Climate: one topic (+ template) per field; `value_template` is the fallback template.

use crate::model::{ClimateState, Entity, EntityState, StateValue};
use crate::state::util::Message;
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
    let mut climate = match &state.value {
        StateValue::Climate(c) => c.clone(),
        _ => ClimateState::default(),
    };
    let mut applied = Applied::NotForThisEntity;
    message.number_field(
        "current_temperature_topic",
        "current_temperature_template",
        &mut applied,
        &mut climate.current,
    )?;
    message.number_field(
        "temperature_state_topic",
        "temperature_state_template",
        &mut applied,
        &mut climate.target,
    )?;
    message.text_field(
        "mode_state_topic",
        "mode_state_template",
        &mut applied,
        &mut climate.mode,
    )?;
    message.text_field(
        "fan_mode_state_topic",
        "fan_mode_state_template",
        &mut applied,
        &mut climate.fan_mode,
    )?;
    message.text_field(
        "action_topic",
        "action_template",
        &mut applied,
        &mut climate.action,
    )?;
    if applied == Applied::Changed {
        state.value = StateValue::Climate(climate);
    }
    Ok(applied)
}
