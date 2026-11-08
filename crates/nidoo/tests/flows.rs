//! Flows on the hub, driven without a broker: `ingest` feeds it, the publisher channel is
//! inspected.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nidoo::config::Config;
use nidoo::hub::{Hub, HubError};
use nidoo_api::{ConnectionStatus, RunStatus, RunView, StepOutcome, StepView, WsEvent};
use nidoo_core::command::Publish;
use nidoo_core::flow::{Flow, NodeKind};
use nidoo_core::model::{Area, AreasDocument, Command};
use tokio::sync::mpsc;

use crate::common;

const LED_ID: &str = "0x60a423fffe94d1c6_light_zigbee2mqtt";
const MOTION_ID: &str = "0x00158d008b5a2d65_occupancy_zigbee2mqtt";
const LED_STATE_TOPIC: &str = "zigbee2mqtt/rooms/living/led_ceiling";
const MOTION_STATE_TOPIC: &str = "zigbee2mqtt/rooms/living/motion_north";
const LED_COMMAND_TOPIC: &str = "zigbee2mqtt/rooms/living/led_ceiling/set";
const FLOW_TOPIC: &str = "nidoo/automation/living";

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../nidoo-core/tests/fixtures/"
    ));
    std::fs::read(path.join(name)).expect("fixture")
}

fn hub() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let mut cfg = Config::default();
    cfg.commands.confirm_timeout_ms = 50;
    cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
    let (tx, rx) = mpsc::channel(64);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    (hub, rx)
}

/// A hub that knows the LED and the motion sensor.
fn hub_with_entities() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let (hub, rx) = hub();
    hub.ingest(
        "nidoo/light/0x60a423fffe94d1c6/light/config",
        &fixture("discovery/light__0x60a423fffe94d1c6__light.json"),
    );
    hub.ingest(
        "nidoo/binary_sensor/0x00158d008b5a2d65/occupancy/config",
        &fixture("discovery/binary_sensor__0x00158d008b5a2d65__occupancy.json"),
    );
    (hub, rx)
}

fn reference_json() -> String {
    serde_json::json!({
        "id": "living", "name": "Living LED on motion", "enabled": true, "mode": "restart",
        "nodes": [
            { "id": "motion", "type": "trigger.state", "params": { "entity": MOTION_ID } },
            { "id": "moving", "type": "if", "params": { "op": "eq", "value": true } },
            { "id": "on", "type": "command",
              "params": { "entity": LED_ID, "command": { "type": "turn_on" } } },
            { "id": "wait", "type": "delay", "params": { "for": "5m" } },
            { "id": "off", "type": "command",
              "params": { "entity": LED_ID, "command": { "type": "turn_off" } } }
        ],
        "edges": [
            { "from": "motion", "to": "moving" },
            { "from": "moving", "port": "true", "to": "on" },
            { "from": "moving", "port": "false", "to": "wait" },
            { "from": "wait", "to": "off" }
        ]
    })
    .to_string()
}

fn reference_flow() -> Flow {
    Flow::decode(reference_json().as_bytes()).expect("reference flow")
}

async fn next_event(rx: &mut tokio::sync::broadcast::Receiver<WsEvent>) -> WsEvent {
    tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("an event within 1 s")
        .expect("open channel")
}

fn step(node: &str) -> StepView {
    StepView {
        node: node.to_owned(),
        at: 1,
        outcome: StepOutcome::Ok,
        detail: String::new(),
        ports: None,
    }
}

#[tokio::test]
async fn retained_flow_is_listed_and_broadcast() {
    let (hub, _publisher) = hub_with_entities();
    let mut events = hub.subscribe();

    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    match next_event(&mut events).await {
        WsEvent::FlowsChanged { flows } => assert_eq!(flows.len(), 1),
        other => panic!("expected FlowsChanged, got {other:?}"),
    }
    assert_eq!(hub.snapshot().flows.len(), 1);
    assert_eq!(hub.flow_definitions().len(), 1);
    assert!(hub.flow("living").is_some());

    // The same payload again changes nothing.
    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    assert!(events.try_recv().is_err());
}

#[tokio::test]
async fn set_flow_publishes_retained_and_lists() {
    let (hub, mut publisher) = hub_with_entities();
    let view = hub.set_flow(reference_flow()).await.expect("set_flow");
    assert_eq!(view.flow.definition.id, "living");

    let publish = publisher.try_recv().expect("a publish");
    assert_eq!(publish.topic, FLOW_TOPIC);
    assert!(publish.retain);
    assert!(hub.flow("living").is_some());
}

#[tokio::test]
async fn set_flow_rejects_unknown_entity_and_bad_command() {
    let (hub, mut publisher) = hub_with_entities();

    let mut flow = reference_flow();
    for node in &mut flow.nodes {
        if let NodeKind::TriggerState(params) = &mut node.kind {
            params.entity = "nope".to_owned();
        }
    }
    match hub.set_flow(flow).await {
        Err(HubError::InvalidFlow(reason)) => assert!(reason.contains("nope"), "{reason}"),
        other => panic!("expected InvalidFlow, got {other:?}"),
    }

    let mut flow = reference_flow();
    for node in &mut flow.nodes {
        if node.id == "off"
            && let NodeKind::Command(params) = &mut node.kind
        {
            params.command = Command::SetPosition { position: 50 };
        }
    }
    match hub.set_flow(flow).await {
        Err(HubError::InvalidFlow(reason)) => assert!(reason.contains("off"), "{reason}"),
        other => panic!("expected InvalidFlow, got {other:?}"),
    }
    assert!(publisher.try_recv().is_err(), "nothing was published");
}

#[tokio::test]
async fn set_flow_refused_while_offline() {
    let (hub, _publisher) = hub_with_entities();
    hub.set_connection(ConnectionStatus::Connecting);
    assert!(matches!(
        hub.set_flow(reference_flow()).await,
        Err(HubError::Publish(_))
    ));
    assert!(hub.flow("living").is_none());
}

#[tokio::test]
async fn set_flow_refuses_alarm_codes() {
    let (hub, mut publisher) = hub_with_entities();
    for command in [
        Command::Arm {
            mode: nidoo_core::model::AlarmMode::Away,
            code: Some("1234".to_owned()),
        },
        Command::Disarm {
            code: Some("1234".to_owned()),
        },
    ] {
        let mut flow = reference_flow();
        for node in &mut flow.nodes {
            if let NodeKind::Command(params) = &mut node.kind {
                params.command = command.clone();
            }
        }
        match hub.set_flow(flow).await {
            Err(HubError::InvalidFlow(reason)) => {
                assert!(reason.contains("alarm codes are not allowed"), "{reason}");
            }
            other => panic!("expected InvalidFlow, got {other:?}"),
        }
    }
    assert!(publisher.try_recv().is_err());
}

#[tokio::test]
async fn run_flow_refused_while_offline() {
    let (hub, _publisher) = hub_with_entities();
    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    hub.set_connection(ConnectionStatus::Connecting);
    assert_eq!(
        hub.run_flow("living"),
        Err(HubError::Publish("broker offline".to_owned()))
    );
}

#[tokio::test]
async fn delete_flow_publishes_empty_retained() {
    let (hub, mut publisher) = hub_with_entities();
    assert!(matches!(
        hub.delete_flow("living").await,
        Err(HubError::UnknownFlow(_))
    ));
    hub.set_flow(reference_flow()).await.expect("set_flow");
    publisher.try_recv().expect("the definition");

    hub.delete_flow("living").await.expect("delete");
    let publish = publisher.try_recv().expect("a publish");
    assert_eq!(publish.topic, FLOW_TOPIC);
    assert!(publish.payload.is_empty());
    assert!(publish.retain);
    assert!(hub.flow("living").is_none());
}

#[tokio::test]
async fn set_flow_enabled_round_trips() {
    let (hub, mut publisher) = hub_with_entities();
    hub.set_flow(reference_flow()).await.expect("set_flow");
    publisher.try_recv().expect("the definition");

    let view = hub.set_flow_enabled("living", false).await.expect("toggle");
    assert!(!view.definition.enabled);
    let publish = publisher.try_recv().expect("a publish");
    assert!(
        publish.payload.contains("\"enabled\":false"),
        "{}",
        publish.payload
    );

    assert!(matches!(
        hub.set_flow_enabled("ghost", true).await,
        Err(HubError::UnknownFlow(_))
    ));
    hub.ingest("nidoo/automation/broken", b"not json");
    assert!(matches!(
        hub.set_flow_enabled("broken", true).await,
        Err(HubError::InvalidFlow(_))
    ));
}

#[tokio::test]
async fn run_flow_rejects_disabled_invalid_unknown_and_active() {
    let (hub, _publisher) = hub_with_entities();
    assert!(matches!(
        hub.run_flow("ghost"),
        Err(HubError::UnknownFlow(_))
    ));

    hub.ingest("nidoo/automation/broken", b"not json");
    assert!(matches!(
        hub.run_flow("broken"),
        Err(HubError::InvalidFlow(_))
    ));

    let mut disabled = reference_flow();
    disabled.id = "off_flow".to_owned();
    disabled.enabled = false;
    hub.ingest("nidoo/automation/off_flow", disabled.encode().as_bytes());
    assert_eq!(
        hub.run_flow("off_flow"),
        Err(HubError::Rejected("flow is disabled".to_owned()))
    );

    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    let mut requests = hub.take_engine_requests().expect("the receiver");
    let run = hub.run_flow("living").expect("run");
    assert_eq!(run.status, RunStatus::Running);
    assert!(run.trigger.is_none());
    assert_eq!(
        hub.run_flow("living"),
        Err(HubError::Rejected("a run is active".to_owned()))
    );
    assert!(hub.take_engine_requests().is_none());

    // The edit reloaded the engine, then the run was requested.
    assert!(matches!(
        requests.try_recv(),
        Ok(nidoo::hub::EngineRequest::Reload)
    ));
    let mut saw_run = false;
    while let Ok(request) = requests.try_recv() {
        if let nidoo::hub::EngineRequest::Run {
            flow_id, run_id, ..
        } = request
            && flow_id == "living"
            && run_id == run.id
        {
            saw_run = true;
        }
    }
    assert!(saw_run);
}

#[tokio::test]
async fn run_bookkeeping_broadcasts() {
    let (hub, _publisher) = hub_with_entities();
    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    let mut events = hub.subscribe();

    let run = hub.begin_run("living", None).expect("begin");
    match next_event(&mut events).await {
        WsEvent::FlowRunChanged { flow_id, run: seen } => {
            assert_eq!(flow_id, "living");
            assert_eq!(seen.status, RunStatus::Running);
        }
        other => panic!("expected FlowRunChanged, got {other:?}"),
    }
    assert!(matches!(
        hub.begin_run("ghost", None),
        Err(HubError::UnknownFlow(_))
    ));

    hub.record_step("living", &run.id, step("motion"));
    match next_event(&mut events).await {
        WsEvent::FlowRunChanged { run: seen, .. } => assert_eq!(seen.steps.len(), 1),
        other => panic!("expected FlowRunChanged, got {other:?}"),
    }

    hub.end_run("living", &run.id, RunStatus::Done);
    match next_event(&mut events).await {
        WsEvent::FlowRunChanged { run: seen, .. } => {
            assert_eq!(seen.status, RunStatus::Done);
            assert!(seen.ended_at.is_some());
        }
        other => panic!("expected FlowRunChanged, got {other:?}"),
    }
    assert!(hub.flow("living").expect("flow").active_run.is_none());

    let skipped = hub
        .add_skipped_run("living", "motion", None)
        .expect("skipped");
    assert_eq!(skipped.status, RunStatus::Skipped);
    assert!(matches!(
        next_event(&mut events).await,
        WsEvent::FlowRunChanged { .. }
    ));
}

#[tokio::test]
async fn entity_scalar_reads_the_registry() {
    let (hub, _publisher) = hub_with_entities();
    assert!(hub.entity_scalar(MOTION_ID).is_none());
    hub.ingest(MOTION_STATE_TOPIC, br#"{"occupancy": true}"#);
    assert_eq!(
        hub.entity_scalar(MOTION_ID),
        Some(nidoo_core::flow::Scalar::Bool(true))
    );
    assert!(hub.entity_scalar("nope").is_none());
    let _ = LED_STATE_TOPIC;
}

#[tokio::test]
async fn nested_automation_topics_are_ignored() {
    let (hub, _publisher) = hub_with_entities();
    hub.ingest("nidoo/automation/a/b", reference_json().as_bytes());
    assert!(hub.flows().is_empty());
}

// ---- the engine ----

/// Lets the engine task drain what it has been sent (current-thread runtimes only).
async fn settle() {
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

/// A hub with the entities and a running engine.
fn engine_hub() -> (
    Arc<Hub>,
    mpsc::Receiver<Publish>,
    tokio::task::JoinHandle<()>,
) {
    let (hub, rx) = hub_with_entities();
    let engine = hub.start_engine();
    (hub, rx, engine)
}

/// Plays the broker: records every command published to the LED and echoes the state it asks
/// for, so `send_command` confirms.
fn echo(mut publisher: mpsc::Receiver<Publish>, hub: Arc<Hub>) -> Arc<Mutex<Vec<String>>> {
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&log);
    tokio::spawn(async move {
        while let Some(publish) = publisher.recv().await {
            if publish.topic != LED_COMMAND_TOPIC {
                continue;
            }
            sink.lock().expect("log").push(publish.payload.clone());
            let on = serde_json::from_str::<serde_json::Value>(&publish.payload)
                .is_ok_and(|v| v["state"] == "ON");
            let echo: &[u8] = if on {
                br#"{"state":"ON","brightness":100}"#
            } else {
                br#"{"state":"OFF","brightness":100}"#
            };
            hub.ingest(LED_STATE_TOPIC, echo);
        }
    });
    log
}

fn motion(hub: &Hub, on: bool) {
    let payload = format!(r#"{{"occupancy":{on},"battery":97}}"#);
    hub.ingest(MOTION_STATE_TOPIC, payload.as_bytes());
}

/// The next run event the predicate accepts, within 2 s.
async fn next_run(
    rx: &mut tokio::sync::broadcast::Receiver<WsEvent>,
    accept: impl Fn(&RunView) -> bool,
) -> RunView {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let WsEvent::FlowRunChanged { run, .. } = rx.recv().await.expect("open channel")
                && accept(&run)
            {
                return run;
            }
        }
    })
    .await
    .expect("the expected run event within 2 s")
}

