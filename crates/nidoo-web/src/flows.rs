//! Pure helpers behind the flow screens: labels, the run highlight and run times (geometry
//! lives in `flow_layout`, sentences in `flow_text`, pan and zoom in `flow_viewport`). No
//! leptos and no browser types, so everything here is tested natively.

use std::collections::{BTreeMap, BTreeSet};

use nidoo_api::{EntityView, RunStatus, RunView, StepOutcome, StepView};
use nidoo_core::command::command_name;
use nidoo_core::flow::{Flow, Node, NodeKind, parse_duration};

use crate::flow_text::alarm_names;
use crate::flow_text::{off_peak_edge_text, sun_summary, value_words};
use crate::format::entity_title;
use crate::i18n::Lang;
use crate::i18n::flows::{self as t, FlowText, flow_text};
use crate::schedule::describe;

fn entity_label(lang: Lang, id: &str, entities: &BTreeMap<String, EntityView>) -> String {
    entities
        .get(id)
        .map_or_else(|| id.to_owned(), |e| entity_title(lang, e))
}

pub(crate) fn humanise(command: &str) -> String {
    let spaced = command.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `Enable <flow>` / `Disable this flow`: what a flow switch does, in words.
fn flow_switch_text(lang: Lang, p: &nidoo_core::flow::FlowEnableParams) -> String {
    t::flow_switch(
        lang,
        p.enabled,
        p.flow
            .as_deref()
            .unwrap_or_else(|| flow_text(lang, FlowText::ThisFlow)),
    )
}

/// A command's name as a short label: "Turn on" / "Allumer"; a command with no word of its own
/// keeps the prettified id.
fn command_label(lang: Lang, name: &str) -> String {
    let known = match name {
        "turn_on" => Some(FlowText::TurnOn),
        "turn_off" => Some(FlowText::TurnOff),
        "toggle" => Some(FlowText::Toggle),
        "open" => Some(FlowText::Open),
        "close" => Some(FlowText::Close),
        "stop" => Some(FlowText::Stop),
        "press" => Some(FlowText::Press),
        _ => None,
    };
    known.map_or_else(|| humanise(name), |k| flow_text(lang, k).to_owned())
}

fn summary_label(lang: Lang, node: &Node) -> String {
    if let Some(name) = &node.name {
        return name.clone();
    }
    match &node.kind {
        NodeKind::TriggerState(_) => flow_text(lang, FlowText::NodeTrigger).into(),
        NodeKind::TriggerSchedule(p) => describe(lang, &p.cron),
        NodeKind::TriggerSun(_) => flow_text(lang, FlowText::NodeSun).into(),
        NodeKind::TriggerOffPeak(_) => flow_text(lang, FlowText::NodeOffPeak).into(),
        NodeKind::TriggerAlarm(_) => flow_text(lang, FlowText::NodeAlarm).into(),
        NodeKind::TriggerSilent(_) => flow_text(lang, FlowText::NodeSilent).into(),
        NodeKind::Limit(_) => flow_text(lang, FlowText::NodeLimit).into(),
        NodeKind::TriggerManual(_) => flow_text(lang, FlowText::NodeManual).into(),
        NodeKind::TriggerSignal(_) => flow_text(lang, FlowText::NodeSignal).into(),
        NodeKind::If(_) => flow_text(lang, FlowText::NodeIf).into(),
        NodeKind::Delay(p) => {
            let head = flow_text(lang, FlowText::NodeDelay);
            match &p.up_to {
                Some(up_to) => format!("{head} {}–{up_to}", p.for_),
                None => format!("{head} {}", p.for_),
            }
        }
        NodeKind::Command(p) => command_label(lang, command_name(&p.command)),
        NodeKind::FlowEnable(p) => flow_switch_text(lang, p),
        NodeKind::FlowLabel(p) => {
            format!("{}: {}", flow_text(lang, FlowText::NodeFlowLabel), p.label)
        }
        NodeKind::BackupRun(_) => flow_text(lang, FlowText::NodeBackupRun).into(),
        NodeKind::TriggerWakeUp(_) => flow_text(lang, FlowText::NodeWakeUp).into(),
        NodeKind::ClimateHold(_) => flow_text(lang, FlowText::NodeClimateHold).into(),
        NodeKind::ClimateEco(_) => flow_text(lang, FlowText::NodeClimateEco).into(),
        NodeKind::Notify(_) => flow_text(lang, FlowText::Notify).into(),
        NodeKind::Speak(_) => flow_text(lang, FlowText::Speak).into(),
        NodeKind::Ask(_) => flow_text(lang, FlowText::Ask).into(),
        NodeKind::Switch(_) => flow_text(lang, FlowText::SwitchCard).into(),
        NodeKind::TimeBetween(_) => flow_text(lang, FlowText::TimeCard).into(),
        NodeKind::Math(_) => flow_text(lang, FlowText::MathCard).into(),
    }
}

/// Just the trigger node(s) in words: "Every 10 minutes" for a schedule, the entity label
/// for a state trigger. Multiple triggers are joined by " · ".
///
/// `flows` is `(id, name)` of every flow, to name the alarms a wake-up trigger follows.
pub fn trigger_summary(
    lang: Lang,
    flow: &Flow,
    entities: &BTreeMap<String, EntityView>,
    flows: &[(String, String)],
) -> String {
    let parts: Vec<String> = flow
        .nodes
        .iter()
        .filter_map(|n| {
            let text = match &n.kind {
                NodeKind::TriggerState(p) => Some(entity_label(lang, &p.entity, entities)),
                NodeKind::TriggerSignal(p) => Some(t::sends(
                    lang,
                    &entity_label(lang, &p.entity, entities),
                    p.signal.as_deref(),
                )),
                NodeKind::TriggerSchedule(p) => Some(describe(lang, &p.cron)),
                NodeKind::TriggerSun(p) => Some(sun_summary(lang, p.which, p.offset())),
                NodeKind::TriggerWakeUp(p) => Some(t::wake_up_sentence(
                    lang,
                    p.before_minutes,
                    &alarm_names(&p.alarms, flows),
                )),
                NodeKind::TriggerManual(_) => Some(flow_text(lang, FlowText::NodeManual).into()),
                NodeKind::TriggerSilent(p) => Some(t::silent_for(
                    lang,
                    &entity_label(lang, &p.entity, entities),
                    &p.for_,
                )),
                _ => None,
            };
            // The catch-up tail the card's sentence carries.
            text.map(|text| match n.kind.catch_up_minutes() {
                0 => text,
                m => format!("{text}{}", t::catch_up_suffix(lang, m)),
            })
        })
        .collect();
    if parts.is_empty() {
        flow_text(lang, FlowText::NoTrigger).to_owned()
    } else {
        parts.join(" · ")
    }
}

/// One line for the flow list: the layers joined by ` → `, siblings by ` / `.
pub fn summary(lang: Lang, flow: &Flow) -> String {
    flow.layers()
        .iter()
        .map(|layer| {
            layer
                .iter()
                .filter_map(|id| flow.node(id))
                .map(|n| summary_label(lang, n))
                .collect::<Vec<_>>()
                .join(" / ")
        })
        .collect::<Vec<_>>()
        .join(" → ")
}

/// A JSON value as the cards print it: strings bare, everything else as JSON.
pub fn value_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// What fired a run, for its header: `manual` for a run started by hand, `"{entity title} =
/// {value}"` for an entity change, press or signal, and `schedule` or `sun` for a timed run
/// (read from its first step, the trigger node). Computed here instead of printing the
/// engine's detail, which carries the raw entity id.
pub fn trigger_line(
    lang: Lang,
    run: &RunView,
    flow: &Flow,
    entities: &BTreeMap<String, EntityView>,
) -> String {
    if run.manual {
        let head = flow_text(lang, FlowText::ManualRun);
        // A card with a value starts the run with it: "manual · on".
        return match run.trigger.as_ref().filter(|t| t.entity.is_none()) {
            Some(t) => format!("{head} · {}", value_words(lang, &t.value, None, None)),
            None => head.to_owned(),
        };
    }
    let first = run.steps.first().and_then(|s| flow.node(&s.node));
    // The device that decided rides along as the entity: the edge says more than its reading.
    if let Some(NodeKind::TriggerOffPeak(p)) = first.map(|n| &n.kind) {
        return flow_text(lang, off_peak_edge_text(p.edge)).to_owned();
    }
    // The silent entity rides along as the entity, the window as the value: say it as a sentence.
    if let Some(NodeKind::TriggerSilent(p)) = first.map(|n| &n.kind) {
        return t::silent_for(lang, &entity_label(lang, &p.entity, entities), &p.for_);
    }
    if let Some((trigger, id)) = run
        .trigger
        .as_ref()
        .and_then(|t| t.entity.as_deref().map(|id| (t, id)))
    {
        return format!(
            "{} = {}",
            entity_label(lang, id, entities),
            value_words(
                lang,
                &trigger.value,
                entities.get(id).and_then(|e| e.unit.as_deref()),
                entities.get(id),
            )
        );
    }
    match first.map(|n| &n.kind) {
        Some(NodeKind::TriggerSchedule(_)) => flow_text(lang, FlowText::ScheduleRun).to_owned(),
        Some(NodeKind::TriggerSun(_)) => flow_text(lang, FlowText::SunRun).to_owned(),
        Some(NodeKind::TriggerWakeUp(_)) => flow_text(lang, FlowText::WakeUpRun).to_owned(),
        _ => String::new(),
    }
}

/// The non-trigger nodes nothing leads to; they can never run and are drawn faded.
pub fn orphans(flow: &Flow) -> BTreeSet<String> {
    flow.nodes
        .iter()
        .filter(|n| !n.kind.is_trigger())
        .filter(|n| !flow.edges.iter().any(|e| e.to == n.id))
        .map(|n| n.id.clone())
        .collect()
}

/// The `delay` node a running run is parked on, with the second it started waiting at. The
/// engine records a `delay` step only after the wait, so while it waits the run's last step is
/// the node before it (an `if`, `switch` or `time.between`, whose step records the `ports`
/// taken, with `detail` as the fallback for an `if`; or a command): the parked node is that
/// step's first successor that is a `delay`. The countdown ends at
/// `since + parse_duration(for)`.
pub fn parked_node(flow: &Flow, run: &RunView) -> Option<(String, u64)> {
    if run.status != RunStatus::Running {
        return None;
    }
    let step = run.steps.last()?;
    let node = flow.node(&step.node)?;
    let next: Vec<_> = match taken_ports(node, step) {
        Some(ports) => ports
            .iter()
            .flat_map(|p| flow.successors(&step.node, Some(p)))
            .collect(),
        None if node.kind.ports().is_empty() => flow.successors(&step.node, None),
        None => Vec::new(),
    };
    next.into_iter()
        .find(|n| matches!(n.kind, NodeKind::Delay(_)))
        .map(|n| (n.id.clone(), step.at))
}

/// The ports a step of a branching node (`if`, `switch`, `time.between`) took, or `None` for
/// a node without ports. The hub records them in `ports`; an older hub did not, and an `if`
/// then named its branch in `detail`. A branching step with neither took no port.
fn taken_ports(node: &Node, step: &StepView) -> Option<Vec<String>> {
    if node.kind.ports().is_empty() {
        return None;
    }
    Some(match (&step.ports, &node.kind) {
        (Some(ports), _) => ports.clone(),
        (None, NodeKind::If(_)) => vec![step.detail.clone()],
        (None, _) => Vec::new(),
    })
}

/// The second a parked delay ends at: `since` plus the delay's duration. A random delay's
/// pick stays on the hub, so it counts down to the latest it can end (`up_to`).
pub fn parked_deadline(flow: &Flow, node_id: &str, since: u64) -> Option<u64> {
    let NodeKind::Delay(p) = &flow.node(node_id)?.kind else {
        return None;
    };
    let longest = p.up_to.as_deref().unwrap_or(&p.for_);
    Some(since + parse_duration(longest).ok()?.as_secs())
}

/// What a run touched: the latest outcome of each visited node and the edges it followed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Highlight {
    pub nodes: BTreeMap<String, StepOutcome>,
    /// `(from, port, to)` of every lit edge.
    pub edges: BTreeSet<(String, Option<String>, String)>,
}

