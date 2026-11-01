//! `trigger.sun` timers and `op: is_dark` on the hub's flow engine, driven without a broker on
//! a paused tokio clock. The engine's wall clock is the real clock at start plus the tokio time
//! elapsed, so each test derives the instants it needs from `Utc::now()`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use nidoo::config::Config;
use nidoo::hub::Hub;
use nidoo_api::{ConnectionStatus, RunStatus, RunView, WsEvent};
use nidoo_core::command::Publish;
use nidoo_core::flow::sun::{is_dark_at, is_dark_with_offset};
use tokio::sync::mpsc;

const LED_ID: &str = "0x60a423fffe94d1c6_light_zigbee2mqtt";
const LED_STATE_TOPIC: &str = "zigbee2mqtt/rooms/living/led_ceiling";
const LED_COMMAND_TOPIC: &str = "zigbee2mqtt/rooms/living/led_ceiling/set";
const MOTION_ID: &str = "0x00158d008b5a2d65_occupancy_zigbee2mqtt";
const MOTION_STATE_TOPIC: &str = "zigbee2mqtt/rooms/living/motion_north";

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../nidoo-core/tests/fixtures/"
    ));
    std::fs::read(path.join(name)).expect("fixture")
}

/// A hub at (`lat`, `lon`) in `zone` that knows the LED and the motion sensor, with a running
/// engine and a broker stand-in that echoes the LED's commands.
fn test_hub_with_point(lat: f64, lon: f64, zone: &str) -> Arc<Hub> {
    let mut cfg = Config::default();
    cfg.commands.confirm_timeout_ms = 50;
    cfg.topics.discovery_prefixes = vec!["oikos".to_owned()];
    cfg.flows.timezone = Some(zone.to_owned());
    cfg.flows.latitude = Some(lat);
    cfg.flows.longitude = Some(lon);
    let (tx, mut publisher) = mpsc::channel::<Publish>(64);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    hub.ingest(
        "oikos/light/0x60a423fffe94d1c6/light/config",
        &fixture("discovery/light__0x60a423fffe94d1c6__light.json"),
    );
    hub.ingest(
        "oikos/binary_sensor/0x00158d008b5a2d65/occupancy/config",
        &fixture("discovery/binary_sensor__0x00158d008b5a2d65__occupancy.json"),
    );
    let echo_hub = Arc::clone(&hub);
    tokio::spawn(async move {
        while let Some(publish) = publisher.recv().await {
            if publish.topic == LED_COMMAND_TOPIC {
                echo_hub.ingest(LED_STATE_TOPIC, br#"{"state":"ON","brightness":100}"#);
            }
        }
    });
    let _engine = hub.start_engine();
    hub
}

fn sun_flow(id: &str, which: &str, offset: i32) -> String {
    serde_json::json!({
        "id": id, "name": id, "enabled": true, "mode": "single",
        "nodes": [
            { "id": "sun", "type": "trigger.sun",
              "params": { "which": which, "offset_minutes": offset } },
            { "id": "on", "type": "command",
              "params": { "entity": LED_ID, "command": { "type": "turn_on" } } }
        ],
        "edges": [{ "from": "sun", "to": "on" }]
    })
    .to_string()
}

fn state_then_is_dark_flow(id: &str) -> String {
    state_then_if_flow(id, serde_json::json!({ "op": "is_dark" }))
}

fn state_then_if_flow(id: &str, if_params: serde_json::Value) -> String {
    serde_json::json!({
        "id": id, "name": id, "enabled": true, "mode": "single",
        "nodes": [
            { "id": "motion", "type": "trigger.state", "params": { "entity": MOTION_ID } },
            { "id": "dark", "type": "if", "params": if_params },
            { "id": "on", "type": "command",
              "params": { "entity": LED_ID, "command": { "type": "turn_on" } } }
        ],
        "edges": [
            { "from": "motion", "to": "dark" },
            { "from": "dark", "port": "true", "to": "on" }
        ]
    })
    .to_string()
}

fn publish_flow(hub: &Hub, id: &str, json: &str) {
    hub.ingest(&format!("oikos/automation/{id}"), json.as_bytes());
}

/// Lets the engine task drain what it has been sent (current-thread runtimes only).
async fn settle() {
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

async fn next_done_run(rx: &mut tokio::sync::broadcast::Receiver<WsEvent>) -> RunView {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let WsEvent::FlowRunChanged { run, .. } = rx.recv().await.expect("open channel")
                && run.status == RunStatus::Done
            {
                return run;
            }
        }
    })
    .await
    .expect("a finished run within 2 s")
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