async fn next_run_event(
    rx: &mut tokio::sync::broadcast::Receiver<WsEvent>,
    status: RunStatus,
) -> RunView {
    next_run(rx, |run| run.status == status).await
}

/// Asserts no run event arrives for 200 ms.
async fn no_run_event(rx: &mut tokio::sync::broadcast::Receiver<WsEvent>) {
    let quiet = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            if let Ok(WsEvent::FlowRunChanged { run, .. }) = rx.recv().await {
                return run;
            }
        }
    })
    .await;
    assert!(quiet.is_err(), "unexpected run event: {quiet:?}");
}

/// The reference flow with `edit` applied to its JSON (nodes: 0 motion, 1 moving, 2 on, 3 wait,
/// 4 off).
fn tweaked(edit: impl FnOnce(&mut serde_json::Value)) -> String {
    let mut value: serde_json::Value = serde_json::from_str(&reference_json()).expect("json");
    edit(&mut value);
    value.to_string()
}

fn published(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    log.lock().expect("log").clone()
}

fn step_nodes(run: &RunView) -> Vec<&str> {
    run.steps.iter().map(|s| s.node.as_str()).collect()
}

#[tokio::test]
async fn motion_true_turns_the_led_on() {
    let (hub, publisher, _engine) = engine_hub();
    let log = echo(publisher, Arc::clone(&hub));
    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    settle().await;
    let mut events = hub.subscribe();

    motion(&hub, false);
    motion(&hub, true);
    let run = next_run_event(&mut events, RunStatus::Done).await;

    assert_eq!(step_nodes(&run), ["motion", "moving", "on"]);
    assert_eq!(run.steps[0].detail, format!("{MOTION_ID} = true"));
    assert_eq!(run.steps[1].detail, "true");
    assert_eq!(run.steps[2].detail, "accepted");
    let trigger = run.trigger.expect("trigger");
    assert_eq!(trigger.value, serde_json::json!(true));
    assert_eq!(trigger.previous, serde_json::json!(false));
    assert!(published(&log)[0].contains("\"state\":\"ON\""), "{log:?}");
}

#[tokio::test(start_paused = true)]
async fn motion_false_turns_off_after_the_delay() {
    let (hub, publisher, _engine) = engine_hub();
    let log = echo(publisher, Arc::clone(&hub));
    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    settle().await;
    let mut events = hub.subscribe();

    motion(&hub, false);
    motion(&hub, true);
    next_run_event(&mut events, RunStatus::Done).await;
    assert_eq!(published(&log).len(), 1, "only the ON so far");
    motion(&hub, false);
    let waiting = next_run(&mut events, |r| {
        r.status == RunStatus::Running && step_nodes(r) == ["motion", "moving"]
    })
    .await;
    assert_eq!(waiting.steps[1].detail, "false");
    settle().await;

    tokio::time::advance(Duration::from_secs(4 * 60 + 59)).await;
    settle().await;
    assert_eq!(published(&log).len(), 1, "still waiting at 4m59s");
    no_run_event(&mut events).await;
    tokio::time::advance(Duration::from_secs(2)).await;
    let run = next_run_event(&mut events, RunStatus::Done).await;
    assert_eq!(step_nodes(&run), ["motion", "moving", "wait", "off"]);
    assert_eq!(run.steps[2].detail, "waited 5m");
    let log = published(&log);
    assert_eq!(log.len(), 2, "{log:?}");
    assert!(log[1].contains("OFF"), "{log:?}");
}

