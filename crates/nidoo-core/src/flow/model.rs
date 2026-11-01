//! The flow document: nodes, edges and the lenient JSON form stored on the broker.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::scalar::Scalar;
use super::template::Lang;
use crate::model::Command;
use crate::model::ui_override::strip_nulls;

/// What a trigger does while a run of the same flow is still active.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum FlowMode {
    /// The new trigger is ignored and recorded as a skipped run.
    #[default]
    Single,
    /// The active run is cancelled and a new one starts ("N minutes after the last motion").
    Restart,
}

/// A whole automation. Unknown keys are kept in [`Flow::extra`] so a newer writer's fields survive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Flow {
    /// `[a-z0-9_-]{1,64}`, the last topic segment.
    pub id: String,
    /// Free text shown in the UI.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The room the UI lists the flow under (an area id); it does not change what the flow does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
    /// A disabled flow keeps its definition but never runs. Defaults to `true`.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// `single` (the default) ignores a trigger while a run is active; `restart` cancels the
    /// active run and starts a new one, which is how to express "N minutes after the last event".
    #[serde(default)]
    pub mode: FlowMode,
    /// The steps; the trigger nodes (`trigger.state`, `trigger.schedule`) start a run.
    #[serde(default)]
    pub nodes: Vec<Node>,
    /// The connections between nodes, followed from a trigger onwards.
    #[serde(default)]
    pub edges: Vec<Edge>,
    /// Fields this hub does not know, preserved verbatim.
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

impl Default for Flow {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            description: None,
            area: None,
            enabled: true,
            mode: FlowMode::default(),
            nodes: Vec::new(),
            edges: Vec::new(),
            extra: BTreeMap::new(),
        }
    }
}

/// True when the flow reads the sun: it has a `trigger.sun` node or an `if` with `op: is_dark`.
pub fn uses_sun_or_is_dark(flow: &Flow) -> bool {
    flow.nodes.iter().any(|node| match &node.kind {
        NodeKind::TriggerSun(_) => true,
        NodeKind::If(params) => params.op == Op::IsDark,
        _ => false,
    })
}

impl Flow {
    /// Decodes a retained payload leniently: nulls read as absent (at the top level, on each node
    /// and in each node's params), unknown keys are kept. The error is serde's message.
    pub fn decode(payload: &[u8]) -> Result<Flow, String> {
        let mut value: Value = serde_json::from_slice(payload).map_err(|e| e.to_string())?;
        let Some(object) = value.as_object_mut() else {
            return Err("a flow must be a JSON object".to_owned());
        };
        strip_nulls(object);
        if let Some(Value::Array(nodes)) = object.get_mut("nodes") {
            for node in nodes {
                if let Some(node) = node.as_object_mut() {
                    strip_nulls(node);
                    if let Some(Value::Object(params)) = node.get_mut("params") {
                        strip_nulls(params);
                    }
                }
            }
        }
        serde_json::from_value(value).map_err(|e| e.to_string())
    }

    /// Encodes as compact JSON; `{}` only if serialisation somehow fails.
    pub fn encode(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_owned())
    }
}

/// One step of a flow. On the wire: `{"id", "type", "params", "name"?, "position"?}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Node {
    /// Unique within the flow; edges refer to it.
    pub id: String,
    /// Label shown on the canvas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Canvas placement; the hub ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    #[serde(flatten)]
    pub kind: NodeKind,
    /// Fields this hub does not know, preserved verbatim.
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// Canvas coordinates of a node.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

