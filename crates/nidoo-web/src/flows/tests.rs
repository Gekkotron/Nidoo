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
    let motion = timed.nodes.iter_mut().find(|n| n.id == "motion").unwrap();
    motion.kind = NodeKind::TriggerWakeUp(
        serde_json::from_value(serde_json::json!({"before_minutes": 45, "alarms": []})).unwrap(),
    );
    assert_eq!(trigger_line(Lang::En, &run, &timed, &none), "wake-up");
    assert_eq!(trigger_line(Lang::Fr, &run, &timed, &none), "réveil");
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
    let schedule =
        node(r#"{ "id": "s", "type": "trigger.schedule", "params": { "cron": "*/10 * * * *" } }"#);
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
    assert_eq!(trigger_summary(Lang::En, &flow, &entities, &[]), "m");
    flow.nodes.push(node(
        r#"{ "id": "s", "type": "trigger.schedule", "params": { "cron": "*/10 * * * *" } }"#,
    ));
    assert_eq!(
        trigger_summary(Lang::En, &flow, &entities, &[]),
        "m · Every 10 minutes"
    );
    flow.nodes.retain(|n| !n.kind.is_trigger());
    assert_eq!(
        trigger_summary(Lang::En, &flow, &entities, &[]),
        "No trigger"
    );
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
        trigger_summary(Lang::En, &flow, &BTreeMap::new(), &[]),
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
        trigger_summary(Lang::Fr, &Flow::default(), &BTreeMap::new(), &[]),
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

#[test]
fn trigger_summary_names_alarms_and_notes_catch_up() {
    let flow = Flow::decode(
        br#"{
          "id": "f", "name": "F",
          "nodes": [
            { "id": "w", "type": "trigger.wake_up",
              "params": { "before_minutes": 45, "alarms": ["wake-up-1"], "catch_up_minutes": 60 } }
          ],
          "edges": []
        }"#,
    )
    .unwrap();
    let names = [("wake-up-1".to_owned(), "Weekdays".to_owned())];
    assert_eq!(
        trigger_summary(Lang::En, &flow, &BTreeMap::new(), &names),
        "45\u{a0}min before Weekdays, catches up within 60\u{a0}min"
    );
}