#[tokio::test(start_paused = true)]
async fn random_delay_waits_within_its_range() {
    let (hub, publisher, _engine) = engine_hub();
    let log = echo(publisher, Arc::clone(&hub));
    let json = reference_json().replace(r#""for":"5m""#, r#""for":"1m","up_to":"2m""#);
    assert!(json.contains("up_to"), "{json}");
    hub.ingest(FLOW_TOPIC, json.as_bytes());
    settle().await;
    let mut events = hub.subscribe();

    motion(&hub, true);
    motion(&hub, false);
    next_run(&mut events, |r| {
        r.status == RunStatus::Running && step_nodes(r) == ["motion", "moving"]
    })
    .await;
    settle().await;

    tokio::time::advance(Duration::from_secs(59)).await;
    settle().await;
    assert!(
        published(&log).iter().all(|p| !p.contains("OFF")),
        "still waiting at 59s"
    );
    tokio::time::advance(Duration::from_secs(62)).await;
    let run = next_run_event(&mut events, RunStatus::Done).await;
    assert_eq!(step_nodes(&run), ["motion", "moving", "wait", "off"]);
    assert!(
        run.steps[2].detail.starts_with("waited ") && run.steps[2].detail.ends_with(" (1m to 2m)"),
        "{}",
        run.steps[2].detail
    );
    assert!(published(&log).last().is_some_and(|p| p.contains("OFF")));
}

#[tokio::test(start_paused = true)]
async fn motion_back_cancels_the_pending_off() {
    let (hub, publisher, _engine) = engine_hub();
    let log = echo(publisher, Arc::clone(&hub));
    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    settle().await;
    let mut events = hub.subscribe();

    motion(&hub, true);
    motion(&hub, false);
    let first = next_run(&mut events, |r| r.steps.len() == 2).await;
    settle().await;
    tokio::time::advance(Duration::from_secs(2 * 60)).await;
    motion(&hub, true);

    let cancelled = next_run_event(&mut events, RunStatus::Cancelled).await;
    assert_eq!(cancelled.id, first.id);
    let second = next_run_event(&mut events, RunStatus::Done).await;
    assert_ne!(second.id, first.id);

    tokio::time::advance(Duration::from_secs(10 * 60)).await;
    settle().await;
    assert!(
        published(&log).iter().all(|p| !p.contains("OFF")),
        "{log:?}"
    );
}

/// The flow is armed while the entities have no value yet (the seed holds nothing); the
/// retained replay that follows must only be recorded.
#[tokio::test]
async fn boot_replay_starts_nothing() {
    let (hub, _publisher, _engine) = engine_hub();
    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    settle().await;
    let mut events = hub.subscribe();

    hub.ingest(LED_STATE_TOPIC, br#"{"state":"OFF"}"#);
    motion(&hub, true);
    no_run_event(&mut events).await;
}

#[tokio::test]
async fn boot_replay_after_reconnect_starts_nothing() {
    let (hub, _publisher, _engine) = engine_hub();
    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    settle().await;
    let mut events = hub.subscribe();

    hub.set_connection(ConnectionStatus::Offline {
        reason: "test".to_owned(),
    });
    hub.set_connection(ConnectionStatus::Online);
    hub.ingest(LED_STATE_TOPIC, br#"{"state":"OFF"}"#);
    motion(&hub, true);
    no_run_event(&mut events).await;
}

/// A flow edited between the reconnect and the replay must not re-seed the entities the
/// disconnect made unknown from the registry's stale values.
#[tokio::test]
async fn reload_after_reconnect_does_not_reseed() {
    let (hub, publisher, _engine) = engine_hub();
    let _log = echo(publisher, Arc::clone(&hub));
    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    settle().await;
    let mut events = hub.subscribe();

    motion(&hub, false);
    hub.set_connection(ConnectionStatus::Offline {
        reason: "test".to_owned(),
    });
    hub.set_connection(ConnectionStatus::Online);
    settle().await;
    let mut flow = reference_flow();
    flow.name = "edited".to_owned();
    hub.ingest(FLOW_TOPIC, flow.encode().as_bytes());
    settle().await;
    motion(&hub, true);
    no_run_event(&mut events).await;

    motion(&hub, false);
    next_run_event(&mut events, RunStatus::Running).await;
}

#[tokio::test]
async fn reconnect_replay_is_silent() {
    let (hub, publisher, _engine) = engine_hub();
    let _log = echo(publisher, Arc::clone(&hub));
    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    settle().await;
    let mut events = hub.subscribe();

    motion(&hub, false);
    hub.set_connection(ConnectionStatus::Offline {
        reason: "test".to_owned(),
    });
    hub.set_connection(ConnectionStatus::Online);
    motion(&hub, true);
    no_run_event(&mut events).await;

    motion(&hub, false);
    next_run_event(&mut events, RunStatus::Running).await;
}

/// The retained replay after a reconnect is usually byte-identical to the last state, so the
/// registry emits no change for it; the engine must still be armed for the next real change.
#[tokio::test]
async fn identical_replay_then_real_motion_fires() {
    let (hub, publisher, _engine) = engine_hub();
    let _log = echo(publisher, Arc::clone(&hub));
    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    settle().await;
    let mut events = hub.subscribe();

    motion(&hub, false);
    hub.set_connection(ConnectionStatus::Offline {
        reason: "test".to_owned(),
    });
    hub.set_connection(ConnectionStatus::Online);
    motion(&hub, false);
    no_run_event(&mut events).await;

    motion(&hub, true);
    let run = next_run_event(&mut events, RunStatus::Done).await;
    assert_eq!(step_nodes(&run), ["motion", "moving", "on"]);
    assert_eq!(run.steps[2].detail, "accepted");
}

/// The seed path itself: the entity has no value when the flow is armed, its first report is
/// recorded silently and the next change fires.
#[tokio::test]
async fn lagged_then_identical_report_reseeds() {
    let (hub, publisher, _engine) = engine_hub();
    let _log = echo(publisher, Arc::clone(&hub));
    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    settle().await;
    let mut events = hub.subscribe();

    motion(&hub, false);
    no_run_event(&mut events).await;

    motion(&hub, true);
    next_run_event(&mut events, RunStatus::Done).await;
}

#[tokio::test]
async fn availability_only_change_does_not_fire() {
    let (hub, _publisher, _engine) = engine_hub();
    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    settle().await;
    let mut events = hub.subscribe();

    motion(&hub, false);
    hub.ingest("zigbee2mqtt/bridge/state", br#"{"state":"offline"}"#);
    no_run_event(&mut events).await;
}

#[tokio::test]
async fn single_mode_records_a_skipped_run() {
    let (hub, publisher, _engine) = engine_hub();
    let _log = echo(publisher, Arc::clone(&hub));
    hub.ingest(
        FLOW_TOPIC,
        tweaked(|v| v["mode"] = "single".into()).as_bytes(),
    );
    settle().await;
    let mut events = hub.subscribe();

    motion(&hub, false);
    motion(&hub, true);
    next_run_event(&mut events, RunStatus::Done).await;
    motion(&hub, false);
    next_run(&mut events, |r| r.steps.len() == 2).await;
    motion(&hub, true);

    let skipped = next_run_event(&mut events, RunStatus::Skipped).await;
    assert_eq!(skipped.steps[0].detail, "a run is active");
}

/// Starts the reference flow waiting in its delay; returns the event stream and the log.
async fn waiting_flow(
    hub: &Arc<Hub>,
    publisher: mpsc::Receiver<Publish>,
) -> (
    tokio::sync::broadcast::Receiver<WsEvent>,
    Arc<Mutex<Vec<String>>>,
    RunView,
) {
    let log = echo(publisher, Arc::clone(hub));
    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    settle().await;
    let mut events = hub.subscribe();
    motion(hub, true);
    motion(hub, false);
    let run = next_run(&mut events, |r| r.steps.len() == 2).await;
    settle().await;
    (events, log, run)
}

async fn assert_cancelled_without_off(
    events: &mut tokio::sync::broadcast::Receiver<WsEvent>,
    log: &Arc<Mutex<Vec<String>>>,
    run: &RunView,
) {
    let cancelled = next_run_event(events, RunStatus::Cancelled).await;
    assert_eq!(cancelled.id, run.id);
    tokio::time::advance(Duration::from_secs(10 * 60)).await;
    settle().await;
    assert!(published(log).iter().all(|p| !p.contains("OFF")), "{log:?}");
}

#[tokio::test(start_paused = true)]
async fn disabling_cancels_the_run() {
    let (hub, publisher, _engine) = engine_hub();
    let (mut events, log, run) = waiting_flow(&hub, publisher).await;
    hub.set_flow_enabled("living", false).await.expect("toggle");
    assert_cancelled_without_off(&mut events, &log, &run).await;
}

#[tokio::test(start_paused = true)]
async fn deleting_cancels_the_run() {
    let (hub, publisher, _engine) = engine_hub();
    let (_events, log, _run) = waiting_flow(&hub, publisher).await;
    hub.delete_flow("living").await.expect("delete");
    // The store drops a deleted flow's runs, so there is no `Cancelled` to observe; what
    // matters is that the pending OFF never goes out.
    settle().await;
    tokio::time::advance(Duration::from_secs(10 * 60)).await;
    settle().await;
    assert!(
        published(&log).iter().all(|p| !p.contains("OFF")),
        "{log:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn replacing_cancels_the_run() {
    let (hub, publisher, _engine) = engine_hub();
    let (mut events, log, run) = waiting_flow(&hub, publisher).await;
    let mut flow = reference_flow();
    for node in &mut flow.nodes {
        if let NodeKind::Delay(params) = &mut node.kind {
            params.for_ = "4m".to_owned();
        }
    }
    hub.set_flow(flow).await.expect("set_flow");
    assert_cancelled_without_off(&mut events, &log, &run).await;
}

/// The run waiting in its delay is still running after an edit that does not change what the
/// flow does, then finishes and turns the LED off.
async fn assert_run_survives(
    hub: &Hub,
    events: &mut tokio::sync::broadcast::Receiver<WsEvent>,
    log: &Arc<Mutex<Vec<String>>>,
    run: &RunView,
) {
    settle().await;
    while let Ok(event) = events.try_recv() {
        if let WsEvent::FlowRunChanged { run: seen, .. } = event {
            assert_ne!(seen.status, RunStatus::Cancelled, "{seen:?}");
        }
    }
    let view = hub.flow("living").expect("flow");
    assert_eq!(view.active_run.as_deref(), Some(run.id.as_str()));
    let current = view.runs.iter().find(|r| r.id == run.id).expect("the run");
    assert_eq!(current.status, RunStatus::Running);

    tokio::time::advance(Duration::from_secs(5 * 60 + 1)).await;
    let done = next_run_event(events, RunStatus::Done).await;
    assert_eq!(done.id, run.id);
    assert_eq!(step_nodes(&done), ["motion", "moving", "wait", "off"]);
    let log = published(log);
    assert!(log.last().is_some_and(|p| p.contains("OFF")), "{log:?}");
}

#[tokio::test(start_paused = true)]
async fn moving_to_another_room_keeps_the_run() {
    let (hub, publisher, _engine) = engine_hub();
    with_living_area(&hub).await;
    let (mut events, log, run) = waiting_flow(&hub, publisher).await;
    hub.set_flow_area("living", Some("living".into()))
        .await
        .expect("move");
    assert_run_survives(&hub, &mut events, &log, &run).await;
}

#[tokio::test(start_paused = true)]
async fn renaming_keeps_the_run() {
    let (hub, publisher, _engine) = engine_hub();
    let (mut events, log, run) = waiting_flow(&hub, publisher).await;
    let mut flow = reference_flow();
    flow.name = "Living LED, renamed".to_owned();
    flow.description = Some("now with a description".to_owned());
    hub.set_flow(flow).await.expect("set_flow");
    assert_run_survives(&hub, &mut events, &log, &run).await;
}

#[tokio::test]
async fn gt_on_a_bool_fails_the_run() {
    let (hub, publisher, _engine) = engine_hub();
    let _log = echo(publisher, Arc::clone(&hub));
    let flow = tweaked(|v| v["nodes"][1]["params"] = serde_json::json!({"op": "gt", "value": 1}));
    hub.ingest(FLOW_TOPIC, flow.as_bytes());
    settle().await;
    let mut events = hub.subscribe();

    motion(&hub, false);
    motion(&hub, true);
    let run = next_run_event(&mut events, RunStatus::Failed).await;
    let last = run.steps.last().expect("a step");
    assert_eq!(last.node, "moving");
    assert_eq!(last.outcome, StepOutcome::Failed);
    assert_eq!(last.detail, "both sides must be numbers");
}

#[tokio::test]
async fn string_to_filter_never_fires() {
    let (hub, _publisher, _engine) = engine_hub();
    let flow = tweaked(|v| v["nodes"][0]["params"]["to"] = "true".into());
    hub.ingest(FLOW_TOPIC, flow.as_bytes());
    settle().await;
    let mut events = hub.subscribe();

    motion(&hub, false);
    motion(&hub, true);
    no_run_event(&mut events).await;
}

#[tokio::test]
async fn command_failure_ends_the_run_failed() {
    let (hub, _publisher, _engine) = engine_hub();
    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    settle().await;
    let mut events = hub.subscribe();

    motion(&hub, false);
    motion(&hub, true);
    let run = next_run_event(&mut events, RunStatus::Failed).await;
    let last = run.steps.last().expect("a step");
    assert_eq!(last.node, "on");
    assert_eq!(last.outcome, StepOutcome::Failed);
    assert_eq!(last.detail, "no state change within 50 ms");
}

#[tokio::test]
async fn run_flow_executes_without_trigger_data() {
    let (hub, publisher, _engine) = engine_hub();
    let _log = echo(publisher, Arc::clone(&hub));
    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    settle().await;
    let mut events = hub.subscribe();

    hub.run_flow("living").expect("run");
    let run = next_run_event(&mut events, RunStatus::Failed).await;
    assert!(run.manual);
    assert_eq!(run.steps[0].detail, "manual");
    assert_eq!(run.steps[1].detail, "no triggering entity");

    motion(&hub, true);
    let flow = tweaked(|v| v["nodes"][1]["params"]["entity"] = MOTION_ID.into());
    hub.ingest(FLOW_TOPIC, flow.as_bytes());
    settle().await;
    hub.run_flow("living").expect("run");
    let run = next_run_event(&mut events, RunStatus::Done).await;
    assert!(run.trigger.is_none());
    assert_eq!(step_nodes(&run), ["motion", "moving", "on"]);
}

#[tokio::test]
async fn missing_entity_at_run_time_fails_the_step() {
    let (hub, _publisher, _engine) = engine_hub();
    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    settle().await;
    hub.ingest("nidoo/light/0x60a423fffe94d1c6/light/config", b"");
    let mut events = hub.subscribe();

    motion(&hub, false);
    motion(&hub, true);
    let run = next_run_event(&mut events, RunStatus::Failed).await;
    let last = run.steps.last().expect("a step");
    assert_eq!(last.node, "on");
    assert!(last.detail.contains("unknown entity"), "{}", last.detail);
}

#[tokio::test(start_paused = true)]
async fn a_deleted_flow_never_publishes_after_its_delay() {
    let (hub, mut publisher, _engine) = engine_hub();
    hub.ingest(FLOW_TOPIC, reference_json().as_bytes());
    settle().await;
    let mut events = hub.subscribe();
    motion(&hub, true);
    motion(&hub, false);
    next_run(&mut events, |r| r.steps.len() == 2).await;
    settle().await;

    // No settle between the delete and the clock: the delay may wake before the engine has
    // processed the reload that aborts it.
    hub.delete_flow("living").await.expect("delete");
    tokio::time::advance(Duration::from_secs(10 * 60)).await;

    while let Ok(publish) = publisher.try_recv() {
        assert_ne!(publish.topic, LED_COMMAND_TOPIC, "{publish:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn if_without_entity_reads_the_registry() {
    let (hub, publisher, _engine) = engine_hub();
    let _log = echo(publisher, Arc::clone(&hub));
    let flow = tweaked(|v| {
        v["mode"] = "single".into();
        v["nodes"]
            .as_array_mut()
            .expect("nodes")
            .push(serde_json::json!(
                { "id": "pause", "type": "delay", "params": { "for": "1s" } }
            ));
        v["edges"][0] = serde_json::json!({ "from": "motion", "to": "pause" });
        v["edges"]
            .as_array_mut()
            .expect("edges")
            .push(serde_json::json!(
                { "from": "pause", "to": "moving" }
            ));
    });
    hub.ingest(FLOW_TOPIC, flow.as_bytes());
    settle().await;
    let mut events = hub.subscribe();

    motion(&hub, false);
    motion(&hub, true);
    next_run(&mut events, |r| r.steps.len() == 1).await;
    settle().await;
    motion(&hub, false);
    tokio::time::advance(Duration::from_secs(2)).await;

    let run = next_run(&mut events, |r| {
        r.status == RunStatus::Running && step_nodes(r).contains(&"moving")
    })
    .await;
    assert_eq!(run.trigger.expect("trigger").value, serde_json::json!(true));
    assert_eq!(run.steps.last().expect("step").detail, "false");
}

/// A flow that sends `turn_on` to the LED on a cron schedule.
fn schedule_json(enabled: bool) -> String {
    serde_json::json!({
        "id": "tick", "name": "Every ten minutes", "enabled": enabled, "mode": "single",
        "nodes": [
            { "id": "clock", "type": "trigger.schedule", "params": { "cron": "*/10 * * * *" } },
            { "id": "on", "type": "command",
              "params": { "entity": LED_ID, "command": { "type": "turn_on" } } }
        ],
        "edges": [ { "from": "clock", "to": "on" } ]
    })
    .to_string()
}

/// Seconds from now to the next `*/10` tick, as the engine sees it.
fn until_next_ten_minutes() -> Duration {
    let now = chrono::Utc::now().timestamp();
    Duration::from_secs((600 - now.rem_euclid(600)) as u64)
}

#[tokio::test(start_paused = true)]
async fn schedule_fires_each_tick_while_enabled() {
    let (hub, publisher, _engine) = engine_hub();
    let log = echo(publisher, Arc::clone(&hub));
    hub.ingest("nidoo/automation/tick", schedule_json(true).as_bytes());
    settle().await;
    let mut events = hub.subscribe();

    tokio::time::advance(until_next_ten_minutes() + Duration::from_secs(1)).await;
    let run = next_run_event(&mut events, RunStatus::Done).await;
    assert_eq!(step_nodes(&run), ["clock", "on"]);
    assert_eq!(run.steps[0].detail, "schedule */10 * * * *");
    assert!(!run.manual, "a scheduled run is not manual");
    assert!(run.trigger.is_none());
    assert_eq!(published(&log).len(), 1);

    tokio::time::advance(Duration::from_secs(10 * 60)).await;
    next_run_event(&mut events, RunStatus::Done).await;
    assert_eq!(published(&log).len(), 2);
}

#[tokio::test(start_paused = true)]
async fn schedule_never_fires_on_enable() {
    let (hub, publisher, _engine) = engine_hub();
    let log = echo(publisher, Arc::clone(&hub));
    hub.ingest("nidoo/automation/tick", schedule_json(true).as_bytes());
    settle().await;
    assert!(published(&log).is_empty());
    tokio::time::advance(Duration::from_secs(1)).await;
    settle().await;
    assert!(published(&log).is_empty());
}

#[tokio::test(start_paused = true)]
async fn disabled_schedule_never_fires() {
    let (hub, publisher, _engine) = engine_hub();
    let log = echo(publisher, Arc::clone(&hub));
    hub.ingest("nidoo/automation/tick", schedule_json(true).as_bytes());
    settle().await;
    hub.set_flow_enabled("tick", false).await.expect("disable");
    settle().await;
    tokio::time::advance(Duration::from_secs(30 * 60)).await;
    settle().await;
    assert!(published(&log).is_empty());
}

#[tokio::test(start_paused = true)]
async fn tick_while_offline_is_skipped() {
    let (hub, publisher, _engine) = engine_hub();
    let log = echo(publisher, Arc::clone(&hub));
    hub.ingest("nidoo/automation/tick", schedule_json(true).as_bytes());
    settle().await;
    let mut events = hub.subscribe();

    hub.set_connection(ConnectionStatus::Offline {
        reason: "test".to_owned(),
    });
    settle().await;
    tokio::time::advance(until_next_ten_minutes() + Duration::from_secs(1)).await;
    let skipped = next_run_event(&mut events, RunStatus::Skipped).await;
    assert_eq!(skipped.steps[0].node, "clock");
    assert!(
        skipped.steps[0].detail.contains("offline"),
        "{:?}",
        skipped.steps[0]
    );
    assert!(published(&log).is_empty());

    hub.set_connection(ConnectionStatus::Online);
    settle().await;
    tokio::time::advance(Duration::from_secs(10 * 60)).await;
    next_run_event(&mut events, RunStatus::Done).await;
    assert_eq!(published(&log).len(), 1);
}

/// A manual-run flow `id`: a state trigger, then `nodes` chained in order (no ports).
fn chain_json(id: &str, enabled: bool, nodes: Vec<serde_json::Value>) -> String {
    let mut all = vec![serde_json::json!(
        { "id": "t", "type": "trigger.state", "params": { "entity": MOTION_ID } }
    )];
    all.extend(nodes);
    let edges: Vec<_> = all
        .windows(2)
        .map(|pair| serde_json::json!({ "from": pair[0]["id"], "to": pair[1]["id"] }))
        .collect();
    serde_json::json!({
        "id": id, "name": id, "enabled": enabled, "mode": "single",
        "nodes": all, "edges": edges
    })
    .to_string()
}

fn turn(id: &str, command: &str) -> serde_json::Value {
    serde_json::json!({ "id": id, "type": "command",
        "params": { "entity": LED_ID, "command": { "type": command } } })
}

#[tokio::test]
async fn flow_enable_enables_another_flow() {
    let (hub, _publisher, _engine) = engine_hub();
    let flow_enable = serde_json::json!({ "id": "e", "type": "flow.enable",
        "params": { "flow": "b", "enabled": true } });
    hub.ingest(
        "nidoo/automation/a",
        chain_json("a", true, vec![flow_enable]).as_bytes(),
    );
    hub.ingest(
        "nidoo/automation/b",
        chain_json("b", false, vec![turn("on", "turn_on")]).as_bytes(),
    );
    settle().await;
    let mut events = hub.subscribe();
    hub.run_flow("a").expect("run");
    let run = next_run_event(&mut events, RunStatus::Done).await;
    assert_eq!(run.steps.last().expect("step").detail, "enabled b");
    assert!(hub.flow("b").expect("b").definition.enabled);
}

#[tokio::test]
async fn self_disable_keeps_the_running_run() {
    let (hub, publisher, _engine) = engine_hub();
    let log = echo(publisher, Arc::clone(&hub));
    let flow_enable = serde_json::json!({ "id": "e", "type": "flow.enable",
        "params": { "enabled": false } });
    hub.ingest(
        "nidoo/automation/a",
        chain_json("a", true, vec![flow_enable, turn("on", "turn_on")]).as_bytes(),
    );
    settle().await;
    let mut events = hub.subscribe();
    hub.run_flow("a").expect("run");
    let run = next_run_event(&mut events, RunStatus::Done).await;
    assert_eq!(step_nodes(&run), ["t", "e", "on"]);
    assert_eq!(run.steps[1].detail, "disabled a");
    assert!(published(&log).iter().any(|p| p.contains("ON")), "{log:?}");
    assert!(!hub.flow("a").expect("a").definition.enabled);
}

fn missing_flow(missing: Option<&str>) -> String {
    let mut params = serde_json::json!({ "op": "eq", "value": "ON", "entity": LED_ID });
    if let Some(missing) = missing {
        params["missing"] = missing.into();
    }
    let check = serde_json::json!({ "id": "chk", "type": "if", "params": params });
    let mut value: serde_json::Value =
        serde_json::from_str(&chain_json("a", true, vec![check, turn("on", "turn_on")]))
            .expect("json");
    value["nodes"]
        .as_array_mut()
        .expect("nodes")
        .push(turn("off", "turn_off"));
    value["edges"] = serde_json::json!([
        { "from": "t", "to": "chk" },
        { "from": "chk", "port": "true", "to": "on" },
        { "from": "chk", "port": "false", "to": "off" }
    ]);
    value.to_string()
}

#[tokio::test]
async fn if_missing_false_takes_false() {
    let (hub, publisher, _engine) = engine_hub();
    let log = echo(publisher, Arc::clone(&hub));
    hub.ingest("nidoo/automation/a", missing_flow(Some("false")).as_bytes());
    settle().await;
    let mut events = hub.subscribe();
    hub.run_flow("a").expect("run");
    let run = next_run_event(&mut events, RunStatus::Done).await;
    assert_eq!(run.steps[1].detail, "missing → false");
    let sent = published(&log);
    assert!(sent.len() == 1 && sent[0].contains("OFF"), "{sent:?}");
}

#[tokio::test]
async fn if_missing_default_still_fails() {
    let (hub, publisher, _engine) = engine_hub();
    let log = echo(publisher, Arc::clone(&hub));
    hub.ingest("nidoo/automation/a", missing_flow(None).as_bytes());
    settle().await;
    let mut events = hub.subscribe();
    hub.run_flow("a").expect("run");
    // Without `missing` the step behaves exactly as before: `compare` of a missing value is
    // plain false, with no "missing →" note.
    let run = next_run_event(&mut events, RunStatus::Done).await;
    assert_eq!(run.steps[1].detail, "false");
    let sent = published(&log);
    assert!(sent.len() == 1 && sent[0].contains("OFF"), "{sent:?}");
}

#[tokio::test]
async fn set_flow_rejects_unknown_target_flow() {
    let (hub, _publisher) = hub_with_entities();
    let flow_enable = serde_json::json!({ "id": "e", "type": "flow.enable",
        "params": { "flow": "nope", "enabled": true } });
    let flow = Flow::decode(chain_json("a", true, vec![flow_enable]).as_bytes()).expect("flow");
    match hub.set_flow(flow).await {
        Err(HubError::InvalidFlow(reason)) => {
            assert!(reason.contains("unknown flow `nope`"), "{reason}");
        }
        other => panic!("expected InvalidFlow, got {other:?}"),
    }
    // Targeting itself or an existing flow validates.
    let own = serde_json::json!({ "id": "e", "type": "flow.enable",
        "params": { "flow": "a", "enabled": true } });
    let flow = Flow::decode(chain_json("a", true, vec![own]).as_bytes()).expect("flow");
    hub.set_flow(flow).await.expect("self target validates");
}

const BUTTON_ID: &str = "virtual_boost";
const BUTTON_TOPIC: &str = "nidoo/virtual/boost";

fn button_flow(trigger_params: serde_json::Value) -> Flow {
    Flow::decode(
        serde_json::json!({
            "id": "boost", "name": "Boost", "enabled": true, "mode": "single",
            "nodes": [
                { "id": "press", "type": "trigger.state", "params": trigger_params },
                { "id": "on", "type": "command",
                  "params": { "entity": LED_ID, "command": { "type": "turn_on" } } }
            ],
            "edges": [{ "from": "press", "to": "on" }]
        })
        .to_string()
        .as_bytes(),
    )
    .expect("button flow")
}

async fn hub_with_button() -> (
    Arc<Hub>,
    mpsc::Receiver<Publish>,
    tokio::task::JoinHandle<()>,
) {
    let (hub, publisher, engine) = engine_hub();
    let spec = nidoo_api::VirtualSpec {
        kind: nidoo_api::VirtualKind::Button,
        name: "Boost".to_owned(),
        min: None,
        max: None,
        step: None,
        unit: None,
        options: None,
        initial: None,
    };
    hub.set_virtual("boost", spec).await.expect("button");
    (hub, publisher, engine)
}

#[tokio::test]
async fn button_press_fires_its_flow() {
    let (hub, publisher, _engine) = hub_with_button().await;
    let log = echo(publisher, Arc::clone(&hub));
    hub.set_flow(button_flow(serde_json::json!({ "entity": BUTTON_ID })))
        .await
        .expect("set_flow");
    settle().await;
    assert!(hub.subscription_topics().contains(BUTTON_TOPIC));
    let mut events = hub.subscribe();

    hub.ingest_message(BUTTON_TOPIC, b"PRESS", false);
    let run = next_run_event(&mut events, RunStatus::Done).await;

    assert_eq!(step_nodes(&run), ["press", "on"]);
    assert_eq!(published(&log).len(), 1, "{log:?}");
}

#[tokio::test]
async fn retained_button_message_never_fires() {
    let (hub, publisher, _engine) = hub_with_button().await;
    let log = echo(publisher, Arc::clone(&hub));
    hub.set_flow(button_flow(serde_json::json!({ "entity": BUTTON_ID })))
        .await
        .expect("set_flow");
    settle().await;
    let mut events = hub.subscribe();

    hub.ingest_message(BUTTON_TOPIC, b"PRESS", true);
    settle().await;

    assert!(published(&log).is_empty(), "{log:?}");
    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(
                event,
                WsEvent::ButtonPressed { .. } | WsEvent::FlowRunChanged { .. }
            ),
            "{event:?}"
        );
    }
}

#[tokio::test]
async fn button_trigger_refuses_to_filter() {
    let (hub, _publisher, _engine) = hub_with_button().await;
    for params in [
        serde_json::json!({ "entity": BUTTON_ID, "to": "PRESS" }),
        serde_json::json!({ "entity": BUTTON_ID, "from": "PRESS" }),
    ] {
        match hub.set_flow(button_flow(params)).await {
            Err(HubError::InvalidFlow(reason)) => {
                assert!(
                    reason.contains("a button trigger takes no to/from"),
                    "{reason}"
                );
            }
            other => panic!("expected InvalidFlow, got {other:?}"),
        }
    }
}

/// Whether the MQTT task's resubscribe wake-up fired (consuming it).
async fn woken(wakeup: &tokio::sync::Notify) -> bool {
    tokio::time::timeout(Duration::from_millis(50), wakeup.notified())
        .await
        .is_ok()
}

#[tokio::test]
async fn local_flow_writes_wake_the_resubscribe() {
    let (hub, _publisher, _engine) = hub_with_button().await;
    let wakeup = hub.subscriptions_wakeup();
    assert!(!woken(&wakeup).await, "nothing watches the button yet");

    let flow = || button_flow(serde_json::json!({ "entity": BUTTON_ID }));
    hub.set_flow(flow()).await.expect("set_flow");
    assert!(woken(&wakeup).await, "a new watched button must wake it");

    // The same flow again, and the broker's echo of it, change nothing.
    hub.set_flow(flow()).await.expect("set_flow again");
    hub.ingest("nidoo/automation/boost", flow().encode().as_bytes());
    assert!(!woken(&wakeup).await, "an unchanged set must not wake it");

    hub.delete_flow("boost").await.expect("delete");
    assert!(woken(&wakeup).await, "a dropped watch must wake it");
    assert!(!hub.subscription_topics().contains(BUTTON_TOPIC));
}

const REMOTE_ID: &str = "device_automation.0xd4fe12345";
const REMOTE_TOPIC: &str = "zigbee2mqtt/rooms/hall/remote/action";

/// Teaches the hub a remote with the `on` and `off` actions.
fn add_remote(hub: &Hub) {
    for payload in ["on", "off"] {
        let config = serde_json::json!({
            "automation_type": "trigger",
            "topic": REMOTE_TOPIC,
            "payload": payload,
            "type": "action",
            "subtype": payload,
            "device": {"identifiers": ["zigbee2mqtt_0xd4fe12345"], "name": "rooms/hall/remote"}
        });
        hub.ingest(
            &format!("nidoo/device_automation/0xd4fe12345/action_{payload}/config"),
            config.to_string().as_bytes(),
        );
    }
}

fn signal_flow(trigger_params: serde_json::Value) -> Flow {
    Flow::decode(
        serde_json::json!({
            "id": "remote", "name": "Remote", "enabled": true, "mode": "single",
            "nodes": [
                { "id": "press", "type": "trigger.signal", "params": trigger_params },
                { "id": "on", "type": "command",
                  "params": { "entity": LED_ID, "command": { "type": "turn_on" } } }
            ],
            "edges": [{ "from": "press", "to": "on" }]
        })
        .to_string()
        .as_bytes(),
    )
    .expect("signal flow")
}

#[tokio::test]
async fn signal_fires_matching_trigger_only() {
    let (hub, publisher, _engine) = engine_hub();
    add_remote(&hub);
    let log = echo(publisher, Arc::clone(&hub));
    hub.set_flow(signal_flow(
        serde_json::json!({ "entity": REMOTE_ID, "signal": "on" }),
    ))
    .await
    .expect("set_flow");
    settle().await;
    assert!(hub.subscription_topics().contains(REMOTE_TOPIC));
    let mut events = hub.subscribe();

    hub.ingest_message(REMOTE_TOPIC, b"off", false);
    no_run_event(&mut events).await;
    assert!(published(&log).is_empty(), "{log:?}");

    hub.ingest_message(REMOTE_TOPIC, b"on", false);
    let run = next_run_event(&mut events, RunStatus::Done).await;
    assert_eq!(step_nodes(&run), ["press", "on"]);
    assert_eq!(published(&log).len(), 1, "{log:?}");
}

#[tokio::test]
async fn retained_signal_never_fires_a_flow() {
    let (hub, publisher, _engine) = engine_hub();
    add_remote(&hub);
    let log = echo(publisher, Arc::clone(&hub));
    hub.set_flow(signal_flow(serde_json::json!({ "entity": REMOTE_ID })))
        .await
        .expect("set_flow");
    settle().await;
    let mut events = hub.subscribe();

    hub.ingest_message(REMOTE_TOPIC, b"on", true);
    settle().await;

    assert!(published(&log).is_empty(), "{log:?}");
    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(
                event,
                WsEvent::SignalReceived { .. } | WsEvent::FlowRunChanged { .. }
            ),
            "{event:?}"
        );
    }
}

#[tokio::test]
async fn signal_trigger_rejects_unannounced_signal() {
    let (hub, _publisher, _engine) = engine_hub();
    add_remote(&hub);
    match hub
        .set_flow(signal_flow(
            serde_json::json!({ "entity": REMOTE_ID, "signal": "warp" }),
        ))
        .await
    {
        Err(HubError::InvalidFlow(reason)) => assert!(reason.contains("warp"), "{reason}"),
        other => panic!("expected InvalidFlow, got {other:?}"),
    }
    // A card entity is no signal source.
    match hub
        .set_flow(signal_flow(serde_json::json!({ "entity": LED_ID })))
        .await
    {
        Err(HubError::InvalidFlow(_)) => {}
        other => panic!("expected InvalidFlow, got {other:?}"),
    }
    hub.set_flow(signal_flow(
        serde_json::json!({ "entity": REMOTE_ID, "signal": "off" }),
    ))
    .await
    .expect("announced signal validates");
}

#[tokio::test]
async fn state_trigger_on_a_remote_is_rejected() {
    let (hub, _publisher, _engine) = engine_hub();
    add_remote(&hub);
    let flow = button_flow(serde_json::json!({ "entity": REMOTE_ID }));
    match hub.set_flow(flow).await {
        Err(HubError::InvalidFlow(reason)) => {
            assert!(reason.contains("trigger.signal"), "{reason}");
        }
        other => panic!("expected InvalidFlow, got {other:?}"),
    }
}

fn silent_flow(entity: &str) -> Flow {
    Flow::decode(
        serde_json::json!({
            "id": "alive", "name": "Alive", "enabled": true, "mode": "single",
            "nodes": [
                { "id": "quiet", "type": "trigger.silent", "params": { "entity": entity, "for": "50m" } },
                { "id": "on", "type": "command",
                  "params": { "entity": LED_ID, "command": { "type": "turn_on" } } }
            ],
            "edges": [{ "from": "quiet", "to": "on" }]
        })
        .to_string()
        .as_bytes(),
    )
    .expect("silent flow")
}

#[tokio::test]
async fn silent_trigger_on_an_entity_that_never_reports_is_rejected() {
    let (hub, _publisher, _engine) = hub_with_button().await;
    add_remote(&hub);
    for entity in [REMOTE_ID, BUTTON_ID] {
        match hub.set_flow(silent_flow(entity)).await {
            Err(HubError::InvalidFlow(reason)) => {
                assert!(reason.contains("never reports"), "{entity}: {reason}");
            }
            other => panic!("expected InvalidFlow for {entity}, got {other:?}"),
        }
    }
    hub.set_flow(silent_flow(LED_ID))
        .await
        .expect("a light reports state");
}

#[tokio::test]
async fn repeated_signal_records_last_and_at_every_time() {
    let (hub, _publisher) = hub_with_entities();
    add_remote(&hub);
    let mut events = hub.subscribe();

    hub.ingest_message(REMOTE_TOPIC, b"on", false);
    hub.ingest_message(REMOTE_TOPIC, b"on", false);

    let (mut changed, mut signals) = (0, 0);
    while let Ok(event) = events.try_recv() {
        match event {
            WsEvent::StateChanged { id, .. } if id == REMOTE_ID => changed += 1,
            WsEvent::SignalReceived { .. } => signals += 1,
            _ => {}
        }
    }
    assert_eq!((changed, signals), (2, 2));
    let state = remote_state(&hub);
    let nidoo_core::model::StateValue::Generic(generic) = state.value else {
        panic!("not generic: {state:?}");
    };
    assert_eq!(
        generic.fields["last"],
        nidoo_core::model::GenericValue::Text("on".to_owned())
    );
    assert!(matches!(
        generic.fields["at"],
        nidoo_core::model::GenericValue::Number(_)
    ));

    // A retained replay neither signals nor touches the state.
    let before = generic.fields["at"].clone();
    hub.ingest_message(REMOTE_TOPIC, b"on", true);
    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(event, WsEvent::SignalReceived { .. }),
            "{event:?}"
        );
    }
    let state = remote_state(&hub);
    let nidoo_core::model::StateValue::Generic(generic) = state.value else {
        panic!()
    };
    assert_eq!(generic.fields["at"], before);
}

fn remote_state(hub: &Hub) -> nidoo_core::model::EntityState {
    hub.snapshot()
        .entities
        .into_iter()
        .find(|e| e.id == REMOTE_ID)
        .map(|e| e.state)
        .expect("remote state")
}

async fn with_living_area(hub: &Hub) {
    hub.set_areas(AreasDocument {
        areas: vec![Area {
            id: "living".into(),
            name: "Living room".into(),
            ..Area::default()
        }],
    })
    .await
    .expect("set_areas");
}

fn in_area(area: &str) -> Flow {
    let mut flow = reference_flow();
    flow.area = Some(area.to_owned());
    flow
}

#[tokio::test]
async fn set_flow_keeps_known_area() {
    let (hub, mut publisher) = hub_with_entities();
    with_living_area(&hub).await;
    while publisher.try_recv().is_ok() {}
    let outcome = hub.set_flow(in_area("living")).await.expect("set_flow");
    assert_eq!(outcome.flow.definition.area.as_deref(), Some("living"));
    let publish = publisher.try_recv().expect("the definition");
    assert!(
        publish.payload.contains("\"area\":\"living\""),
        "{}",
        publish.payload
    );
}

#[tokio::test]
async fn area_name_resolves_to_id() {
    let (hub, _publisher) = hub_with_entities();
    with_living_area(&hub).await;
    let outcome = hub
        .set_flow(in_area("Living room"))
        .await
        .expect("set_flow");
    assert_eq!(outcome.flow.definition.area.as_deref(), Some("living"));
}

#[tokio::test]
async fn unknown_area_is_rejected() {
    let (hub, _publisher) = hub_with_entities();
    with_living_area(&hub).await;
    match hub.set_flow(in_area("ghost")).await {
        Err(HubError::InvalidFlow(reason)) => assert!(reason.contains("unknown area"), "{reason}"),
        other => panic!("expected InvalidFlow, got {other:?}"),
    }
}

#[tokio::test]
async fn unassigned_is_not_a_flow_area() {
    let (hub, _publisher) = hub_with_entities();
    with_living_area(&hub).await;
    assert!(matches!(
        hub.set_flow(in_area("Unassigned")).await,
        Err(HubError::InvalidFlow(_))
    ));
}

/// `kitchen` is declared but no entity lives there, so once the document drops it the room is
/// gone for good (`living` would stay an implicit area through the fixture's topics).
#[tokio::test]
async fn stale_area_is_kept_on_resave() {
    let (hub, _publisher) = hub_with_entities();
    hub.set_areas(AreasDocument {
        areas: vec![Area {
            id: "kitchen".into(),
            name: "Kitchen".into(),
            ..Area::default()
        }],
    })
    .await
    .expect("set_areas");
    hub.set_flow(in_area("kitchen")).await.expect("first save");
    hub.set_areas(AreasDocument::default())
        .await
        .expect("set_areas");
    assert!(
        hub.snapshot().areas.iter().all(|area| area.id != "kitchen"),
        "kitchen must be gone"
    );

    let outcome = hub
        .set_flow(in_area("kitchen"))
        .await
        .expect("resave keeps the stale area");
    assert_eq!(outcome.flow.definition.area.as_deref(), Some("kitchen"));

    let mut other = in_area("kitchen");
    other.id = "other".to_owned();
    match hub.set_flow(other).await {
        Err(HubError::InvalidFlow(reason)) => assert!(reason.contains("unknown area"), "{reason}"),
        other => panic!("expected InvalidFlow, got {other:?}"),
    }

    let view = hub.set_flow_enabled("living", false).await.expect("toggle");
    assert!(!view.definition.enabled);
    assert_eq!(view.definition.area.as_deref(), Some("kitchen"));
}

#[tokio::test]
async fn set_flow_area_moves_and_clears() {
    let (hub, mut publisher) = hub_with_entities();
    with_living_area(&hub).await;
    hub.set_flow(reference_flow()).await.expect("set_flow");
    while publisher.try_recv().is_ok() {}

    let view = hub
        .set_flow_area("living", Some("living".into()))
        .await
        .expect("move");
    assert_eq!(view.definition.area.as_deref(), Some("living"));
    publisher.try_recv().expect("a publish");

    let view = hub.set_flow_area("living", None).await.expect("clear");
    assert_eq!(view.definition.area, None);
    let publish = publisher.try_recv().expect("a publish");
    assert!(!publish.payload.contains("\"area\""), "{}", publish.payload);

    assert!(matches!(
        hub.set_flow_area("living", Some("ghost".into())).await,
        Err(HubError::InvalidFlow(_))
    ));
    assert!(matches!(
        hub.set_flow_area("nobody", None).await,
        Err(HubError::UnknownFlow(_))
    ));
    hub.ingest("nidoo/automation/broken", b"not json");
    assert!(matches!(
        hub.set_flow_area("broken", None).await,
        Err(HubError::InvalidFlow(_))
    ));
}

#[tokio::test]
async fn retained_unknown_area_loads_valid() {
    let (hub, _publisher) = hub_with_entities();
    let mut value: serde_json::Value = serde_json::from_str(&reference_json()).unwrap();
    value["area"] = "ghost".into();
    hub.ingest(FLOW_TOPIC, value.to_string().as_bytes());
    let view = hub.flow("living").expect("flow");
    assert!(view.invalid.is_none(), "{:?}", view.invalid);
}

/// Runs flow `id` (a lone notify node, `params`) once and returns the retained history list the
/// hub published afterwards.
async fn run_notify(
    hub: &Arc<Hub>,
    publisher: &mut mpsc::Receiver<Publish>,
    id: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let notify = serde_json::json!({ "id": "n", "type": "notify", "params": params });
    hub.ingest(
        &format!("nidoo/automation/{id}"),
        chain_json(id, true, vec![notify]).as_bytes(),
    );
    settle().await;
    let mut events = hub.subscribe();
    hub.run_flow(id).expect("run");
    let run = next_run_event(&mut events, RunStatus::Done).await;
    assert_eq!(run.steps.last().expect("step").outcome, StepOutcome::Ok);
    settle().await;
    let mut last = None;
    while let Ok(publish) = publisher.try_recv() {
        if publish.topic == "nidoo/push/history" {
            last = Some(publish);
        }
    }
    let last = last.expect("a history publish");
    serde_json::from_str(&last.payload).expect("history json")
}

/// Connects the hub and lets the push service finish its settle delay.
async fn push_ready(hub: &Hub) {
    hub.on_connected();
    hub.on_suback();
    tokio::time::sleep(Duration::from_secs(2)).await;
    tokio::task::yield_now().await;
}

#[tokio::test(start_paused = true)]
async fn notify_step_submits_v1_alert() {
    let (hub, mut publisher, _engine) = engine_hub();
    push_ready(&hub).await;
    let list = run_notify(
        &hub,
        &mut publisher,
        "door",
        serde_json::json!({ "title": "Door open", "channel": "security", "severity": "warning" }),
    )
    .await;
    let alert = &list[0];
    assert_eq!(alert["title"], "Door open", "{list}");
    assert_eq!(alert["source"], "flow", "{list}");
    assert_eq!(alert["channel"], "security");
    let id = alert["id"].as_str().expect("id");
    assert!(id.starts_with("flow-door-"), "{id}");
    let run_id = hub.flow("door").expect("flow").runs[0].id.clone();
    assert_eq!(id, format!("flow-door-{run_id}"));
}

const TEMP_ID: &str = "0x00124b0008fd6568_temperature_1_zigbee2mqtt";
const TEMP_CONFIG_TOPIC: &str = "nidoo/sensor/0x00124b0008fd6568/temperature_1/config";

/// Adds a temperature sensor (°C) reading 10.14 to the hub.
fn ingest_temperature(hub: &Hub) {
    hub.ingest(
        TEMP_CONFIG_TOPIC,
        &fixture("discovery/sensor__0x00124b0008fd6568__temperature_1.json"),
    );
    hub.ingest(
        "zigbee2mqtt/greenhouse/soil_sensors",
        br#"{"temperature_1": 10.14}"#,
    );
}

#[tokio::test(start_paused = true)]
async fn notify_renders_values_in_the_flow_language() {
    let (hub, mut publisher, _engine) = engine_hub();
    ingest_temperature(&hub);
    hub.ingest(MOTION_STATE_TOPIC, br#"{"occupancy": false}"#);
    push_ready(&hub).await;
    let list = run_notify(
        &hub,
        &mut publisher,
        "temp",
        serde_json::json!({
            "title": format!("Temp {{{TEMP_ID}:1}}"),
            "body": format!("{{{MOTION_ID}}} {{trigger}}"),
            "lang": "fr",
        }),
    )
    .await;
    assert_eq!(list[0]["title"], "Temp 10,1\u{a0}°C", "{list}");
    // A manual run has no trigger value.
    assert_eq!(list[0]["body"], "Rien ?", "{list}");
    let flow = hub.flow("temp").expect("flow");
    let step = &flow.runs[0].steps.last().expect("step").detail;
    assert!(step.ends_with(": Temp 10,1\u{a0}°C"), "{step}");
}

#[tokio::test]
async fn set_flow_rejects_unknown_entity_in_notify_text() {
    let (hub, mut publisher) = hub_with_entities();
    let notify = serde_json::json!(
        { "id": "n", "type": "notify", "params": { "title": "T {nope}" } }
    );
    let flow = Flow::decode(chain_json("temp", true, vec![notify]).as_bytes()).expect("flow");
    match hub.set_flow(flow).await {
        Err(HubError::InvalidFlow(reason)) => assert!(reason.contains("nope"), "{reason}"),
        other => panic!("expected InvalidFlow, got {other:?}"),
    }
    assert!(publisher.try_recv().is_err(), "nothing was published");
}

#[tokio::test(start_paused = true)]
async fn notify_with_a_vanished_entity_still_sends() {
    let (hub, mut publisher, _engine) = engine_hub();
    ingest_temperature(&hub);
    hub.ingest(TEMP_CONFIG_TOPIC, b"");
    push_ready(&hub).await;
    let list = run_notify(
        &hub,
        &mut publisher,
        "temp",
        serde_json::json!({ "title": format!("T {{{TEMP_ID}}}") }),
    )
    .await;
    assert_eq!(list[0]["title"], "T ?", "{list}");
}

#[tokio::test(start_paused = true)]
async fn notify_with_fixed_id_replaces_previous() {
    let (hub, mut publisher, _engine) = engine_hub();
    push_ready(&hub).await;
    let params = serde_json::json!({ "title": "Washer", "id": "washer" });
    run_notify(&hub, &mut publisher, "washer", params.clone()).await;
    let list = run_notify(&hub, &mut publisher, "washer", params).await;
    let same: Vec<_> = list
        .as_array()
        .expect("list")
        .iter()
        .filter(|a| a["id"] == "washer")
        .collect();
    assert_eq!(same.len(), 1, "{list}");
}

/// The reference flow plus a Manual card `test_on` (`params`) wired to `target`.
fn with_card(mode: &str, params: serde_json::Value, target: &str) -> String {
    tweaked(|v| {
        v["mode"] = mode.into();
        v["nodes"].as_array_mut().expect("nodes").push(
            serde_json::json!({ "id": "test_on", "type": "trigger.manual", "params": params }),
        );
        v["edges"]
            .as_array_mut()
            .expect("edges")
            .push(serde_json::json!({ "from": "test_on", "to": target }));
    })
}

#[tokio::test]
async fn card_press_runs_from_the_card_with_its_value() {
    let (hub, publisher, _engine) = engine_hub();
    let _log = echo(publisher, Arc::clone(&hub));
    hub.ingest(
        FLOW_TOPIC,
        with_card("restart", serde_json::json!({ "value": true }), "moving").as_bytes(),
    );
    settle().await;
    let mut events = hub.subscribe();

    hub.run_flow_from("living", Some("test_on")).expect("run");
    let run = next_run_event(&mut events, RunStatus::Done).await;
    assert!(run.manual);
    assert_eq!(step_nodes(&run), ["test_on", "moving", "on"]);
    let trigger = run.trigger.as_ref().expect("trigger");
    assert_eq!(trigger.value, serde_json::json!(true));
    assert_eq!(trigger.entity, None);
    assert_eq!(run.steps[0].detail, "manual: true");
}

#[tokio::test(start_paused = true)]
async fn run_now_carries_the_first_manual_card_value() {
    let (hub, publisher, _engine) = engine_hub();
    let _log = echo(publisher, Arc::clone(&hub));
    let flow = tweaked(|v| {
        v["nodes"] = serde_json::json!([
            { "id": "go", "type": "trigger.manual", "params": { "value": false } },
            { "id": "moving", "type": "if", "params": { "op": "eq", "value": true } },
            { "id": "on", "type": "command",
              "params": { "entity": LED_ID, "command": { "type": "turn_on" } } },
            { "id": "wait", "type": "delay", "params": { "for": "5m" } }
        ]);
        v["edges"] = serde_json::json!([
            { "from": "go", "to": "moving" },
            { "from": "moving", "port": "true", "to": "on" },
            { "from": "moving", "port": "false", "to": "wait" }
        ]);
    });
    hub.ingest(
        "nidoo/automation/x",
        flow.replace("\"living\"", "\"x\"").as_bytes(),
    );
    settle().await;
    let mut events = hub.subscribe();

    hub.run_flow("x").expect("run");
    next_run(&mut events, |r| r.steps.len() == 2).await;
    settle().await;
    tokio::time::advance(Duration::from_secs(5 * 60 + 1)).await;
    let run = next_run_event(&mut events, RunStatus::Done).await;
    assert_eq!(step_nodes(&run), ["go", "moving", "wait"]);
    assert_eq!(run.steps[0].detail, "manual: false");
}

#[tokio::test]
async fn valueless_card_step_detail_is_manual() {
    let (hub, publisher, _engine) = engine_hub();
    let _log = echo(publisher, Arc::clone(&hub));
    let flow = tweaked(|v| {
        v["nodes"] = serde_json::json!([
            { "id": "go", "type": "trigger.manual", "params": {} },
            { "id": "on", "type": "command",
              "params": { "entity": LED_ID, "command": { "type": "turn_on" } } }
        ]);
        v["edges"] = serde_json::json!([{ "from": "go", "to": "on" }]);
    });
    hub.ingest(FLOW_TOPIC, flow.as_bytes());
    settle().await;
    let mut events = hub.subscribe();

    hub.run_flow("living").expect("run");
    let run = next_run_event(&mut events, RunStatus::Done).await;
    assert_eq!(run.steps[0].detail, "manual");
    assert!(run.trigger.is_none());
}

#[tokio::test]
async fn run_flow_from_rejects_a_card_that_is_not_manual() {
    let (hub, _publisher, _engine) = engine_hub();
    hub.ingest(
        FLOW_TOPIC,
        with_card("restart", serde_json::json!({ "value": true }), "moving").as_bytes(),
    );
    settle().await;
    for node in ["motion", "ghost"] {
        assert_eq!(
            hub.run_flow_from("living", Some(node)),
            Err(HubError::Rejected(format!(
                "`{node}` is not a manual trigger of this flow"
            )))
        );
    }
}

#[tokio::test(start_paused = true)]
async fn card_press_restarts_an_active_restart_flow() {
    let (hub, publisher, _engine) = engine_hub();
    let _log = echo(publisher, Arc::clone(&hub));
    hub.ingest(
        FLOW_TOPIC,
        with_card("restart", serde_json::json!({ "value": false }), "moving").as_bytes(),
    );
    settle().await;
    let mut events = hub.subscribe();

    let first = hub.run_flow_from("living", Some("test_on")).expect("press");
    next_run(&mut events, |r| r.id == first.id && r.steps.len() == 2).await;
    settle().await;
    let second = hub.run_flow_from("living", Some("test_on")).expect("press");
    assert_ne!(first.id, second.id);
    let cancelled = next_run_event(&mut events, RunStatus::Cancelled).await;
    assert_eq!(cancelled.id, first.id);
    let parked = next_run(&mut events, |r| r.id == second.id && r.steps.len() == 2).await;
    assert_eq!(step_nodes(&parked), ["test_on", "moving"]);
    settle().await;

    let view = hub.flow("living").expect("flow");
    assert_eq!(view.active_run.as_deref(), Some(second.id.as_str()));
    let status = |id: &str| view.runs.iter().find(|r| r.id == id).expect("run").status;
    assert_eq!(status(&first.id), RunStatus::Cancelled);
    assert_eq!(status(&second.id), RunStatus::Running);
    assert_eq!(view.runs.len(), 2);

    // The aborted task finishing must not end the new run: it still completes on its own.
    tokio::time::advance(Duration::from_secs(5 * 60 + 1)).await;
    let done = next_run_event(&mut events, RunStatus::Done).await;
    assert_eq!(done.id, second.id);
}

#[tokio::test(start_paused = true)]
async fn card_press_refused_while_single_flow_is_active() {
    let (hub, publisher, _engine) = engine_hub();
    let _log = echo(publisher, Arc::clone(&hub));
    hub.ingest(
        FLOW_TOPIC,
        with_card("single", serde_json::json!({ "value": false }), "moving").as_bytes(),
    );
    settle().await;
    let mut events = hub.subscribe();

    let first = hub.run_flow_from("living", Some("test_on")).expect("press");
    next_run(&mut events, |r| r.steps.len() == 2).await;
    settle().await;
    assert_eq!(
        hub.run_flow_from("living", Some("test_on")),
        Err(HubError::Rejected("a run is active".to_owned()))
    );
    let view = hub.flow("living").expect("flow");
    assert_eq!(view.runs.len(), 1);
    assert_eq!(view.active_run.as_deref(), Some(first.id.as_str()));
}

#[tokio::test(start_paused = true)]
async fn run_now_still_refused_while_restart_flow_is_active() {
    let (hub, publisher, _engine) = engine_hub();
    let _log = echo(publisher, Arc::clone(&hub));
    hub.ingest(
        FLOW_TOPIC,
        with_card("restart", serde_json::json!({ "value": false }), "moving").as_bytes(),
    );
    settle().await;
    let mut events = hub.subscribe();

    hub.run_flow_from("living", Some("test_on")).expect("press");
    next_run(&mut events, |r| r.steps.len() == 2).await;
    settle().await;
    assert_eq!(
        hub.run_flow("living"),
        Err(HubError::Rejected("a run is active".to_owned()))
    );
}

/// Runs a flow whose only trigger is a Manual card (`params`) into a notify titled
/// `{trigger}`, and returns the published title.
async fn manual_notify_title(
    hub: &Arc<Hub>,
    publisher: &mut mpsc::Receiver<Publish>,
    id: &str,
    params: serde_json::Value,
) -> String {
    let flow = serde_json::json!({
        "id": id, "name": id, "enabled": true, "mode": "single",
        "nodes": [
            { "id": "go", "type": "trigger.manual", "params": params },
            { "id": "n", "type": "notify", "params": { "title": "{trigger}" } }
        ],
        "edges": [{ "from": "go", "to": "n" }]
    });
    hub.ingest(
        &format!("nidoo/automation/{id}"),
        flow.to_string().as_bytes(),
    );
    settle().await;
    let mut events = hub.subscribe();
    hub.run_flow_from(id, Some("go")).expect("run");
    next_run_event(&mut events, RunStatus::Done).await;
    settle().await;
    let mut last = None;
    while let Ok(publish) = publisher.try_recv() {
        if publish.topic == "nidoo/push/history" {
            last = Some(publish);
        }
    }
    let list: serde_json::Value =
        serde_json::from_str(&last.expect("a history publish").payload).expect("history json");
    list[0]["title"].as_str().expect("title").to_owned()
}

#[tokio::test(start_paused = true)]
async fn manual_value_reaches_notify_text() {
    let (hub, mut publisher, _engine) = engine_hub();
    push_ready(&hub).await;
    let title = manual_notify_title(
        &hub,
        &mut publisher,
        "hi",
        serde_json::json!({ "value": "hello" }),
    )
    .await;
    // The template renderer capitalises the first letter of what it renders.
    assert_eq!(title, "Hello");
    let title = manual_notify_title(&hub, &mut publisher, "bare", serde_json::json!({})).await;
    assert_eq!(title, "?");
}

// ---- math cards ----

const METER_ID: &str = "virtual_meter";
const TARGET_ID: &str = "virtual_target";
const METER_TOPIC: &str = "nidoo/virtual/meter";
const TARGET_TOPIC: &str = "nidoo/virtual/target";

async fn add_virtual_number(hub: &Hub, id: &str, min: f64, max: f64) {
    let spec = nidoo_api::VirtualSpec {
        kind: nidoo_api::VirtualKind::Number,
        name: id.to_owned(),
        min: Some(min),
        max: Some(max),
        step: Some(1.0),
        unit: None,
        options: None,
        initial: None,
    };
    hub.set_virtual(id, spec).await.expect("virtual number");
}

/// A hub with the engine, a `meter` (-5000..5000) and a `target` (0..100) virtual number.
async fn math_hub() -> (
    Arc<Hub>,
    mpsc::Receiver<Publish>,
    tokio::task::JoinHandle<()>,
) {
    let (hub, mut publisher, engine) = engine_hub();
    add_virtual_number(&hub, "meter", -5000.0, 5000.0).await;
    add_virtual_number(&hub, "target", 0.0, 100.0).await;
    while publisher.try_recv().is_ok() {}
    (hub, publisher, engine)
}

/// Plays the broker for virtual entities: every publish is logged as `(topic, payload)` and,
/// unless its topic is in `mute`, comes back as the retained state.
fn math_echo(
    mut publisher: mpsc::Receiver<Publish>,
    hub: Arc<Hub>,
    mute: &'static [&'static str],
) -> Arc<Mutex<Vec<(String, String)>>> {
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&log);
    tokio::spawn(async move {
        while let Some(publish) = publisher.recv().await {
            sink.lock()
                .expect("log")
                .push((publish.topic.clone(), publish.payload.clone()));
            if publish.topic.starts_with("nidoo/virtual/")
                && !mute.contains(&publish.topic.as_str())
            {
                hub.ingest_message(&publish.topic, publish.payload.as_bytes(), true);
            }
        }
    });
    log
}

