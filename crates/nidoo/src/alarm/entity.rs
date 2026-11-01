//! The discovery config of the alarm panel entity the hub publishes while the alarm is on.

use nidoo_alarm::topics;
use serde_json::{Value, json};

use super::ALARM_ENTITY_ID;

/// Where the retained config lives: under the hub's discovery prefix, as any entity.
pub fn config_topic(discovery_prefix: &str) -> String {
    format!(
        "{}/alarm_control_panel/alarm/house/config",
        discovery_prefix.trim_end_matches('/')
    )
}

/// The panel's discovery config. It names no code, so arming and disarming never ask for one.
pub fn build_config(alarm_prefix: &str) -> Value {
    json!({
        "name": "Alarm",
        "unique_id": ALARM_ENTITY_ID,
        "state_topic": topics::panel_state_topic(alarm_prefix),
        "command_topic": topics::panel_command_topic(alarm_prefix),
        "json_attributes_topic": topics::panel_attributes_topic(alarm_prefix),
        "supported_features": ["arm_away", "arm_home", "arm_night"],
        "code_arm_required": false,
        "code_disarm_required": false,
        "device": {"identifiers": ["oikos_alarm"], "name": "Alarm"},
    })
}