/// The highlight of `run` on `flow`. An edge is lit when both ends were visited (or its end is
/// the delay the run is parked on) and, from a branching node, its port is one the latest step
/// of that node took (`ports`; for an `if` from an older hub, its `detail`).
pub fn highlight(flow: &Flow, run: Option<&RunView>) -> Highlight {
    let mut out = Highlight::default();
    let Some(run) = run else { return out };
    let mut taken: BTreeMap<&str, Option<Vec<String>>> = BTreeMap::new();
    for step in &run.steps {
        out.nodes.insert(step.node.clone(), step.outcome);
        let ports = flow.node(&step.node).and_then(|n| taken_ports(n, step));
        taken.insert(&step.node, ports);
    }
    let parked = parked_node(flow, run).map(|(id, _)| id);
    for edge in &flow.edges {
        let to_parked = parked.as_deref() == Some(edge.to.as_str());
        if !out.nodes.contains_key(&edge.from) || !(out.nodes.contains_key(&edge.to) || to_parked) {
            continue;
        }
        let branching = flow
            .node(&edge.from)
            .is_some_and(|n| !n.kind.ports().is_empty());
        if branching {
            let took = taken
                .get(edge.from.as_str())
                .and_then(Option::as_ref)
                .is_some_and(|ports| {
                    ports
                        .iter()
                        .any(|p| Some(p.as_str()) == edge.port.as_deref())
                });
            if !took {
                continue;
            }
        }
        out.edges
            .insert((edge.from.clone(), edge.port.clone(), edge.to.clone()));
    }
    out
}

/// The word shown on a run's badge.
pub fn status_word(lang: Lang, status: RunStatus) -> &'static str {
    t::run_status(lang, status)
}

/// `at - start` as a short log offset: `+0 s`, `+45 s`, `+2 min 10 s`, `+1 h 03 min`, with a
/// no-break space between a number and its unit (the units read the same in both languages). A
/// step whose timestamp is before the run's start reads `+0 s` (clock skew should not go
/// backwards here, but we clamp instead of printing a negative).
pub fn step_offset(at: u64, start: u64) -> String {
    format!("+{}", t::short_span(at.saturating_sub(start)))
}

/// `end - start` as a short run duration (same scale as [`step_offset`] without the `+`), or
/// `None` when the run is still running.
pub fn run_duration(start: u64, end: Option<u64>) -> Option<String> {
    Some(t::short_span(end?.saturating_sub(start)))
}

/// The CSS class of a run's badge.
pub fn status_class(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Running => "run--running",
        RunStatus::Done => "run--done",
        RunStatus::Failed => "run--failed",
        RunStatus::Cancelled => "run--cancelled",
        RunStatus::Skipped => "run--skipped",
    }
}

#[cfg(test)]
mod tests;
