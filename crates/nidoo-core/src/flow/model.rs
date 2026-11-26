//! The flow document: nodes, edges and the lenient JSON form stored on the broker.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::labels::LabelAction;
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
    /// Label ids the UI groups the flow under; they do not change what the flow does.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
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
            labels: Vec::new(),
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
    /// Starts a run a set number of minutes before the next enabled wake-up alarm.
    #[serde(rename = "trigger.wake_up")]
    TriggerWakeUp(TriggerWakeUpParams),
    /// Starts a run when its card is pressed, optionally carrying a fixed value.
    #[serde(rename = "trigger.manual")]
    TriggerManual(TriggerManualParams),
    /// Starts a run when a remote, an event entity or a tag scanner sends a signal.
    #[serde(rename = "trigger.signal")]
    TriggerSignal(TriggerSignalParams),
    /// Starts a run when off-peak starts or ends (from the off-peak device, else the windows).
    #[serde(rename = "trigger.off_peak")]
    TriggerOffPeak(TriggerOffPeakParams),
    /// Starts a run when the built-in alarm changes mode.
    #[serde(rename = "trigger.alarm")]
    TriggerAlarm(TriggerAlarmParams),
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
    /// Picks a label of a choice, or turns every flow carrying a label on or off.
    #[serde(rename = "flow.label")]
    FlowLabel(FlowLabelParams),
    /// Backs up the hub to the FTP server set in Settings → Backup and waits until it ends;
    /// continues on `ok` or `failed`.
    #[serde(rename = "backup.run")]
    BackupRun(BackupRunParams),
    /// Holds the setpoint of climate zones for a while, then lets the schedule take over again.
    #[serde(rename = "climate.hold")]
    ClimateHold(ClimateHoldParams),
    /// Puts climate zones in eco for a while, then lets the schedule take over again.
    #[serde(rename = "climate.eco")]
    ClimateEco(ClimateEcoParams),
    /// Sends a phone push notification.
    #[serde(rename = "notify")]
    Notify(NotifyParams),
    /// Says a sentence on the speaker satellites of one room, or of every room.
    #[serde(rename = "speak")]
    Speak(SpeakParams),
    /// Says a question on the satellites and branches on the spoken answer.
    #[serde(rename = "ask")]
    Ask(AskParams),
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
            NodeKind::TriggerWakeUp(_) => "trigger.wake_up",
            NodeKind::TriggerManual(_) => "trigger.manual",
            NodeKind::TriggerSignal(_) => "trigger.signal",
            NodeKind::TriggerOffPeak(_) => "trigger.off_peak",
            NodeKind::TriggerAlarm(_) => "trigger.alarm",
            NodeKind::TriggerSilent(_) => "trigger.silent",
            NodeKind::If(_) => "if",
            NodeKind::Delay(_) => "delay",
            NodeKind::Limit(_) => "limit",
            NodeKind::Command(_) => "command",
            NodeKind::FlowEnable(_) => "flow.enable",
            NodeKind::FlowLabel(_) => "flow.label",
            NodeKind::BackupRun(_) => "backup.run",
            NodeKind::ClimateHold(_) => "climate.hold",
            NodeKind::ClimateEco(_) => "climate.eco",
            NodeKind::Notify(_) => "notify",
            NodeKind::Speak(_) => "speak",
            NodeKind::Ask(_) => "ask",
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
                | NodeKind::TriggerWakeUp(_)
                | NodeKind::TriggerManual(_)
                | NodeKind::TriggerSignal(_)
                | NodeKind::TriggerOffPeak(_)
                | NodeKind::TriggerAlarm(_)
                | NodeKind::TriggerSilent(_)
        )
    }

    /// The catch-up window in minutes of a schedule, sun or wake-up trigger; 0 for every other
    /// kind and when absent.
    pub fn catch_up_minutes(&self) -> u32 {
        match self {
            NodeKind::TriggerSchedule(p) => p.catch_up_minutes,
            NodeKind::TriggerSun(p) => p.catch_up_minutes,
            NodeKind::TriggerWakeUp(p) => p.catch_up_minutes,
            _ => None,
        }
        .unwrap_or(0)
    }

    /// The named output ports: `true` and `false` for `if`; `1` to `n` (then `otherwise` when
    /// set) for `switch`; `in` and `out` for `time.between`; `1` to `n` then `none` for `ask`;
    /// none for the others (which have one unnamed output).
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
            NodeKind::BackupRun(_) => vec![
                BACKUP_OK_PORT.into(),
                BACKUP_PARTIAL_PORT.into(),
                BACKUP_FAILED_PORT.into(),
            ],
            NodeKind::Ask(p) => {
                let mut ports: Vec<String> = (1..=p.answers.len()).map(|i| i.to_string()).collect();
                ports.push(ASK_NONE_PORT.into());
                ports
            }
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
    /// Only trigger when a number crosses above this: the previous value was not above it and the
    /// new one is (`25` fires once on 24.8 → 25.3, not on 25.3 → 25.6). With `below` too, fires
    /// when the value enters the range between them. Not with `to` or `from`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub above: Option<f64>,
    /// Only trigger when a number crosses below this (see `above`). Not with `to` or `from`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub below: Option<f64>,
    /// Fire only once the condition has held this long: a duration such as `30s`, `5m` or
    /// `1h30m`, from `1s` to `168h` ("no motion for 5 min": `to: false`, `for: "5m"`). A report
    /// that breaks the condition cancels the wait; without `to`, `above` or `below` the value
    /// must stay unchanged. A wait in progress is lost when the hub restarts.
    #[serde(rename = "for", default, skip_serializing_if = "Option::is_none")]
    pub for_: Option<String>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