/// The node types, tagged by `type` with their `params`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(tag = "type", content = "params")]
pub enum NodeKind {
    /// Starts a run when an entity's state changes.
    #[serde(rename = "trigger.state")]
    TriggerState(TriggerStateParams),
    /// Starts a run on a cron schedule (five fields, local time).
    #[serde(rename = "trigger.schedule")]
    TriggerSchedule(TriggerScheduleParams),
    /// Starts a run at a solar event (sunrise, sunset, dusk or dawn), with an optional offset.
    #[serde(rename = "trigger.sun")]
    TriggerSun(TriggerSunParams),
    /// Starts a run when its card is pressed, optionally carrying a fixed value.
    #[serde(rename = "trigger.manual")]
    TriggerManual(TriggerManualParams),
    /// Starts a run when a remote, an event entity or a tag scanner sends a signal.
    #[serde(rename = "trigger.signal")]
    TriggerSignal(TriggerSignalParams),
    /// Starts a run when off-peak starts or ends (from the off-peak device, else the windows).
    #[serde(rename = "trigger.off_peak")]
    TriggerOffPeak(TriggerOffPeakParams),
    /// Starts a run when an entity has sent no report for a duration.
    #[serde(rename = "trigger.silent")]
    TriggerSilent(TriggerSilentParams),
    /// Branches on a comparison of an entity (or the trigger value) with a constant.
    #[serde(rename = "if")]
    If(IfParams),
    /// Waits before continuing.
    #[serde(rename = "delay")]
    Delay(DelayParams),
    /// Lets a run through at most `max` times per rolling window; further runs end here.
    #[serde(rename = "limit")]
    Limit(LimitParams),
    /// Sends a command to an entity.
    #[serde(rename = "command")]
    Command(CommandParams),
    /// Enables or disables a flow (its own by default).
    #[serde(rename = "flow.enable")]
    FlowEnable(FlowEnableParams),
    /// Sends a phone push notification.
    #[serde(rename = "notify")]
    Notify(NotifyParams),
    /// Branches on the first matching rule (or every matching rule) over an entity's value.
    #[serde(rename = "switch")]
    Switch(SwitchParams),
    /// Branches on whether the house clock is inside a daily time range.
    #[serde(rename = "time.between")]
    TimeBetween(TimeBetweenParams),
    /// Computes one number from entity readings and earlier math results, optionally saving it
    /// to a number entity.
    #[serde(rename = "math")]
    Math(MathParams),
}

impl NodeKind {
    /// The wire tag of this node type.
    pub fn type_name(&self) -> &'static str {
        match self {
            NodeKind::TriggerState(_) => "trigger.state",
            NodeKind::TriggerSchedule(_) => "trigger.schedule",
            NodeKind::TriggerSun(_) => "trigger.sun",
            NodeKind::TriggerManual(_) => "trigger.manual",
            NodeKind::TriggerSignal(_) => "trigger.signal",
            NodeKind::TriggerOffPeak(_) => "trigger.off_peak",
            NodeKind::TriggerSilent(_) => "trigger.silent",
            NodeKind::If(_) => "if",
            NodeKind::Delay(_) => "delay",
            NodeKind::Limit(_) => "limit",
            NodeKind::Command(_) => "command",
            NodeKind::FlowEnable(_) => "flow.enable",
            NodeKind::Notify(_) => "notify",
            NodeKind::Switch(_) => "switch",
            NodeKind::TimeBetween(_) => "time.between",
            NodeKind::Math(_) => "math",
        }
    }

    /// Whether this node type starts a run.
    pub fn is_trigger(&self) -> bool {
        matches!(
            self,
            NodeKind::TriggerState(_)
                | NodeKind::TriggerSchedule(_)
                | NodeKind::TriggerSun(_)
                | NodeKind::TriggerManual(_)
                | NodeKind::TriggerSignal(_)
                | NodeKind::TriggerOffPeak(_)
                | NodeKind::TriggerSilent(_)
        )
    }

    /// The named output ports: `true` and `false` for `if`; `1` to `n` (then `otherwise` when
    /// set) for `switch`; `in` and `out` for `time.between`; none for the others (which have one
    /// unnamed output).
    pub fn ports(&self) -> Vec<String> {
        match self {
            NodeKind::If(_) => vec!["true".into(), "false".into()],
            NodeKind::Switch(p) => {
                let mut ports: Vec<String> = (1..=p.rules.len()).map(|i| i.to_string()).collect();
                if p.otherwise {
                    ports.push("otherwise".into());
                }
                ports
            }
            NodeKind::TimeBetween(_) => vec!["in".into(), "out".into()],
            _ => Vec::new(),
        }
    }
}

/// Params of `trigger.state`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TriggerStateParams {
    /// Entity id (from `search` / `list_entities`).
    pub entity: String,
    /// A value inside the entity's state (`battery`, `position` …); absent reads its headline value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// Only trigger when the new value equals this (a JSON value such as `true`, `"ON"`, `21.5`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Value>,
    /// Only trigger when the previous value equalled this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Value>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// Params of `trigger.signal`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TriggerSignalParams {
    /// Entity id (`device_automation.<node>`, `event.<...>`, `tag.<...>`).
    pub entity: String,
    /// Only trigger when the signal name equals this; absent means any signal of `entity`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// Params of `trigger.schedule`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TriggerScheduleParams {
    /// Five-field cron expression: minute hour day-of-month month day-of-week.
    pub cron: String,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// Params of `trigger.manual`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TriggerManualParams {
    /// The trigger value the run carries; absent means a run without a value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Scalar>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// Which solar event a `trigger.sun` fires on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SunWhich {
    Sunrise,
    #[default]
    Sunset,
    Dusk,
    Dawn,
}