#[tokio::test(start_paused = true)]
async fn sun_trigger_fires_at_dusk_and_rearms() {
    let hub = test_hub_with_point(45.75, 4.85, "Europe/Paris");
    settle().await;
    publish_flow(&hub, "evening", &sun_flow("evening", "dusk", 0));
    settle().await;
    let mut events = hub.subscribe();

    tokio::time::advance(Duration::from_secs(26 * 3600)).await;
    let run = next_done_run(&mut events).await;
    assert_eq!(run.steps[0].node, "sun");
    assert_eq!(run.steps[0].detail, "sun dusk +0m");
    assert!(!run.manual, "a sun run is not manual");

    // Re-armed: another dusk follows within a day and a half.
    tokio::time::advance(Duration::from_secs(36 * 3600)).await;
    next_done_run(&mut events).await;

    hub.set_flow_enabled("evening", false)
        .await
        .expect("disable");
    settle().await;
    tokio::time::advance(Duration::from_secs(48 * 3600)).await;
    settle().await;
    no_run_event(&mut events).await;
}

#[tokio::test(start_paused = true)]
async fn if_is_dark_takes_true_branch_at_night_regardless_of_trigger() {
    let (lat, lon) = (48.8566, 2.3522);
    let hub = test_hub_with_point(lat, lon, "Europe/Paris");
    settle().await;
    publish_flow(
        &hub,
        "night_switch",
        &state_then_is_dark_flow("night_switch"),
    );
    settle().await;
    hub.ingest(MOTION_STATE_TOPIC, br#"{"occupancy":false}"#);
    settle().await;

    // Advance the engine clock to a minute at which the sun is below civil twilight.
    let start = Utc::now();
    let minutes = (1..=24 * 60)
        .find(|m| is_dark_at(lat, lon, start + chrono::Duration::minutes(*m)))
        .expect("it is dark at some point in a day");
    tokio::time::advance(Duration::from_secs(minutes as u64 * 60)).await;
    settle().await;

    let mut events = hub.subscribe();
    hub.ingest(MOTION_STATE_TOPIC, br#"{"occupancy":true}"#);
    let run = next_done_run(&mut events).await;
    let step = run
        .steps
        .iter()
        .find(|s| s.node == "dark")
        .expect("dark step");
    assert!(step.detail.contains("is_dark → true"), "{}", step.detail);
    assert!(run.steps.iter().any(|s| s.node == "on"));
}

#[tokio::test(start_paused = true)]
async fn if_is_dark_negative_offset_widens_the_dark() {
    let (lat, lon) = (48.8566, 2.3522);
    let hub = test_hub_with_point(lat, lon, "Europe/Paris");
    settle().await;
    publish_flow(
        &hub,
        "early_dark",
        &state_then_if_flow(
            "early_dark",
            serde_json::json!({ "op": "is_dark", "offset_minutes": -60 }),
        ),
    );
    settle().await;
    hub.ingest(MOTION_STATE_TOPIC, br#"{"occupancy":false}"#);
    settle().await;

    // A minute in the hour before civil dusk or after civil dawn: dark only thanks to the offset.
    let start = Utc::now();
    let minutes = (1..=24 * 60)
        .find(|m| {
            let at = start + chrono::Duration::minutes(*m);
            is_dark_with_offset(lat, lon, at, -60) && !is_dark_at(lat, lon, at)
        })
        .expect("a twilight hour falls in a day");
    tokio::time::advance(Duration::from_secs(minutes as u64 * 60)).await;
    settle().await;

    let mut events = hub.subscribe();
    hub.ingest(MOTION_STATE_TOPIC, br#"{"occupancy":true}"#);
    let run = next_done_run(&mut events).await;
    let step = run
        .steps
        .iter()
        .find(|s| s.node == "dark")
        .expect("dark step");
    assert!(step.detail.contains("is_dark → true"), "{}", step.detail);
}
