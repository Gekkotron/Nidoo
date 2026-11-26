use super::*;
use crate::flow::{
    AskAnswer, AskParams, Edge, FlowLabelParams, MathParams, Node, NodeKind, reference_flow,
};
use crate::flow::{command_with_number, parse_formula};

fn edge(from: &str, to: &str, port: Option<&str>) -> Edge {
    Edge {
        from: from.into(),
        to: to.into(),
        port: port.map(Into::into),
    }
}

#[test]
fn reference_flow_validates() {
    assert_eq!(validate(&reference_flow()), Ok(()));
}

#[test]
fn if_edge_without_port_is_missing_port() {
    let mut f = reference_flow();
    f.edges[1].port = None;
    assert_eq!(
        validate(&f),
        Err(FlowError::MissingPort {
            node: "moving".into(),
            kind: "an if"
        })
    );
    assert!(
        validate(&f)
            .unwrap_err()
            .to_string()
            .contains("node `moving` is an if and needs a port")
    );
}

#[test]
fn port_on_a_command_is_unexpected() {
    let mut f = reference_flow();
    f.edges.push(edge("on", "wait", Some("true")));
    assert_eq!(
        validate(&f),
        Err(FlowError::UnexpectedPort { node: "on".into() })
    );
}

#[test]
fn unknown_port_name() {
    let mut f = reference_flow();
    f.edges[1].port = Some("maybe".into());
    assert_eq!(
        validate(&f),
        Err(FlowError::UnknownPort {
            node: "moving".into(),
            port: "maybe".into()
        })
    );
}

#[test]
fn edge_to_missing_node() {
    let mut f = reference_flow();
    f.edges[3].to = "nope".into();
    let err = validate(&f).unwrap_err();
    assert_eq!(
        err,
        FlowError::UnknownNode {
            edge_index: 3,
            node: "nope".into()
        }
    );
    assert_eq!(err.to_string(), "edge 3 names a missing node `nope`");
}

#[test]
fn cycle_is_rejected() {
    let mut f = reference_flow();
    f.edges.push(edge("off", "wait", None));
    match validate(&f) {
        Err(FlowError::Cycle(path)) => {
            assert!(path.contains(&"wait".to_owned()) && path.contains(&"off".to_owned()));
            assert!(!path.contains(&"motion".to_owned()));
        }
        other => panic!("expected a cycle, got {other:?}"),
    }
    let text = FlowError::Cycle(vec!["wait".into(), "off".into(), "wait".into()]).to_string();
    assert_eq!(text, "cycle through wait → off → wait");
}

#[test]
fn flow_without_trigger_is_rejected() {
    let mut f = reference_flow();
    f.nodes.remove(0);
    f.edges.remove(0);
    assert_eq!(validate(&f), Err(FlowError::NoTrigger));
}

#[test]
fn random_delay_needs_an_upper_bound_at_least_the_lower_one() {
    let mut f = reference_flow();
    if let NodeKind::Delay(p) = &mut f.nodes[3].kind {
        p.up_to = Some("2h".into());
    }
    assert_eq!(validate(&f), Ok(()));
    if let NodeKind::Delay(p) = &mut f.nodes[3].kind {
        p.up_to = Some("5m".into());
    }
    assert_eq!(validate(&f), Ok(()));
    if let NodeKind::Delay(p) = &mut f.nodes[3].kind {
        p.up_to = Some("4m".into());
    }
    let err = validate(&f).unwrap_err();
    assert_eq!(
        err.to_string(),
        "node `wait`: `up_to` (4m) is shorter than `for` (5m)"
    );
}

#[test]
fn bad_duration_and_bad_if_value() {
    let mut f = reference_flow();
    if let NodeKind::Delay(p) = &mut f.nodes[3].kind {
        p.for_ = "5".into();
    }
    assert!(matches!(
        validate(&f),
        Err(FlowError::BadDuration { node, .. }) if node == "wait"
    ));
    let mut f = reference_flow();
    if let NodeKind::Delay(p) = &mut f.nodes[3].kind {
        p.up_to = Some("2x".into());
    }
    assert!(matches!(
        validate(&f),
        Err(FlowError::BadDuration { node, reason })
            if node == "wait" && reason.starts_with("up_to: ")
    ));
    let mut f = reference_flow();
    if let NodeKind::If(p) = &mut f.nodes[1].kind {
        p.value = Some(serde_json::json!([1]));
    }
    let err = validate(&f).unwrap_err();
    assert_eq!(
        err,
        FlowError::BadValue {
            node: "moving".into()
        }
    );
    assert_eq!(
        err.to_string(),
        "node `moving`: value must be a boolean, a number or a string"
    );
}

#[test]
fn field_without_entity_is_refused() {
    let flow = |kind: &str, params: serde_json::Value| {
        let text = serde_json::json!({
            "id": "f", "name": "F",
            "nodes": [
                { "id": "t", "type": "trigger.state", "params": { "entity": "r" } },
                { "id": "mid", "type": kind, "params": params }
            ],
            "edges": [{ "from": "t", "to": "mid" }]
        });
        Flow::decode(text.to_string().as_bytes()).unwrap()
    };
    let if_flow = flow(
        "if",
        serde_json::json!({ "op": "lte", "value": 55, "field": "battery" }),
    );
    let switch_flow = flow(
        "switch",
        serde_json::json!({ "field": "battery", "rules": [{ "op": "lte", "value": 55 }] }),
    );
    for f in [if_flow, switch_flow] {
        let err = validate(&f).unwrap_err();
        assert_eq!(err, FlowError::FieldWithoutEntity { node: "mid".into() });
        assert_eq!(err.to_string(), "node `mid`: a field needs an entity");
    }
}