fn sent_to(log: &Arc<Mutex<Vec<(String, String)>>>, topic: &str) -> Vec<String> {
    log.lock()
        .expect("log")
        .iter()
        .filter(|(t, _)| t == topic)
        .map(|(_, p)| p.clone())
        .collect()
}

fn math_node(
    id: &str,
    inputs: serde_json::Value,
    formula: &str,
    name: &str,
    save_to: Option<&str>,
) -> serde_json::Value {
    let mut params = serde_json::json!({ "inputs": inputs, "formula": formula, "name": name });
    if let Some(save_to) = save_to {
        params["save_to"] = save_to.into();
    }
    serde_json::json!({ "id": id, "type": "math", "params": params })
}

fn meter_input() -> serde_json::Value {
    serde_json::json!([{ "name": "meter", "entity": METER_ID }])
}

/// A flow triggered by `meter` with `nodes` and `edges` after the trigger node `t`.
fn math_flow_json(
    id: &str,
    nodes: Vec<serde_json::Value>,
    edges: Vec<serde_json::Value>,
) -> String {
    let mut all = vec![serde_json::json!(
        { "id": "t", "type": "trigger.state", "params": { "entity": METER_ID } }
    )];
    all.extend(nodes);
    serde_json::json!({
        "id": id, "name": id, "enabled": true, "mode": "single", "nodes": all, "edges": edges
    })
    .to_string()
}