/// Params of `trigger.sun`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TriggerSunParams {
    /// The solar event to fire on.
    pub which: SunWhich,
    /// Minutes added to the event time (negative fires earlier); absent means 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset_minutes: Option<i32>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

impl TriggerSunParams {
    /// The offset in minutes, 0 when absent.
    pub fn offset(&self) -> i32 {
        self.offset_minutes.unwrap_or(0)
    }
}

/// Which change of the off-peak state a `trigger.off_peak` fires on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum OffPeakEdge {
    /// Peak to off-peak.
    Start,
    /// Off-peak to peak.
    End,
}

impl OffPeakEdge {
    /// The name this edge carries in the flow JSON, also the trigger value of a run.
    pub fn wire_name(&self) -> &'static str {
        match self {
            OffPeakEdge::Start => "start",
            OffPeakEdge::End => "end",
        }
    }
}

/// Params of `trigger.off_peak`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TriggerOffPeakParams {
    /// Fire when off-peak starts or when it ends.
    pub edge: OffPeakEdge,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// Params of `trigger.silent`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TriggerSilentParams {
    /// The entity to watch (an id from `search` / `list_entities`).
    pub entity: String,
    /// How long the entity may stay without any report before the trigger fires: a duration
    /// such as `50m` or `2h`, from `1m` to `168h`. Fires once per silence; a report of the
    /// entity, changed or not, restarts the clock. The trigger value is this text.
    #[serde(rename = "for")]
    pub for_: String,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// Params of `limit`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct LimitParams {
    /// How many runs may pass this node in one window, from 1 to 100.
    pub max: u32,
    /// The rolling window: a duration such as `24h`, from `1m` to `168h`. A run over the limit
    /// ends at this node.
    pub per: String,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// The most passes a `limit` may allow in one window.
pub const MAX_LIMIT: u32 = 100;

/// Params of `if`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct IfParams {
    /// How the value is compared with `value`: `eq`, `ne`, `gt`, `lt`, `gte` or `lte`.
    pub op: Op,
    /// The constant the value is compared with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// The entity whose current value is read; when absent the value that fired the trigger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
    /// A value inside the entity's state (`battery`, `position` …); absent reads its headline value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// What to do when the value is missing (unknown or unavailable); absent behaves as `fail`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<Missing>,
    /// Compare the mean of the entity's readings over this window (`15m`, `1h`, at most
    /// `24h`), the newest included, instead of its current value; `value` must then be a number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub average: Option<String>,
    /// With `op: is_dark` only: minutes added to civil dusk and taken from civil dawn (negative
    /// makes it dark earlier in the evening and until later in the morning); absent means 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset_minutes: Option<i32>,
    /// Compare the result of the math card with this `name` instead of an entity; it cannot be
    /// combined with `entity`, `average` or `missing`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

impl IfParams {
    /// The `is_dark` offset in minutes, 0 when absent.
    pub fn dark_offset(&self) -> i32 {
        self.offset_minutes.unwrap_or(0)
    }
}

/// The most rules a `switch` accepts.
pub const MAX_SWITCH_RULES: usize = 10;

fn is_false(value: &bool) -> bool {
    !*value
}

/// Comparison operators of a `switch` rule: those of `if`, plus `between`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum RuleOp {
    Eq,
    Ne,
    Gt,
    Lt,
    Gte,
    Lte,
    /// `value <= x <= to`, both bounds included.
    Between,
}

impl RuleOp {
    /// The lowercase name this operator carries in the flow JSON.
    pub fn wire_name(self) -> &'static str {
        match self {
            RuleOp::Eq => "eq",
            RuleOp::Ne => "ne",
            RuleOp::Gt => "gt",
            RuleOp::Lt => "lt",
            RuleOp::Gte => "gte",
            RuleOp::Lte => "lte",
            RuleOp::Between => "between",
        }
    }
}

/// One rule of a `switch`; its output port is its 1-based position.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SwitchRule {
    /// How the value is compared.
    pub op: RuleOp,
    /// The constant compared with (the lower bound for `between`).
    pub value: Value,
    /// The upper bound, with `op: between` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Value>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// Params of `switch`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SwitchParams {
    /// The entity whose current value is read; when absent the value that fired the trigger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
    /// A value inside the entity's state (`battery`, `position` …); absent reads its headline value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// Compare the mean of the entity's readings over this window (`15m`, `1h`) instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub average: Option<String>,
    /// The rules, 1 to 10, tried in order.
    pub rules: Vec<SwitchRule>,
    /// Add an `otherwise` port taken when no rule matches.
    #[serde(default, skip_serializing_if = "is_false")]
    pub otherwise: bool,
    /// Take every matching rule instead of the first one only.
    #[serde(default, skip_serializing_if = "is_false")]
    pub all: bool,
    /// Switch on the result of the math card with this `name` instead of an entity; it cannot be
    /// combined with `entity` or `average`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// Params of `time.between`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TimeBetweenParams {
    /// Start of the range, `HH:MM` 24-hour, included.
    pub from: String,
    /// End of the range, `HH:MM` 24-hour, excluded; earlier than `from` crosses midnight.
    pub to: String,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// The longest `average` window an `if` accepts, in seconds.
