use serde_json::json;

use super::*;

#[test]
fn state_bounds_read_numbers_only() {
    let p = |above, below| TriggerStateParams {
        above,
        below,
        ..Default::default()
    };
    assert_eq!(p(None, None).in_bounds(&Scalar::Number(3.0)), None);
    assert_eq!(
        p(Some(25.0), None).in_bounds(&Scalar::Number(25.0)),
        Some(false)
    );
    assert_eq!(
        p(Some(25.0), None).in_bounds(&Scalar::Number(25.1)),
        Some(true)
    );
    assert_eq!(
        p(None, Some(18.0)).in_bounds(&Scalar::Number(17.9)),
        Some(true)
    );
    assert_eq!(
        p(Some(18.0), Some(22.0)).in_bounds(&Scalar::Number(20.0)),
        Some(true)
    );
    assert_eq!(
        p(Some(18.0), Some(22.0)).in_bounds(&Scalar::Number(22.0)),
        Some(false)
    );
    assert_eq!(
        p(Some(1.0), None).in_bounds(&Scalar::Text("on".into())),
        Some(false)
    );
}

#[test]
fn state_for_round_trips_as_for() {
    let text = r#"{"entity":"m","to":false,"for":"5m"}"#;
    let p: TriggerStateParams = serde_json::from_str(text).unwrap();
    assert_eq!(p.for_.as_deref(), Some("5m"));
    assert!(p.extra.is_empty());
    assert_eq!(serde_json::to_string(&p).unwrap(), text);
}