impl TriggerStateParams {
    /// Whether `value` satisfies the `above` / `below` bounds; `None` when the trigger has neither.
    /// A value that is not a number never satisfies them.
    pub fn in_bounds(&self, value: &Scalar) -> Option<bool> {
        if self.above.is_none() && self.below.is_none() {
            return None;
        }
        let Scalar::Number(n) = value else {
            return Some(false);
        };
        Some(self.above.is_none_or(|a| *n > a) && self.below.is_none_or(|b| *n < b))
    }
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
    /// Minutes after a missed fire during which a late hub start still runs the flow; absent or
    /// 0 means never catch up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catch_up_minutes: Option<u32>,
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
    /// Minutes after a missed fire during which a late hub start still runs the flow; absent or
    /// 0 means never catch up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catch_up_minutes: Option<u32>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

impl TriggerSunParams {
    /// The offset in minutes, 0 when absent.
    pub fn offset(&self) -> i32 {
        self.offset_minutes.unwrap_or(0)
    }
}

/// Params of `trigger.wake_up`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TriggerWakeUpParams {
    /// Minutes before the alarm time to fire (0 to 240).
    pub before_minutes: u32,
    /// Only these wake-up alarms (by id); empty means any enabled wake-up alarm.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alarms: Vec<String>,
    /// Minutes after a missed fire during which a late hub start still runs the flow; absent or
    /// 0 means never catch up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catch_up_minutes: Option<u32>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// A target temperature in degrees Celsius, or `Off`. JSON: a number or the string `"off"`
/// (the same as `nidoo_climate::doc::Setpoint`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClimateSetpoint {
    Temp(f64),
    Off,
}

impl Serialize for ClimateSetpoint {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            ClimateSetpoint::Temp(t) => s.serialize_f64(*t),
            ClimateSetpoint::Off => s.serialize_str("off"),
        }
    }
}

impl<'de> Deserialize<'de> for ClimateSetpoint {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Num(f64),
            Str(String),
        }
        match Repr::deserialize(d)? {
            Repr::Num(t) => Ok(ClimateSetpoint::Temp(t)),
            Repr::Str(s) if s == "off" => Ok(ClimateSetpoint::Off),
            Repr::Str(s) => Err(serde::de::Error::custom(format!(
                "expected a number or \"off\", got {s:?}"
            ))),
        }
    }
}

#[cfg(feature = "schemars")]
impl schemars::JsonSchema for ClimateSetpoint {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ClimateSetpoint".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "oneOf": [{ "type": "number" }, { "const": "off" }] })
    }
}

/// Params of `climate.hold`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ClimateHoldParams {
    /// Climate zone ids; at least one.
    pub zones: Vec<String>,
    /// The temperature to hold in °C, or `"off"`.
    pub setpoint: ClimateSetpoint,
    /// `heat` or `cool`; absent keeps the zone's current mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Hold for this many minutes (1 to 1440). Exactly one of `minutes` and `until`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes: Option<u32>,
    /// Hold until this local time (`HH:MM`, 00:00 to 23:59).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// Params of `climate.eco`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ClimateEcoParams {
    /// Climate zone ids; at least one.
    pub zones: Vec<String>,
    /// Stay in eco for this many minutes (1 to 1440). Exactly one of `minutes` and `until`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes: Option<u32>,
    /// Stay in eco until this local time (`HH:MM`, 00:00 to 23:59).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
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

/// A mode of the built-in alarm a `trigger.alarm` filters on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum AlarmModeFilter {
    Disarmed,
    Arming,
    /// Any of `armed_away`, `armed_home` and `armed_night`.
    Armed,
    ArmedAway,
    ArmedHome,
    ArmedNight,
    Pending,
    Triggered,
}

