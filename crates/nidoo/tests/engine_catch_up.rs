//! Opt-in catch-up of missed time triggers on the hub's flow engine, driven without a broker on
//! a paused tokio clock. A restart is simulated by building a fresh hub from the retained
//! documents the old one published. The engine's wall clock is the real clock at start plus the
//! tokio time elapsed, so each test derives the instants it needs from `Utc::now()`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Timelike, Utc};
use chrono_tz::Europe::Paris;
use nidoo::config::Config;
use nidoo::hub::Hub;
use nidoo_api::{ConnectionStatus, RunStatus, RunView};
use nidoo_core::command::Publish;
use nidoo_core::flow::SunWhich;
use nidoo_core::flow::sun::sun_events;
use nidoo_core::flow::wake_up::{self, WakeUp};
use tokio::sync::mpsc;

const LED_ID: &str = "0x60a423fffe94d1c6_light_zigbee2mqtt";
const LED_STATE_TOPIC: &str = "zigbee2mqtt/rooms/living/led_ceiling";
const LED_COMMAND_TOPIC: &str = "zigbee2mqtt/rooms/living/led_ceiling/set";
const LAT: f64 = 48.8566;
const LON: f64 = 2.3522;

type Log = Arc<Mutex<Vec<Publish>>>;

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../nidoo-core/tests/fixtures/"
    ));
    std::fs::read(path.join(name)).expect("fixture")
}

/// A retained document: its topic and payload.
type Retained = (String, String);

