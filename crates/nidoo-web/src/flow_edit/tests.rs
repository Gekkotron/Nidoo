use nidoo_core::flow::{
    CommandParams, DelayParams, FlowEnableParams, FlowLabelParams, IfParams, NotifyParams, Op,
    TriggerScheduleParams, TriggerStateParams,
};
use nidoo_core::flow::{RuleOp, SwitchParams, SwitchRule, TimeBetweenParams};
use serde_json::json;

use super::*;

fn state(entity: &str, to: Option<serde_json::Value>) -> NodeKind {
    NodeKind::TriggerState(TriggerStateParams {
        entity: entity.into(),
        to,
        ..Default::default()
    })
}
fn signal(entity: &str, signal: Option<&str>) -> NodeKind {
    NodeKind::TriggerSignal(nidoo_core::flow::TriggerSignalParams {
        entity: entity.into(),
        signal: signal.map(str::to_owned),
        ..Default::default()
    })
}

#[test]
fn picking_a_remote_turns_a_state_trigger_into_a_signal_trigger() {
    let got = retarget_trigger(&state("door", Some(json!(true))), "remote".into(), true);
    assert_eq!(got, signal("remote", None));
}

#[test]
fn picking_a_sensor_turns_a_signal_trigger_back_into_a_state_trigger() {
    let got = retarget_trigger(&signal("remote", Some("on")), "door".into(), false);
    assert_eq!(got, state("door", None));
}

#[test]
fn another_entity_of_the_same_kind_resets_the_filters() {
    let with_field = NodeKind::TriggerState(TriggerStateParams {
        entity: "door".into(),
        field: Some("battery".into()),
        to: Some(json!(true)),
        from: Some(json!(false)),
        ..Default::default()
    });
    assert_eq!(
        retarget_trigger(&with_field, "window".into(), false),
        state("window", None)
    );
    assert_eq!(
        retarget_trigger(&signal("r1", Some("on")), "r2".into(), true),
        signal("r2", None)
    );
}

#[test]
fn the_same_entity_keeps_the_filters() {
    let k = state("door", Some(json!(true)));
    assert_eq!(retarget_trigger(&k, "door".into(), false), k);
    let s = signal("r1", Some("on"));
    assert_eq!(retarget_trigger(&s, "r1".into(), true), s);
}

#[test]
fn clearing_the_entity_keeps_the_card_type() {
    assert_eq!(
        retarget_trigger(&signal("r1", Some("on")), String::new(), false),
        signal("", None)
    );
    assert_eq!(
        retarget_trigger(&state("door", Some(json!(true))), String::new(), true),
        state("", None)
    );
}

#[test]
fn retargeting_keeps_id_position_and_edges() {
    let mut t = node("trigger_1", state("door", None));
    t.position = Some(Position { x: 10.0, y: 20.0 });
    let mut f = flow(vec![
        t,
        node("delay_1", NodeKind::Delay(DelayParams::default())),
    ]);
    f.edges.push(Edge {
        from: "trigger_1".into(),
        to: "delay_1".into(),
        port: None,
    });
    let before = f.clone();
    let n = f.nodes.iter_mut().find(|n| n.id == "trigger_1").unwrap();
    n.kind = retarget_trigger(&n.kind, "remote".into(), true);
    assert_eq!(f.edges, before.edges);
    assert_eq!(f.nodes[0].id, "trigger_1");
    assert_eq!(f.nodes[0].position, before.nodes[0].position);
    assert!(matches!(f.nodes[0].kind, NodeKind::TriggerSignal(_)));
}

#[test]
fn other_cards_are_not_retargeted() {
    let d = NodeKind::Delay(DelayParams::default());
    assert_eq!(retarget_trigger(&d, "door".into(), false), d);
}

fn node(id: &str, kind: NodeKind) -> Node {
    Node {
        id: id.into(),
        name: None,
        position: None,
        kind,
        extra: Default::default(),
    }
}
fn trigger(id: &str) -> Node {
    node(id, NodeKind::TriggerState(TriggerStateParams::default()))
}
fn delay(id: &str) -> Node {
    node(id, NodeKind::Delay(DelayParams::default()))
}
fn if_kind() -> NodeKind {
    NodeKind::If(IfParams {
        op: Op::Eq,
        value: Some(json!(true)),
        entity: None,
        field: None,
        missing: None,
        average: None,
        offset_minutes: None,
        result: None,
        extra: Default::default(),
    })
}
fn flow(nodes: Vec<Node>) -> Flow {
    Flow {
        id: "f".into(),
        name: "F".into(),
        nodes,
        ..Flow::default()
    }
}