#[test]
fn labels_default_empty_and_are_omitted() {
    let flow = Flow::decode(br#"{"id":"a","name":"A"}"#).unwrap();
    assert!(flow.labels.is_empty());
    assert!(!flow.encode().contains("labels"));
    let flow = Flow::decode(br#"{"id":"a","name":"A","labels":["ete","hiver"]}"#).unwrap();
    assert_eq!(flow.labels, ["ete", "hiver"]);
    let again = Flow::decode(flow.encode().as_bytes()).unwrap();
    assert_eq!(again, flow);
}

#[test]
fn ask_node_round_trips_and_lists_ports() {
    let json = r#"{"id":"q","type":"ask","params":{"text":"Je chauffe ?","room":"sejour","answers":[{"label":"Oui"},{"label":"Seulement en haut","words":["en haut"]},{"label":"Non"}],"name":"temp","default":21.0}}"#;
    let node: Node = serde_json::from_str(json).unwrap();
    assert_eq!(node.kind.type_name(), "ask");
    assert_eq!(node.kind.ports(), vec!["1", "2", "3", "none"]);
    assert_eq!(
        serde_json::to_value(&node).unwrap(),
        serde_json::from_str::<Value>(json).unwrap()
    );
}

#[test]
fn math_names_include_ask_names() {
    let flow: Flow = serde_json::from_value(json!({
        "id": "f", "name": "f",
        "nodes": [
            {"id": "m", "type": "math", "params": {"inputs": [], "formula": "1", "name": "a"}},
            {"id": "q", "type": "ask", "params": {"text": "T", "answers": [{"label": "Oui"}], "name": "temp"}},
            {"id": "r", "type": "ask", "params": {"text": "T", "answers": [{"label": "Oui"}]}}
        ]
    }))
    .unwrap();
    assert_eq!(math_names(&flow), BTreeSet::from(["a", "temp"]));
}

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
fn speak_node_round_trips() {
    let text = r#"{"id":"s","type":"speak","params":{"text":"Dinner is ready","room":"kitchen"}}"#;
    let node: Node = serde_json::from_str(text).unwrap();
    assert!(!node.kind.is_trigger());
    assert!(node.kind.ports().is_empty());
    assert_eq!(node.kind.type_name(), "speak");
    assert!(matches!(&node.kind, NodeKind::Speak(p)
        if p.text == "Dinner is ready" && p.room.as_deref() == Some("kitchen")));
    assert_eq!(
        serde_json::to_value(&node).unwrap(),
        serde_json::from_str::<serde_json::Value>(text).unwrap()
    );
    // No room: every room, and nothing is written back.
    let all = r#"{"id":"s","type":"speak","params":{"text":"Hi"}}"#;
    let node: Node = serde_json::from_str(all).unwrap();
    assert!(matches!(&node.kind, NodeKind::Speak(p) if p.room.is_none()));
    assert_eq!(
        serde_json::to_value(&node).unwrap(),
        serde_json::from_str::<serde_json::Value>(all).unwrap()
    );
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
    let node: Node =
        serde_json::from_str(r#"{"id":"n","type":"notify","params":{"title":"T","lang":"fr"}}"#)
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
fn flow_label_round_trips() {
    let text = r#"{"id":"season","type":"flow.label","params":{"label":"ete","action":"pick"}}"#;
    let node: Node = serde_json::from_str(text).unwrap();
    assert_eq!(node.kind.type_name(), "flow.label");
    assert_eq!(
        node.kind,
        NodeKind::FlowLabel(FlowLabelParams {
            label: "ete".into(),
            action: LabelAction::Pick,
            extra: BTreeMap::new(),
        })
    );
    assert!(!node.kind.is_trigger());
    assert_eq!(
        serde_json::to_value(&node).unwrap(),
        serde_json::from_str::<Value>(text).unwrap()
    );
}

#[test]
fn flow_enable_defaults_to_self() {
    let node: Node =
        serde_json::from_str(r#"{"id":"stop","type":"flow.enable","params":{"enabled":false}}"#)
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
fn trigger_alarm_round_trips_and_defaults() {
    let raw = r#"{"id":"a","type":"trigger.alarm","params":{"to":"armed_away"}}"#;
    let node: Node = serde_json::from_str(raw).unwrap();
    match &node.kind {
        NodeKind::TriggerAlarm(p) => {
            assert_eq!(p.to, Some(AlarmModeFilter::ArmedAway));
            assert_eq!(p.from, None);
        }
        other => panic!("{other:?}"),
    }
    assert!(node.kind.is_trigger());
    assert_eq!(node.kind.type_name(), "trigger.alarm");
    assert_eq!(
        serde_json::to_value(&node).unwrap(),
        serde_json::from_str::<Value>(raw).unwrap()
    );
    let empty: Node =
        serde_json::from_str(r#"{"id":"a","type":"trigger.alarm","params":{}}"#).unwrap();
    match &empty.kind {
        NodeKind::TriggerAlarm(p) => assert!(p.to.is_none() && p.from.is_none()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn alarm_mode_filter_matches() {
    let states = [
        "disarmed",
        "arming",
        "armed_away",
        "armed_home",
        "armed_night",
        "pending",
        "triggered",
    ];
    for filter in AlarmModeFilter::ALL {
        for state in states {
            let want = match filter {
                AlarmModeFilter::Armed => state.starts_with("armed_"),
                other => other.as_str() == state,
            };
            assert_eq!(filter.matches(state), want, "{filter:?} vs {state}");
        }
        assert!(!filter.matches("unknown"));
    }
    assert!(!AlarmModeFilter::Armed.matches("arming"));
    assert!(!AlarmModeFilter::Armed.matches("pending"));
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
        serde_json::from_str(r#"{"id":"c","type":"if","params":{"op":"is_off_peak"}}"#).unwrap();
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
        serde_json::from_str(r#"{"op":"lt","value":65,"entity":"t","missing":"false"}"#).unwrap();
    assert_eq!(p.missing, Some(Missing::False));
    let p: IfParams = serde_json::from_str(r#"{"op":"lt","value":65}"#).unwrap();
    assert_eq!(p.missing, None);
    assert!(serde_json::to_value(&p).unwrap().get("missing").is_none());
}

#[test]
fn if_average_round_trips_and_defaults() {
    let p: IfParams = serde_json::from_str(r#"{"op":"lte","value":70,"average":"15m"}"#).unwrap();
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

#[test]
fn backup_run_round_trips_with_defaults() {
    let node: Node = serde_json::from_str(r#"{"id":"b","type":"backup.run","params":{}}"#).unwrap();
    let NodeKind::BackupRun(ref p) = node.kind else {
        panic!("not a backup.run");
    };
    assert_eq!(p.timeout_minutes, 120);
    assert_eq!(p.destination, BackupTarget::Ftp);
    assert_eq!(node.kind.ports(), vec!["ok", "partial", "failed"]);
    assert_eq!(node.kind.type_name(), "backup.run");
    let back = serde_json::to_value(&node).unwrap();
    assert_eq!(back["type"], "backup.run");
    assert_eq!(back["params"]["destination"], "ftp");
    assert_eq!(back["params"]["timeout_minutes"], 120);
    let again: Node = serde_json::from_value(back).unwrap();
    assert_eq!(again, node);
}

#[test]
fn wake_up_trigger_round_trips() {
    let json = r#"{"id":"t","type":"trigger.wake_up","params":{"before_minutes":45,"alarms":["wake-up-1"],"catch_up_minutes":60}}"#;
    let node: Node = serde_json::from_str(json).unwrap();
    assert!(node.kind.is_trigger());
    assert_eq!(node.kind.type_name(), "trigger.wake_up");
    assert_eq!(node.kind.catch_up_minutes(), 60);
    let back: Node = serde_json::from_str(&serde_json::to_string(&node).unwrap()).unwrap();
    assert_eq!(back, node);
}

#[test]
fn climate_hold_off_parses() {
    let parse = |sp: &str| {
        serde_json::from_str::<Node>(&format!(
            r#"{{"id":"h","type":"climate.hold","params":{{"zones":["z"],"setpoint":{sp},"minutes":30}}}}"#
        ))
    };
    let kind = |sp: &str| match parse(sp).unwrap().kind {
        NodeKind::ClimateHold(p) => p.setpoint,
        other => panic!("{other:?}"),
    };
    assert_eq!(kind("\"off\""), ClimateSetpoint::Off);
    assert_eq!(kind("21.5"), ClimateSetpoint::Temp(21.5));
    assert!(parse("\"warm\"").is_err());
}

#[test]
fn catch_up_absent_is_zero() {
    let json = r#"{"id":"s","type":"trigger.schedule","params":{"cron":"0 7 * * *"}}"#;
    let node: Node = serde_json::from_str(json).unwrap();
    assert_eq!(node.kind.catch_up_minutes(), 0);
    assert_eq!(serde_json::to_string(&node).unwrap(), json);
}