pub const MAX_AVERAGE_SECS: u64 = 24 * 3600;

/// How an `if` behaves when the compared entity has no value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Missing {
    /// The step fails (the default).
    Fail,
    /// Take the `true` port.
    True,
    /// Take the `false` port.
    False,
}

/// Params of `flow.enable`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct FlowEnableParams {
    /// The flow to switch; absent means the flow this node belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<String>,
    /// The state to set: `true` enables, `false` disables.
    pub enabled: bool,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// Params of `notify`: the fields of a v1 push alert.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct NotifyParams {
    /// The alert title; required, not empty.
    /// Takes `{entity_id}` placeholders filled at run time (see the flows docs).
    pub title: String,
    /// The alert body.
    /// Takes `{entity_id}` placeholders filled at run time (see the flows docs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// One of `alarm`, `security`, `camera`, `energy`, `climate`, `system`, `misc` (the default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// One of `info` (the default), `warning`, `critical`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    /// The entity the alert is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    /// An image to show with the alert.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    /// Deliver without sound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub silent: Option<bool>,
    /// A fixed alert id: a later run replaces the earlier alert. Absent means
    /// `flow-<flow id>-<run id>`, one alert per run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The language the text values are written in: `en` (the default) or `fr`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<Lang>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// Params of `delay`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DelayParams {
    /// A duration such as `90s`, `5m` or `1h30m`.
    #[serde(rename = "for")]
    pub for_: String,
    /// When set, each run waits a random duration between `for` and this one (same
    /// format, not shorter than `for`) instead of exactly `for`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up_to: Option<String>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

impl DelayParams {
    /// Parses `for` and `up_to`, refusing an `up_to` shorter than `for`.
    pub fn bounds(&self) -> Result<(std::time::Duration, Option<std::time::Duration>), String> {
        use super::parse_duration;
        let low = parse_duration(&self.for_).map_err(|e| e.to_string())?;
        let Some(up_to) = self.up_to.as_deref() else {
            return Ok((low, None));
        };
        let high = parse_duration(up_to).map_err(|e| format!("up_to: {e}"))?;
        if high < low {
            return Err(format!(
                "`up_to` ({up_to}) is shorter than `for` ({})",
                self.for_
            ));
        }
        Ok((low, Some(high)))
    }
}

/// Params of `command`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CommandParams {
    /// The entity to command (an id from `search` / `list_entities`).
    pub entity: String,
    /// The command, the same JSON as `send_command`'s, e.g. `{"type": "turn_on"}`.
    pub command: Command,
    /// The `name` of a math card whose result replaces the command's number: `set_number`'s
    /// `value`, `set_temperature`'s `temperature`, `set_position`'s `position` or `turn_on`'s
    /// `brightness`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_from: Option<String>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// One named reading a math card's formula uses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MathInput {
    /// How the formula refers to the reading: `[a-z][a-z0-9_]{0,31}`, unique within the card.
    pub name: String,
    /// The entity whose current value is read.
    pub entity: String,
    /// Read the mean of the entity's readings over this window (`15m`, `1h`, at most `24h`)
    /// instead of its current value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub average: Option<String>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// Params of `math`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MathParams {
    /// The readings the formula uses, 0 to 10.
    #[serde(default)]
    pub inputs: Vec<MathInput>,
    /// The expression, 1 to 500 characters: numbers, input names, `trigger`, other math cards'
    /// names, `+ - * / %`, comparisons, `and`, `or`, `not`, and the functions `min`, `max`,
    /// `clamp`, `abs`, `round`, `floor`, `ceil`, `if` and `interp`.
    pub formula: String,
    /// The result's name, used by later cards; same form as an input name.
    pub name: String,
    /// A number entity the result is written to with `set_number`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_to: Option<String>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// The names of the flow's math cards. Two cards may share a name (branches computing the same
/// result).
pub fn math_names(flow: &Flow) -> BTreeSet<&str> {
    flow.nodes
        .iter()
        .filter_map(|node| match &node.kind {
            NodeKind::Math(p) => Some(p.name.as_str()),
            _ => None,
        })
        .collect()
}