#[test]
fn a_manual_card_gets_a_manual_id() {
    let flow = Flow::default();
    let kind = NodeKind::TriggerManual(Default::default());
    assert_eq!(next_node_id(&flow, &kind), "manual_1");
}

#[test]
fn rejects_into_trigger() {
    let f = flow(vec![trigger("t"), delay("d")]);
    assert_eq!(
        can_connect(&f, "d", None, "t"),
        Err(ConnectError::IntoTrigger)
    );
}

#[test]
fn rejects_self_loop() {
    let f = flow(vec![delay("d")]);
    assert_eq!(can_connect(&f, "d", None, "d"), Err(ConnectError::SelfLoop));
}

#[test]
fn rejects_unknown_node() {
    let f = flow(vec![delay("d")]);
    assert_eq!(
        can_connect(&f, "d", None, "x"),
        Err(ConnectError::UnknownNode)
    );
    assert_eq!(
        can_connect(&f, "x", None, "d"),
        Err(ConnectError::UnknownNode)
    );
}

#[test]
fn rejects_duplicate() {
    let mut f = flow(vec![trigger("t"), delay("d")]);
    connect(&mut f, "t", None, "d").unwrap();
    assert_eq!(
        connect(&mut f, "t", None, "d"),
        Err(ConnectError::Duplicate)
    );
    assert_eq!(f.edges.len(), 1);
}

#[test]
fn rejects_cycle() {
    let mut f = flow(vec![delay("a"), delay("b"), delay("c")]);
    connect(&mut f, "a", None, "b").unwrap();
    connect(&mut f, "b", None, "c").unwrap();
    assert_eq!(connect(&mut f, "c", None, "a"), Err(ConnectError::Cycle));
    assert_eq!(f.edges.len(), 2);
}

#[test]
fn if_needs_port() {
    let mut f = flow(vec![trigger("t"), node("i", if_kind()), delay("d")]);
    assert_eq!(can_connect(&f, "i", None, "d"), Err(ConnectError::BadPort));
    assert_eq!(
        can_connect(&f, "i", Some("maybe"), "d"),
        Err(ConnectError::BadPort)
    );
    assert_eq!(
        can_connect(&f, "t", Some("true"), "d"),
        Err(ConnectError::BadPort)
    );
    assert_eq!(connect(&mut f, "i", Some("true"), "d"), Ok(()));
    assert_eq!(connect(&mut f, "i", Some("false"), "d"), Ok(()));
    assert_eq!(f.edges[0].port.as_deref(), Some("true"));
}

#[test]
fn allows_fan_in_and_out() {
    let mut f = flow(vec![trigger("t"), trigger("u"), delay("a"), delay("b")]);
    assert_eq!(connect(&mut f, "t", None, "a"), Ok(()));
    assert_eq!(connect(&mut f, "t", None, "b"), Ok(()));
    assert_eq!(connect(&mut f, "u", None, "a"), Ok(()));
    assert_eq!(f.edges.len(), 3);
}

#[test]
fn remove_edge_by_key() {
    let mut f = flow(vec![trigger("t"), delay("d")]);
    connect(&mut f, "t", None, "d").unwrap();
    let key: EdgeKey = ("t".into(), None, "d".into());
    assert!(remove_edge(&mut f, &key));
    assert!(!remove_edge(&mut f, &key));
    assert!(f.edges.is_empty());
}

fn switch_kind(rules: usize, otherwise: bool) -> NodeKind {
    NodeKind::Switch(SwitchParams {
        rules: (0..rules).map(|i| rule(i as i64)).collect(),
        otherwise,
        ..Default::default()
    })
}
/// A delay that passes `validate`.
fn wait(id: &str) -> Node {
    node(
        id,
        NodeKind::Delay(DelayParams {
            for_: "5s".into(),
            ..Default::default()
        }),
    )
}
fn rule(v: i64) -> SwitchRule {
    SwitchRule {
        op: RuleOp::Eq,
        value: json!(v),
        to: None,
        extra: Default::default(),
    }
}
/// Trigger `t` into switch `s` (3 rules + otherwise), one target per exit.
fn switch_flow() -> Flow {
    let mut f = flow(vec![
        trigger("t"),
        node("s", switch_kind(3, true)),
        wait("a"),
        wait("b"),
        wait("c"),
        wait("o"),
    ]);
    connect(&mut f, "t", None, "s").unwrap();
    for (port, to) in [("1", "a"), ("2", "b"), ("3", "c"), ("otherwise", "o")] {
        connect(&mut f, "s", Some(port), to).unwrap();
    }
    assert_eq!(nidoo_core::flow::validate(&f), Ok(()));
    f
}
fn exits(f: &Flow) -> Vec<(String, String)> {
    let mut v: Vec<_> = f
        .edges
        .iter()
        .filter(|e| e.from == "s")
        .map(|e| (e.port.clone().unwrap(), e.to.clone()))
        .collect();
    v.sort();
    v
}
fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

