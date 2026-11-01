//! One mapper per entity kind, selected by `protocol_kind`.

mod alarm;
mod binary_sensor;
mod button;
mod climate;
mod cover;
mod light;
mod number_select;
mod sensor;
mod switch;
mod vacuum;
mod water_heater;

pub(crate) use alarm::STATES as ALARM_STATES;
pub(crate) use cover::STATES as COVER_STATES;
pub(crate) use vacuum::STATES as VACUUM_STATES;

use super::{Applied, StateError};
use crate::model::{Entity, EntityState, Kind, StateValue};
use crate::template::Templates;
use crate::types;

/// Applies the message to the entity's value through the mapper of its `protocol_kind`.
pub(super) fn apply(
    entity: &Entity,
    state: &mut EntityState,
    topic: &str,
    payload: &str,
    templates: &Templates,
) -> Result<Applied, StateError> {
    match entity.protocol_kind {
        Kind::Light => light::apply(entity, state, topic, payload, templates),
        Kind::Switch => switch::apply(entity, state, topic, payload, templates),
        Kind::Cover => cover::apply(entity, state, topic, payload, templates),
        Kind::Climate => climate::apply(entity, state, topic, payload, templates),
        Kind::Sensor => sensor::apply(entity, state, topic, payload, templates),
        Kind::BinarySensor => binary_sensor::apply(entity, state, topic, payload, templates),
        Kind::AlarmPanel => alarm::apply(entity, state, topic, payload, templates),
        Kind::Number | Kind::Select => {
            number_select::apply(entity, state, topic, payload, templates)
        }
        Kind::Button => button::apply(entity, state, topic, payload, templates),
        Kind::Vacuum => vacuum::apply(entity, state, topic, payload, templates),
        Kind::WaterHeater => water_heater::apply(entity, state, topic, payload, templates),
        Kind::Generic => types::type_def(&entity.component)
            .map(|def| types::state::apply(entity, def, state, topic, payload, templates))
            .unwrap_or(Ok(Applied::NotForThisEntity)),
    }
}

/// Stores `value` as the entity's value; clearing to `Unknown` counts only when there was one.
fn set_value(state: &mut EntityState, value: StateValue) -> Applied {
    if value == StateValue::Unknown && state.value == value {
        return Applied::Unchanged;
    }
    state.value = value;
    Applied::Changed
}