/// The command with its number replaced by `value`, or `None` when the command takes none:
/// `set_number`, `set_temperature`, `set_position` (rounded, kept in 0..=100) and `turn_on`
/// (brightness rounded, kept in 0..=255, other fields kept).
pub fn command_with_number(command: &Command, value: f64) -> Option<Command> {
    Some(match command {
        Command::SetNumber { .. } => Command::SetNumber { value },
        Command::SetTemperature { .. } => Command::SetTemperature { temperature: value },
        Command::SetPosition { .. } => Command::SetPosition {
            position: value.round().clamp(0.0, 100.0) as u8,
        },
        Command::TurnOn { color, effect, .. } => Command::TurnOn {
            brightness: Some(value.round().clamp(0.0, 255.0) as u8),
            color: *color,
            effect: effect.clone(),
        },
        _ => return None,
    })
}

/// A connection from a node's output to another node; `port` picks an `if` branch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Edge {
    /// The id of the source node.
    pub from: String,
    /// The id of the node that runs next.
    pub to: String,
    /// `true` or `false` for an edge leaving an `if`, which requires it; absent for every
    /// other node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
}

/// Comparison operators of the `if` node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Op {
    Eq,
    Ne,
    Gt,
    Lt,
    Gte,
    Lte,
    IsDark,
    IsOffPeak,
}

