//! Water heater: `mode_state_topic`, `current_temperature_topic`, `temperature_state_topic`
//! (+ templates, `value_template` as the fallback).

use crate::model::{Entity, EntityState, StateValue, WaterHeaterState};
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
    let mut heater = match &state.value {
        StateValue::WaterHeater(h) => h.clone(),
        _ => WaterHeaterState::default(),
    };
    let mut applied = Applied::NotForThisEntity;
    message.text_field(
        "mode_state_topic",
        "mode_state_template",
        &mut applied,
        &mut heater.mode,
    )?;
    message.number_field(
        "current_temperature_topic",
        "current_temperature_template",
        &mut applied,
        &mut heater.current,
    )?;
    message.number_field(
        "temperature_state_topic",
        "temperature_state_template",
        &mut applied,
        &mut heater.target,
    )?;
    if applied == Applied::Changed {
        state.value = StateValue::WaterHeater(heater);
    }
    Ok(applied)
}
