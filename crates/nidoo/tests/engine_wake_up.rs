//! `trigger.wake_up` timers on the hub's flow engine, driven without a broker on a paused
//! tokio clock. The engine's wall clock is the real clock at start plus the tokio time elapsed,
//! so each test derives the alarm times it needs from `Utc::now()`.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Timelike, Utc};
use chrono_tz::Europe::Paris;
use nidoo::config::Config;
use nidoo::hub::Hub;
use nidoo_api::{ConnectionStatus, RunStatus, RunView, WsEvent};
use nidoo_core::command::Publish;
use nidoo_core::flow::wake_up::{self, WakeUp};
use tokio::sync::mpsc;

const LED_ID: &str = "0x60a423fffe94d1c6_light_zigbee2mqtt";
const LED_STATE_TOPIC: &str = "zigbee2mqtt/rooms/living/led_ceiling";
const LED_COMMAND_TOPIC: &str = "zigbee2mqtt/rooms/living/led_ceiling/set";

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../nidoo-core/tests/fixtures/"
    ));
    std::fs::read(path.join(name)).expect("fixture")
}

/// A hub that knows the LED, with a running engine and a broker stand-in echoing its commands.
fn test_hub() -> Arc<Hub> {
    let mut cfg = Config::default();
    cfg.commands.confirm_timeout_ms = 50;
    cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
    cfg.flows.timezone = Some("Europe/Paris".to_owned());
    let (tx, mut publisher) = mpsc::channel::<Publish>(64);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    hub.ingest(
        "nidoo/light/0x60a423fffe94d1c6/light/config",
        &fixture("discovery/light__0x60a423fffe94d1c6__light.json"),
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

/// The alarm flow ringing `minutes` from now (rounded up to the minute), every day.
fn alarm_json(id: &str, enabled: bool, minutes: i64) -> String {
    let ring = Utc::now() + chrono::Duration::minutes(minutes) + chrono::Duration::seconds(59);
    let ring: DateTime<_> = ring.with_timezone(&Paris);
    let settings = WakeUp {
        version: 1,
        hour: ring.hour() as u8,
        minute: ring.minute() as u8,
        days: vec![0, 1, 2, 3, 4, 5, 6],
        player: "music_node_player".to_owned(),
        ..Default::default()
    };
    serde_json::to_string(&wake_up::to_flow(id, id, enabled, &settings)).unwrap()
}

fn heating_flow(id: &str, before: u32, alarms: &[&str]) -> String {
    serde_json::json!({
        "id": id, "name": id, "enabled": true, "mode": "single",
        "nodes": [
            { "id": "wake", "type": "trigger.wake_up",
              "params": { "before_minutes": before, "alarms": alarms } },
            { "id": "on", "type": "command",
              "params": { "entity": LED_ID, "command": { "type": "turn_on" } } }
        ],
        "edges": [{ "from": "wake", "to": "on" }]
    })
    .to_string()
}

fn publish_flow(hub: &Hub, id: &str, json: &str) {
    hub.ingest(&format!("nidoo/automation/{id}"), json.as_bytes());
}

async fn settle() {
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

async fn advance(minutes: u64) {
    tokio::time::advance(Duration::from_secs(minutes * 60)).await;
    settle().await;
}

/// Finished runs of the heating flow seen on `rx` within a short real wait.
async fn heating_runs(rx: &mut tokio::sync::broadcast::Receiver<WsEvent>) -> Vec<RunView> {
    let mut runs = Vec::new();
    let _ = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            if let Ok(WsEvent::FlowRunChanged { run, .. }) = rx.recv().await
                && run.flow_id == "heat"
                && run.status == RunStatus::Done
            {
                runs.push(run);
            }
        }
    })
    .await;
    runs
}

#[tokio::test(start_paused = true)]
async fn fires_before_the_alarm() {
    let hub = test_hub();
    settle().await;
    publish_flow(&hub, "wake-up-1", &alarm_json("wake-up-1", true, 60));
    publish_flow(&hub, "heat", &heating_flow("heat", 45, &[]));
    settle().await;
    let mut events = hub.subscribe();

    advance(14).await;
    assert!(heating_runs(&mut events).await.is_empty());
    advance(2).await;
    let runs = heating_runs(&mut events).await;
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0].steps[0].detail, "wake-up -45m");
}

#[tokio::test(start_paused = true)]
async fn moved_alarm_rearms() {
    let hub = test_hub();
    settle().await;
    publish_flow(&hub, "wake-up-1", &alarm_json("wake-up-1", true, 60));
    publish_flow(&hub, "heat", &heating_flow("heat", 45, &[]));
    settle().await;
    publish_flow(&hub, "wake-up-1", &alarm_json("wake-up-1", true, 90));
    settle().await;
    let mut events = hub.subscribe();

    advance(20).await;
    assert!(heating_runs(&mut events).await.is_empty());
    advance(30).await;
    assert_eq!(heating_runs(&mut events).await.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn disabled_alarm_disarms() {
    let hub = test_hub();
    settle().await;
    publish_flow(&hub, "wake-up-1", &alarm_json("wake-up-1", true, 60));
    publish_flow(&hub, "heat", &heating_flow("heat", 45, &[]));
    settle().await;
    publish_flow(&hub, "wake-up-1", &alarm_json("wake-up-1", false, 60));
    settle().await;
    let mut events = hub.subscribe();

    advance(20).await;
    assert!(heating_runs(&mut events).await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn alarms_filter() {
    let hub = test_hub();
    settle().await;
    publish_flow(&hub, "wake-up-1", &alarm_json("wake-up-1", true, 60));
    publish_flow(&hub, "wake-up-2", &alarm_json("wake-up-2", true, 90));
    publish_flow(&hub, "heat", &heating_flow("heat", 45, &["wake-up-2"]));
    settle().await;
    let mut events = hub.subscribe();

    // The first alarm's pre-ring (15 min) is ignored.
    advance(20).await;
    assert!(heating_runs(&mut events).await.is_empty());
    advance(30).await;
    assert_eq!(heating_runs(&mut events).await.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn no_alarm_never_fires() {
    let hub = test_hub();
    settle().await;
    publish_flow(&hub, "heat", &heating_flow("heat", 45, &[]));
    settle().await;
    let mut events = hub.subscribe();

    advance(25 * 60).await;
    assert!(heating_runs(&mut events).await.is_empty());
}