/// A hub that knows the LED. The retained documents are ingested before the engine starts,
/// like a broker replaying them, and the link is online when the engine starts. Every publish
/// the hub makes is logged.
fn boot(retained: &[Retained]) -> (Arc<Hub>, Log) {
    let mut cfg = Config::default();
    cfg.commands.confirm_timeout_ms = 50;
    cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
    cfg.flows.timezone = Some("Europe/Paris".to_owned());
    cfg.flows.latitude = Some(LAT);
    cfg.flows.longitude = Some(LON);
    let (tx, mut publisher) = mpsc::channel::<Publish>(64);
    let hub = Hub::new(&cfg, tx);
    hub.ingest(
        "nidoo/light/0x60a423fffe94d1c6/light/config",
        &fixture("discovery/light__0x60a423fffe94d1c6__light.json"),
    );
    for (topic, payload) in retained {
        hub.ingest(topic, payload.as_bytes());
    }
    hub.set_connection(ConnectionStatus::Online);
    let log: Log = Arc::default();
    let (echo_hub, echo_log) = (Arc::clone(&hub), Arc::clone(&log));
    tokio::spawn(async move {
        while let Some(publish) = publisher.recv().await {
            if publish.topic == LED_COMMAND_TOPIC {
                echo_hub.ingest(LED_STATE_TOPIC, br#"{"state":"ON","brightness":100}"#);
            }
            echo_log.lock().unwrap().push(publish);
        }
    });
    let _engine = hub.start_engine();
    (hub, log)
}

/// What a broker would still hold after `log`: the last retained payload per topic of
/// `initial` and of the log, empty ones gone.
fn retained_after(initial: &[Retained], log: &Log) -> Vec<Retained> {
    let mut held: BTreeMap<String, String> = initial.iter().cloned().collect();
    for publish in log.lock().unwrap().iter().filter(|p| p.retain) {
        held.insert(publish.topic.clone(), publish.payload.clone());
    }
    held.into_iter()
        .filter(|(_, payload)| !payload.is_empty())
        .collect()
}

fn marker_topic(flow: &str, node: &str) -> String {
    format!("nidoo/automation/{flow}/fired/{node}")
}

fn marker(flow: &str, node: &str, due: i64) -> Retained {
    (marker_topic(flow, node), format!(r#"{{"due":{due}}}"#))
}

/// The marker payloads published for `flow`/`node`, oldest first; an empty string is a clear.
fn marker_publishes(log: &Log, flow: &str, node: &str) -> Vec<String> {
    let topic = marker_topic(flow, node);
    log.lock()
        .unwrap()
        .iter()
        .filter(|p| p.topic == topic)
        .map(|p| {
            assert!(p.retain, "markers are retained");
            p.payload.clone()
        })
        .collect()
}

fn marker_due(payload: &str) -> i64 {
    serde_json::from_str::<serde_json::Value>(payload).unwrap()["due"]
        .as_i64()
        .unwrap()
}

/// A flow whose trigger node is `at`, followed by a command.
fn flow_doc(id: &str, trigger: serde_json::Value) -> String {
    serde_json::json!({
        "id": id, "name": id, "enabled": true, "mode": "single",
        "nodes": [
            trigger,
            { "id": "on", "type": "command",
              "params": { "entity": LED_ID, "command": { "type": "turn_on" } } }
        ],
        "edges": [{ "from": "at", "to": "on" }]
    })
    .to_string()
}

fn schedule_trigger(node: &str, cron: &str, catch_up: Option<u32>) -> serde_json::Value {
    let mut params = serde_json::json!({ "cron": cron });
    if let Some(minutes) = catch_up {
        params["catch_up_minutes"] = minutes.into();
    }
    serde_json::json!({ "id": node, "type": "trigger.schedule", "params": params })
}

fn flow_retained(id: &str, json: String) -> Retained {
    (format!("nidoo/automation/{id}"), json)
}

/// A daily cron firing at the minute `minutes_ago` before now (Paris time), with that instant.
fn daily_cron_ago(minutes_ago: i64) -> (String, DateTime<chrono_tz::Tz>) {
    let at = (Utc::now() - chrono::Duration::minutes(minutes_ago)).with_timezone(&Paris);
    let due = at.with_second(0).unwrap().with_nanosecond(0).unwrap();
    (format!("{} {} * * *", due.minute(), due.hour()), due)
}

fn clock(due: DateTime<chrono_tz::Tz>) -> String {
    due.format("%H:%M").to_string()
}

fn yesterday(due: DateTime<chrono_tz::Tz>) -> i64 {
    due.timestamp() - 86_400
}

async fn settle() {
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

async fn advance(seconds: u64) {
    tokio::time::advance(Duration::from_secs(seconds)).await;
    settle().await;
}

/// The finished runs of `flow`, after a short real wait for the run task to end.
async fn done_runs(hub: &Hub, flow: &str) -> Vec<RunView> {
    tokio::time::sleep(Duration::from_millis(200)).await;
    settle().await;
    hub.flow(flow)
        .map(|view| view.runs)
        .unwrap_or_default()
        .into_iter()
        .filter(|run| run.status == RunStatus::Done)
        .collect()
}

#[tokio::test(start_paused = true)]
async fn restart_inside_window_fires_once() {
    let (cron, due) = daily_cron_ago(25);
    let retained = vec![
        flow_retained(
            "morning",
            flow_doc("morning", schedule_trigger("at", &cron, Some(60))),
        ),
        marker("morning", "at", yesterday(due)),
    ];
    let (hub, log) = boot(&retained);
    settle().await;
    advance(9).await;
    assert!(
        done_runs(&hub, "morning").await.is_empty(),
        "not before the settle"
    );
    advance(2).await;

    let runs = done_runs(&hub, "morning").await;
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(
        runs[0].steps[0].detail,
        format!("caught up: due {}", clock(due))
    );
    let published = marker_publishes(&log, "morning", "at");
    assert_eq!(published.len(), 1, "{published:?}");
    assert_eq!(marker_due(&published[0]), due.timestamp());
}

#[tokio::test(start_paused = true)]
async fn restart_outside_window_does_not_fire() {
    let (cron, due) = daily_cron_ago(90);
    let retained = vec![
        flow_retained(
            "morning",
            flow_doc("morning", schedule_trigger("at", &cron, Some(60))),
        ),
        marker("morning", "at", yesterday(due)),
    ];
    let (hub, log) = boot(&retained);
    settle().await;
    advance(11).await;

    assert!(done_runs(&hub, "morning").await.is_empty());
    assert!(marker_publishes(&log, "morning", "at").is_empty());
}

#[tokio::test(start_paused = true)]
async fn second_restart_does_not_refire() {
    let (cron, due) = daily_cron_ago(25);
    let initial = vec![
        flow_retained(
            "morning",
            flow_doc("morning", schedule_trigger("at", &cron, Some(60))),
        ),
        marker("morning", "at", yesterday(due)),
    ];
    let (hub, log) = boot(&initial);
    settle().await;
    advance(11).await;
    assert_eq!(done_runs(&hub, "morning").await.len(), 1);

    let held = retained_after(&initial, &log);
    let (second, second_log) = boot(&held);
    settle().await;
    advance(11).await;
    assert!(done_runs(&second, "morning").await.is_empty());
    assert!(marker_publishes(&second_log, "morning", "at").is_empty());
}

#[tokio::test(start_paused = true)]
async fn missed_ticks_fire_once() {
    let start = Utc::now();
    let retained = vec![
        flow_retained(
            "often",
            flow_doc("often", schedule_trigger("at", "*/5 * * * *", Some(60))),
        ),
        marker("often", "at", start.timestamp() - 50 * 60),
    ];
    let (hub, log) = boot(&retained);
    settle().await;
    advance(10).await;

    tokio::time::sleep(Duration::from_millis(200)).await;
    settle().await;
    let runs = hub.flow("often").unwrap().runs;
    assert_eq!(runs.len(), 1, "{runs:?}");
    // Only the latest missed tick is recorded, whichever path fired it.
    let latest = (start.timestamp() + 10) / 300 * 300;
    let published = marker_publishes(&log, "often", "at");
    assert_eq!(
        marker_due(published.last().unwrap()),
        latest,
        "{published:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn link_back_catches_up_skipped_tick() {
    let retained = vec![flow_retained(
        "often",
        flow_doc("often", schedule_trigger("at", "*/5 * * * *", Some(60))),
    )];
    let (hub, _log) = boot(&retained);
    settle().await;
    // The first check seeds the marker.
    advance(10).await;
    assert!(done_runs(&hub, "often").await.is_empty());

    hub.set_connection(ConnectionStatus::Offline {
        reason: "test".to_owned(),
    });
    settle().await;
    // At least one tick passes while the link is down: it is only recorded as skipped.
    advance(6 * 60).await;
    assert!(done_runs(&hub, "often").await.is_empty());

    hub.set_connection(ConnectionStatus::Online);
    settle().await;
    advance(10).await;
    assert_eq!(done_runs(&hub, "often").await.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn no_marker_seeds_without_firing() {
    let (cron, due) = daily_cron_ago(25);
    let retained = vec![flow_retained(
        "morning",
        flow_doc("morning", schedule_trigger("at", &cron, Some(60))),
    )];
    let (hub, log) = boot(&retained);
    settle().await;
    advance(11).await;

    assert!(done_runs(&hub, "morning").await.is_empty());
    let published = marker_publishes(&log, "morning", "at");
    assert_eq!(published.len(), 1, "{published:?}");
    assert_eq!(marker_due(&published[0]), due.timestamp());
}

#[tokio::test(start_paused = true)]
async fn renamed_node_seeds_without_firing() {
    let (cron, due) = daily_cron_ago(25);
    let retained = vec![
        flow_retained(
            "morning",
            flow_doc("morning", schedule_trigger("at", &cron, Some(60))),
        ),
        marker("morning", "at", yesterday(due)),
    ];
    let (hub, log) = boot(&retained);
    settle().await;

    let renamed = serde_json::json!({
        "id": "morning", "name": "morning", "enabled": true, "mode": "single",
        "nodes": [
            schedule_trigger("alarm", &cron, Some(60)),
            { "id": "on", "type": "command",
              "params": { "entity": LED_ID, "command": { "type": "turn_on" } } }
        ],
        "edges": [{ "from": "alarm", "to": "on" }]
    });
    let flow = serde_json::from_value(renamed).unwrap();
    hub.set_flow(flow).await.expect("saved");
    settle().await;
    assert_eq!(
        marker_publishes(&log, "morning", "at"),
        [""],
        "the old marker is cleared"
    );

    advance(11).await;
    assert!(done_runs(&hub, "morning").await.is_empty());
    let seeded = marker_publishes(&log, "morning", "alarm");
    assert_eq!(seeded.len(), 1, "{seeded:?}");
    assert!(marker_due(&seeded[0]) >= due.timestamp());
}

#[tokio::test(start_paused = true)]
async fn deleting_a_flow_clears_its_markers() {
    let (cron, due) = daily_cron_ago(25);
    let retained = vec![
        flow_retained(
            "morning",
            flow_doc("morning", schedule_trigger("at", &cron, Some(60))),
        ),
        marker("morning", "at", due.timestamp()),
    ];
    let (hub, log) = boot(&retained);
    settle().await;
    hub.delete_flow("morning").await.expect("deleted");
    settle().await;
    assert_eq!(marker_publishes(&log, "morning", "at"), [""]);
}

#[tokio::test(start_paused = true)]
async fn edit_never_catches_up() {
    let (hub, _log) = boot(&[]);
    settle().await;
    // The start-up check is over: nothing was there to catch up.
    advance(20).await;

    let (cron, due) = daily_cron_ago(25);
    hub.ingest(
        &marker_topic("morning", "at"),
        marker("morning", "at", yesterday(due)).1.as_bytes(),
    );
    hub.ingest(
        "nidoo/automation/morning",
        flow_doc("morning", schedule_trigger("at", &cron, Some(60))).as_bytes(),
    );
    settle().await;
    advance(30).await;

    assert!(done_runs(&hub, "morning").await.is_empty());
}

#[tokio::test(start_paused = true)]
async fn absent_option_keeps_old_behaviour() {
    let (cron, due) = daily_cron_ago(25);
    let retained = vec![
        flow_retained(
            "morning",
            flow_doc("morning", schedule_trigger("at", &cron, None)),
        ),
        marker("morning", "at", yesterday(due)),
    ];
    let (hub, log) = boot(&retained);
    settle().await;
    advance(11).await;

    assert!(done_runs(&hub, "morning").await.is_empty());
    assert!(
        log.lock()
            .unwrap()
            .iter()
            .all(|p| !p.topic.contains("/fired/")),
        "no marker is published without the option"
    );
}

#[tokio::test(start_paused = true)]
async fn wake_up_catch_up() {
    // The alarm rang 10 minutes ago, so a 15-minute pre-ring was due 25 minutes ago.
    let (_, ring) = daily_cron_ago(10);
    let settings = WakeUp {
        version: 1,
        hour: ring.hour() as u8,
        minute: ring.minute() as u8,
        days: vec![0, 1, 2, 3, 4, 5, 6],
        player: "music_node_player".to_owned(),
        ..Default::default()
    };
    let alarm =
        serde_json::to_string(&wake_up::to_flow("wake-up-1", "alarm", true, &settings)).unwrap();
    // A second alarm, outside the trigger's filter, whose pre-ring (20 minutes ago) is later
    // than the first one's and inside the window: it must not count.
    let (_, other_ring) = daily_cron_ago(5);
    let other_settings = WakeUp {
        hour: other_ring.hour() as u8,
        minute: other_ring.minute() as u8,
        ..settings.clone()
    };
    let other = serde_json::to_string(&wake_up::to_flow(
        "wake-up-2",
        "other",
        true,
        &other_settings,
    ))
    .unwrap();
    let heat = serde_json::json!({
        "id": "heat", "name": "heat", "enabled": true, "mode": "single",
        "nodes": [
            { "id": "at", "type": "trigger.wake_up",
              "params": { "before_minutes": 15, "alarms": ["wake-up-1"], "catch_up_minutes": 60 } },
            { "id": "on", "type": "command",
              "params": { "entity": LED_ID, "command": { "type": "turn_on" } } }
        ],
        "edges": [{ "from": "at", "to": "on" }]
    })
    .to_string();
    let due = ring - chrono::Duration::minutes(15);
    let retained = vec![
        flow_retained("wake-up-1", alarm),
        flow_retained("wake-up-2", other),
        flow_retained("heat", heat),
        marker("heat", "at", yesterday(due)),
    ];
    let (hub, log) = boot(&retained);
    settle().await;
    advance(11).await;

    let runs = done_runs(&hub, "heat").await;
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(
        runs[0].steps[0].detail,
        format!("caught up: due {}", clock(due))
    );
    assert_eq!(
        marker_due(&marker_publishes(&log, "heat", "at")[0]),
        due.timestamp()
    );
}

#[tokio::test(start_paused = true)]
async fn sun_catch_up() {
    // The sun event closest to 25 minutes ago, with the offset that lands the trigger there.
    let target = Utc::now() - chrono::Duration::minutes(25);
    let today = target.date_naive();
    let mut best: Option<(SunWhich, &str, DateTime<Utc>)> = None;
    for days in -1..=1 {
        let events = sun_events(LAT, LON, today + chrono::Duration::days(days));
        for (which, name, at) in [
            (SunWhich::Sunrise, "sunrise", events.sunrise),
            (SunWhich::Sunset, "sunset", events.sunset),
            (SunWhich::Dawn, "dawn", events.dawn),
            (SunWhich::Dusk, "dusk", events.dusk),
        ] {
            let Some(at) = at else { continue };
            if best.is_none_or(|(_, _, b)| (at - target).abs() < (b - target).abs()) {
                best = Some((which, name, at));
            }
        }
    }
    let (_, name, event) = best.expect("a sun event");
    let offset = (target - event).num_minutes();
    let due = (event + chrono::Duration::minutes(offset)).with_timezone(&Paris);
    let flow = serde_json::json!({
        "id": "sun", "name": "sun", "enabled": true, "mode": "single",
        "nodes": [
            { "id": "at", "type": "trigger.sun",
              "params": { "which": name, "offset_minutes": offset, "catch_up_minutes": 60 } },
            { "id": "on", "type": "command",
              "params": { "entity": LED_ID, "command": { "type": "turn_on" } } }
        ],
        "edges": [{ "from": "at", "to": "on" }]
    })
    .to_string();
    let retained = vec![
        flow_retained("sun", flow),
        marker("sun", "at", yesterday(due)),
    ];
    let (hub, log) = boot(&retained);
    settle().await;
    advance(11).await;

    let runs = done_runs(&hub, "sun").await;
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(
        runs[0].steps[0].detail,
        format!("caught up: due {}", clock(due))
    );
    let published = marker_publishes(&log, "sun", "at");
    assert_eq!(published.len(), 1, "{published:?}");
    assert_eq!(marker_due(&published[0]), due.timestamp());
}

/// A flow with a daily trigger 2 minutes ahead; runs until it has fired once, online.
async fn fire_normally(catch_up: Option<u32>) -> (Arc<Hub>, Log, DateTime<chrono_tz::Tz>) {
    let (cron, due) = daily_cron_ago(-2);
    let retained = vec![flow_retained(
        "morning",
        flow_doc("morning", schedule_trigger("at", &cron, catch_up)),
    )];
    let (hub, log) = boot(&retained);
    settle().await;
    advance(10).await;
    advance(180).await;
    (hub, log, due)
}

#[tokio::test(start_paused = true)]
async fn normal_fire_writes_marker() {
    let (hub, log, due) = fire_normally(Some(60)).await;

    assert_eq!(done_runs(&hub, "morning").await.len(), 1);
    let published = marker_publishes(&log, "morning", "at");
    assert_eq!(
        marker_due(published.last().unwrap()),
        due.timestamp(),
        "{published:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn normal_fire_without_option_writes_no_marker() {
    let (hub, log, _) = fire_normally(None).await;

    assert_eq!(done_runs(&hub, "morning").await.len(), 1);
    assert!(
        log.lock()
            .unwrap()
            .iter()
            .all(|p| !p.topic.contains("/fired/"))
    );
}

#[tokio::test(start_paused = true)]
async fn disabled_then_enabled_does_not_catch_up() {
    let (cron, due) = daily_cron_ago(25);
    let retained = vec![
        flow_retained(
            "morning",
            flow_doc("morning", schedule_trigger("at", &cron, Some(60))),
        ),
        marker("morning", "at", yesterday(due)),
    ];
    let (hub, log) = boot(&retained);
    settle().await;
    hub.set_flow_enabled("morning", false)
        .await
        .expect("disabled");
    settle().await;
    assert!(
        marker_publishes(&log, "morning", "at").is_empty(),
        "disabling leaves the marker alone"
    );
    hub.set_flow_enabled("morning", true)
        .await
        .expect("enabled");
    settle().await;
    advance(11).await;

    assert!(done_runs(&hub, "morning").await.is_empty());
    let published = marker_publishes(&log, "morning", "at");
    assert_eq!(published.len(), 1, "{published:?}");
    assert!(marker_due(&published[0]) >= due.timestamp());
}

/// The retained documents a broker would hold after `log`, with every marker moved `seconds`
/// back: as if the flow had been saved or enabled that long ago.
fn aged(initial: &[Retained], log: &Log, seconds: i64) -> Vec<Retained> {
    retained_after(initial, log)
        .into_iter()
        .map(|(topic, payload)| {
            if topic.contains("/fired/") {
                let due = marker_due(&payload) - seconds;
                (topic, format!(r#"{{"due":{due}}}"#))
            } else {
                (topic, payload)
            }
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn new_flow_catches_up_first_miss() {
    let (hub, log) = boot(&[]);
    settle().await;
    let (cron, due) = daily_cron_ago(25);
    let flow =
        serde_json::from_str(&flow_doc("fresh", schedule_trigger("at", &cron, Some(60)))).unwrap();
    hub.set_flow(flow).await.expect("saved");
    settle().await;
    let published = marker_publishes(&log, "fresh", "at");
    assert_eq!(published.len(), 1, "saving seeds the marker: {published:?}");
    let now = Utc::now().timestamp();
    assert!((now - 5..=now + 5).contains(&marker_due(&published[0])));

    // The hub went down before the first due: restart half an hour after the save.
    let retained = aged(&[], &log, 1800);
    let (hub, _log) = boot(&retained);
    settle().await;
    advance(11).await;
    let runs = done_runs(&hub, "fresh").await;
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(
        runs[0].steps[0].detail,
        format!("caught up: due {}", clock(due))
    );
}

#[tokio::test(start_paused = true)]
async fn reenabled_flow_catches_up_later_miss() {
    let (cron, due) = daily_cron_ago(25);
    let retained = vec![
        flow_retained(
            "morning",
            flow_doc("morning", schedule_trigger("at", &cron, Some(60))),
        ),
        marker("morning", "at", yesterday(due)),
    ];
    let (hub, log) = boot(&retained);
    settle().await;
    hub.set_flow_enabled("morning", false)
        .await
        .expect("disabled");
    hub.set_flow_enabled("morning", true)
        .await
        .expect("enabled");
    settle().await;

    // Enabling moved the marker to now; the hub then went down for half an hour.
    let held = aged(&retained, &log, 1800);
    let (hub, _log) = boot(&held);
    settle().await;
    advance(11).await;
    let runs = done_runs(&hub, "morning").await;
    assert_eq!(runs.len(), 1, "{runs:?}");
}

#[tokio::test(start_paused = true)]
async fn saving_a_disabled_flow_keeps_its_markers() {
    let (cron, due) = daily_cron_ago(25);
    let retained = vec![
        flow_retained(
            "morning",
            flow_doc("morning", schedule_trigger("at", &cron, Some(60))),
        ),
        marker("morning", "at", due.timestamp()),
    ];
    let (hub, log) = boot(&retained);
    settle().await;
    let mut flow = hub.flow("morning").unwrap().definition;
    flow.enabled = false;
    hub.set_flow(flow).await.expect("saved");
    settle().await;
    assert!(marker_publishes(&log, "morning", "at").is_empty());
}

#[tokio::test(start_paused = true)]
async fn sun_without_the_option_keeps_old_behaviour() {
    let flow = serde_json::json!({
        "id": "sun", "name": "sun", "enabled": true, "mode": "single",
        "nodes": [
            { "id": "at", "type": "trigger.sun",
              "params": { "which": "sunrise", "offset_minutes": 0 } },
            { "id": "on", "type": "command",
              "params": { "entity": LED_ID, "command": { "type": "turn_on" } } }
        ],
        "edges": [{ "from": "at", "to": "on" }]
    })
    .to_string();
    let retained = vec![
        flow_retained("sun", flow),
        marker("sun", "at", Utc::now().timestamp() - 86_400),
    ];
    let (hub, log) = boot(&retained);
    settle().await;
    advance(11).await;
    assert!(done_runs(&hub, "sun").await.is_empty());
    assert!(
        log.lock()
            .unwrap()
            .iter()
            .all(|p| !p.topic.contains("/fired/"))
    );
}

#[tokio::test(start_paused = true)]
async fn wake_up_without_the_option_keeps_old_behaviour() {
    let (_, ring) = daily_cron_ago(10);
    let settings = WakeUp {
        version: 1,
        hour: ring.hour() as u8,
        minute: ring.minute() as u8,
        days: vec![0, 1, 2, 3, 4, 5, 6],
        player: "music_node_player".to_owned(),
        ..Default::default()
    };
    let alarm =
        serde_json::to_string(&wake_up::to_flow("wake-up-1", "alarm", true, &settings)).unwrap();
    let heat = serde_json::json!({
        "id": "heat", "name": "heat", "enabled": true, "mode": "single",
        "nodes": [
            { "id": "at", "type": "trigger.wake_up",
              "params": { "before_minutes": 15, "alarms": ["wake-up-1"] } },
            { "id": "on", "type": "command",
              "params": { "entity": LED_ID, "command": { "type": "turn_on" } } }
        ],
        "edges": [{ "from": "at", "to": "on" }]
    })
    .to_string();
    let retained = vec![
        flow_retained("wake-up-1", alarm),
        flow_retained("heat", heat),
        marker("heat", "at", Utc::now().timestamp() - 86_400),
    ];
    let (hub, log) = boot(&retained);
    settle().await;
    advance(11).await;
    assert!(done_runs(&hub, "heat").await.is_empty());
    assert!(
        log.lock()
            .unwrap()
            .iter()
            .all(|p| !p.topic.contains("/fired/"))
    );
}
