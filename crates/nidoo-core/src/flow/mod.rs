//! Flows: n8n-style automations as a small graph of typed nodes.
//! Pure data and helpers, no I/O, so the hub, the API and the web canvas share one definition.

mod clock;
mod duration;
mod field;
mod graph;
pub mod math;
mod model;
mod scalar;
mod schedule;
pub mod sun;
mod switch;
pub mod template;
mod validate;

pub use clock::{in_time_range, parse_clock};
pub use duration::{
    DurationError, MAX_WINDOW, MIN_WINDOW, format_duration, parse_duration, pick_duration, window,
};
pub use field::{FieldInfo, brightness_percent, field_scalar, fields, headline_scalar};
pub use math::{
    FUNCTIONS, Formula, FormulaError, Function, MAX_DEPTH, MAX_FORMULA_CHARS, MAX_INPUTS,
    MAX_TABLE_POINTS, fit_to_entity, is_valid_name, parse_formula,
};
pub use model::{
    CommandParams, DelayParams, Edge, Flow, FlowEnableParams, FlowMode, IfParams, LimitParams,
    MAX_AVERAGE_SECS, MAX_LIMIT, MAX_SWITCH_RULES, MathInput, MathParams, Missing, Node, NodeKind,
    NotifyParams, OffPeakEdge, Op, Position, RuleOp, SunWhich, SwitchParams, SwitchRule,
    TimeBetweenParams, TriggerManualParams, TriggerOffPeakParams, TriggerScheduleParams,
    TriggerSignalParams, TriggerSilentParams, TriggerStateParams, TriggerSunParams,
    command_with_number, math_names, uses_sun_or_is_dark,
};
pub use scalar::{CompareError, Scalar, compare};
pub use schedule::{Schedule, ScheduleError};
pub use sun::{
    APPARENT_ALTITUDE_DEG, CIVIL_ALTITUDE_DEG, SunEvents, is_dark_at, is_dark_with_offset,
    sun_altitude, sun_events,
};
pub use switch::switch_exits;
pub use template::{Lang, Reading, Template, TemplateError};
pub use validate::{
    FlowError, NOTIFY_CHANNELS, NOTIFY_SEVERITIES, check_math_card, is_valid_flow_id,
    is_valid_node_id, notify_channel_error, notify_severity_error, validate,
};

/// Shared fixture for the flow tests: the reference flow of the design.
#[cfg(test)]
pub(crate) fn reference_flow() -> Flow {
    Flow::decode(
        br#"{
      "id": "living_led_motion", "name": "Living LED on motion", "enabled": true, "mode": "restart",
      "nodes": [
        { "id": "motion", "type": "trigger.state", "params": { "entity": "m" } },
        { "id": "moving", "type": "if", "params": { "op": "eq", "value": true } },
        { "id": "on", "type": "command", "params": { "entity": "l", "command": { "type": "turn_on" } } },
        { "id": "wait", "type": "delay", "params": { "for": "5m" } },
        { "id": "off", "type": "command", "params": { "entity": "l", "command": { "type": "turn_off" } } }
      ],
      "edges": [
        { "from": "motion", "to": "moving" },
        { "from": "moving", "port": "true", "to": "on" },
        { "from": "moving", "port": "false", "to": "wait" },
        { "from": "wait", "to": "off" }
      ]
    }"#,
    )
    .unwrap()
}
