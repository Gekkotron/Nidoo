//! Vacuum (state schema): `state_topic` carries JSON `{state, battery_level, fan_speed}`.

use serde_json::Value;

use crate::model::{Entity, EntityState, StateValue, VacuumState};
use crate::state::util::{self, Message};
use crate::state::{Applied, StateError};
use crate::template::Templates;

/// The activities Home Assistant accepts from an MQTT vacuum.
pub(crate) const STATES: &[&str] = &["cleaning", "docked", "paused", "idle", "returning", "error"];

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
    if !message.is_topic("state_topic") {
        return Ok(Applied::NotForThisEntity);
    }
    let Some(object) = util::parse_object(payload) else {
        return Ok(Applied::Unchanged);
    };
    let mut vacuum = match &state.value {
        StateValue::Vacuum(v) => v.clone(),
        _ => VacuumState::default(),
    };
    let mut changed = false;
    match object.get("state") {
        Some(Value::String(activity)) => {
            if let Some(known) = STATES.iter().find(|s| activity.eq_ignore_ascii_case(s)) {
                vacuum.state = Some((*known).to_owned());
                changed = true;
            }
        }
        Some(Value::Null) => {
            vacuum.state = None;
            changed = true;
        }
        _ => {}
    }
    if let Some(level) = object
        .get("battery_level")
        .and_then(Value::as_f64)
        .and_then(util::to_u8_100)
    {
        vacuum.battery = Some(level);
        changed = true;
    }
    if let Some(speed) = object.get("fan_speed").and_then(Value::as_str) {
        vacuum.fan_speed = Some(speed.to_owned());
        changed = true;
    }
    if !changed {
        return Ok(Applied::Unchanged);
    }
    state.value = StateValue::Vacuum(vacuum);
    Ok(Applied::Changed)
}