#[test]
fn delete_rule_renumbers_later_exits() {
    let mut f = switch_flow();
    assert_eq!(delete_rule(&mut f, "s", 1), 1);
    assert_eq!(
        exits(&f),
        pairs(&[("1", "a"), ("2", "c"), ("otherwise", "o")])
    );
    assert_eq!(nidoo_core::flow::validate(&f), Ok(()));
}

fn ask_flow() -> Flow {
    let answer = |l: &str| nidoo_core::flow::AskAnswer {
        label: l.into(),
        words: vec![],
    };
    let mut f = flow(vec![
        trigger("t"),
        node(
            "s",
            NodeKind::Ask(nidoo_core::flow::AskParams {
                text: "Hi".into(),
                answers: vec![answer("a"), answer("b"), answer("c")],
                ..Default::default()
            }),
        ),
        wait("a"),
        wait("b"),
        wait("c"),
        wait("o"),
    ]);
    connect(&mut f, "t", None, "s").unwrap();
    for (port, to) in [("1", "a"), ("2", "b"), ("3", "c"), ("none", "o")] {
        connect(&mut f, "s", Some(port), to).unwrap();
    }
    assert_eq!(nidoo_core::flow::validate(&f), Ok(()));
    f
}

#[test]
fn delete_answer_renumbers_later_exits() {
    let mut f = ask_flow();
    assert_eq!(delete_answer(&mut f, "s", 1), 1);
    assert_eq!(exits(&f), pairs(&[("1", "a"), ("2", "c"), ("none", "o")]));
    assert_eq!(nidoo_core::flow::validate(&f), Ok(()));
}

#[test]
fn move_answer_keeps_wires_with_their_answer() {
    let mut f = ask_flow();
    move_answer(&mut f, "s", 0, 2);
    assert_eq!(
        exits(&f),
        pairs(&[("1", "b"), ("2", "c"), ("3", "a"), ("none", "o")])
    );
    let NodeKind::Ask(p) = &f.node("s").unwrap().kind else {
        panic!()
    };
    let labels: Vec<_> = p.answers.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(labels, ["b", "c", "a"]);
    assert_eq!(nidoo_core::flow::validate(&f), Ok(()));
}

#[test]
fn add_answer_appends_and_stops_at_six() {
    let mut f = ask_flow();
    let new = || nidoo_core::flow::AskAnswer {
        label: "x".into(),
        words: vec![],
    };
    add_answer(&mut f, "s", new());
    assert_eq!(exits(&f).len(), 4);
    for _ in 0..5 {
        add_answer(&mut f, "s", new());
    }
    let NodeKind::Ask(p) = &f.node("s").unwrap().kind else {
        panic!()
    };
    assert_eq!(p.answers.len(), nidoo_core::flow::ASK_MAX_ANSWERS);
}

#[test]
fn ask_ids_are_numbered_ask() {
    let f = ask_flow();
    let kind = f.node("s").unwrap().kind.clone();
    assert_eq!(next_node_id(&f, &kind), "ask_1");
}

#[test]
fn move_rule_carries_its_wires() {
    let mut f = switch_flow();
    move_rule(&mut f, "s", 0, 2);
    assert_eq!(
        exits(&f),
        pairs(&[("1", "b"), ("2", "c"), ("3", "a"), ("otherwise", "o")])
    );
    assert_eq!(nidoo_core::flow::validate(&f), Ok(()));
    move_rule(&mut f, "s", 2, 0);
    assert_eq!(
        exits(&f),
        pairs(&[("1", "a"), ("2", "b"), ("3", "c"), ("otherwise", "o")])
    );
    let NodeKind::Switch(p) = &f.node("s").unwrap().kind else {
        panic!()
    };
    let values: Vec<_> = p.rules.iter().map(|r| r.value.clone()).collect();
    assert_eq!(values, [json!(0), json!(1), json!(2)]);
}