impl AlarmModeFilter {
    /// Every filter, in the order an editor lists them.
    pub const ALL: [AlarmModeFilter; 8] = [
        AlarmModeFilter::Disarmed,
        AlarmModeFilter::Armed,
        AlarmModeFilter::ArmedAway,
        AlarmModeFilter::ArmedHome,
        AlarmModeFilter::ArmedNight,
        AlarmModeFilter::Arming,
        AlarmModeFilter::Pending,
        AlarmModeFilter::Triggered,
    ];

    /// The name this filter carries in the flow JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            AlarmModeFilter::Disarmed => "disarmed",
            AlarmModeFilter::Arming => "arming",
            AlarmModeFilter::Armed => "armed",
            AlarmModeFilter::ArmedAway => "armed_away",
            AlarmModeFilter::ArmedHome => "armed_home",
            AlarmModeFilter::ArmedNight => "armed_night",
            AlarmModeFilter::Pending => "pending",
            AlarmModeFilter::Triggered => "triggered",
        }
    }

    /// Whether the alarm state name `state` (such as `armed_away`) is covered by this filter.
    pub fn matches(self, state: &str) -> bool {
        match self {
            AlarmModeFilter::Armed => {
                matches!(state, "armed_away" | "armed_home" | "armed_night")
            }
            other => other.as_str() == state,
        }
    }
}

/// Params of `trigger.alarm`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TriggerAlarmParams {
    /// Only trigger when the alarm enters this mode; absent means any change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<AlarmModeFilter>,
    /// Only trigger when the alarm was in this mode before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<AlarmModeFilter>,
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

/// Params of `flow.label`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct FlowLabelParams {
    /// The label id (see `list_flow_labels`).
    pub label: String,
    /// `pick` turns the label's flows on and the flows of its choice's other labels off (only
    /// for a label in a choice); `on` / `off` switch every flow carrying the label.
    #[serde(default)]
    pub action: LabelAction,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// The port `backup.run` takes when the backup is written.
pub const BACKUP_OK_PORT: &str = "ok";
/// The port `backup.run` takes when the backup is written without Zigbee2MQTT's data. Left
/// unwired, the run goes on from [`BACKUP_OK_PORT`].
pub const BACKUP_PARTIAL_PORT: &str = "partial";
/// The port `backup.run` takes when the backup fails, or the wait ends first.
pub const BACKUP_FAILED_PORT: &str = "failed";

/// Where `backup.run` sends the backup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum BackupTarget {
    /// The FTP server set in Settings → Backup.
    #[default]
    Ftp,
}

/// Params of `backup.run`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct BackupRunParams {
    /// Where the backup goes; only `ftp` (the default) exists.
    #[serde(default)]
    pub destination: BackupTarget,
    /// The time limit in minutes (1 to 1440, default 120). When the step joins a backup already
    /// running, it is how long the step waits: on expiry it takes `failed` and the backup goes on.
    #[serde(default = "default_backup_timeout")]
    pub timeout_minutes: u32,
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

impl Default for BackupRunParams {
    fn default() -> Self {
        Self {
            destination: BackupTarget::default(),
            timeout_minutes: default_backup_timeout(),
            extra: BTreeMap::new(),
        }
    }
}

fn default_backup_timeout() -> u32 {
    120
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

/// Params of `speak`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SpeakParams {
    /// The sentence to say; required, not empty.
    /// Takes `{entity_id}` placeholders filled at run time, as `notify` does.
    pub text: String,
    /// The room (an area id; a room name is resolved when the flow is saved). Absent means
    /// every room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
}

/// The port an `ask` card leaves by when nobody answered or the answer matched nothing.
pub const ASK_NONE_PORT: &str = "none";

/// The most answers an `ask` card takes.
pub const ASK_MAX_ANSWERS: usize = 6;

/// One answer of an `ask` card.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AskAnswer {
    /// The answer's name, spoken back and matched against what was heard; not empty.
    pub label: String,
    /// Other words that also mean this answer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<String>,
}

/// Params of `ask`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AskParams {
    /// The question to say; required, not empty. Takes `{entity_id}` placeholders.
    pub text: String,
    /// The room (an area id; a room name is resolved when the flow is saved). Absent means
    /// every room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    /// One to six answers; the card leaves by port `1` to `n` in this order, or `none`.
    pub answers: Vec<AskAnswer>,
    /// Names a number said in the answer, for later cards (same form as a math name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The value of `name` when the answer holds no number; needs `name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<f64>,
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

/// The names of math results, from math cards and ask cards. Two cards may share a name
/// (branches computing the same result).
pub fn math_names(flow: &Flow) -> BTreeSet<&str> {
    flow.nodes
        .iter()
        .filter_map(|node| match &node.kind {
            NodeKind::Math(p) => Some(p.name.as_str()),
            NodeKind::Ask(p) => p.name.as_deref(),
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
mod tests;
