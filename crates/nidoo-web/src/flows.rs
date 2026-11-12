//! Pure helpers behind the flow screens: labels, the run highlight and run times (geometry
//! lives in `flow_layout`, sentences in `flow_text`, pan and zoom in `flow_viewport`). No
//! leptos and no browser types, so everything here is tested natively.

use std::collections::{BTreeMap, BTreeSet};

use nidoo_api::{EntityView, RunStatus, RunView, StepOutcome, StepView};
use nidoo_core::command::command_name;
use nidoo_core::flow::{Flow, Node, NodeKind, parse_duration};

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
pub fn trigger_summary(lang: Lang, flow: &Flow, entities: &BTreeMap<String, EntityView>) -> String {
    let parts: Vec<String> = flow
        .nodes
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::TriggerState(p) => Some(entity_label(lang, &p.entity, entities)),
            NodeKind::TriggerSignal(p) => Some(t::sends(
                lang,
                &entity_label(lang, &p.entity, entities),
                p.signal.as_deref(),
            )),
            NodeKind::TriggerSchedule(p) => Some(describe(lang, &p.cron)),
            NodeKind::TriggerSun(p) => Some(sun_summary(lang, p.which, p.offset())),
            NodeKind::TriggerManual(_) => Some(flow_text(lang, FlowText::NodeManual).into()),
            NodeKind::TriggerSilent(p) => Some(t::silent_for(
                lang,
                &entity_label(lang, &p.entity, entities),
                &p.for_,
            )),
            _ => None,
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
mod tests {
    use super::*;
    use nidoo_api::TriggerInfo;

    fn ported(mut s: StepView, ports: &[&str]) -> StepView {
        s.ports = Some(ports.iter().map(|p| (*p).to_owned()).collect());
        s
    }

    fn switch_flow() -> Flow {
        Flow::decode(
            br#"{
      "id": "f", "name": "F", "mode": "restart",
      "nodes": [
        { "id": "t", "type": "trigger.state", "name": "T", "params": { "entity": "m" } },
        { "id": "sw", "type": "switch", "params": { "entity": "m", "rules": [
          { "op": "eq", "value": 1 }, { "op": "eq", "value": 2 }, { "op": "eq", "value": 3 } ] } },
        { "id": "a", "type": "command", "params": { "entity": "l", "command": { "type": "turn_on" } } },
        { "id": "wait", "type": "delay", "params": { "for": "5m" } },
        { "id": "c", "type": "command", "params": { "entity": "l", "command": { "type": "turn_off" } } },
        { "id": "d", "type": "command", "params": { "entity": "l", "command": { "type": "turn_off" } } }
      ],
      "edges": [
        { "from": "t", "to": "sw" },
        { "from": "sw", "port": "1", "to": "a" },
        { "from": "sw", "port": "2", "to": "wait" },
        { "from": "sw", "port": "3", "to": "c" },
        { "from": "sw", "port": "otherwise", "to": "d" }
      ]
    }"#,
        )
        .unwrap()
    }

    fn reference() -> Flow {
        Flow::decode(
            br#"{
      "id": "f", "name": "F", "mode": "restart",
      "nodes": [
        { "id": "motion", "type": "trigger.state", "name": "Motion", "params": { "entity": "m" } },
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

    #[test]
    fn summary_of_reference_flow() {
        assert_eq!(
            summary(Lang::En, &reference()),
            "Motion → If → Turn on / Delay 5m → Turn off"
        );
    }

    fn step(node: &str, outcome: StepOutcome, detail: &str) -> StepView {
        StepView {
            node: node.into(),
            at: 1,
            outcome,
            detail: detail.into(),
            ports: None,
        }
    }

    #[test]
    fn highlight_lights_the_false_branch() {
        let run = RunView {
            id: "r".into(),
            flow_id: "f".into(),
            status: RunStatus::Running,
            started_at: 1,
            ended_at: None,
            trigger: None,
            manual: false,
            steps: vec![
                step("motion", StepOutcome::Ok, ""),
                step("moving", StepOutcome::Ok, "false"),
                step("wait", StepOutcome::Ok, "5m"),
            ],
        };
        let h = highlight(&reference(), Some(&run));
        let nodes: Vec<_> = h.nodes.keys().map(String::as_str).collect();
        assert_eq!(nodes, ["motion", "moving", "wait"]);
        let edge =
            |f: &str, p: Option<&str>, t: &str| (f.to_owned(), p.map(str::to_owned), t.to_owned());
        assert_eq!(
            h.edges,
            BTreeSet::from([
                edge("motion", None, "moving"),
                edge("moving", Some("false"), "wait"),
            ])
        );
        assert!(!h.edges.contains(&edge("moving", Some("true"), "on")));
        assert_eq!(highlight(&reference(), None), Highlight::default());
    }

    fn lamp(id: &str) -> EntityView {
        use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
        EntityView {
            id: id.into(),
            device_id: None,
            name: "Lampe".into(),
            device_name: None,
            label: None,
            kind: Kind::Light,
            component: Kind::Light.component().to_string(),
            protocol_kind: Kind::Light,
            device_class: None,
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: "salon".into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride::default(),
            state: EntityState::default(),
        }
    }

    fn running(steps: Vec<StepView>) -> RunView {
        RunView {
            id: "r".into(),
            flow_id: "f".into(),
            status: RunStatus::Running,
            started_at: 1,
            ended_at: None,
            trigger: None,
            manual: false,
            steps,
        }
    }

    #[test]
    fn parked_node_follows_the_branch_taken() {
        let flow = reference();
        let mut moving = step("moving", StepOutcome::Ok, "false");
        moving.at = 100;
        let run = running(vec![step("motion", StepOutcome::Ok, ""), moving]);
        assert_eq!(parked_node(&flow, &run), Some(("wait".to_owned(), 100)));
        assert_eq!(parked_deadline(&flow, "wait", 100), Some(400));
        // A random delay counts down to the latest it can end.
        let mut random = flow.clone();
        for node in &mut random.nodes {
            if let NodeKind::Delay(p) = &mut node.kind {
                p.up_to = Some("10m".into());
            }
        }
        assert_eq!(parked_deadline(&random, "wait", 100), Some(700));
        let label = random.node("wait").map(|n| summary_label(Lang::En, n));
        assert_eq!(label.as_deref(), Some("Delay 5m–10m"));
        // The true branch leads to a command, not a delay.
        let run = running(vec![step("moving", StepOutcome::Ok, "true")]);
        assert_eq!(parked_node(&flow, &run), None);
        // A finished run, or one with no steps, parks nowhere.
        let mut done = running(vec![step("moving", StepOutcome::Ok, "false")]);
        done.status = RunStatus::Done;
        assert_eq!(parked_node(&flow, &done), None);
        assert_eq!(parked_node(&flow, &running(vec![])), None);
        // A plain node's successors are followed without a port.
        let mut chain = reference();
        chain.edges.retain(|e| e.from != "motion");
        chain.edges.push(nidoo_core::flow::Edge {
            from: "motion".into(),
            port: None,
            to: "wait".into(),
        });
        let run = running(vec![step("motion", StepOutcome::Ok, "")]);
        assert_eq!(parked_node(&chain, &run).map(|p| p.0), Some("wait".into()));
    }

    #[test]
    fn highlight_lights_the_edge_into_the_parked_delay() {
        let run = running(vec![
            step("motion", StepOutcome::Ok, ""),
            step("moving", StepOutcome::Ok, "false"),
        ]);
        let h = highlight(&reference(), Some(&run));
        assert!(h.edges.contains(&(
            "moving".to_owned(),
            Some("false".to_owned()),
            "wait".to_owned()
        )));
        assert!(!h.nodes.contains_key("wait"));
    }

    #[test]
    fn trigger_line_reads_off_peak_runs() {
        let none = BTreeMap::new();
        let mut run = running(vec![step("motion", StepOutcome::Ok, "")]);
        let mut flow = reference();
        let motion = flow.nodes.iter_mut().find(|n| n.id == "motion").unwrap();
        motion.kind = NodeKind::TriggerOffPeak(
            serde_json::from_value(serde_json::json!({"edge": "start"})).unwrap(),
        );
        // The device that decided rides along as the entity; the line still names the edge.
        run.trigger = Some(TriggerInfo {
            entity: Some("sensor.tariff".into()),
            field: None,
            value: serde_json::json!("start"),
            previous: serde_json::json!("end"),
        });
        assert_eq!(
            trigger_line(Lang::En, &run, &flow, &none),
            "Off-peak starts"
        );
        // The windows decided (no entity); the node that fired names the edge.
        let motion = flow.nodes.iter_mut().find(|n| n.id == "motion").unwrap();
        motion.kind = NodeKind::TriggerOffPeak(
            serde_json::from_value(serde_json::json!({"edge": "end"})).unwrap(),
        );
        run.trigger = Some(TriggerInfo {
            entity: None,
            field: None,
            value: serde_json::json!("end"),
            previous: serde_json::json!("start"),
        });
        assert_eq!(trigger_line(Lang::En, &run, &flow, &none), "Off-peak ends");
    }

    #[test]
    fn trigger_line_names_what_fired_the_run() {
        let none = BTreeMap::new();
        let mut run = running(vec![step("motion", StepOutcome::Ok, "")]);
        run.trigger = Some(TriggerInfo {
            entity: Some("m".into()),
            field: None,
            value: serde_json::json!(true),
            previous: serde_json::json!(false),
        });
        assert_eq!(trigger_line(Lang::En, &run, &reference(), &none), "m = on");
        let lamp = lamp("m");
        let lamps: BTreeMap<_, _> = [("m".to_owned(), lamp)].into();
        run.trigger = Some(TriggerInfo {
            entity: Some("m".into()),
            field: None,
            value: serde_json::json!(true),
            previous: serde_json::json!(false),
        });
        run.manual = false;
        assert_eq!(
            trigger_line(Lang::Fr, &run, &reference(), &lamps),
            "Lampe = allumée"
        );
        // Only a run started by hand is manual, whatever its trigger node.
        run.trigger = None;
        run.manual = true;
        assert_eq!(trigger_line(Lang::En, &run, &reference(), &none), "manual");
        // A press on a Manual card with a value starts the run with it, with no entity.
        run.trigger = Some(TriggerInfo {
            entity: None,
            field: None,
            value: serde_json::json!(true),
            previous: serde_json::Value::Null,
        });
        assert_eq!(
            trigger_line(Lang::En, &run, &reference(), &none),
            "manual · on"
        );
        run.trigger = None;
        // A timed run carries no trigger; its first step names the trigger node.
        let mut timed = reference();
        let motion = timed.nodes.iter_mut().find(|n| n.id == "motion").unwrap();
        motion.kind = NodeKind::TriggerSchedule(
            serde_json::from_value(serde_json::json!({"cron": "0 7 * * *"})).unwrap(),
        );
        run.manual = false;
        assert_eq!(trigger_line(Lang::En, &run, &timed, &none), "schedule");
        let motion = timed.nodes.iter_mut().find(|n| n.id == "motion").unwrap();
        motion.kind = NodeKind::TriggerSun(
            serde_json::from_value(serde_json::json!({"which": "sunset"})).unwrap(),
        );
        assert_eq!(trigger_line(Lang::En, &run, &timed, &none), "sun");
    }

    #[test]
    fn orphans_are_non_triggers_without_incoming_edges() {
        let mut flow = reference();
        assert!(orphans(&flow).is_empty());
        flow.edges.retain(|e| e.to != "off");
        assert_eq!(orphans(&flow), BTreeSet::from(["off".to_owned()]));
    }

    fn node(json: &str) -> Node {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn new_nodes_have_summary_labels() {
        let schedule = node(
            r#"{ "id": "s", "type": "trigger.schedule", "params": { "cron": "*/10 * * * *" } }"#,
        );
        let this = node(r#"{ "id": "a", "type": "flow.enable", "params": { "enabled": false } }"#);
        let other = node(
            r#"{ "id": "b", "type": "flow.enable", "params": { "flow": "force-heating-loop", "enabled": true } }"#,
        );
        assert_eq!(summary_label(Lang::En, &schedule), "Every 10 minutes");
        assert_eq!(summary_label(Lang::En, &this), "Disable this flow");
        assert_eq!(summary_label(Lang::En, &other), "Enable force-heating-loop");
    }

    #[test]
    fn trigger_summary_joins_every_trigger() {
        let entities = BTreeMap::new();
        let mut flow = reference();
        assert_eq!(trigger_summary(Lang::En, &flow, &entities), "m");
        flow.nodes.push(node(
            r#"{ "id": "s", "type": "trigger.schedule", "params": { "cron": "*/10 * * * *" } }"#,
        ));
        assert_eq!(
            trigger_summary(Lang::En, &flow, &entities),
            "m · Every 10 minutes"
        );
        flow.nodes.retain(|n| !n.kind.is_trigger());
        assert_eq!(trigger_summary(Lang::En, &flow, &entities), "No trigger");
    }

    #[test]
    fn trigger_summary_describes_sun() {
        let flow = Flow::decode(
            br#"{
          "id": "f", "name": "F",
          "nodes": [
            { "id": "s", "type": "trigger.sun", "params": { "which": "sunset", "offset_minutes": 15 } }
          ],
          "edges": []
        }"#,
        )
        .unwrap();
        assert_eq!(
            trigger_summary(Lang::En, &flow, &BTreeMap::new()),
            "15\u{a0}min after sunset"
        );
    }

    #[test]
    fn schedule_triggers_are_not_orphans() {
        let flow = Flow::decode(
            br#"{
          "id": "f", "name": "F",
          "nodes": [
            { "id": "tick", "type": "trigger.schedule", "params": { "cron": "* * * * *" } },
            { "id": "on", "type": "command", "params": { "entity": "l", "command": { "type": "turn_on" } } }
          ],
          "edges": [ { "from": "tick", "to": "on" } ]
        }"#,
        )
        .unwrap();
        assert!(orphans(&flow).is_empty());
    }

    #[test]
    fn labels_and_statuses_in_french() {
        assert_eq!(
            summary(Lang::Fr, &reference()),
            "Motion → Si → Allumer / Attente 5m → Éteindre"
        );
        assert_eq!(
            trigger_summary(Lang::Fr, &Flow::default(), &BTreeMap::new()),
            "Aucun déclencheur"
        );
        assert_eq!(status_word(Lang::Fr, RunStatus::Done), "Terminé");
        let none = BTreeMap::new();
        let mut run = running(vec![]);
        run.manual = true;
        assert_eq!(trigger_line(Lang::Fr, &run, &reference(), &none), "manuel");
        let this = node(r#"{ "id": "a", "type": "flow.enable", "params": { "enabled": false } }"#);
        assert_eq!(
            summary_label(Lang::Fr, &this),
            "Désactiver cette automatisation"
        );
    }

    #[test]
    fn status_words_and_classes() {
        assert_eq!(status_word(Lang::En, RunStatus::Cancelled), "Cancelled");
        assert_eq!(status_class(RunStatus::Running), "run--running");
        assert_eq!(status_class(RunStatus::Skipped), "run--skipped");
    }

    #[test]
    fn step_offsets_and_run_durations_read_as_short_log_times() {
        assert_eq!(step_offset(1_000, 1_000), "+0\u{a0}s");
        assert_eq!(step_offset(1_045, 1_000), "+45\u{a0}s");
        assert_eq!(step_offset(1_130, 1_000), "+2\u{a0}min 10\u{a0}s");
        assert_eq!(
            step_offset(1_000 + 3_600 + 180, 1_000),
            "+1\u{a0}h 03\u{a0}min"
        );
        // Clock skew does not print a negative.
        assert_eq!(step_offset(100, 500), "+0\u{a0}s");
        assert_eq!(run_duration(1_000, Some(1_045)), Some("45\u{a0}s".into()));
        assert_eq!(
            run_duration(1_000, Some(1_130)),
            Some("2\u{a0}min 10\u{a0}s".into())
        );
        assert_eq!(run_duration(1_000, None), None);
    }

    #[test]
    fn highlight_reads_step_ports() {
        let run = running(vec![
            step("motion", StepOutcome::Ok, ""),
            ported(
                step("moving", StepOutcome::Ok, "average 30m = 18.6 → true"),
                &["true"],
            ),
            step("on", StepOutcome::Ok, ""),
        ]);
        let h = highlight(&reference(), Some(&run));
        let edge = |p: &str, t: &str| ("moving".to_owned(), Some(p.to_owned()), t.to_owned());
        assert!(h.edges.contains(&edge("true", "on")));
        assert!(!h.edges.contains(&edge("false", "wait")));
    }

    #[test]
    fn highlight_switch_all_exits() {
        let run = running(vec![
            step("t", StepOutcome::Ok, ""),
            ported(step("sw", StepOutcome::Ok, ""), &["1", "3"]),
            step("a", StepOutcome::Ok, ""),
            step("c", StepOutcome::Ok, ""),
        ]);
        let h = highlight(&switch_flow(), Some(&run));
        let lit: Vec<_> = h
            .edges
            .iter()
            .filter(|(f, _, _)| f == "sw")
            .map(|(_, p, _)| p.clone().unwrap())
            .collect();
        assert_eq!(lit, ["1", "3"]);
    }

    #[test]
    fn parked_on_delay_behind_switch_exit() {
        let mut sw = ported(step("sw", StepOutcome::Ok, ""), &["2"]);
        sw.at = 50;
        let run = running(vec![step("t", StepOutcome::Ok, ""), sw]);
        assert_eq!(
            parked_node(&switch_flow(), &run),
            Some(("wait".to_owned(), 50))
        );
        // A switch that took exit 1 is not parked on the delay behind exit 2.
        let run = running(vec![ported(step("sw", StepOutcome::Ok, ""), &["1"])]);
        assert_eq!(parked_node(&switch_flow(), &run), None);
    }

    #[test]
    fn highlight_if_without_ports_falls_back_to_detail() {
        let run = running(vec![
            step("motion", StepOutcome::Ok, ""),
            step("moving", StepOutcome::Ok, "true"),
            step("on", StepOutcome::Ok, ""),
        ]);
        let h = highlight(&reference(), Some(&run));
        assert!(h.edges.contains(&(
            "moving".to_owned(),
            Some("true".to_owned()),
            "on".to_owned()
        )));
        assert!(
            !h.edges
                .iter()
                .any(|(_, p, _)| p.as_deref() == Some("false"))
        );
    }
}