fn edge(from: &str, to: &str) -> serde_json::Value {
    serde_json::json!({ "from": from, "to": to })
}

fn port_edge(from: &str, port: &str, to: &str) -> serde_json::Value {
    serde_json::json!({ "from": from, "port": port, "to": to })
}

/// Saves a flow (the meter already holds its initial -5000, so any other reading is a change)
/// and returns the events.
async fn arm_math_flow(hub: &Arc<Hub>, json: String) -> tokio::sync::broadcast::Receiver<WsEvent> {
    hub.ingest("nidoo/automation/m", json.as_bytes());
    settle().await;
    hub.subscribe()
}

fn target_state(hub: &Hub) -> Option<nidoo_core::model::StateValue> {
    hub.entity_view(TARGET_ID).map(|view| view.state.value)
}

fn target_number(hub: &Hub) -> f64 {
    match target_state(hub) {
        Some(nidoo_core::model::StateValue::Number(n)) => n,
        other => panic!("target is not a number: {other:?}"),
    }
}

async fn run_single_math(
    formula: &str,
    save_to: Option<&str>,
    reading: &[u8],
) -> (RunView, Arc<Mutex<Vec<(String, String)>>>, Arc<Hub>) {
    let (hub, publisher, engine) = math_hub().await;
    drop(engine); // dropping a JoinHandle detaches the task
    let log = math_echo(publisher, Arc::clone(&hub), &[]);
    let json = math_flow_json(
        "m",
        vec![math_node("calc", meter_input(), formula, "slider", save_to)],
        vec![edge("t", "calc")],
    );
    let mut events = arm_math_flow(&hub, json).await;
    hub.ingest_message(METER_TOPIC, reading, true);
    let run = next_run(&mut events, |r| r.status != RunStatus::Running).await;
    (run, log, hub)
}