#[test]
fn bad_if_average() {
    let set = |average: &str, value: serde_json::Value| {
        let mut f = reference_flow();
        if let NodeKind::If(p) = &mut f.nodes[1].kind {
            p.average = Some(average.into());
            p.value = Some(value);
        }
        validate(&f)
    };
    let reason = |r: Result<(), FlowError>| match r {
        Err(FlowError::BadOperand { node, reason }) if node == "moving" => reason,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(set("15m", serde_json::json!(70)), Ok(()));
    assert_eq!(set("24h", serde_json::json!(70.5)), Ok(()));
    assert!(reason(set("15", serde_json::json!(70))).starts_with("average: "));
    assert_eq!(
        reason(set("24h1s", serde_json::json!(70))),
        "average must be at most 24h"
    );
    assert_eq!(
        reason(set("15m", serde_json::json!(true))),
        "value must be a number with average"
    );
}

#[test]
fn ids_are_checked() {
    let mut f = reference_flow();
    f.id = "Living LED".into();
    assert_eq!(validate(&f), Err(FlowError::BadFlowId("Living LED".into())));
    let mut f = reference_flow();
    f.nodes[0].id = "A B".into();
    assert_eq!(validate(&f), Err(FlowError::BadNodeId("A B".into())));
    let mut f = reference_flow();
    let dup: Node = f.nodes[4].clone();
    f.nodes.push(dup);
    assert_eq!(validate(&f), Err(FlowError::DuplicateNodeId("off".into())));
    assert!(is_valid_flow_id(&"a".repeat(64)) && !is_valid_flow_id(&"a".repeat(65)));
    assert!(is_valid_node_id(&"a".repeat(32)) && !is_valid_node_id(&"a".repeat(33)));
    assert!(!is_valid_flow_id("") && !is_valid_node_id(""));
}

fn schedule_flow(cron: &str, then: &str) -> Flow {
    let node = |id: &str, json: &str| -> Node {
        serde_json::from_str(&format!(r#"{{"id":"{id}",{json}}}"#)).unwrap()
    };
    let mut f = Flow {
        id: "tick_flow".into(),
        name: "t".into(),
        ..Flow::default()
    };
    f.nodes.push(node(
        "tick",
        &format!(r#""type":"trigger.schedule","params":{{"cron":"{cron}"}}"#),
    ));
    f.nodes.push(node(
        "act",
        r#""type":"command","params":{"entity":"l","command":{"type":"turn_on"}}"#,
    ));
    f.nodes.push(node(
        "check",
        r#""type":"if","params":{"op":"eq","value":true}"#,
    ));
    f.edges.push(edge("tick", then, None));
    f
}

fn notify_flow(params: &str) -> Flow {
    let mut f = schedule_flow("*/10 * * * *", "act");
    f.nodes.push(
        serde_json::from_str(&format!(
            r#"{{"id":"n","type":"notify","params":{params}}}"#
        ))
        .unwrap(),
    );
    f
}

fn speak_flow(params: &str) -> Flow {
    let mut f = schedule_flow("*/10 * * * *", "act");
    f.nodes.push(
        serde_json::from_str(&format!(r#"{{"id":"n","type":"speak","params":{params}}}"#)).unwrap(),
    );
    f
}

#[test]
fn validate_rejects_empty_speak_text() {
    let err = validate(&speak_flow(r#"{"text":"  "}"#)).unwrap_err();
    assert_eq!(
        err,
        FlowError::BadSpeak {
            node: "n".into(),
            reason: "text must not be empty".into()
        }
    );
    let err = validate(&speak_flow(r#"{"text":"Temp {x"}"#)).unwrap_err();
    assert!(
        matches!(&err, FlowError::BadSpeak { reason, .. } if reason.starts_with("text:")),
        "{err}"
    );
    assert!(validate(&speak_flow(r#"{"text":"Dinner","room":"kitchen"}"#)).is_ok());
}

fn ask_flow(params: &str) -> Flow {
    let mut f = schedule_flow("*/10 * * * *", "act");
    f.nodes.push(
        serde_json::from_str(&format!(r#"{{"id":"n","type":"ask","params":{params}}}"#)).unwrap(),
    );
    f
}

fn bad_ask(params: &str, prefix: &str) {
    let err = validate(&ask_flow(params)).unwrap_err();
    assert!(
        matches!(&err, FlowError::BadAsk { node, reason } if node == "n" && reason.contains(prefix)),
        "{params}: {err}"
    );
}

#[test]
fn validate_rejects_bad_ask_params() {
    bad_ask(
        r#"{"text":" ","answers":[{"label":"Oui"}]}"#,
        "text must not be empty",
    );
    let err = validate(&ask_flow(r#"{"text":"T {x","answers":[{"label":"Oui"}]}"#)).unwrap_err();
    assert!(
        matches!(&err, FlowError::BadAsk { reason, .. } if reason.starts_with("text:")),
        "{err}"
    );
    bad_ask(r#"{"text":"T","answers":[]}"#, "1 to 6 answers");
    let seven = [r#"{"label":"A"}"#; 7].join(",");
    bad_ask(
        &format!(r#"{{"text":"T","answers":[{seven}]}}"#),
        "1 to 6 answers",
    );
    bad_ask(
        r#"{"text":"T","answers":[{"label":"Oui"},{"label":" oui "}]}"#,
        "duplicate answer",
    );
    bad_ask(
        r#"{"text":"T","answers":[{"label":"Éte"},{"label":"ete"}]}"#,
        "duplicate answer",
    );
    bad_ask(
        r#"{"text":"T","answers":[{"label":""}]}"#,
        "answer label must not be empty",
    );
    bad_ask(
        r#"{"text":"T","answers":[{"label":"Oui"}],"name":"Temp"}"#,
        "name:",
    );
    bad_ask(
        r#"{"text":"T","answers":[{"label":"Oui"}],"default":21}"#,
        "default needs a name",
    );
    assert!(
        validate(&ask_flow(
            r#"{"text":"T","answers":[{"label":"Oui"},{"label":"Non"}],"name":"temp"}"#
        ))
        .is_ok()
    );
}

#[test]
fn validate_refuses_an_ask_edge_past_its_answers() {
    let mut f = ask_flow(r#"{"text":"T","answers":[{"label":"A"},{"label":"B"},{"label":"C"}]}"#);
    f.edges.push(Edge {
        from: "n".into(),
        to: "act".into(),
        port: Some("4".into()),
    });
    assert!(matches!(validate(&f), Err(FlowError::UnknownPort { .. })));
    f.edges.last_mut().unwrap().port = Some("none".into());
    assert!(validate(&f).is_ok());
}

#[test]
fn validate_rejects_empty_notify_title() {
    let err = validate(&notify_flow(r#"{"title":"  "}"#)).unwrap_err();
    assert_eq!(
        err,
        FlowError::BadNotify {
            node: "n".into(),
            reason: "title must not be empty".into()
        }
    );
    assert!(validate(&notify_flow(r#"{"title":"Door"}"#)).is_ok());
}

#[test]
fn validate_rejects_bad_notify_templates() {
    let err = validate(&notify_flow(r#"{"title":"Temp {x"}"#)).unwrap_err();
    assert!(
        matches!(&err, FlowError::BadNotify { reason, .. }
            if reason.starts_with("title:") && reason.contains("unclosed {")),
        "{err:?}"
    );
    let err = validate(&notify_flow(r#"{"title":"T","body":"{a b}"}"#)).unwrap_err();
    assert!(
        matches!(&err, FlowError::BadNotify { reason, .. } if reason.starts_with("body:")),
        "{err:?}"
    );
    assert!(
        validate(&notify_flow(
            r#"{"title":"{weather_temperature:0}","body":"{{ok}}","lang":"fr"}"#
        ))
        .is_ok()
    );
}

#[test]
fn validate_rejects_unknown_notify_channel() {
    let err = validate(&notify_flow(r#"{"title":"T","channel":"pager"}"#)).unwrap_err();
    assert!(
        matches!(&err, FlowError::BadNotify { reason, .. } if reason.contains("pager")),
        "{err}"
    );
    let err = validate(&notify_flow(r#"{"title":"T","severity":"loud"}"#)).unwrap_err();
    assert!(
        matches!(&err, FlowError::BadNotify { reason, .. } if reason.contains("loud")),
        "{err}"
    );
    assert!(
        validate(&notify_flow(
            r#"{"title":"T","channel":"camera","severity":"critical"}"#
        ))
        .is_ok()
    );
}

#[test]
fn bad_flow_target_is_rejected() {
    let mut f = schedule_flow("*/10 * * * *", "act");
    f.nodes.push(
        serde_json::from_str(
            r#"{"id":"stop","type":"flow.enable","params":{"flow":"Bad Id","enabled":false}}"#,
        )
        .unwrap(),
    );
    let err = validate(&f).unwrap_err();
    assert_eq!(
        err,
        FlowError::BadFlowTarget {
            node: "stop".into(),
            flow: "Bad Id".into()
        }
    );
    assert_eq!(
        err.to_string(),
        "node `stop`: `Bad Id` is not a valid flow id"
    );
}

#[test]
fn flow_label_needs_a_label() {
    let mut f = schedule_flow("*/10 * * * *", "act");
    f.nodes.push(
        serde_json::from_str(
            r#"{"id":"season","type":"flow.label","params":{"label":" ","action":"on"}}"#,
        )
        .unwrap(),
    );
    let err = validate(&f).unwrap_err();
    assert_eq!(err.to_string(), "node `season`: choose a label");
    f.nodes.last_mut().unwrap().kind = NodeKind::FlowLabel(FlowLabelParams {
        label: "ete".into(),
        ..FlowLabelParams::default()
    });
    assert!(validate(&f).is_ok());
}

#[test]
fn backup_run_partial_edge_validates() {
    let mut f = schedule_flow("*/10 * * * *", "bk");
    f.nodes
        .push(serde_json::from_str(r#"{"id":"bk","type":"backup.run","params":{}}"#).unwrap());
    f.edges.push(edge("bk", "act", Some("partial")));
    assert!(validate(&f).is_ok());
}

#[test]
fn backup_run_timeout_bounds() {
    let mut f = schedule_flow("*/10 * * * *", "act");
    f.nodes
        .push(serde_json::from_str(r#"{"id":"bk","type":"backup.run","params":{}}"#).unwrap());
    assert!(validate(&f).is_ok());
    for (minutes, ok) in [(0, false), (1, true), (1440, true), (1441, false)] {
        f.nodes.last_mut().unwrap().kind = NodeKind::BackupRun(crate::flow::BackupRunParams {
            timeout_minutes: minutes,
            ..Default::default()
        });
        match validate(&f) {
            Ok(()) => assert!(ok, "{minutes} should be refused"),
            Err(err) => {
                assert!(!ok, "{minutes} should pass");
                assert_eq!(
                    err.to_string(),
                    "timeout must be 1 to 1440 minutes in node `bk`"
                );
            }
        }
    }
}

#[test]
fn bad_cron_is_rejected() {
    let f = schedule_flow("61 * * * *", "act");
    assert!(matches!(
        validate(&f),
        Err(FlowError::BadCron { node, .. }) if node == "tick"
    ));
}

#[test]
fn schedule_alone_is_a_trigger() {
    assert_eq!(validate(&schedule_flow("*/10 * * * *", "act")), Ok(()));
}

#[test]
fn if_without_entity_after_a_schedule_is_rejected() {
    let mut f = schedule_flow("*/10 * * * *", "check");
    f.edges.push(edge("check", "act", Some("true")));
    let err = validate(&f).unwrap_err();
    assert_eq!(
        err,
        FlowError::IfNeedsEntity {
            node: "check".into()
        }
    );
    assert_eq!(
        err.to_string(),
        "node `check`: an if after a schedule, sun or manual trigger without a value must name an entity"
    );
}

#[test]
fn if_without_entity_reached_only_from_a_state_trigger_is_fine() {
    let mut f = schedule_flow("*/10 * * * *", "act");
    let state: Node =
        serde_json::from_str(r#"{"id":"motion","type":"trigger.state","params":{"entity":"m"}}"#)
            .unwrap();
    f.nodes.push(state);
    f.edges.push(edge("motion", "check", None));
    f.edges.push(edge("check", "act", Some("true")));
    assert_eq!(validate(&f), Ok(()));
}

fn decode_flow(nodes: serde_json::Value, edges: serde_json::Value) -> Flow {
    let json = serde_json::json!({
        "id": "sun_flow",
        "name": "t",
        "nodes": nodes,
        "edges": edges,
    });
    Flow::decode(json.to_string().as_bytes()).unwrap()
}

fn manual_flow(manual_params: serde_json::Value, mid: serde_json::Value) -> Flow {
    decode_flow(
        serde_json::json!([
            { "id": "m", "type": "trigger.manual", "params": manual_params },
            mid,
            { "id": "act", "type": "command",
              "params": { "entity": "l", "command": { "type": "turn_on" } } },
        ]),
        serde_json::json!([
            { "from": "m", "to": "mid" },
            { "from": "mid", "to": "act", "port": "true" },
        ]),
    )
}

fn if_node(id: &str, params: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "id": id, "type": "if", "params": params })
}

#[test]
fn manual_alone_is_a_trigger() {
    let flow = decode_flow(
        serde_json::json!([
            { "id": "m", "type": "trigger.manual", "params": {} },
            { "id": "act", "type": "command",
              "params": { "entity": "l", "command": { "type": "turn_on" } } },
        ]),
        serde_json::json!([{ "from": "m", "to": "act" }]),
    );
    assert_eq!(validate(&flow), Ok(()));
}

#[test]
fn manual_round_trips() {
    let json = r#"{"id":"f","name":"t","nodes":[{"id":"m","type":"trigger.manual","params":{"value":21.5}}],"edges":[]}"#;
    let flow = Flow::decode(json.as_bytes()).unwrap();
    let encoded = flow.encode();
    assert!(encoded.contains(r#""type":"trigger.manual""#), "{encoded}");
    assert!(encoded.contains(r#""value":21.5"#), "{encoded}");
    let flow = decode_flow(
        serde_json::json!([{ "id": "m", "type": "trigger.manual", "params": { "value": null } }]),
        serde_json::json!([]),
    );
    match &flow.nodes[0].kind {
        NodeKind::TriggerManual(p) => assert_eq!(p.value, None),
        other => panic!("{other:?}"),
    }
}

#[test]
fn if_without_entity_after_valueless_manual_is_rejected() {
    let flow = decode_flow(
        serde_json::json!([
            { "id": "m", "type": "trigger.manual", "params": {} },
            if_node("check", serde_json::json!({ "op": "eq", "value": true })),
            { "id": "act", "type": "command",
              "params": { "entity": "l", "command": { "type": "turn_on" } } },
        ]),
        serde_json::json!([
            { "from": "m", "to": "check" },
            { "from": "check", "to": "act", "port": "true" },
        ]),
    );
    let err = validate(&flow).unwrap_err();
    assert_eq!(
        err,
        FlowError::IfNeedsEntity {
            node: "check".into()
        }
    );
    assert_eq!(
        err.to_string(),
        "node `check`: an if after a schedule, sun or manual trigger without a value must name an entity"
    );
}

#[test]
fn switch_without_entity_after_valueless_manual_is_rejected() {
    let flow = decode_flow(
        serde_json::json!([
            { "id": "m", "type": "trigger.manual", "params": {} },
            { "id": "s", "type": "switch", "params": { "rules": [{ "op": "eq", "value": 1 }] } },
            { "id": "act", "type": "command",
              "params": { "entity": "l", "command": { "type": "turn_on" } } },
        ]),
        serde_json::json!([
            { "from": "m", "to": "s" },
            { "from": "s", "to": "act", "port": "1" },
        ]),
    );
    let err = validate(&flow).unwrap_err();
    assert_eq!(err, FlowError::SwitchNeedsEntity { node: "s".into() });
    assert_eq!(
        err.to_string(),
        "node `s`: a switch after a schedule, sun or manual trigger without a value must name an entity"
    );
}

#[test]
fn if_without_entity_after_valued_manual_is_fine() {
    let flow = manual_flow(
        serde_json::json!({ "value": true }),
        if_node("mid", serde_json::json!({ "op": "eq", "value": true })),
    );
    assert_eq!(validate(&flow), Ok(()));
}

#[test]
fn average_after_valued_manual_needs_entity() {
    let flow = manual_flow(
        serde_json::json!({ "value": 20 }),
        if_node(
            "mid",
            serde_json::json!({ "op": "gt", "value": 19, "average": "15m" }),
        ),
    );
    let err = validate(&flow).unwrap_err();
    assert_eq!(err, FlowError::AverageNeedsEntity { node: "mid".into() });
    assert_eq!(
        err.to_string(),
        "node `mid`: an average after a manual trigger must name an entity"
    );
}

#[test]
fn average_with_entity_after_manual_is_fine() {
    let flow = manual_flow(
        serde_json::json!({ "value": 20 }),
        if_node(
            "mid",
            serde_json::json!({ "op": "gt", "value": 19, "average": "15m", "entity": "t" }),
        ),
    );
    assert_eq!(validate(&flow), Ok(()));
}

#[test]
fn mixed_paths_valueless_manual_and_state_is_rejected() {
    let flow = decode_flow(
        serde_json::json!([
            { "id": "ev", "type": "trigger.state", "params": { "entity": "t" } },
            { "id": "m", "type": "trigger.manual", "params": {} },
            if_node("mid", serde_json::json!({ "op": "eq", "value": true })),
            { "id": "act", "type": "command",
              "params": { "entity": "l", "command": { "type": "turn_on" } } },
        ]),
        serde_json::json!([
            { "from": "ev", "to": "mid" },
            { "from": "m", "to": "mid" },
            { "from": "mid", "to": "act", "port": "true" },
        ]),
    );
    assert_eq!(
        validate(&flow),
        Err(FlowError::IfNeedsEntity { node: "mid".into() })
    );
}

fn sun_flow(id: &str, which: crate::flow::model::SunWhich, offset: Option<i32>) -> Flow {
    let mut params = serde_json::json!({ "which": which });
    if let Some(offset) = offset {
        params["offset_minutes"] = offset.into();
    }
    decode_flow(
        serde_json::json!([{ "id": id, "type": "trigger.sun", "params": params }]),
        serde_json::json!([]),
    )
}

fn flow_with_if_params_json(params_json: &str) -> Flow {
    let params: serde_json::Value = serde_json::from_str(params_json).unwrap();
    decode_flow(
        serde_json::json!([
            { "id": "ev", "type": "trigger.state", "params": { "entity": "t" } },
            { "id": "chk", "type": "if", "params": params },
            { "id": "act", "type": "command",
              "params": { "entity": "l", "command": { "type": "turn_on" } } },
        ]),
        serde_json::json!([
            { "from": "ev", "to": "chk" },
            { "from": "chk", "to": "act", "port": "true" },
        ]),
    )
}

fn sun_then_if_flow(if_params: serde_json::Value) -> Flow {
    decode_flow(
        serde_json::json!([
            { "id": "ev", "type": "trigger.sun", "params": { "which": "sunset" } },
            { "id": "chk", "type": "if", "params": if_params },
            { "id": "act", "type": "command",
              "params": { "entity": "l", "command": { "type": "turn_on" } } },
        ]),
        serde_json::json!([
            { "from": "ev", "to": "chk" },
            { "from": "chk", "to": "act", "port": "true" },
        ]),
    )
}

fn sun_then_is_dark_flow() -> Flow {
    sun_then_if_flow(serde_json::json!({ "op": "is_dark" }))
}

fn sun_then_plain_if_flow() -> Flow {
    sun_then_if_flow(serde_json::json!({ "op": "lt", "value": 1, "missing": "false" }))
}

#[test]
fn trigger_sun_offset_boundary() {
    use crate::flow::model::SunWhich;
    assert_eq!(
        validate(&sun_flow("ev", SunWhich::Sunset, Some(720))),
        Ok(())
    );
    assert_eq!(
        validate(&sun_flow("ev", SunWhich::Sunset, Some(-720))),
        Ok(())
    );
    assert!(matches!(
        validate(&sun_flow("ev", SunWhich::Sunset, Some(721))),
        Err(FlowError::BadSunOffset {
            offset_minutes: 721,
            ..
        })
    ));
    assert!(matches!(
        validate(&sun_flow("ev", SunWhich::Sunset, Some(-721))),
        Err(FlowError::BadSunOffset {
            offset_minutes: -721,
            ..
        })
    ));
}

#[test]
fn op_is_dark_rejects_entity_value_and_missing() {
    for (field, patch) in [
        ("value", r#"{"op":"is_dark","value":1}"#),
        ("entity", r#"{"op":"is_dark","entity":"e"}"#),
        ("missing", r#"{"op":"is_dark","missing":"true"}"#),
        ("average", r#"{"op":"is_dark","average":"15m"}"#),
        ("result", r#"{"op":"is_dark","result":"x"}"#),
    ] {
        let err = validate(&flow_with_if_params_json(patch)).unwrap_err();
        assert!(
            matches!(err, FlowError::BadOperand { ref reason, .. } if reason.contains(field)),
            "{field}: {err:?}"
        );
    }
}

#[test]
fn is_dark_offset_is_bounded_and_only_for_is_dark() {
    assert_eq!(
        validate(&sun_then_if_flow(
            serde_json::json!({ "op": "is_dark", "offset_minutes": -45 })
        )),
        Ok(())
    );
    assert!(matches!(
        validate(&sun_then_if_flow(
            serde_json::json!({ "op": "is_dark", "offset_minutes": 721 })
        )),
        Err(FlowError::BadSunOffset {
            offset_minutes: 721,
            ..
        })
    ));
    assert!(matches!(
        validate(&sun_then_if_flow(
            serde_json::json!({ "op": "eq", "entity": "l", "value": 1, "offset_minutes": -45 })
        )),
        Err(FlowError::BadOperand { reason, .. }) if reason.contains("offset_minutes")
    ));
}

#[test]
fn non_is_dark_op_requires_value() {
    let err = validate(&flow_with_if_params_json(r#"{"op":"lt","entity":"t"}"#)).unwrap_err();
    assert!(
        matches!(err, FlowError::BadOperand { ref reason, .. } if reason == "value is required for op lt")
    );
}

#[test]
fn op_is_off_peak_rejects_entity_value_missing_average_and_offset() {
    for (field, patch) in [
        ("value", r#"{"op":"is_off_peak","value":1}"#),
        ("entity", r#"{"op":"is_off_peak","entity":"e"}"#),
        ("missing", r#"{"op":"is_off_peak","missing":"true"}"#),
        ("average", r#"{"op":"is_off_peak","average":"15m"}"#),
        ("result", r#"{"op":"is_off_peak","result":"x"}"#),
        (
            "offset_minutes",
            r#"{"op":"is_off_peak","offset_minutes":5}"#,
        ),
    ] {
        let err = validate(&flow_with_if_params_json(patch)).unwrap_err();
        assert!(
            matches!(err, FlowError::BadOperand { ref reason, .. }
                if *reason == format!("{field} is not allowed with op: is_off_peak")),
            "{field}: {err:?}"
        );
    }
}

fn state_flow(params: serde_json::Value) -> Flow {
    decode_flow(
        serde_json::json!([
            { "id": "t", "type": "trigger.state", "params": params },
            { "id": "wait", "type": "delay", "params": { "for": "1s" } },
        ]),
        serde_json::json!([{ "from": "t", "to": "wait" }]),
    )
}

#[test]
fn state_for_and_crossings_validate() {
    use serde_json::json;
    for ok in [
        json!({ "entity": "m", "to": false, "for": "5m" }),
        json!({ "entity": "m", "for": "30s" }),
        json!({ "entity": "t", "above": 25 }),
        json!({ "entity": "t", "below": 18.5, "for": "10m" }),
        json!({ "entity": "t", "above": 18, "below": 22 }),
    ] {
        assert_eq!(validate(&state_flow(ok.clone())), Ok(()), "{ok}");
    }
    assert!(matches!(
        validate(&state_flow(json!({ "entity": "m", "for": "0s" }))),
        Err(FlowError::BadDuration { reason, .. }) if reason.starts_with("for:")
    ));
    assert!(matches!(
        validate(&state_flow(json!({ "entity": "m", "for": "200h" }))),
        Err(FlowError::BadDuration { .. })
    ));
    for bad in [
        json!({ "entity": "t", "above": 25, "to": 30 }),
        json!({ "entity": "t", "below": 25, "from": 20 }),
        json!({ "entity": "t", "above": 22, "below": 18 }),
        json!({ "entity": "t", "above": 20, "below": 20 }),
    ] {
        assert!(
            matches!(
                validate(&state_flow(bad.clone())),
                Err(FlowError::BadThreshold { .. })
            ),
            "{bad}"
        );
    }
}

fn silent_flow(for_: &str) -> Flow {
    decode_flow(
        serde_json::json!([
            { "id": "quiet", "type": "trigger.silent",
              "params": { "entity": "binary_sensor.smoke", "for": for_ } },
            { "id": "wait", "type": "delay", "params": { "for": "1s" } },
        ]),
        serde_json::json!([{ "from": "quiet", "to": "wait" }]),
    )
}

fn limit_flow(max: u32, per: &str, port: Option<&str>) -> Flow {
    let mut edge = serde_json::json!({ "from": "cap", "to": "wait" });
    if let Some(port) = port {
        edge["port"] = serde_json::json!(port);
    }
    decode_flow(
        serde_json::json!([
            { "id": "go", "type": "trigger.manual", "params": {} },
            { "id": "cap", "type": "limit", "params": { "max": max, "per": per } },
            { "id": "wait", "type": "delay", "params": { "for": "1s" } },
        ]),
        serde_json::json!([{ "from": "go", "to": "cap" }, edge]),
    )
}

#[test]
fn silent_is_a_trigger() {
    assert_eq!(validate(&silent_flow("50m")), Ok(()));
}

#[test]
fn window_bounds() {
    for bad in ["59s", "169h", "5", "0m"] {
        assert!(
            matches!(validate(&silent_flow(bad)), Err(FlowError::BadDuration { ref node, .. }) if node == "quiet"),
            "silent {bad}"
        );
        assert!(
            matches!(validate(&limit_flow(1, bad, None)), Err(FlowError::BadDuration { ref node, .. }) if node == "cap"),
            "limit {bad}"
        );
    }
    for good in ["1m", "168h"] {
        assert_eq!(validate(&silent_flow(good)), Ok(()), "silent {good}");
        assert_eq!(validate(&limit_flow(1, good, None)), Ok(()), "limit {good}");
    }
}

fn alarm_flow(params: serde_json::Value) -> Flow {
    decode_flow(
        serde_json::json!([
            { "id": "al", "type": "trigger.alarm", "params": params },
            { "id": "act", "type": "command",
              "params": { "entity": "l", "command": { "type": "turn_on" } } },
        ]),
        serde_json::json!([{ "from": "al", "to": "act" }]),
    )
}

#[test]
fn trigger_alarm_rejects_same_from_and_to() {
    for mode in [
        "disarmed",
        "arming",
        "armed_away",
        "armed_home",
        "armed_night",
        "pending",
        "triggered",
    ] {
        assert_eq!(
            validate(&alarm_flow(serde_json::json!({ "from": mode, "to": mode }))),
            Err(FlowError::BadAlarmTrigger { node: "al".into() }),
            "{mode}"
        );
    }
}

#[test]
fn trigger_alarm_accepts_distinct_or_armed_pairs() {
    for params in [
        serde_json::json!({}),
        serde_json::json!({ "to": "armed" }),
        serde_json::json!({ "from": "disarmed", "to": "armed" }),
        serde_json::json!({ "from": "armed", "to": "armed" }),
        serde_json::json!({ "from": "armed_home", "to": "armed_away" }),
    ] {
        assert_eq!(validate(&alarm_flow(params.clone())), Ok(()), "{params}");
    }
}

#[test]
fn limit_max_bounds() {
    for bad in [0, 101] {
        assert!(
            matches!(validate(&limit_flow(bad, "1h", None)), Err(FlowError::BadLimit { ref node, .. }) if node == "cap"),
            "max {bad}"
        );
    }
    for good in [1, 100] {
        assert_eq!(
            validate(&limit_flow(good, "1h", None)),
            Ok(()),
            "max {good}"
        );
    }
}

#[test]
fn limit_edge_has_no_port() {
    assert_eq!(
        validate(&limit_flow(2, "1h", Some("x"))),
        Err(FlowError::UnexpectedPort { node: "cap".into() })
    );
}

#[test]
fn if_is_off_peak_is_valid_after_a_schedule() {
    let flow = decode_flow(
        serde_json::json!([
            { "id": "tick", "type": "trigger.schedule", "params": { "cron": "0 * * * *" } },
            { "id": "chk", "type": "if", "params": { "op": "is_off_peak" } },
            { "id": "act", "type": "command",
              "params": { "entity": "l", "command": { "type": "turn_on" } } },
        ]),
        serde_json::json!([
            { "from": "tick", "to": "chk" },
            { "from": "chk", "to": "act", "port": "true" },
        ]),
    );
    assert_eq!(validate(&flow), Ok(()));
}

#[test]
fn if_after_trigger_off_peak_may_read_the_trigger_value() {
    let flow = decode_flow(
        serde_json::json!([
            { "id": "op", "type": "trigger.off_peak", "params": { "edge": "end" } },
            { "id": "chk", "type": "if", "params": { "op": "eq", "value": "end" } },
            { "id": "act", "type": "command",
              "params": { "entity": "l", "command": { "type": "turn_on" } } },
        ]),
        serde_json::json!([
            { "from": "op", "to": "chk" },
            { "from": "chk", "to": "act", "port": "true" },
        ]),
    );
    assert_eq!(validate(&flow), Ok(()));
}

#[test]
fn if_is_dark_is_valid_right_after_trigger_sun() {
    assert_eq!(validate(&sun_then_is_dark_flow()), Ok(()));
}

#[test]
fn plain_if_without_entity_after_trigger_sun_is_still_rejected() {
    assert_eq!(
        validate(&sun_then_plain_if_flow()),
        Err(FlowError::IfNeedsEntity { node: "chk".into() })
    );
}

fn switch_flow(params: serde_json::Value, edges: serde_json::Value) -> Flow {
    decode_flow(
        serde_json::json!([
            { "id": "m", "type": "trigger.state", "params": { "entity": "sensor" } },
            { "id": "s", "type": "switch", "params": params },
            { "id": "a", "type": "command", "params": { "entity": "l", "command": { "type": "turn_on" } } },
            { "id": "b", "type": "command", "params": { "entity": "l", "command": { "type": "turn_off" } } },
        ]),
        edges,
    )
}

fn two_rule_edges() -> serde_json::Value {
    serde_json::json!([
        { "from": "m", "to": "s" },
        { "from": "s", "to": "a", "port": "1" },
        { "from": "s", "to": "b", "port": "2" },
    ])
}

fn two_rules() -> serde_json::Value {
    serde_json::json!({ "rules": [
        { "op": "lt", "value": 18 },
        { "op": "between", "value": 18, "to": 22 } ] })
}

fn switch_reason(params: serde_json::Value) -> String {
    match validate(&switch_flow(params, two_rule_edges())) {
        Err(FlowError::BadOperand { node, reason }) if node == "s" => reason,
        other => panic!("expected BadOperand on s, got {other:?}"),
    }
}

fn time_flow(params: serde_json::Value, edges: serde_json::Value) -> Flow {
    decode_flow(
        serde_json::json!([
            { "id": "m", "type": "trigger.schedule", "params": { "cron": "0 7 * * *" } },
            { "id": "t", "type": "time.between", "params": params },
            { "id": "a", "type": "command", "params": { "entity": "l", "command": { "type": "turn_on" } } },
            { "id": "b", "type": "command", "params": { "entity": "l", "command": { "type": "turn_off" } } },
        ]),
        edges,
    )
}

fn time_edges() -> serde_json::Value {
    serde_json::json!([
        { "from": "m", "to": "t" },
        { "from": "t", "to": "a", "port": "in" },
        { "from": "t", "to": "b", "port": "out" },
    ])
}

#[test]
fn valid_switch_and_time_flows_pass() {
    assert_eq!(
        validate(&switch_flow(two_rules(), two_rule_edges())),
        Ok(())
    );
    let with_otherwise = serde_json::json!({
        "average": "15m", "otherwise": true, "all": true,
        "rules": [{ "op": "gte", "value": 1 }, { "op": "between", "value": 2, "to": 2 }] });
    let edges = serde_json::json!([
        { "from": "m", "to": "s" },
        { "from": "s", "to": "a", "port": "otherwise" },
    ]);
    assert_eq!(validate(&switch_flow(with_otherwise, edges)), Ok(()));
    let text = serde_json::json!({ "rules": [{ "op": "eq", "value": "on" }] });
    assert_eq!(validate(&switch_flow(text, serde_json::json!([]))), Ok(()));
    assert_eq!(
        validate(&time_flow(
            serde_json::json!({ "from": "22:00", "to": "06:00" }),
            time_edges()
        )),
        Ok(())
    );
}

#[test]
fn if_and_switch_result_need_a_number_value() {
    let reason = |json: &str| match validate(&flow_with_if_params_json(json)) {
        Err(FlowError::BadOperand { node, reason }) if node == "chk" => reason,
        other => panic!("expected BadOperand on chk, got {other:?}"),
    };
    assert_eq!(
        reason(r#"{"op":"eq","result":"x","value":"on"}"#),
        "value must be a number with result"
    );
    assert_eq!(
        reason(r#"{"op":"eq","result":"x","value":true}"#),
        "value must be a number with result"
    );
    assert_eq!(
        switch_reason(serde_json::json!({
            "result": "x", "rules": [{ "op": "lt", "value": 1 }, { "op": "eq", "value": "on" }]
        })),
        "rule 2: value must be a number with result"
    );
}

#[test]
fn switch_rule_errors() {
    let rules = |r: serde_json::Value| serde_json::json!({ "rules": r });
    assert_eq!(
        switch_reason(rules(serde_json::json!([]))),
        "rules must hold 1 to 10 entries"
    );
    let eleven: Vec<_> = (0..11)
        .map(|i| serde_json::json!({ "op": "eq", "value": i }))
        .collect();
    assert_eq!(
        switch_reason(rules(serde_json::json!(eleven))),
        "rules must hold 1 to 10 entries"
    );
    assert_eq!(
        switch_reason(rules(serde_json::json!([
            { "op": "eq", "value": 1 }, { "op": "eq", "value": [1] }]))),
        "rule 2: value must be a boolean, a number or a string"
    );
    assert_eq!(
        switch_reason(rules(serde_json::json!([{ "op": "between", "value": 1 }]))),
        "rule 1: to is required for op between"
    );
    assert_eq!(
        switch_reason(rules(
            serde_json::json!([{ "op": "lt", "value": 1, "to": 2 }])
        )),
        "rule 1: to is only allowed with op between"
    );
    assert_eq!(
        switch_reason(rules(serde_json::json!([
            { "op": "between", "value": "a", "to": 2 }]))),
        "rule 1: between needs numbers"
    );
    assert_eq!(
        switch_reason(rules(serde_json::json!([
            { "op": "between", "value": 1, "to": true }]))),
        "rule 1: between needs numbers"
    );
    assert_eq!(
        switch_reason(rules(serde_json::json!([
            { "op": "between", "value": 5, "to": 4 }]))),
        "rule 1: to must be at least value"
    );
    assert_eq!(
        switch_reason(serde_json::json!({ "average": "15m", "rules": [
            { "op": "eq", "value": 1 }, { "op": "eq", "value": "x" }] })),
        "rule 2: value must be a number with average"
    );
    assert!(
        switch_reason(serde_json::json!({ "average": "soon", "rules": [
            { "op": "eq", "value": 1 }] }))
        .starts_with("average: ")
    );
    assert_eq!(
        switch_reason(serde_json::json!({ "average": "25h", "rules": [
            { "op": "eq", "value": 1 }] })),
        "average must be at most 24h"
    );
}

#[test]
fn switch_ports_checked() {
    let edges = serde_json::json!([
        { "from": "m", "to": "s" }, { "from": "s", "to": "a", "port": "3" }]);
    assert_eq!(
        validate(&switch_flow(two_rules(), edges)),
        Err(FlowError::UnknownPort {
            node: "s".into(),
            port: "3".into()
        })
    );
    let edges = serde_json::json!([
        { "from": "m", "to": "s" }, { "from": "s", "to": "a", "port": "otherwise" }]);
    assert_eq!(
        validate(&switch_flow(two_rules(), edges)),
        Err(FlowError::UnknownPort {
            node: "s".into(),
            port: "otherwise".into()
        })
    );
    let edges = serde_json::json!([
        { "from": "m", "to": "s" }, { "from": "s", "to": "a" }]);
    let err = validate(&switch_flow(two_rules(), edges)).unwrap_err();
    assert_eq!(
        err,
        FlowError::MissingPort {
            node: "s".into(),
            kind: "a switch"
        }
    );
    assert_eq!(err.to_string(), "node `s` is a switch and needs a port");
}

#[test]
fn switch_after_schedule_needs_entity() {
    let flow = |params: serde_json::Value| {
        decode_flow(
            serde_json::json!([
                { "id": "tick", "type": "trigger.schedule", "params": { "cron": "0 7 * * *" } },
                { "id": "s", "type": "switch", "params": params },
                { "id": "a", "type": "command", "params": { "entity": "l", "command": { "type": "turn_on" } } },
            ]),
            serde_json::json!([
                { "from": "tick", "to": "s" },
                { "from": "s", "to": "a", "port": "1" },
            ]),
        )
    };
    let err = validate(&flow(
        serde_json::json!({ "rules": [{ "op": "eq", "value": 1 }] }),
    ))
    .unwrap_err();
    assert_eq!(err, FlowError::SwitchNeedsEntity { node: "s".into() });
    assert_eq!(
        err.to_string(),
        "node `s`: a switch after a schedule, sun or manual trigger without a value must name an entity"
    );
    assert_eq!(
        validate(&flow(serde_json::json!({
            "entity": "sensor", "rules": [{ "op": "eq", "value": 1 }] }))),
        Ok(())
    );
}

#[test]
fn time_between_errors() {
    let reason = |params: serde_json::Value| match validate(&time_flow(params, time_edges())) {
        Err(FlowError::BadOperand { node, reason }) if node == "t" => reason,
        other => panic!("expected BadOperand on t, got {other:?}"),
    };
    assert_eq!(
        reason(serde_json::json!({ "from": "7:00", "to": "08:00" })),
        "from: `7:00` is not a HH:MM time"
    );
    assert_eq!(
        reason(serde_json::json!({ "from": "07:00", "to": "24:00" })),
        "to: `24:00` is not a HH:MM time"
    );
    assert_eq!(
        reason(serde_json::json!({ "from": "07:00", "to": "07:00" })),
        "from and to must differ"
    );
    let edges = serde_json::json!([{ "from": "m", "to": "t" }, { "from": "t", "to": "a" }]);
    let err = validate(&time_flow(
        serde_json::json!({ "from": "07:00", "to": "08:00" }),
        edges,
    ))
    .unwrap_err();
    assert_eq!(
        err,
        FlowError::MissingPort {
            node: "t".into(),
            kind: "a time.between"
        }
    );
}

#[test]
fn time_between_after_schedule_is_fine() {
    // time_flow starts from a schedule trigger and has no entity to name.
    assert_eq!(
        validate(&time_flow(
            serde_json::json!({ "from": "07:00", "to": "08:00" }),
            time_edges()
        )),
        Ok(())
    );
}

const INTERP: &str = "round(interp(smoothed, [0, 38, 81, 160, 274, 438, 621, 843, 1062, 1270, 1450, 1650, 1947, 2100, 2180, 2200], [0, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55, 60, 70, 80, 90, 100]))";

/// A state trigger feeding one math card `m` with the given params, then nothing.
fn math_flow(params: serde_json::Value) -> Flow {
    Flow::decode(
        serde_json::json!({
            "id": "f", "name": "F",
            "nodes": [
                { "id": "t", "type": "trigger.state", "params": { "entity": "e" } },
                { "id": "m", "type": "math", "params": params }
            ],
            "edges": [{ "from": "t", "to": "m" }]
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap()
}

fn math_reason(params: serde_json::Value) -> String {
    match validate(&math_flow(params)) {
        Err(FlowError::BadMath { node, reason }) => {
            assert_eq!(node, "m");
            reason
        }
        other => panic!("expected BadMath, got {other:?}"),
    }
}

#[test]
fn math_input_rules() {
    let many: Vec<_> = (0..11)
        .map(|i| serde_json::json!({ "name": format!("a{i}"), "entity": "e" }))
        .collect();
    assert_eq!(
        math_reason(serde_json::json!({ "inputs": many, "formula": "1", "name": "r" })),
        "at most 10 inputs"
    );
    assert_eq!(
        math_reason(serde_json::json!({
            "inputs": [{ "name": "Bad", "entity": "e" }], "formula": "1", "name": "r"
        })),
        "`Bad` is not a valid name"
    );
    assert_eq!(
        math_reason(serde_json::json!({ "formula": "1", "name": "min" })),
        "`min` is not a valid name"
    );
    assert_eq!(
        math_reason(serde_json::json!({
            "inputs": [{ "name": "a", "entity": "e" }, { "name": "a", "entity": "f" }],
            "formula": "a", "name": "r"
        })),
        "input `a` is used twice"
    );
}

#[test]
fn math_input_average_follows_the_if_rules() {
    let with = |average: &str| {
        math_reason(serde_json::json!({
            "inputs": [{ "name": "a", "entity": "e", "average": average }],
            "formula": "a", "name": "r"
        }))
    };
    assert_eq!(with("25h"), "average must be at most 24h");
    assert!(with("soon").starts_with("average: "));
}

#[test]
fn math_formula_rules() {
    let parse_reason = parse_formula("1 +").unwrap_err().reason;
    assert_eq!(
        math_reason(serde_json::json!({ "formula": "1 +", "name": "r" })),
        parse_reason
    );
    assert_eq!(
        math_reason(serde_json::json!({ "formula": "x + 1", "name": "r" })),
        "unknown name `x`"
    );
    assert_eq!(
        math_reason(serde_json::json!({ "formula": "r + 1", "name": "r" })),
        "a math card cannot read its own result `r`"
    );
}

#[test]
fn math_card_may_read_trigger_and_another_card() {
    let mut flow = math_flow(serde_json::json!({ "formula": "trigger", "name": "a" }));
    flow.nodes.push(Node {
        id: "n".into(),
        name: None,
        position: None,
        kind: NodeKind::Math(MathParams {
            formula: "a + 1".into(),
            name: "b".into(),
            ..MathParams::default()
        }),
        extra: Default::default(),
    });
    flow.edges.push(edge("m", "n", None));
    assert_eq!(validate(&flow), Ok(()));
    // Two cards may share a name, and an input may shadow nothing it does not own.
    flow.nodes[2].kind = NodeKind::Math(MathParams {
        formula: "1".into(),
        name: "a".into(),
        ..MathParams::default()
    });
    assert_eq!(validate(&flow), Ok(()));
}

#[test]
fn math_card_may_read_an_ask_name() {
    let mut flow = math_flow(serde_json::json!({ "formula": "temp + 1", "name": "b" }));
    flow.nodes.push(Node {
        id: "q".into(),
        name: None,
        position: None,
        kind: NodeKind::Ask(AskParams {
            text: "T".into(),
            answers: vec![AskAnswer {
                label: "A".into(),
                words: vec![],
            }],
            name: Some("temp".into()),
            ..AskParams::default()
        }),
        extra: Default::default(),
    });
    flow.edges.push(edge("t", "q", None));
    assert_eq!(validate(&flow), Ok(()));
}

/// A trigger, a math card `slider`, then `after` (a node JSON) fed by it.
fn linked_flow(after: serde_json::Value) -> Flow {
    let mut after = after;
    after["id"] = "x".into();
    Flow::decode(
        serde_json::json!({
            "id": "f", "name": "F",
            "nodes": [
                { "id": "t", "type": "trigger.state", "params": { "entity": "e" } },
                { "id": "m", "type": "math",
                  "params": { "formula": "trigger", "name": "slider" } },
                after
            ],
            "edges": [{ "from": "t", "to": "m" }, { "from": "m", "to": "x" }]
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap()
}

fn bad_math(flow: &Flow, node: &str, reason: &str) {
    assert_eq!(
        validate(flow),
        Err(FlowError::BadMath {
            node: node.into(),
            reason: reason.into()
        })
    );
}

fn command_node(value_from: &str, command: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "type": "command", "params": {
        "entity": "n", "command": command, "value_from": value_from } })
}

#[test]
fn value_from_rules() {
    let set_number = serde_json::json!({ "type": "set_number", "value": 0 });
    assert_eq!(
        validate(&linked_flow(command_node("slider", set_number.clone()))),
        Ok(())
    );
    bad_math(
        &linked_flow(command_node("nope", set_number)),
        "x",
        "no math card is named `nope`",
    );
    bad_math(
        &linked_flow(command_node(
            "slider",
            serde_json::json!({ "type": "toggle" }),
        )),
        "x",
        "value_from works with set_number, set_temperature, set_position and turn_on",
    );
}

#[test]
fn result_rules_on_if_and_switch() {
    let if_with = |extra: serde_json::Value| {
        let mut params = serde_json::json!({ "op": "gt", "value": 3, "result": "slider" });
        params
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        linked_flow(serde_json::json!({ "type": "if", "params": params }))
    };
    assert_eq!(validate(&if_with(serde_json::json!({}))), Ok(()));
    let clash = "result cannot be combined with entity, average or missing";
    bad_math(&if_with(serde_json::json!({ "entity": "e" })), "x", clash);
    bad_math(&if_with(serde_json::json!({ "average": "5m" })), "x", clash);
    bad_math(
        &if_with(serde_json::json!({ "missing": "true" })),
        "x",
        clash,
    );
    bad_math(
        &if_with(serde_json::json!({ "result": "nope" })),
        "x",
        "no math card is named `nope`",
    );
    let switch_with = |extra: serde_json::Value| {
        let mut params = serde_json::json!({
            "rules": [{ "op": "gt", "value": 3 }], "result": "slider" });
        params
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        linked_flow(serde_json::json!({ "type": "switch", "params": params }))
    };
    assert_eq!(validate(&switch_with(serde_json::json!({}))), Ok(()));
    bad_math(
        &switch_with(serde_json::json!({ "entity": "e" })),
        "x",
        clash,
    );
    bad_math(
        &switch_with(serde_json::json!({ "result": "nope" })),
        "x",
        "no math card is named `nope`",
    );
}

#[test]
fn result_counts_as_naming_the_value_after_a_schedule() {
    let flow = Flow::decode(
        br#"{ "id": "f", "name": "F", "nodes": [
              { "id": "t", "type": "trigger.schedule", "params": { "cron": "0 * * * *" } },
              { "id": "m", "type": "math", "params": { "formula": "1", "name": "r" } },
              { "id": "i", "type": "if", "params": { "op": "gt", "value": 0, "result": "r" } }
            ], "edges": [{ "from": "t", "to": "m" }, { "from": "m", "to": "i" }] }"#,
    )
    .unwrap();
    assert_eq!(validate(&flow), Ok(()));
}

#[test]
fn renamed_math_card_refuses_stale_value_from() {
    let mut flow = linked_flow(command_node(
        "slider",
        serde_json::json!({ "type": "set_number", "value": 0 }),
    ));
    assert_eq!(validate(&flow), Ok(()));
    if let NodeKind::Math(p) = &mut flow.nodes[1].kind {
        p.name = "percent".into();
    }
    bad_math(&flow, "x", "no math card is named `slider`");
}

#[test]
fn math_flow_is_valid() {
    let flow = Flow::decode(
        serde_json::json!({
            "id": "heater", "name": "Heater", "mode": "single",
            "nodes": [
                { "id": "t", "type": "trigger.state", "params": { "entity": "grid" } },
                { "id": "d", "type": "math", "params": {
                    "inputs": [
                        { "name": "grid", "entity": "g" },
                        { "name": "heater", "entity": "h", "average": "1m" }],
                    "formula": "clamp(heater - 100 - grid, 0, 1650)", "name": "desired" } },
                { "id": "s", "type": "math", "params": {
                    "inputs": [{ "name": "prev", "entity": "virtual_target" }],
                    "formula": "if(abs(desired - prev) > 300, 0.1 * prev + 0.9 * desired, 0.6 * prev + 0.4 * desired)",
                    "name": "smoothed", "save_to": "virtual_target" } },
                { "id": "p", "type": "math", "params": {
                    "formula": INTERP, "name": "slider", "save_to": "group_heater" } },
                { "id": "c", "type": "command", "params": {
                    "entity": "group_heater",
                    "command": { "type": "set_number", "value": 0 }, "value_from": "slider" } },
                { "id": "i", "type": "if", "params": {
                    "op": "gt", "value": 300, "result": "desired" } }
            ],
            "edges": [
                { "from": "t", "to": "d" }, { "from": "d", "to": "s" },
                { "from": "s", "to": "p" }, { "from": "p", "to": "c" },
                { "from": "c", "to": "i" }
            ]
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(validate(&flow), Ok(()));
}

#[test]
fn math_round_trips() {
    let flow = math_flow(serde_json::json!({
        "inputs": [{ "name": "a", "entity": "e", "average": "5m", "tag": "kept too" }],
        "formula": "a", "name": "r", "save_to": "virtual_x", "note": "kept"
    }));
    let NodeKind::Math(p) = &flow.nodes[1].kind else {
        panic!("not a math card");
    };
    assert_eq!(p.inputs[0].average.as_deref(), Some("5m"));
    assert_eq!(
        p.inputs[0].extra.get("tag"),
        Some(&serde_json::json!("kept too"))
    );
    assert_eq!(p.save_to.as_deref(), Some("virtual_x"));
    assert_eq!(p.extra.get("note"), Some(&serde_json::json!("kept")));
    assert_eq!(Flow::decode(flow.encode().as_bytes()).unwrap(), flow);
    assert_eq!(flow.nodes[1].kind.type_name(), "math");
    assert!(flow.nodes[1].kind.ports().is_empty());
    assert!(!flow.nodes[1].kind.is_trigger());
}

#[test]
fn command_with_number_cases() {
    use crate::model::{Command, Hs};
    assert_eq!(
        command_with_number(&Command::SetPosition { position: 0 }, 104.6),
        Some(Command::SetPosition { position: 100 })
    );
    assert_eq!(
        command_with_number(&Command::SetNumber { value: 0.0 }, 2.5),
        Some(Command::SetNumber { value: 2.5 })
    );
    assert_eq!(
        command_with_number(&Command::SetTemperature { temperature: 0.0 }, 19.5),
        Some(Command::SetTemperature { temperature: 19.5 })
    );
    let colour = Hs { h: 10.0, s: 20.0 };
    assert_eq!(
        command_with_number(
            &Command::TurnOn {
                brightness: None,
                color: Some(colour),
                effect: None
            },
            127.5
        ),
        Some(Command::TurnOn {
            brightness: Some(128),
            color: Some(colour),
            effect: None
        })
    );
    assert_eq!(command_with_number(&Command::Toggle, 1.0), None);
}

fn one_node_flow(json: &str) -> Flow {
    let mut f = schedule_flow("*/10 * * * *", "act");
    f.nodes.push(serde_json::from_str(json).unwrap());
    f
}

#[test]
fn wake_up_before_minutes_bounds() {
    let wake = |n: u32| {
        decode_flow(
            serde_json::json!([{ "id": "w", "type": "trigger.wake_up",
                "params": { "before_minutes": n } }]),
            serde_json::json!([]),
        )
    };
    assert_eq!(validate(&wake(240)), Ok(()));
    assert!(matches!(
        validate(&wake(241)),
        Err(FlowError::BadWakeUpBefore { minutes: 241, .. })
    ));
}

#[test]
fn catch_up_minutes_bounds() {
    let trig = |ty: &str, params: serde_json::Value, c: u32| {
        let mut params = params;
        params["catch_up_minutes"] = c.into();
        decode_flow(
            serde_json::json!([{ "id": "t", "type": ty, "params": params }]),
            serde_json::json!([]),
        )
    };
    let cases = [
        (
            "trigger.schedule",
            serde_json::json!({ "cron": "0 7 * * *" }),
        ),
        ("trigger.sun", serde_json::json!({ "which": "sunset" })),
        (
            "trigger.wake_up",
            serde_json::json!({ "before_minutes": 30 }),
        ),
    ];
    for (ty, params) in cases {
        assert_eq!(validate(&trig(ty, params.clone(), 720)), Ok(()), "{ty}");
        assert!(
            matches!(
                validate(&trig(ty, params, 721)),
                Err(FlowError::BadCatchUp { minutes: 721, .. })
            ),
            "{ty}"
        );
    }
}

#[test]
fn climate_step_checks() {
    let step = |ty: &str, params: &str| {
        one_node_flow(&format!(r#"{{"id":"c","type":"{ty}","params":{params}}}"#))
    };
    let ok = r#"{"zones":["z"],"setpoint":21,"minutes":60}"#;
    assert_eq!(validate(&step("climate.hold", ok)), Ok(()));
    assert_eq!(
        validate(&step("climate.eco", r#"{"zones":["z"],"until":"23:59"}"#)),
        Ok(())
    );
    let bad = [
        r#"{"zones":[],"setpoint":21,"minutes":60}"#,
        r#"{"zones":["z"],"setpoint":21}"#,
        r#"{"zones":["z"],"setpoint":21,"minutes":60,"until":"07:00"}"#,
        r#"{"zones":["z"],"setpoint":21,"minutes":0}"#,
        r#"{"zones":["z"],"setpoint":21,"minutes":1441}"#,
        r#"{"zones":["z"],"setpoint":21,"until":"24:00"}"#,
        r#"{"zones":["z"],"setpoint":21,"minutes":60,"mode":"fan"}"#,
    ];
    for params in bad {
        assert!(
            matches!(
                validate(&step("climate.hold", params)),
                Err(FlowError::BadClimateStep { .. })
            ),
            "{params}"
        );
    }
    assert!(matches!(
        validate(&step("climate.eco", r#"{"zones":[],"minutes":5}"#)),
        Err(FlowError::BadClimateStep { .. })
    ));
}

#[test]
fn if_without_entity_after_wake_up_is_refused() {
    let f = decode_flow(
        serde_json::json!([
            { "id": "w", "type": "trigger.wake_up", "params": { "before_minutes": 30 } },
            { "id": "i", "type": "if", "params": { "op": "eq", "value": true } },
            { "id": "a", "type": "command",
              "params": { "entity": "l", "command": { "type": "turn_on" } } },
        ]),
        serde_json::json!([
            { "from": "w", "to": "i" },
            { "from": "i", "to": "a", "port": "true" },
        ]),
    );
    assert_eq!(
        validate(&f),
        Err(FlowError::IfNeedsEntity { node: "i".into() })
    );
}