#[test]
fn add_rule_appends_without_touching_wires() {
    let mut f = switch_flow();
    add_rule(&mut f, "s", rule(9));
    assert_eq!(f.node("s").unwrap().kind.ports().len(), 5);
    assert_eq!(exits(&f).len(), 4);
    assert_eq!(nidoo_core::flow::validate(&f), Ok(()));
}

#[test]
fn set_otherwise_off_drops_its_wires() {
    let mut f = switch_flow();
    assert_eq!(set_otherwise(&mut f, "s", false), 1);
    assert_eq!(exits(&f), pairs(&[("1", "a"), ("2", "b"), ("3", "c")]));
    assert_eq!(nidoo_core::flow::validate(&f), Ok(()));
    assert_eq!(set_otherwise(&mut f, "s", true), 0);
    assert_eq!(nidoo_core::flow::validate(&f), Ok(()));
}

#[test]
fn can_connect_accepts_switch_and_time_ports() {
    let f = flow(vec![
        node("s", switch_kind(2, false)),
        node(
            "m",
            NodeKind::TimeBetween(TimeBetweenParams {
                from: "08:00".into(),
                to: "20:00".into(),
                extra: Default::default(),
            }),
        ),
        delay("d"),
    ]);
    assert_eq!(can_connect(&f, "s", Some("2"), "d"), Ok(()));
    assert_eq!(
        can_connect(&f, "s", Some("3"), "d"),
        Err(ConnectError::BadPort)
    );
    assert_eq!(can_connect(&f, "s", None, "d"), Err(ConnectError::BadPort));
    assert_eq!(can_connect(&f, "m", Some("in"), "d"), Ok(()));
    assert_eq!(can_connect(&f, "m", Some("out"), "d"), Ok(()));
    assert_eq!(can_connect(&f, "m", None, "d"), Err(ConnectError::BadPort));
}

#[test]
fn splice_uses_first_port() {
    for (kind, first) in [
        (switch_kind(2, true), "1"),
        (
            NodeKind::TimeBetween(TimeBetweenParams {
                from: "08:00".into(),
                to: "20:00".into(),
                extra: Default::default(),
            }),
            "in",
        ),
    ] {
        let mut f = flow(vec![trigger("t"), delay("d"), node("n", kind)]);
        connect(&mut f, "t", None, "d").unwrap();
        let key: EdgeKey = ("t".into(), None, "d".into());
        assert_eq!(insert_into_edge(&mut f, &key, "n"), Ok(()));
        assert_eq!(wires(&f), [("t", None, "n"), ("n", Some(first), "d")]);
    }
}

#[test]
fn insert_splits_the_wire() {
    let mut f = flow(vec![trigger("t"), delay("d"), delay("n")]);
    connect(&mut f, "t", None, "d").unwrap();
    let key: EdgeKey = ("t".into(), None, "d".into());
    assert_eq!(insert_into_edge(&mut f, &key, "n"), Ok(()));
    assert_eq!(wires(&f), [("t", None, "n"), ("n", None, "d")]);
}

#[test]
fn insert_keeps_the_port_and_leaves_an_if_by_true() {
    let mut f = flow(vec![
        trigger("t"),
        node("i", if_kind()),
        delay("d"),
        node("j", if_kind()),
    ]);
    connect(&mut f, "t", None, "i").unwrap();
    connect(&mut f, "i", Some("false"), "d").unwrap();
    let key: EdgeKey = ("i".into(), Some("false".into()), "d".into());
    assert_eq!(insert_into_edge(&mut f, &key, "j"), Ok(()));
    assert!(
        f.edges
            .iter()
            .any(|e| e.from == "i" && e.port.as_deref() == Some("false") && e.to == "j")
    );
    assert!(
        f.edges
            .iter()
            .any(|e| e.from == "j" && e.port.as_deref() == Some("true") && e.to == "d")
    );
    assert_eq!(f.edges.len(), 3);
}

#[test]
fn insert_refuses_wired_cards_triggers_and_unknown_wires() {
    let mut f = flow(vec![trigger("t"), delay("d"), delay("w"), trigger("u")]);
    connect(&mut f, "t", None, "d").unwrap();
    connect(&mut f, "t", None, "w").unwrap();
    let key: EdgeKey = ("t".into(), None, "d".into());
    let before = f.clone();
    assert_eq!(
        insert_into_edge(&mut f, &key, "w"),
        Err(ConnectError::AlreadyWired)
    );
    assert_eq!(
        insert_into_edge(&mut f, &key, "u"),
        Err(ConnectError::IntoTrigger)
    );
    let gone: EdgeKey = ("t".into(), None, "x".into());
    assert_eq!(
        insert_into_edge(&mut f, &gone, "d"),
        Err(ConnectError::UnknownNode)
    );
    assert_eq!(f, before);
}