#[tokio::test]
async fn math_saves_rounded_result() {
    let (run, log, hub) = run_single_math("meter / 2", Some(TARGET_ID), b"95").await;
    assert_eq!(run.status, RunStatus::Done, "{:?}", run.steps);
    let detail = &run.steps.last().expect("step").detail;
    assert!(
        detail.ends_with("→ slider = 47.5 → virtual_target = 48"),
        "{detail}"
    );
    assert_eq!(sent_to(&log, TARGET_TOPIC), ["48"]);
    settle().await;
    assert_eq!(target_number(&hub), 48.0);
}

#[tokio::test]
async fn math_clamps_and_says_so() {
    let (run, _log, hub) = run_single_math("meter", Some(TARGET_ID), b"500").await;
    assert_eq!(run.status, RunStatus::Done, "{:?}", run.steps);
    let detail = &run.steps.last().expect("step").detail;
    assert!(detail.contains("(kept in range)"), "{detail}");
    settle().await;
    assert_eq!(target_number(&hub), 100.0);
}

#[tokio::test(start_paused = true)]
async fn math_result_feeds_command_if_and_notify() {
    let (hub, publisher, _engine) = math_hub().await;
    push_ready(&hub).await;
    let log = math_echo(publisher, Arc::clone(&hub), &[]);
    let notify = serde_json::json!({ "id": "n", "type": "notify",
        "params": { "title": "T", "body": "Half {half:0}" } });
    let command = serde_json::json!({ "id": "set", "type": "command", "params": {
        "entity": TARGET_ID, "value_from": "half", "command": { "type": "set_number", "value": 0 } } });
    let check = serde_json::json!({ "id": "big", "type": "if",
        "params": { "result": "half", "op": "gt", "value": 10 } });
    let json = math_flow_json(
        "m",
        vec![
            math_node("calc", meter_input(), "meter / 2", "half", None),
            check,
            command,
            notify,
        ],
        vec![
            edge("t", "calc"),
            edge("calc", "big"),
            port_edge("big", "true", "set"),
            edge("set", "n"),
        ],
    );
    let mut events = arm_math_flow(&hub, json).await;
    hub.ingest_message(METER_TOPIC, b"60", true);
    let run = next_run(&mut events, |r| r.status != RunStatus::Running).await;

    assert_eq!(run.status, RunStatus::Done, "{:?}", run.steps);
    assert_eq!(step_nodes(&run), ["t", "calc", "big", "set", "n"]);
    assert_eq!(sent_to(&log, TARGET_TOPIC), ["30"]);
    settle().await;
    assert_eq!(target_number(&hub), 30.0);
    let history = sent_to(&log, "nidoo/push/history");
    let list: serde_json::Value =
        serde_json::from_str(history.last().expect("history")).expect("json");
    assert_eq!(list[0]["body"], "Half 30", "{list}");
}

#[tokio::test]
async fn unknown_input_fails_the_step() {
    let (hub, publisher, _engine) = math_hub().await;
    let log = math_echo(publisher, Arc::clone(&hub), &[]);
    hub.ingest(
        TEMP_CONFIG_TOPIC,
        &fixture("discovery/sensor__0x00124b0008fd6568__temperature_1.json"),
    );
    let inputs = serde_json::json!([{ "name": "meter", "entity": TEMP_ID }]);
    let json = math_flow_json(
        "m",
        vec![math_node(
            "calc",
            inputs,
            "meter",
            "slider",
            Some(TARGET_ID),
        )],
        vec![edge("t", "calc")],
    );
    let mut events = arm_math_flow(&hub, json).await;
    hub.ingest_message(METER_TOPIC, b"5", true);
    let run = next_run(&mut events, |r| r.status != RunStatus::Running).await;

    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.steps.last().expect("step").detail, "meter is unknown");
    settle().await;
    assert!(sent_to(&log, TARGET_TOPIC).is_empty());
}

#[tokio::test]
async fn unknown_input_in_the_untaken_branch_is_not_read() {
    let (hub, publisher, _engine) = math_hub().await;
    let _log = math_echo(publisher, Arc::clone(&hub), &[]);
    hub.ingest(
        TEMP_CONFIG_TOPIC,
        &fixture("discovery/sensor__0x00124b0008fd6568__temperature_1.json"),
    );
    let inputs = serde_json::json!([
        { "name": "meter", "entity": METER_ID },
        { "name": "ghost", "entity": TEMP_ID },
        { "name": "unused", "entity": TEMP_ID },
    ]);
    let json = math_flow_json(
        "m",
        vec![math_node(
            "calc",
            inputs,
            "if(meter > 0, meter * 2, ghost)",
            "slider",
            None,
        )],
        vec![edge("t", "calc")],
    );
    let mut events = arm_math_flow(&hub, json).await;
    hub.ingest_message(METER_TOPIC, b"7", true);
    let run = next_run(&mut events, |r| r.status != RunStatus::Running).await;

    assert_eq!(run.status, RunStatus::Done, "{:?}", run.steps);
    let detail = &run.steps.last().expect("step").detail;
    assert_eq!(detail, "meter=7, ghost=?, unused=? → slider = 14");
}