impl Op {
    /// The lowercase name this operator carries in the flow JSON.
    pub fn wire_name(&self) -> &'static str {
        match self {
            Op::Eq => "eq",
            Op::Ne => "ne",
            Op::Gt => "gt",
            Op::Lt => "lt",
            Op::Gte => "gte",
            Op::Lte => "lte",
            Op::IsDark => "is_dark",
            Op::IsOffPeak => "is_off_peak",
        }
    }
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn if_without_field_serialises_as_before() {
        let text = r#"{"op":"lte","value":55,"entity":"r"}"#;
        let p: IfParams = serde_json::from_str(text).unwrap();
        assert_eq!(p.field, None);
        assert_eq!(serde_json::to_string(&p).unwrap(), text);
    }

    #[test]
    fn if_field_round_trips() {
        let text = r#"{"op":"lte","value":55,"entity":"r","field":"battery"}"#;
        let p: IfParams = serde_json::from_str(text).unwrap();
        assert_eq!(p.field.as_deref(), Some("battery"));
        assert!(p.extra.is_empty());
        assert_eq!(serde_json::to_string(&p).unwrap(), text);
    }

    #[test]
    fn area_round_trips_and_is_omitted_when_none() {
        let flow = Flow::decode(br#"{"id":"a","name":"A","area":"living"}"#).unwrap();
        assert_eq!(flow.area.as_deref(), Some("living"));
        assert!(flow.encode().contains(r#""area":"living""#));
        let bare = Flow::decode(br#"{"id":"a","name":"A"}"#).unwrap();
        assert_eq!(bare.area, None);
        assert!(!bare.encode().contains("area"));
        assert!(bare.extra.is_empty());
    }

    #[test]
    fn null_area_reads_as_none() {
        let flow = Flow::decode(br#"{"id":"a","name":"A","area":null}"#).unwrap();
        assert_eq!(flow.area, None);
    }

    #[test]
    fn notify_node_round_trips() {
        let text = r#"{"id":"n","type":"notify","params":{"title":"Door open"}}"#;
        let node: Node = serde_json::from_str(text).unwrap();
        assert!(!node.kind.is_trigger());
        assert!(node.kind.ports().is_empty());
        assert_eq!(node.kind.type_name(), "notify");
        assert!(matches!(&node.kind, NodeKind::Notify(p)
            if p.title == "Door open" && p.body.is_none() && p.silent.is_none()));
        assert_eq!(
            serde_json::to_value(&node).unwrap(),
            serde_json::from_str::<serde_json::Value>(text).unwrap()
        );
    }

    #[test]
    fn notify_lang_is_en_or_fr() {
        let node: Node = serde_json::from_str(
            r#"{"id":"n","type":"notify","params":{"title":"T","lang":"fr"}}"#,
        )
        .unwrap();
        assert!(matches!(&node.kind, NodeKind::Notify(p) if p.lang == Some(Lang::Fr)));
        assert!(
            serde_json::to_string(&node)
                .unwrap()
                .contains(r#""lang":"fr""#)
        );
        assert!(
            serde_json::from_str::<Node>(
                r#"{"id":"n","type":"notify","params":{"title":"T","lang":"de"}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn flow_enable_defaults_to_self() {
        let node: Node = serde_json::from_str(
            r#"{"id":"stop","type":"flow.enable","params":{"enabled":false}}"#,
        )
        .unwrap();
        let NodeKind::FlowEnable(p) = node.kind else {
            panic!()
        };
        assert_eq!((p.flow, p.enabled), (None, false));
    }

    #[test]
    fn trigger_signal_round_trips_json() {
        let text = r#"{"id":"s","type":"trigger.signal","params":{"entity":"device_automation.0xd4fe12345","signal":"on"}}"#;
        let node: Node = serde_json::from_str(text).unwrap();
        assert!(node.kind.is_trigger());
        assert_eq!(node.kind.type_name(), "trigger.signal");
        assert!(matches!(&node.kind, NodeKind::TriggerSignal(p)
            if p.entity == "device_automation.0xd4fe12345" && p.signal.as_deref() == Some("on")));
        assert_eq!(
            serde_json::to_value(&node).unwrap(),
            serde_json::from_str::<serde_json::Value>(text).unwrap()
        );
    }

    #[test]
    fn trigger_sun_round_trips_with_offset_and_extra() {
        let node: Node = serde_json::from_str(
            r#"{"id":"ev","type":"trigger.sun","params":{"which":"sunset","offset_minutes":-30,"note":"n"}}"#,
        )
        .unwrap();
        assert!(node.kind.is_trigger());
        assert_eq!(node.kind.type_name(), "trigger.sun");
        match &node.kind {
            NodeKind::TriggerSun(p) => {
                assert_eq!(p.which, SunWhich::Sunset);
                assert_eq!(p.offset(), -30);
                assert_eq!(p.extra.get("note"), Some(&json!("n")));
            }
            other => panic!("expected trigger.sun, got {other:?}"),
        }
        let again: Node = serde_json::from_value(serde_json::to_value(&node).unwrap()).unwrap();
        assert_eq!(again, node);
    }

    #[test]
    fn trigger_sun_offset_defaults_to_zero_and_is_omitted_from_wire() {
        let node: Node =
            serde_json::from_str(r#"{"id":"dawn","type":"trigger.sun","params":{"which":"dawn"}}"#)
                .unwrap();
        match &node.kind {
            NodeKind::TriggerSun(p) => assert_eq!(p.offset(), 0),
            other => panic!("{other:?}"),
        }
        let wire = serde_json::to_value(&node).unwrap();
        assert!(wire["params"].get("offset_minutes").is_none());
    }

    #[test]
    fn if_is_dark_round_trips_without_value_entity_or_missing() {
        let node: Node =
            serde_json::from_str(r#"{"id":"dark","type":"if","params":{"op":"is_dark"}}"#).unwrap();
        match &node.kind {
            NodeKind::If(p) => {
                assert_eq!(p.op, Op::IsDark);
                assert!(p.value.is_none());
                assert!(p.entity.is_none());
                assert!(p.missing.is_none());
            }
            other => panic!("{other:?}"),
        }
        let wire = serde_json::to_value(&node).unwrap();
        assert_eq!(wire["params"]["op"], json!("is_dark"));
        assert!(wire["params"].get("value").is_none());
    }

    #[test]
    fn trigger_off_peak_round_trips() {
        let raw = r#"{"id":"t","type":"trigger.off_peak","params":{"edge":"start"}}"#;
        let node: Node = serde_json::from_str(raw).unwrap();
        match &node.kind {
            NodeKind::TriggerOffPeak(p) => assert_eq!(p.edge, OffPeakEdge::Start),
            other => panic!("{other:?}"),
        }
        assert!(node.kind.is_trigger());
        assert_eq!(node.kind.type_name(), "trigger.off_peak");
        assert_eq!(
            serde_json::to_value(&node).unwrap(),
            serde_json::from_str::<Value>(raw).unwrap()
        );
        let bad = r#"{"id":"t","type":"trigger.off_peak","params":{"edge":"middle"}}"#;
        assert!(serde_json::from_str::<Node>(bad).is_err());
    }

    #[test]
    fn silent_and_limit_round_trip() {
        let silent = r#"{"id":"t","type":"trigger.silent","params":{"entity":"binary_sensor.smoke","for":"50m"}}"#;
        let node: Node = serde_json::from_str(silent).unwrap();
        match &node.kind {
            NodeKind::TriggerSilent(p) => {
                assert_eq!(p.entity, "binary_sensor.smoke");
                assert_eq!(p.for_, "50m");
            }
            other => panic!("{other:?}"),
        }
        assert!(node.kind.is_trigger());
        assert_eq!(node.kind.type_name(), "trigger.silent");
        assert_eq!(
            serde_json::to_value(&node).unwrap(),
            serde_json::from_str::<Value>(silent).unwrap()
        );
        let limit = r#"{"id":"l","type":"limit","params":{"max":4,"per":"24h"}}"#;
        let node: Node = serde_json::from_str(limit).unwrap();
        match &node.kind {
            NodeKind::Limit(p) => {
                assert_eq!(p.max, 4);
                assert_eq!(p.per, "24h");
            }
            other => panic!("{other:?}"),
        }
        assert!(!node.kind.is_trigger());
        assert_eq!(node.kind.type_name(), "limit");
        assert_eq!(
            serde_json::to_value(&node).unwrap(),
            serde_json::from_str::<Value>(limit).unwrap()
        );
    }

    #[test]
    fn if_is_off_peak_round_trips() {
        let node: Node =
            serde_json::from_str(r#"{"id":"c","type":"if","params":{"op":"is_off_peak"}}"#)
                .unwrap();
        match &node.kind {
            NodeKind::If(p) => assert_eq!(p.op, Op::IsOffPeak),
            other => panic!("{other:?}"),
        }
        let wire = serde_json::to_value(&node).unwrap();
        assert_eq!(wire["params"], json!({"op": "is_off_peak"}));
    }

    #[test]
    fn existing_if_round_trips_with_value_present() {
        let p: IfParams = serde_json::from_str(r#"{"op":"lt","value":65,"entity":"t"}"#).unwrap();
        assert_eq!(p.value, Some(json!(65)));
        assert_eq!(p.op, Op::Lt);
    }

    #[test]
    fn if_missing_round_trips_and_defaults() {
        let p: IfParams =
            serde_json::from_str(r#"{"op":"lt","value":65,"entity":"t","missing":"false"}"#)
                .unwrap();
        assert_eq!(p.missing, Some(Missing::False));
        let p: IfParams = serde_json::from_str(r#"{"op":"lt","value":65}"#).unwrap();
        assert_eq!(p.missing, None);
        assert!(serde_json::to_value(&p).unwrap().get("missing").is_none());
    }

    #[test]
    fn if_average_round_trips_and_defaults() {
        let p: IfParams =
            serde_json::from_str(r#"{"op":"lte","value":70,"average":"15m"}"#).unwrap();
        assert_eq!(p.average.as_deref(), Some("15m"));
        assert!(p.extra.is_empty());
        let p: IfParams = serde_json::from_str(r#"{"op":"lt","value":65}"#).unwrap();
        assert_eq!(p.average, None);
        assert!(serde_json::to_value(&p).unwrap().get("average").is_none());
    }

    const REFERENCE: &str = r#"{
      "id": "living_led_motion", "name": "Living LED on motion", "enabled": true, "mode": "restart",
      "nodes": [
        { "id": "motion", "type": "trigger.state", "name": "Motion",
          "params": { "entity": "0x00158d00044c4d4c_occupancy_zigbee2mqtt" } },
        { "id": "moving", "type": "if", "params": { "op": "eq", "value": true } },
        { "id": "on", "type": "command",
          "params": { "entity": "0x9035eafffec07012_light_zigbee2mqtt", "command": { "type": "turn_on" } } },
        { "id": "wait", "type": "delay", "params": { "for": "5m" } },
        { "id": "off", "type": "command",
          "params": { "entity": "0x9035eafffec07012_light_zigbee2mqtt", "command": { "type": "turn_off" } } }
      ],
      "edges": [
        { "from": "motion", "to": "moving" },
        { "from": "moving", "port": "true", "to": "on" },
        { "from": "moving", "port": "false", "to": "wait" },
        { "from": "wait", "to": "off" }
      ]
    }"#;

    #[test]
    fn schedule_node_round_trips() {
        let node: Node = serde_json::from_str(
            r#"{"id":"tick","type":"trigger.schedule","params":{"cron":"*/10 * * * *","x":1}}"#,
        )
        .unwrap();
        assert!(node.kind.is_trigger());
        assert_eq!(node.kind.type_name(), "trigger.schedule");
        assert_eq!(serde_json::to_value(&node).unwrap()["params"]["x"], 1);
    }

    #[test]
    fn decodes_the_reference_flow() {
        let flow = Flow::decode(REFERENCE.as_bytes()).unwrap();
        assert_eq!(flow.nodes.len(), 5);
        assert_eq!(flow.edges.len(), 4);
        assert_eq!(flow.mode, FlowMode::Restart);
        assert!(flow.enabled);
        assert_eq!(flow.nodes[0].name.as_deref(), Some("Motion"));
        match &flow.nodes[1].kind {
            NodeKind::If(p) => {
                assert_eq!(p.op, Op::Eq);
                assert_eq!(p.value, Some(json!(true)));
                assert_eq!(p.entity, None);
            }
            other => panic!("expected if, got {other:?}"),
        }
        match &flow.nodes[3].kind {
            NodeKind::Delay(p) => {
                assert_eq!(p.for_, "5m");
                assert_eq!(p.up_to, None);
            }
            other => panic!("expected delay, got {other:?}"),
        }
        assert_eq!(flow.edges[1].port.as_deref(), Some("true"));
        // Wire shape survives a round trip.
        assert_eq!(Flow::decode(flow.encode().as_bytes()).unwrap(), flow);
        let wire: Value = serde_json::from_str(&flow.encode()).unwrap();
        assert_eq!(wire["nodes"][3]["params"]["for"], json!("5m"));
        assert_eq!(wire["nodes"][3]["type"], json!("delay"));
        assert!(wire["nodes"][3]["params"].get("up_to").is_none());
    }

    #[test]
    fn delay_up_to_round_trips() {
        let node: Node =
            serde_json::from_str(r#"{"id":"w","type":"delay","params":{"for":"1m","up_to":"2h"}}"#)
                .unwrap();
        match &node.kind {
            NodeKind::Delay(p) => {
                assert_eq!(p.up_to.as_deref(), Some("2h"));
                assert!(p.extra.is_empty());
            }
            other => panic!("expected delay, got {other:?}"),
        }
        assert_eq!(
            serde_json::to_value(&node).unwrap()["params"]["up_to"],
            "2h"
        );
    }

    #[test]
    fn enabled_defaults_true_and_mode_defaults_single() {
        let flow = Flow::decode(br#"{"id":"a","name":"A"}"#).unwrap();
        assert!(flow.enabled);
        assert_eq!(flow.mode, FlowMode::Single);
        assert!(flow.nodes.is_empty() && flow.edges.is_empty());
        assert!(Flow::default().enabled);
        let nulls = Flow::decode(br#"{"id":"a","name":"A","enabled":null,"mode":null}"#).unwrap();
        assert!(nulls.enabled);
    }

    #[test]
    fn unknown_keys_round_trip() {
        let payload = br#"{"id":"a","name":"A","colour":"red","nodes":[
            {"id":"w","type":"delay","params":{"for":"1m","note":"x"},"tag":1}]}"#;
        let flow = Flow::decode(payload).unwrap();
        assert_eq!(flow.extra.get("colour"), Some(&json!("red")));
        assert_eq!(flow.nodes[0].extra.get("tag"), Some(&json!(1)));
        let again = Flow::decode(flow.encode().as_bytes()).unwrap();
        assert_eq!(again, flow);
        match &again.nodes[0].kind {
            NodeKind::Delay(p) => assert_eq!(p.extra.get("note"), Some(&json!("x"))),
            other => panic!("expected delay, got {other:?}"),
        }
    }

    #[test]
    fn unknown_node_type_is_an_error() {
        let payload =
            br#"{"id":"a","name":"A","nodes":[{"id":"t","type":"trigger.time","params":{}}]}"#;
        let err = Flow::decode(payload).unwrap_err();
        assert!(err.contains("trigger.time"), "{err}");
    }

    #[test]
    fn ports_of_if_are_true_false_and_others_have_none() {
        let flow = Flow::decode(REFERENCE.as_bytes()).unwrap();
        assert_eq!(flow.nodes[1].kind.ports(), vec!["true", "false"]);
        assert_eq!(flow.nodes[1].kind.type_name(), "if");
        for i in [0, 2, 3, 4] {
            assert!(flow.nodes[i].kind.ports().is_empty());
        }
        assert_eq!(flow.nodes[0].kind.type_name(), "trigger.state");
    }

    #[test]
    fn switch_round_trips_and_lists_ports() {
        let node: Node = serde_json::from_value(json!({ "id": "s", "type": "switch", "params": {
            "entity": "t", "rules": [{ "op": "lt", "value": 18 }, { "op": "between", "value": 18, "to": 22, "x": 1 }],
            "otherwise": true } })).unwrap();
        assert_eq!(node.kind.ports(), vec!["1", "2", "otherwise"]);
        let back = serde_json::to_value(&node).unwrap();
        assert_eq!(back["params"]["rules"][1]["x"], 1);
        assert!(back["params"].get("all").is_none());
    }

    #[test]
    fn time_between_ports() {
        let node: Node = serde_json::from_value(json!({ "id": "t", "type": "time.between",
            "params": { "from": "07:00", "to": "22:30" } }))
        .unwrap();
        assert_eq!(node.kind.ports(), vec!["in", "out"]);
        assert_eq!(node.kind.type_name(), "time.between");
    }
}