fn wires(f: &Flow) -> Vec<(&str, Option<&str>, &str)> {
    f.edges
        .iter()
        .map(|e| (e.from.as_str(), e.port.as_deref(), e.to.as_str()))
        .collect()
}

#[test]
fn remove_node_bridges_its_neighbours() {
    let mut f = flow(vec![trigger("t"), delay("a"), delay("b")]);
    connect(&mut f, "t", None, "a").unwrap();
    connect(&mut f, "a", None, "b").unwrap();
    assert!(remove_node(&mut f, "a"));
    assert_eq!(wires(&f), [("t", None, "b")]);
    assert_eq!(f.nodes.len(), 2);
    assert!(!remove_node(&mut f, "a"));
}

#[test]
fn remove_node_bridges_every_pair_on_the_feeding_port() {
    let mut f = flow(vec![
        trigger("t"),
        node("i", if_kind()),
        delay("a"),
        delay("b"),
        delay("c"),
    ]);
    connect(&mut f, "t", None, "i").unwrap();
    connect(&mut f, "i", Some("false"), "a").unwrap();
    connect(&mut f, "a", None, "b").unwrap();
    connect(&mut f, "a", None, "c").unwrap();
    // `c` is already fed by `i`: no duplicate wire.
    connect(&mut f, "i", Some("false"), "c").unwrap();
    assert!(remove_node(&mut f, "a"));
    assert_eq!(
        wires(&f),
        [
            ("t", None, "i"),
            ("i", Some("false"), "c"),
            ("i", Some("false"), "b"),
        ]
    );
}

#[test]
fn remove_node_at_an_end_leaves_nothing_to_bridge() {
    let mut f = flow(vec![trigger("t"), delay("a"), delay("b")]);
    connect(&mut f, "t", None, "a").unwrap();
    connect(&mut f, "a", None, "b").unwrap();
    assert!(remove_node(&mut f, "t"));
    assert_eq!(wires(&f), [("a", None, "b")]);
    assert!(remove_node(&mut f, "b"));
    assert!(f.edges.is_empty());
}

#[test]
fn duplicate_offsets_and_unwires() {
    let mut original = delay("delay_1");
    original.name = Some("Wait".into());
    original.position = Some(Position { x: 10.0, y: 20.0 });
    original.extra.insert("note".into(), json!("keep"));
    let mut f = flow(vec![trigger("t"), original.clone()]);
    connect(&mut f, "t", None, "delay_1").unwrap();
    let id = duplicate_node(&mut f, "delay_1").unwrap();
    assert_eq!(id, "delay_2");
    let copy = f.node(&id).unwrap();
    assert_eq!(copy.kind, original.kind);
    assert_eq!(copy.name, original.name);
    assert_eq!(copy.extra, original.extra);
    assert_eq!(copy.position, Some(Position { x: 34.0, y: 44.0 }));
    assert_eq!(f.edges.len(), 1);
    assert_eq!(duplicate_node(&mut f, "nope"), None);
}

#[test]
fn stamp_sets_lang_on_every_notify() {
    let notify = |lang: Option<Lang>| {
        NodeKind::Notify(NotifyParams {
            title: "T".into(),
            lang,
            ..Default::default()
        })
    };
    let mut flow = flow(vec![
        node("a", notify(None)),
        node("b", notify(Some(Lang::En))),
        delay("w"),
    ]);
    let before = flow.nodes[2].clone();
    stamp_notify_lang(&mut flow, Lang::Fr);
    for n in &flow.nodes[..2] {
        assert!(matches!(&n.kind, NodeKind::Notify(p) if p.lang == Some(Lang::Fr)));
    }
    assert_eq!(flow.nodes[2], before);
}

#[test]
fn add_node_places_and_names_it() {
    let mut f = flow(vec![]);
    let id = add_node(&mut f, if_kind(), Pt { x: 5.0, y: 6.0 });
    assert_eq!(id, "if_1");
    let n = f.node("if_1").unwrap();
    assert_eq!(n.position, Some(Position { x: 5.0, y: 6.0 }));
    assert_eq!(n.name, None);
    assert!(n.extra.is_empty());
}

#[test]
fn next_id_fills_gap() {
    let f = flow(vec![node("if_1", if_kind()), node("if_3", if_kind())]);
    assert_eq!(next_node_id(&f, &if_kind()), "if_2");
}