#[tokio::test]
async fn command_value_from_is_kept_in_the_entity_range() {
    let (hub, publisher, _engine) = math_hub().await;
    let log = math_echo(publisher, Arc::clone(&hub), &[]);
    let command = serde_json::json!({ "id": "set", "type": "command", "params": {
        "entity": TARGET_ID, "value_from": "big", "command": { "type": "set_number", "value": 0 } } });
    let json = math_flow_json(
        "m",
        vec![
            math_node("calc", meter_input(), "meter * 10", "big", None),
            command,
        ],
        vec![edge("t", "calc"), edge("calc", "set")],
    );
    let mut events = arm_math_flow(&hub, json).await;
    hub.ingest_message(METER_TOPIC, b"50", true);
    let run = next_run(&mut events, |r| r.status != RunStatus::Running).await;

    assert_eq!(run.status, RunStatus::Done, "{:?}", run.steps);
    assert_eq!(sent_to(&log, TARGET_TOPIC), ["100"]);
    assert_eq!(
        run.steps.last().expect("step").detail,
        "accepted (kept in range)"
    );
}

#[tokio::test]
async fn text_input_fails_the_step() {
    let (hub, publisher, _engine) = math_hub().await;
    let _log = math_echo(publisher, Arc::clone(&hub), &[]);
    let select = nidoo_api::VirtualSpec {
        kind: nidoo_api::VirtualKind::Select,
        name: "Mode".to_owned(),
        min: None,
        max: None,
        step: None,
        unit: None,
        options: Some(vec!["eco".to_owned(), "comfort".to_owned()]),
        initial: None,
    };
    hub.set_virtual("mode", select).await.expect("select");
    let inputs = serde_json::json!([{ "name": "mode", "entity": "virtual_mode" }]);
    let json = math_flow_json(
        "m",
        vec![math_node("calc", inputs, "mode", "slider", None)],
        vec![edge("t", "calc")],
    );
    let mut events = arm_math_flow(&hub, json).await;
    hub.ingest_message(METER_TOPIC, b"5", true);
    let run = next_run(&mut events, |r| r.status != RunStatus::Running).await;

    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(
        run.steps.last().expect("step").detail,
        "mode is not a number"
    );
}

#[tokio::test]
async fn refused_save_stops_the_run() {
    let (hub, publisher, _engine) = math_hub().await;
    let log = math_echo(publisher, Arc::clone(&hub), &[TARGET_TOPIC]);
    let json = math_flow_json(
        "m",
        vec![
            math_node("calc", meter_input(), "meter", "slider", Some(TARGET_ID)),
            turn("on", "turn_on"),
        ],
        vec![edge("t", "calc"), edge("calc", "on")],
    );
    let mut events = arm_math_flow(&hub, json).await;
    hub.ingest_message(METER_TOPIC, b"5", true);
    let run = next_run(&mut events, |r| r.status != RunStatus::Running).await;

    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(step_nodes(&run), ["t", "calc"]);
    assert_eq!(run.steps[1].outcome, StepOutcome::Failed);
    let detail = &run.steps[1].detail;
    assert!(
        detail.starts_with("meter=5 → slider = 5 → virtual_target = 5 → "),
        "{detail}"
    );
    assert!(
        detail.len() > "meter=5 → slider = 5 → virtual_target = 5 → ".len(),
        "the refusal reason follows: {detail}"
    );
    assert!(
        sent_to(&log, LED_COMMAND_TOPIC).is_empty(),
        "the command after the refused save never ran"
    );
}

#[tokio::test]
async fn division_by_zero_fails() {
    let (run, _log, _hub) = run_single_math("meter / (meter - meter)", None, b"7").await;
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.steps.last().expect("step").detail, "division by zero");
}

#[tokio::test]
async fn result_missing_on_path_fails() {
    let (hub, publisher, _engine) = math_hub().await;
    let _log = math_echo(publisher, Arc::clone(&hub), &[]);
    let command = serde_json::json!({ "id": "set", "type": "command", "params": {
        "entity": TARGET_ID, "value_from": "half", "command": { "type": "set_number", "value": 0 } } });
    let check = serde_json::json!({ "id": "big", "type": "if",
        "params": { "entity": METER_ID, "op": "gt", "value": 10 } });
    let json = math_flow_json(
        "m",
        vec![
            check,
            math_node("calc", meter_input(), "meter / 2", "half", None),
            command,
        ],
        vec![
            edge("t", "big"),
            port_edge("big", "true", "calc"),
            port_edge("big", "false", "set"),
            edge("calc", "set"),
        ],
    );
    let mut events = arm_math_flow(&hub, json).await;
    hub.ingest_message(METER_TOPIC, b"5", true);
    let run = next_run(&mut events, |r| r.status != RunStatus::Running).await;

    assert_eq!(run.status, RunStatus::Failed, "{:?}", run.steps);
    assert_eq!(
        run.steps.last().expect("step").detail,
        "no result half in this run"
    );
}

#[tokio::test]
async fn trigger_in_formula() {
    let (run, _log, _hub) = run_single_math("trigger * 2", None, b"21").await;
    assert_eq!(run.status, RunStatus::Done, "{:?}", run.steps);
    assert!(
        run.steps
            .last()
            .expect("step")
            .detail
            .ends_with("→ slider = 42")
    );

    let (hub, publisher, _engine) = math_hub().await;
    let _log = math_echo(publisher, Arc::clone(&hub), &[]);
    let manual = serde_json::json!({ "id": "go", "type": "trigger.manual", "params": {} });
    let json = serde_json::json!({
        "id": "m", "name": "m", "enabled": true, "mode": "single",
        "nodes": [manual, math_node("calc", serde_json::json!([]), "trigger * 2", "slider", None)],
        "edges": [edge("go", "calc")]
    })
    .to_string();
    hub.ingest("nidoo/automation/m", json.as_bytes());
    settle().await;
    let mut events = hub.subscribe();
    hub.run_flow("m").expect("run");
    let run = next_run(&mut events, |r| r.status != RunStatus::Running).await;
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(
        run.steps.last().expect("step").detail,
        "trigger has no number"
    );
}

#[tokio::test]
async fn set_flow_refuses_bad_math_targets() {
    let (hub, _publisher, _engine) = math_hub().await;
    let try_flow = |node: serde_json::Value| {
        let json = math_flow_json("m", vec![node], vec![edge("t", "calc")]);
        Flow::decode(json.as_bytes()).expect("flow")
    };

    let flow = try_flow(math_node(
        "calc",
        meter_input(),
        "meter",
        "slider",
        Some(LED_ID),
    ));
    match hub.set_flow(flow).await {
        Err(HubError::InvalidFlow(reason)) => {
            assert!(reason.contains("is not a number entity"), "{reason}");
        }
        other => panic!("expected InvalidFlow, got {other:?}"),
    }

    let flow = try_flow(math_node("calc", meter_input(), "meter", METER_ID, None));
    match hub.set_flow(flow).await {
        Err(HubError::InvalidFlow(reason)) => {
            assert!(reason.contains("is an entity id"), "{reason}");
        }
        other => panic!("expected InvalidFlow, got {other:?}"),
    }

    let inputs = serde_json::json!([{ "name": "meter", "entity": "nope" }]);
    let flow = try_flow(math_node("calc", inputs, "meter", "slider", None));
    match hub.set_flow(flow).await {
        Err(HubError::InvalidFlow(reason)) => {
            assert!(reason.contains("unknown entity"), "{reason}")
        }
        other => panic!("expected InvalidFlow, got {other:?}"),
    }

    let flow = try_flow(math_node(
        "calc",
        meter_input(),
        "meter",
        "slider",
        Some(TARGET_ID),
    ));
    hub.set_flow(flow).await.expect("a valid math flow saves");
}

const VACUUM_ID: &str = "roborock_40313ca8b964";
const CLIMATE_ID: &str = "daikin_192_0_2_150";

/// A hub that knows a vacuum and a climate.
fn hub_with_vacuum_and_climate() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let (hub, rx) = hub();
    hub.ingest(
        "nidoo/vacuum/roborock_40313ca8b964/config",
        &fixture("discovery/vacuum__roborock_40313ca8b964__roborock_40313ca8b964.json"),
    );
    hub.ingest(
        "nidoo/climate/daikin_192_0_2_150/config",
        &fixture("discovery/climate__daikin_192_0_2_150__climate.json"),
    );
    (hub, rx)
}

/// A flow whose state trigger reads `params` of `entity`, then turns nothing on.
fn field_flow(params: serde_json::Value) -> Flow {
    let text = serde_json::json!({
        "id": "fielded", "name": "Fielded", "enabled": true,
        "nodes": [
            { "id": "start", "type": "trigger.state", "params": params },
            { "id": "wait", "type": "delay", "params": { "for": "1m" } }
        ],
        "edges": [{ "from": "start", "to": "wait" }]
    });
    Flow::decode(text.to_string().as_bytes()).expect("field flow")
}

/// A flow with an If reading `params`, after a plain trigger on the vacuum.
fn field_if_flow(params: serde_json::Value) -> Flow {
    let text = serde_json::json!({
        "id": "fielded", "name": "Fielded", "enabled": true,
        "nodes": [
            { "id": "start", "type": "trigger.state", "params": { "entity": VACUUM_ID } },
            { "id": "check", "type": "if", "params": params },
            { "id": "wait", "type": "delay", "params": { "for": "1m" } }
        ],
        "edges": [
            { "from": "start", "to": "check" },
            { "from": "check", "port": "true", "to": "wait" }
        ]
    });
    Flow::decode(text.to_string().as_bytes()).expect("field if flow")
}

fn invalid_reason(result: Result<nidoo_api::FlowWriteOutcome, HubError>) -> String {
    match result {
        Err(HubError::InvalidFlow(reason)) => reason,
        other => panic!("expected InvalidFlow, got {other:?}"),
    }
}

#[tokio::test]
async fn unknown_field_is_refused() {
    let (hub, _publisher) = hub_with_vacuum_and_climate();
    let flow = field_flow(serde_json::json!({ "entity": VACUUM_ID, "field": "brightness" }));
    assert_eq!(
        invalid_reason(hub.set_flow(flow).await),
        format!("node `start`: `{VACUUM_ID}` reports no field `brightness`")
    );
    let flow = field_if_flow(serde_json::json!({
        "entity": VACUUM_ID, "field": "position", "op": "gte", "value": 1
    }));
    assert_eq!(
        invalid_reason(hub.set_flow(flow).await),
        format!("node `check`: `{VACUUM_ID}` reports no field `position`")
    );
    let flow = field_flow(serde_json::json!({ "entity": VACUUM_ID, "field": "battery" }));
    assert!(hub.set_flow(flow).await.is_ok());
}

#[tokio::test]
async fn average_on_unrecorded_field_is_refused() {
    let (hub, _publisher) = hub_with_vacuum_and_climate();
    let flow = field_if_flow(serde_json::json!({
        "entity": VACUUM_ID, "field": "battery", "op": "lte", "value": 55, "average": "15m"
    }));
    assert_eq!(
        invalid_reason(hub.set_flow(flow).await),
        format!("node `check`: `battery` of `{VACUUM_ID}` has no history to average")
    );
}

#[tokio::test]
async fn average_on_recorded_field_is_accepted() {
    let (hub, _publisher) = hub_with_vacuum_and_climate();
    let flow = field_if_flow(serde_json::json!({
        "entity": CLIMATE_ID, "field": "current", "op": "lte", "value": 19, "average": "15m"
    }));
    assert!(hub.set_flow(flow).await.is_ok());
}

#[tokio::test]
async fn entity_value_reads_the_field() {
    let (hub, _publisher) = hub_with_vacuum_and_climate();
    assert_eq!(hub.entity_value(VACUUM_ID, Some("battery")), None);
    hub.ingest(
        "roborock/ha/state",
        br#"{"state": "cleaning", "battery_level": 62}"#,
    );
    assert_eq!(
        hub.entity_value(VACUUM_ID, Some("battery")),
        Some(nidoo_core::flow::Scalar::Number(62.0))
    );
    assert_eq!(
        hub.entity_value(VACUUM_ID, None),
        Some(nidoo_core::flow::Scalar::Text("cleaning".into()))
    );
    assert_eq!(hub.entity_value(VACUUM_ID, Some("fan_speed")), None);
    assert_eq!(hub.entity_value("nope", Some("battery")), None);
}

/// A field trigger on `entity`, then an If with no entity that averages the trigger's value.
fn averaged_after_field_flow(entity: &str, field: &str) -> Flow {
    let text = serde_json::json!({
        "id": "fielded", "name": "Fielded", "enabled": true,
        "nodes": [
            { "id": "start", "type": "trigger.state", "params": { "entity": entity, "field": field } },
            { "id": "wait", "type": "delay", "params": { "for": "1m" } },
            { "id": "check", "type": "if",
              "params": { "op": "lte", "value": 19, "average": "15m" } }
        ],
        "edges": [
            { "from": "start", "to": "wait" },
            { "from": "wait", "to": "check" }
        ]
    });
    Flow::decode(text.to_string().as_bytes()).expect("averaged flow")
}

#[tokio::test]
async fn average_after_an_unrecorded_field_trigger_is_refused() {
    let (hub, _publisher) = hub_with_vacuum_and_climate();
    assert_eq!(
        invalid_reason(
            hub.set_flow(averaged_after_field_flow(VACUUM_ID, "battery"))
                .await
        ),
        format!("node `check`: `battery` of `{VACUUM_ID}` has no history to average")
    );
    assert!(
        hub.set_flow(averaged_after_field_flow(CLIMATE_ID, "current"))
            .await
            .is_ok()
    );
}

/// Declares a living room (`sejour`, named "Séjour") and a kitchen, each with one satellite.
fn rooms_with_satellites(hub: &Hub) {
    hub.ingest(
        "nidoo/ui/_areas",
        r#"{"areas":[{"id":"sejour","name":"Séjour"},{"id":"kitchen","name":"Kitchen"}]}"#
            .as_bytes(),
    );
    common::speech::satellite(hub, "a", "sejour");
    common::speech::satellite(hub, "b", "kitchen");
}

fn speak_node(params: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "id": "s", "type": "speak", "params": params })
}

