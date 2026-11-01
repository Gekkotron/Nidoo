//! Alarm control panel: `state_topic` through `value_template`, one of Home Assistant's
//! alarm states.

use super::set_value;
use crate::model::{Entity, EntityState, StateValue};
use crate::state::util::{Message, Read};
use crate::state::{Applied, StateError};
use crate::template::Templates;

/// The states Home Assistant accepts from an MQTT alarm panel.
pub(crate) const STATES: &[&str] = &[
    "disarmed",
    "armed_home",
    "armed_away",
    "armed_night",
    "armed_vacation",
    "armed_custom_bypass",
    "pending",
    "triggered",
    "arming",
    "disarming",
];

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
        Read::Value(text) => match STATES.iter().find(|s| text.eq_ignore_ascii_case(s)) {
            Some(known) => set_value(state, StateValue::Alarm((*known).to_owned())),
            None => Applied::Unchanged,
        },
    })
}