#[test]
fn next_id_prefixes() {
    let f = flow(vec![]);
    let cases = [
        (NodeKind::TriggerState(Default::default()), "trigger_1"),
        (
            NodeKind::TriggerSchedule(TriggerScheduleParams::default()),
            "schedule_1",
        ),
        (NodeKind::Delay(DelayParams::default()), "delay_1"),
        (
            NodeKind::FlowEnable(FlowEnableParams::default()),
            "flow_switch_1",
        ),
        (
            NodeKind::FlowLabel(FlowLabelParams::default()),
            "flow_label_1",
        ),
    ];
    for (kind, want) in cases {
        assert_eq!(next_node_id(&f, &kind), want);
    }
    let command = NodeKind::Command(CommandParams {
        entity: "e".into(),
        command: serde_json::from_value(json!({"type": "turn_on"})).unwrap(),
        value_from: None,
        extra: Default::default(),
    });
    assert_eq!(next_node_id(&f, &command), "command_1");
}

#[test]
fn flow_uuid_is_a_v4_uuid() {
    assert_eq!(flow_uuid([0; 16]), "00000000-0000-4000-8000-000000000000");
    let id = flow_uuid([0xff; 16]);
    assert_eq!(id, "ffffffff-ffff-4fff-bfff-ffffffffffff");
    assert!(nidoo_core::flow::is_valid_flow_id(&id));
}

#[test]
fn history_undo_redo() {
    let (a, b, c) = (
        flow(vec![]),
        flow(vec![delay("d")]),
        flow(vec![trigger("t")]),
    );
    let mut h = History::new();
    assert!(!h.can_undo());
    h.record(a.clone());
    assert!(h.can_undo());
    let back = h.undo(b.clone()).unwrap();
    assert_eq!(back, a);
    assert!(!h.can_undo());
    assert!(h.can_redo());
    assert_eq!(h.redo(a.clone()), Some(b.clone()));
    assert_eq!(h.undo(b.clone()), Some(a.clone()));
    // A new record clears the future.
    h.record(a);
    assert!(!h.can_redo());
    assert_eq!(h.redo(c), None);
    h.clear();
    assert!(!h.can_undo());
    assert_eq!(h.undo(b), None);
}

#[test]
fn history_caps_at_100() {
    let mut h = History::new();
    for i in 0..150 {
        let mut f = flow(vec![]);
        f.name = i.to_string();
        h.record(f);
    }
    let mut current = flow(vec![]);
    let mut steps = 0;
    while let Some(prev) = h.undo(current.clone()) {
        current = prev;
        steps += 1;
    }
    assert_eq!(steps, 100);
    assert_eq!(current.name, "50");
}

#[test]
fn history_records_once_per_gesture() {
    // Callers record the pre-gesture snapshot once, then mutate freely.
    let mut f = flow(vec![trigger("t")]);
    f.nodes[0].position = Some(Position { x: 0.0, y: 0.0 });
    let original = f.clone();
    let mut h = History::new();
    h.record(f.clone());
    for x in [10.0, 20.0, 30.0] {
        f.nodes[0].position = Some(Position { x, y: 0.0 });
    }
    let restored = h.undo(f).unwrap();
    assert_eq!(restored, original);
    assert!(!h.can_undo());
}

#[test]
fn adding_an_unnamed_math_card_names_it() {
    let mut f = flow(vec![]);
    let blank = || NodeKind::Math(MathParams::default());
    let at = Pt { x: 0.0, y: 0.0 };
    let first = add_node(&mut f, blank(), at);
    let second = add_node(&mut f, blank(), at);
    let name = |id: &str| match &f.node(id).unwrap().kind {
        NodeKind::Math(p) => p.name.clone(),
        other => panic!("not a math card: {other:?}"),
    };
    assert_eq!(name(&first), "result");
    assert_eq!(name(&second), "result2");
}

#[test]
fn new_math_card_takes_the_first_free_result_name() {
    let mut f = Flow::default();
    let name = |kind: &NodeKind| match kind {
        NodeKind::Math(p) => p.name.clone(),
        other => panic!("not a math card: {other:?}"),
    };
    assert_eq!(name(&new_math_card(&f)), "result");
    f.nodes.push(node("m1", new_math_card(&f)));
    assert_eq!(name(&new_math_card(&f)), "result2");
    f.nodes.push(node("m2", new_math_card(&f)));
    assert_eq!(name(&new_math_card(&f)), "result3");
}