#[tokio::test]
async fn set_flow_resolves_the_speak_room_and_refuses_an_unknown_one() {
    let (hub, _publisher) = hub_with_entities();
    rooms_with_satellites(&hub);
    let node = speak_node(serde_json::json!({ "text": "Dinner", "room": "Séjour" }));
    let flow = Flow::decode(chain_json("dinner", true, vec![node]).as_bytes()).expect("flow");
    let saved = hub.set_flow(flow).await.expect("saved").flow;
    let speak = saved
        .definition
        .nodes
        .iter()
        .find(|n| n.id == "s")
        .expect("node");
    assert!(
        matches!(&speak.kind, NodeKind::Speak(p) if p.room.as_deref() == Some("sejour")),
        "{:?}",
        speak.kind
    );

    let node = speak_node(serde_json::json!({ "text": "Dinner", "room": "attic" }));
    let flow = Flow::decode(chain_json("ghost", true, vec![node]).as_bytes()).expect("flow");
    match hub.set_flow(flow).await {
        Err(HubError::InvalidFlow(reason)) => assert!(reason.contains("attic"), "{reason}"),
        other => panic!("expected InvalidFlow, got {other:?}"),
    }
}

#[tokio::test]
async fn speak_step_says_the_sentence_on_the_room_satellites() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (hub, mut publisher, _engine) = engine_hub();
    hub.set_tts(common::speech::ready_tts(dir.path().join("tts")).await);
    rooms_with_satellites(&hub);
    let node = speak_node(serde_json::json!({ "text": "Dinner", "room": "kitchen" }));
    hub.ingest(
        "nidoo/automation/dinner",
        chain_json("dinner", true, vec![node]).as_bytes(),
    );
    settle().await;
    let mut events = hub.subscribe();
    hub.run_flow("dinner").expect("run");
    let run = next_run(&mut events, |run| run.status != RunStatus::Running).await;
    let step = run.steps.last().expect("step");
    assert_eq!(step.outcome, StepOutcome::Ok, "{run:?}");
    assert_eq!(step.detail, "spoke on 1 satellite(s): Dinner");
    let mut spoken = Vec::new();
    while let Ok(publish) = publisher.try_recv() {
        if publish.topic == "nidoo-sound/speak" {
            spoken.push(publish);
        }
    }
    assert_eq!(spoken.len(), 1, "{spoken:?}");
    assert!(!spoken[0].retain);
    let payload: serde_json::Value = serde_json::from_str(&spoken[0].payload).expect("json");
    assert_eq!(payload["players"], serde_json::json!(["b"]));
}

#[tokio::test]
async fn speak_step_without_speech_fails() {
    let (hub, _publisher, _engine) = engine_hub();
    rooms_with_satellites(&hub);
    let node = speak_node(serde_json::json!({ "text": "Dinner" }));
    hub.ingest(
        "nidoo/automation/dinner",
        chain_json("dinner", true, vec![node]).as_bytes(),
    );
    settle().await;
    let mut events = hub.subscribe();
    hub.run_flow("dinner").expect("run");
    let run = next_run(&mut events, |run| run.status != RunStatus::Running).await;
    let step = run.steps.last().expect("step");
    assert_eq!(step.outcome, StepOutcome::Failed, "{run:?}");
    assert_eq!(step.detail, "speech is not set up on this hub");
}

// ---- ask ----

use nidoo::assist::stt::ask::{AskOutcome, AskRequest};
use nidoo::sound::SpeakTarget;

fn ask_node(params: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "id": "q", "type": "ask", "params": params })
}

fn yes_no(extra: serde_json::Value) -> serde_json::Value {
    let mut params = serde_json::json!({
        "text": "Warm it up?",
        "answers": [{ "label": "oui" }, { "label": "non" }]
    });
    for (key, value) in extra.as_object().expect("object") {
        params[key] = value.clone();
    }
    ask_node(params)
}

/// A fake voice service: answers each question from `script` and records `(target, text)`.
fn fake_voice(hub: &Hub, script: Vec<AskOutcome>) -> Arc<Mutex<Vec<(SpeakTarget, String)>>> {
    let (tx, mut rx) = mpsc::channel::<AskRequest>(4);
    hub.set_voice_asks(tx);
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&log);
    tokio::spawn(async move {
        let mut script = script.into_iter();
        while let Some(request) = rx.recv().await {
            sink.lock()
                .expect("log")
                .push((request.target, request.text));
            let _ = request
                .reply
                .send(script.next().unwrap_or(AskOutcome::Silence));
        }
    });
    log
}

fn heard(text: &str) -> AskOutcome {
    AskOutcome::Heard {
        sid: "a".to_owned(),
        text: text.to_owned(),
    }
}

/// Runs a flow `trigger → ask → (port 1) set` and returns the run.
async fn run_ask_flow(hub: &Hub, node: serde_json::Value, after: serde_json::Value) -> RunView {
    let json = serde_json::json!({
        "id": "asker", "name": "asker", "enabled": true, "mode": "single",
        "nodes": [
            { "id": "t", "type": "trigger.state", "params": { "entity": MOTION_ID } },
            node,
            after
        ],
        "edges": [
            { "from": "t", "to": "q" },
            { "from": "q", "port": "1", "to": "set" }
        ]
    })
    .to_string();
    hub.ingest("nidoo/automation/asker", json.as_bytes());
    settle().await;
    let mut events = hub.subscribe();
    hub.run_flow("asker").expect("run");
    // A question may wait up to 45 s (paused time in the tests that wait that long).
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            if let WsEvent::FlowRunChanged { run, .. } = events.recv().await.expect("open channel")
                && run.status != RunStatus::Running
            {
                return run;
            }
        }
    })
    .await
    .expect("the run ends")
}

fn set_target() -> serde_json::Value {
    serde_json::json!({ "id": "set", "type": "command", "params": {
        "entity": TARGET_ID, "value_from": "temp", "command": { "type": "set_number", "value": 0 } } })
}

fn ask_ports(run: &RunView) -> Vec<String> {
    let step = run.steps.iter().find(|s| s.node == "q").expect("ask step");
    step.ports.clone().unwrap_or_default()
}

#[tokio::test]
async fn set_flow_resolves_the_ask_room_and_refuses_an_unknown_one() {
    let (hub, _publisher) = hub_with_entities();
    rooms_with_satellites(&hub);
    let node = yes_no(serde_json::json!({ "room": "Séjour" }));
    let flow = Flow::decode(chain_json("asker", true, vec![node]).as_bytes()).expect("flow");
    let saved = hub.set_flow(flow).await.expect("saved").flow;
    let ask = saved
        .definition
        .nodes
        .iter()
        .find(|n| n.id == "q")
        .expect("node");
    assert!(
        matches!(&ask.kind, NodeKind::Ask(p) if p.room.as_deref() == Some("sejour")),
        "{:?}",
        ask.kind
    );

    let node = yes_no(serde_json::json!({ "room": "attic" }));
    let flow = Flow::decode(chain_json("ghost", true, vec![node]).as_bytes()).expect("flow");
    match hub.set_flow(flow).await {
        Err(HubError::InvalidFlow(reason)) => assert!(reason.contains("attic"), "{reason}"),
        other => panic!("expected InvalidFlow, got {other:?}"),
    }
}

#[tokio::test]
async fn set_flow_refuses_an_ask_name_that_is_an_entity() {
    let (hub, _publisher, _engine) = math_hub().await;
    let node = yes_no(serde_json::json!({ "name": TARGET_ID }));
    let flow = Flow::decode(chain_json("asker", true, vec![node]).as_bytes()).expect("flow");
    match hub.set_flow(flow).await {
        Err(HubError::InvalidFlow(reason)) => assert!(reason.contains("entity"), "{reason}"),
        other => panic!("expected InvalidFlow, got {other:?}"),
    }
}

#[tokio::test]
async fn ask_yes_takes_port_1() {
    let (hub, _publisher, _engine) = engine_hub();
    let log = fake_voice(&hub, vec![heard("oui")]);
    let run = run_ask_flow(&hub, yes_no(serde_json::json!({})), turn("set", "turn_on")).await;
    assert_eq!(ask_ports(&run), ["1"], "{run:?}");
    let step = run.steps.iter().find(|s| s.node == "q").expect("step");
    assert_eq!(step.outcome, StepOutcome::Ok);
    assert_eq!(step.detail, "asked \"Warm it up?\" → heard \"oui\" → oui");
    let log = log.lock().expect("log");
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].0, SpeakTarget::All);
}

#[tokio::test]
async fn ask_number_feeds_value_from() {
    let (hub, publisher, _engine) = math_hub().await;
    let sent = math_echo(publisher, Arc::clone(&hub), &[]);
    fake_voice(&hub, vec![heard("oui à 21")]);
    let run = run_ask_flow(
        &hub,
        yes_no(serde_json::json!({ "name": "temp" })),
        set_target(),
    )
    .await;
    assert_eq!(ask_ports(&run), ["1"], "{run:?}");
    let step = run.steps.iter().find(|s| s.node == "q").expect("step");
    assert!(step.detail.ends_with("→ oui (21)"), "{}", step.detail);
    assert_eq!(sent_to(&sent, TARGET_TOPIC), ["21"]);
}

#[tokio::test]
async fn ask_default_used_without_number() {
    let (hub, publisher, _engine) = math_hub().await;
    let sent = math_echo(publisher, Arc::clone(&hub), &[]);
    fake_voice(&hub, vec![heard("oui")]);
    let run = run_ask_flow(
        &hub,
        yes_no(serde_json::json!({ "name": "temp", "default": 20.0 })),
        set_target(),
    )
    .await;
    assert_eq!(ask_ports(&run), ["1"], "{run:?}");
    let step = run.steps.iter().find(|s| s.node == "q").expect("step");
    assert!(
        step.detail.ends_with("→ oui (default 20)"),
        "{}",
        step.detail
    );
    assert_eq!(sent_to(&sent, TARGET_TOPIC), ["20"]);
}

#[tokio::test]
async fn ask_retries_once_then_none() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (hub, _publisher, _engine) = engine_hub();
    hub.set_tts(common::speech::ready_tts(dir.path().join("tts")).await);
    let log = fake_voice(&hub, vec![heard("bof"), heard("toujours bof")]);
    let run = run_ask_flow(&hub, yes_no(serde_json::json!({})), turn("set", "turn_on")).await;
    assert_eq!(ask_ports(&run), ["none"], "{run:?}");
    let step = run.steps.iter().find(|s| s.node == "q").expect("step");
    assert_eq!(step.outcome, StepOutcome::Ok);
    assert_eq!(
        step.detail,
        "asked \"Warm it up?\" → heard \"bof\", \"toujours bof\" → none"
    );
    let log = log.lock().expect("log");
    assert_eq!(log.len(), 2);
    assert_eq!(log[1].0, SpeakTarget::Satellite("a".to_owned()));
    assert!(log[1].1.starts_with("Je n'ai pas compris."), "{}", log[1].1);
}

#[tokio::test]
async fn ask_silence_goes_none() {
    let (hub, _publisher, _engine) = engine_hub();
    fake_voice(&hub, vec![AskOutcome::Silence]);
    let run = run_ask_flow(&hub, yes_no(serde_json::json!({})), turn("set", "turn_on")).await;
    assert_eq!(ask_ports(&run), ["none"], "{run:?}");
    let step = run.steps.iter().find(|s| s.node == "q").expect("step");
    assert_eq!(step.detail, "asked \"Warm it up?\" → no answer");
}

#[tokio::test]
async fn ask_without_voice_goes_none() {
    let (hub, _publisher, _engine) = engine_hub();
    let run = run_ask_flow(&hub, yes_no(serde_json::json!({})), turn("set", "turn_on")).await;
    assert_eq!(ask_ports(&run), ["none"], "{run:?}");
    let step = run.steps.iter().find(|s| s.node == "q").expect("step");
    assert_eq!(step.outcome, StepOutcome::Ok);
    assert!(
        step.detail.contains("speech input is off"),
        "{}",
        step.detail
    );
}

#[tokio::test]
async fn ask_retry_without_satellite_says_why() {
    let (hub, _publisher, _engine) = engine_hub();
    fake_voice(
        &hub,
        vec![
            heard("bof"),
            AskOutcome::NoSatellite("no free satellite".into()),
        ],
    );
    let run = run_ask_flow(&hub, yes_no(serde_json::json!({})), turn("set", "turn_on")).await;
    assert_eq!(ask_ports(&run), ["none"], "{run:?}");
    let step = run.steps.iter().find(|s| s.node == "q").expect("step");
    assert_eq!(
        step.detail,
        "asked \"Warm it up?\" → heard \"bof\" → retry: no free satellite → none"
    );
}

/// A voice that answers the first question `after` seconds late with `bof`, then never
/// answers the retry. Returns the log of requests.
fn slow_voice(hub: &Hub, after: u64) -> Arc<Mutex<Vec<(SpeakTarget, String)>>> {
    let (tx, mut rx) = mpsc::channel::<AskRequest>(4);
    hub.set_voice_asks(tx);
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&log);
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Some(request) = rx.recv().await {
            let first = {
                let mut log = sink.lock().expect("log");
                log.push((request.target, request.text));
                log.len() == 1
            };
            if first {
                tokio::time::sleep(Duration::from_secs(after)).await;
                let _ = request.reply.send(heard("bof"));
            } else {
                held.push(request.reply); // never answered
            }
        }
    });
    log
}

#[tokio::test(start_paused = true)]
async fn ask_with_its_retry_stays_within_one_limit() {
    let (hub, _publisher, _engine) = engine_hub();
    let log = slow_voice(&hub, 30);
    let started = tokio::time::Instant::now();
    let run = run_ask_flow(&hub, yes_no(serde_json::json!({})), turn("set", "turn_on")).await;
    assert!(
        started.elapsed() <= Duration::from_secs(45),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(ask_ports(&run), ["none"], "{run:?}");
    assert_eq!(log.lock().expect("log").len(), 2, "the retry was asked");
}

#[tokio::test(start_paused = true)]
async fn ask_gives_no_retry_when_under_five_seconds_remain() {
    let (hub, _publisher, _engine) = engine_hub();
    let log = slow_voice(&hub, 42);
    let run = run_ask_flow(&hub, yes_no(serde_json::json!({})), turn("set", "turn_on")).await;
    assert_eq!(ask_ports(&run), ["none"], "{run:?}");
    assert_eq!(log.lock().expect("log").len(), 1, "no retry");
}
