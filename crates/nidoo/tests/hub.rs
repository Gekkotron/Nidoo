//! The hub driven without a broker: `ingest` feeds it, the publisher channel is inspected.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nidoo::config::Config;
use nidoo::history::History;
use nidoo::hub::{Hub, HubError};
use nidoo_api::{ConnectionStatus, VirtualKind, VirtualSpec, WsEvent};
use nidoo_core::command::Publish;
use nidoo_core::history::Incoming;
use nidoo_core::model::{Area, AreasDocument, Command, Kind, StateValue};
use nidoo_core::registry::Change;
use tokio::sync::mpsc;

/// A recorded ZBMINI switch: the `rooms/stair/light` device.
const SWITCH_FIXTURE: &str = "discovery/switch__0x847127fffe24e322__switch.json";
const SWITCH_ID: &str = "0x847127fffe24e322_switch_zigbee2mqtt";
const SWITCH_TOPIC: &str = "nidoo/switch/0x847127fffe24e322/switch/config";

fn fixtures() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../nidoo-core/tests/fixtures/"
    ))
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixtures().join(name)).expect("fixture")
}

fn config() -> Config {
    let mut cfg = Config::default();
    cfg.commands.confirm_timeout_ms = 50;
    cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
    cfg
}

fn hub() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    hub_with_timeout(50)
}

/// A hub whose broker link is up, so its writers publish.
fn hub_with_timeout(confirm_timeout_ms: u64) -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let mut cfg = config();
    cfg.commands.confirm_timeout_ms = confirm_timeout_ms;
    let (tx, rx) = mpsc::channel(64);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    (hub, rx)
}

/// The switch's `state_topic`, from its discovery config.
fn switch_state_topic() -> String {
    let config: serde_json::Value =
        serde_json::from_slice(&fixture(SWITCH_FIXTURE)).expect("switch config");
    config["state_topic"]
        .as_str()
        .expect("state_topic")
        .to_owned()
}

fn feed_switch(hub: &Hub) {
    hub.ingest(SWITCH_TOPIC, &fixture(SWITCH_FIXTURE));
}

async fn next_event(rx: &mut tokio::sync::broadcast::Receiver<WsEvent>) -> WsEvent {
    tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("an event within 1 s")
        .expect("open channel")
}

#[tokio::test]
async fn ingest_broadcasts_and_snapshots() {
    let (hub, _publisher) = hub();
    let mut events = hub.subscribe();

    hub.ingest(
        "zigbee2mqtt/bridge/devices",
        &fixture("bridge_devices.json"),
    );
    // Changes go out sorted, so the remotes seeded from the devices' exposed actions come
    // before the device list.
    let mut remotes = Vec::new();
    loop {
        match next_event(&mut events).await {
            WsEvent::EntityUpserted(view) if view.id.starts_with("device_automation.") => {
                remotes.push(view.id);
            }
            WsEvent::DevicesChanged { devices } => {
                assert_eq!(devices.len(), 66);
                break;
            }
            other => panic!("expected a seeded remote or DevicesChanged, got {other:?}"),
        }
    }
    assert!(!remotes.is_empty());

    feed_switch(&hub);
    match next_event(&mut events).await {
        WsEvent::EntityUpserted(view) => {
            assert_eq!(view.id, SWITCH_ID);
            assert_eq!(view.kind, Kind::Switch);
            assert_eq!(view.state.value, StateValue::Unknown);
        }
        other => panic!("expected EntityUpserted, got {other:?}"),
    }

    hub.ingest(
        &switch_state_topic(),
        &fixture("states/rooms__stair__light.json"),
    );
    match next_event(&mut events).await {
        WsEvent::StateChanged { id, state } => {
            assert_eq!(id, SWITCH_ID);
            assert_eq!(state.value, StateValue::OnOff(false));
        }
        other => panic!("expected StateChanged, got {other:?}"),
    }

    let snapshot = hub.snapshot();
    assert_eq!(snapshot.entities.len(), 1 + remotes.len());
    let switch = snapshot
        .entities
        .iter()
        .find(|e| e.id == SWITCH_ID)
        .expect("the switch");
    assert_eq!(switch.state.value, StateValue::OnOff(false));
    assert_eq!(snapshot.devices.len(), 66);
    assert!(hub.subscription_topics().contains(&switch_state_topic()));
}

#[tokio::test]
async fn set_override_publishes_retained_and_applies() {
    let (hub, mut publisher) = hub();
    feed_switch(&hub);

    let mut patch = serde_json::Map::new();
    patch.insert("showAs".to_owned(), serde_json::Value::from("light"));
    let view = hub.set_override(SWITCH_ID, &patch).await.expect("override");
    assert_eq!(view.kind, Kind::Light);
    assert_eq!(view.ui.show_as.as_deref(), Some("light"));
    assert_eq!(
        publisher.recv().await.expect("a publish"),
        Publish {
            topic: format!("nidoo/ui/{SWITCH_ID}"),
            payload: r#"{"asLight":true,"showAs":"light"}"#.to_owned(),
            retain: true,
        }
    );
    assert_eq!(
        hub.entity_view(SWITCH_ID).expect("view").kind,
        Kind::Light,
        "the registry reflects the override before the broker echoes it"
    );

    let mut clear = serde_json::Map::new();
    clear.insert("showAs".to_owned(), serde_json::Value::Null);
    let view = hub.set_override(SWITCH_ID, &clear).await.expect("override");
    assert_eq!(view.kind, Kind::Switch);
    assert_eq!(
        publisher.recv().await.expect("a publish"),
        Publish {
            topic: format!("nidoo/ui/{SWITCH_ID}"),
            payload: String::new(),
            retain: true,
        }
    );

    let mut bad = serde_json::Map::new();
    bad.insert("nope".to_owned(), serde_json::Value::Bool(true));
    assert!(matches!(
        hub.set_override(SWITCH_ID, &bad).await,
        Err(HubError::Patch(_))
    ));
    assert!(matches!(
        hub.set_override("missing", &patch).await,
        Err(HubError::UnknownEntity(id)) if id == "missing"
    ));
}

#[tokio::test]
async fn send_command_publishes_then_times_out() {
    let (hub, mut publisher) = hub();
    feed_switch(&hub);
    let mut events = hub.subscribe();

    let response = hub
        .send_command(
            SWITCH_ID,
            Command::TurnOn {
                brightness: None,
                color: None,
                effect: None,
            },
        )
        .await
        .expect("mapped and published");

    let published = publisher.try_recv().expect("the command publish");
    assert_eq!(published.topic, "zigbee2mqtt/rooms/stair/light/set");
    assert_eq!(published.payload, "ON");
    assert!(!published.retain);

    assert!(!response.accepted);
    assert_eq!(
        response.reason.as_deref(),
        Some("no state change within 50 ms")
    );
    assert_eq!(response.state, None);

    let mut failed = None;
    while let Ok(event) = events.try_recv() {
        if let WsEvent::CommandFailed { id, reason } = event {
            failed = Some((id, reason));
        }
    }
    assert_eq!(
        failed,
        Some((
            SWITCH_ID.to_owned(),
            "no state change within 50 ms".to_owned()
        ))
    );
}

#[tokio::test]
async fn send_command_confirmed_by_state() {
    let (hub, mut publisher) = hub();
    feed_switch(&hub);
    let state_topic = switch_state_topic();

    let echo = {
        let hub = Arc::clone(&hub);
        tokio::spawn(async move {
            let published = publisher.recv().await.expect("the command publish");
            assert_eq!(published.payload, "ON");
            hub.ingest(&state_topic, br#"{"state":"ON"}"#);
        })
    };

    let response = hub
        .send_command(
            SWITCH_ID,
            Command::TurnOn {
                brightness: None,
                color: None,
                effect: None,
            },
        )
        .await
        .expect("mapped and published");
    echo.await.expect("echo task");

    assert!(response.accepted, "{response:?}");
    assert_eq!(response.reason, None);
    assert_eq!(
        response.state.expect("confirmed state").value,
        StateValue::OnOff(true)
    );
}

#[tokio::test]
async fn command_to_offline_entity_is_rejected_without_publish() {
    let (hub, mut publisher) = hub();
    feed_switch(&hub);
    hub.ingest("zigbee2mqtt/bridge/state", br#"{"state":"offline"}"#);

    let result = hub.send_command(SWITCH_ID, Command::TurnOff).await;
    assert_eq!(
        result,
        Err(HubError::Rejected("entity is offline".to_owned()))
    );
    assert!(
        publisher.try_recv().is_err(),
        "nothing is published for an offline entity"
    );
}

#[tokio::test]
async fn unknown_entity_is_unknown_error() {
    let (hub, mut publisher) = hub();
    feed_switch(&hub);

    let result = hub.send_command("nope", Command::TurnOff).await;
    assert_eq!(result, Err(HubError::UnknownEntity("nope".to_owned())));
    assert!(publisher.try_recv().is_err());
    assert!(hub.entity_view("nope").is_none());
}

/// Sends `TurnOn` to the switch and ingests `echo` on its state topic as soon as the publish
/// is out. Returns the response and what the echo changed.
async fn turn_on_with_echo(
    hub: &Arc<Hub>,
    mut publisher: mpsc::Receiver<Publish>,
    echo: &'static [u8],
) -> (nidoo_api::CommandResponse, Vec<Change>) {
    let state_topic = switch_state_topic();
    let echo_task = {
        let hub = Arc::clone(hub);
        tokio::spawn(async move {
            let published = publisher.recv().await.expect("the command publish");
            assert_eq!(published.payload, "ON");
            hub.ingest(&state_topic, echo)
        })
    };
    let response = hub
        .send_command(
            SWITCH_ID,
            Command::TurnOn {
                brightness: None,
                color: None,
                effect: None,
            },
        )
        .await
        .expect("mapped and published");
    let changes = echo_task.await.expect("echo task");
    (response, changes)
}

/// `response` was accepted with the switch ON (a timeout would carry a reason).
fn assert_confirmed_on(response: &nidoo_api::CommandResponse) {
    assert!(response.accepted, "{response:?}");
    assert_eq!(response.reason, None);
    assert_eq!(
        response.state.as_ref().expect("current state").value,
        StateValue::OnOff(true)
    );
}

/// A message on the entity's topic that carries nothing for it (a partial update without
/// `state`, which the switch's template reads) is neither a change nor a report: the device
/// did not speak about this entity, so the command is not confirmed by it.
#[tokio::test]
async fn send_command_ignores_an_echo_without_its_key() {
    let (hub, publisher) = hub();
    feed_switch(&hub);
    hub.ingest(&switch_state_topic(), br#"{"state":"ON"}"#);

    let (response, changes) = turn_on_with_echo(&hub, publisher, br#"{"linkquality":120}"#).await;

    assert!(
        !changes.contains(&Change::StateChanged(SWITCH_ID.to_owned()))
            && !changes.contains(&Change::StateReported(SWITCH_ID.to_owned())),
        "the echo must say nothing about the switch for this test: {changes:?}"
    );
    assert!(!response.accepted, "{response:?}");
    assert_eq!(
        response.reason.as_deref(),
        Some("no state change within 50 ms")
    );
}

/// The message without the switch's key still reached its topic: it is announced as touched
/// (the liveness signal) but not as seen, so a command is not confirmed by it and nothing else
/// that reads `state_seen` hears it.
#[tokio::test]
async fn a_message_that_reads_nothing_is_touched_but_not_seen() {
    let (hub, _publisher) = hub();
    feed_switch(&hub);
    hub.ingest(&switch_state_topic(), br#"{"state":"ON"}"#);
    let mut seen = hub.subscribe_state_seen();
    let mut touched = hub.subscribe_state_touched();

    let changes = hub.ingest(&switch_state_topic(), br#"{"linkquality":120}"#);

    assert_eq!(changes, vec![Change::StateTouched(SWITCH_ID.to_owned())]);
    assert!(seen.try_recv().is_err(), "nothing was read");
    let signal = touched.try_recv().expect("touched");
    assert_eq!(signal.id, SWITCH_ID);
    assert!(!signal.read);

    hub.ingest(&switch_state_topic(), br#"{"state":"OFF"}"#);
    assert_eq!(seen.try_recv().expect("seen").id, SWITCH_ID);
    assert!(touched.try_recv().expect("touched").read);
}

/// Turning on a switch that is already ON: the echoed `{"state":"ON"}` is a
/// `StateReported` (nothing differs), which confirms the command without any event reaching
/// the WebSocket clients.
#[tokio::test]
async fn send_command_resolves_on_same_state_echo() {
    // A timeout well above the immediate echo: the echo must resolve it, not the clock.
    let (hub, publisher) = hub_with_timeout(500);
    feed_switch(&hub);
    hub.ingest(&switch_state_topic(), br#"{"state":"ON"}"#);
    let mut events = hub.subscribe();

    let (response, changes) = turn_on_with_echo(&hub, publisher, br#"{"state":"ON"}"#).await;

    assert_eq!(changes, vec![Change::StateReported(SWITCH_ID.to_owned())]);
    assert_confirmed_on(&response);
    assert!(
        events
            .try_recv()
            .is_err_and(|err| err == tokio::sync::broadcast::error::TryRecvError::Empty),
        "a report that changed nothing is not broadcast"
    );
}

/// Every Z2M entity lists `bridge/state` as an availability topic. A message there within the
/// window (a retained re-delivery of the same word, or the bridge going offline) says nothing
/// about the command: only a report on the entity's state topic confirms it.
#[tokio::test]
async fn availability_message_does_not_confirm_command() {
    let (hub, mut publisher) = hub_with_timeout(2_000);
    feed_switch(&hub);
    hub.ingest("zigbee2mqtt/bridge/state", br#"{"state":"online"}"#);
    hub.ingest(&switch_state_topic(), br#"{"state":"OFF"}"#);

    let pending = {
        let hub = Arc::clone(&hub);
        tokio::spawn(async move {
            hub.send_command(
                SWITCH_ID,
                Command::TurnOn {
                    brightness: None,
                    color: None,
                    effect: None,
                },
            )
            .await
        })
    };
    let published = tokio::time::timeout(Duration::from_secs(1), publisher.recv())
        .await
        .expect("published within 1 s")
        .expect("the command publish");
    assert_eq!(published.payload, "ON");

    hub.ingest("zigbee2mqtt/bridge/state", br#"{"state":"online"}"#);
    hub.ingest("zigbee2mqtt/bridge/state", br#"{"state":"offline"}"#);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !pending.is_finished(),
        "an availability message must not confirm the command"
    );

    hub.ingest("zigbee2mqtt/bridge/state", br#"{"state":"online"}"#);
    hub.ingest(&switch_state_topic(), br#"{"state":"ON"}"#);
    let response = pending
        .await
        .expect("command task")
        .expect("mapped and published");
    assert!(response.accepted, "{response:?}");
    assert_eq!(
        response.state.expect("confirmed state").value,
        StateValue::OnOff(true)
    );
}

/// While the broker link is down, every writer refuses with `HubError::Publish` before
/// anything is queued or applied locally: a write accepted during an outage would show in
/// the hub, never reach the broker and vanish at the next restart.
#[tokio::test]
async fn write_refused_while_offline() {
    let (hub, mut publisher) = hub();
    feed_switch(&hub);
    hub.set_connection(ConnectionStatus::Offline {
        reason: "connection reset".to_owned(),
    });
    let offline = HubError::Publish("broker offline".to_owned());

    let mut patch = serde_json::Map::new();
    patch.insert("favorite".to_owned(), serde_json::Value::Bool(true));
    assert_eq!(
        hub.set_override(SWITCH_ID, &patch).await.err(),
        Some(offline.clone())
    );
    assert!(
        !hub.entity_view(SWITCH_ID).expect("view").ui.favorite,
        "a refused override is not applied locally"
    );
    assert_eq!(
        hub.clear_override(SWITCH_ID).await.err(),
        Some(offline.clone())
    );
    assert_eq!(
        hub.set_areas(AreasDocument {
            areas: vec![Area {
                id: "stair".to_owned(),
                name: "Stair".to_owned(),
                ..Area::default()
            }],
        })
        .await
        .err(),
        Some(offline.clone())
    );
    let stair = hub
        .snapshot()
        .areas
        .into_iter()
        .find(|a| a.id == "stair")
        .expect("the switch's implicit area");
    assert_eq!(
        stair.name, "stair",
        "still implicit, not declared: {stair:?}"
    );
    assert_eq!(
        hub.send_command(SWITCH_ID, Command::TurnOff).await.err(),
        Some(offline.clone())
    );
    assert!(publisher.try_recv().is_err(), "nothing was queued");

    // Connecting is not online either; once online the same write goes through.
    hub.set_connection(ConnectionStatus::Connecting);
    assert_eq!(
        hub.set_override(SWITCH_ID, &patch).await.err(),
        Some(offline)
    );
    hub.set_connection(ConnectionStatus::Online);
    let view = hub.set_override(SWITCH_ID, &patch).await.expect("override");
    assert!(view.ui.favorite);
    assert_eq!(
        publisher.try_recv().expect("the override publish").payload,
        r#"{"favorite":true}"#
    );
}

/// The area written by an override is stored canonically: a declared name resolves to its
/// id (so the retained JSON and the hub agree), a blank string clears the field instead of
/// becoming an implicit area with an empty id.
#[tokio::test]
async fn set_override_canonicalises_area() {
    let (hub, mut publisher) = hub();
    feed_switch(&hub);
    hub.set_areas(AreasDocument {
        areas: vec![Area {
            id: "living".to_owned(),
            name: "Living".to_owned(),
            ..Area::default()
        }],
    })
    .await
    .expect("areas");
    publisher.try_recv().expect("the areas publish");

    let mut patch = serde_json::Map::new();
    patch.insert("area".to_owned(), serde_json::Value::from("Living"));
    let view = hub.set_override(SWITCH_ID, &patch).await.expect("override");
    assert_eq!(view.area, "living");
    assert_eq!(view.ui.area.as_deref(), Some("living"));
    assert_eq!(
        publisher.try_recv().expect("the override publish").payload,
        r#"{"area":"living"}"#
    );
    assert_eq!(
        hub.entity_detail(SWITCH_ID)
            .and_then(|d| d.raw_override)
            .and_then(|o| o.area),
        Some("living".to_owned())
    );

    // An undeclared area is kept as written (an implicit area).
    patch.insert("area".to_owned(), serde_json::Value::from("Attic"));
    let view = hub.set_override(SWITCH_ID, &patch).await.expect("override");
    assert_eq!(view.area, "Attic");
    publisher.try_recv().expect("the override publish");

    for blank in ["", "   "] {
        patch.insert("area".to_owned(), serde_json::Value::from(blank));
        let view = hub.set_override(SWITCH_ID, &patch).await.expect("override");
        assert_eq!(view.area, "stair", "{blank:?} clears the area");
        assert_eq!(view.ui.area, None);
        assert_eq!(
            publisher.try_recv().expect("the override publish").payload,
            "",
            "{blank:?} leaves a default override, retained empty"
        );
        assert!(hub.snapshot().areas.iter().all(|a| !a.id.trim().is_empty()));
    }
}

/// A discovery payload that cannot become an entity is logged once with its topic: not again
/// on re-delivery, and again only after the
/// topic recovered and broke anew.
#[tokio::test]
async fn invalid_discovery_is_logged_once() {
    let (hub, _publisher) = hub();
    assert_eq!(hub.logged_ignored_len(), 0);

    let garbage: &[u8] = &[0xff, 0xfe, b'{'];
    hub.ingest("nidoo/sensor/garbage/x/config", garbage);
    assert_eq!(hub.ignored().len(), 1);
    assert_eq!(hub.logged_ignored_len(), 1);
    hub.ingest("nidoo/sensor/garbage/x/config", garbage);
    assert_eq!(
        hub.logged_ignored_len(),
        1,
        "a re-delivery is not logged again"
    );

    hub.ingest(
        "nidoo/update/x/config",
        br#"{"unique_id":"u","state_topic":"zigbee2mqtt/u"}"#,
    );
    assert_eq!(
        hub.ignored().len(),
        1,
        "an update is an entity, not ignored"
    );
    assert_eq!(hub.logged_ignored_len(), 1, "a valid config is not logged");

    hub.ingest(
        "nidoo/sensor/garbage/x/config",
        br#"{"unique_id":"g","state_topic":"zigbee2mqtt/g"}"#,
    );
    assert_eq!(hub.ignored().len(), 0);
    assert_eq!(
        hub.logged_ignored_len(),
        0,
        "a topic that recovered is forgotten"
    );

    hub.ingest("nidoo/sensor/garbage/x/config", garbage);
    assert_eq!(
        hub.logged_ignored_len(),
        1,
        "and logged again when it breaks anew"
    );
}

// ----- push alerts -----

const HISTORY_TOPIC: &str = "nidoo/push/history";
const ALERT_ENTRY: &str = r#"{"v":1,"id":"frigate-front-1696","channel":"camera","severity":"warning","title":"Front Door","body":"Person in Porch","image_url":"http://192.168.1.20:5000/api/events/1696/snapshot.jpg","actions":[{"id":"disarm","label":"Disarm"}],"silent":false,"sent_at":1760880000,"source":"frigate"}"#;

fn history_payload() -> Vec<u8> {
    format!("[{ALERT_ENTRY}]").into_bytes()
}

#[tokio::test]
async fn history_payload_fills_alerts_and_broadcasts() {
    let (hub, _publisher) = hub();
    let mut events = hub.subscribe();

    let changes = hub.ingest(HISTORY_TOPIC, &history_payload());

    assert!(changes.is_empty(), "no registry change: {changes:?}");
    match next_event(&mut events).await {
        WsEvent::AlertsChanged { alerts } => {
            assert_eq!(alerts.len(), 1);
            assert_eq!(alerts[0].id, "frigate-front-1696");
            assert_eq!(hub.alerts(), alerts);
            assert_eq!(hub.snapshot().alerts, alerts);
        }
        other => panic!("expected AlertsChanged, got {other:?}"),
    }
}

#[tokio::test]
async fn identical_history_does_not_broadcast_again() {
    let (hub, _publisher) = hub();
    let mut events = hub.subscribe();
    hub.ingest(HISTORY_TOPIC, &history_payload());
    next_event(&mut events).await;

    hub.ingest(HISTORY_TOPIC, &history_payload());

    assert!(
        tokio::time::timeout(Duration::from_millis(100), events.recv())
            .await
            .is_err(),
        "an unchanged history must not be re-broadcast"
    );
}

#[tokio::test]
async fn empty_history_payload_clears_the_list() {
    let (hub, _publisher) = hub();
    let mut events = hub.subscribe();
    hub.ingest(HISTORY_TOPIC, &history_payload());
    next_event(&mut events).await;

    hub.ingest(HISTORY_TOPIC, b"");

    assert!(matches!(
        next_event(&mut events).await,
        WsEvent::AlertsChanged { alerts } if alerts.is_empty()
    ));
    assert!(hub.alerts().is_empty());
}

#[tokio::test]
async fn non_array_history_keeps_previous_list() {
    let (hub, _publisher) = hub();
    let mut events = hub.subscribe();
    hub.ingest(HISTORY_TOPIC, &history_payload());
    next_event(&mut events).await;

    hub.ingest(HISTORY_TOPIC, b"{}");

    assert_eq!(hub.alerts().len(), 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), events.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn reply_publishes_the_app_payload() {
    let (hub, mut publisher) = hub();
    hub.ingest(HISTORY_TOPIC, &history_payload());

    hub.reply("frigate-front-1696", "disarm")
        .expect("reply accepted");

    let published = publisher.recv().await.expect("a publish");
    assert_eq!(published.topic, "nidoo/push/responses");
    assert!(!published.retain);
    let value: serde_json::Value = serde_json::from_str(&published.payload).expect("json");
    assert_eq!(value["id"], "frigate-front-1696");
    assert_eq!(value["action"], "disarm");
    assert_eq!(value["install_id"], "nidoo-web");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    let ts = value["ts"].as_u64().expect("ts in seconds");
    assert!(now.abs_diff(ts) <= 5, "ts {ts} is not within 5 s of {now}");
    let raw = &published.payload;
    let pos = |key: &str| raw.find(key).expect(key);
    assert!(pos("\"id\"") < pos("\"action\""));
    assert!(pos("\"action\"") < pos("\"install_id\""));
    assert!(pos("\"install_id\"") < pos("\"ts\""));
}

#[tokio::test]
async fn reply_rejects_unknown_alert_action_and_offline() {
    let (hub, _publisher) = hub();
    hub.ingest(HISTORY_TOPIC, &history_payload());

    assert!(matches!(
        hub.reply("nope", "disarm"),
        Err(HubError::UnknownAlert(id)) if id == "nope"
    ));
    assert!(matches!(
        hub.reply("frigate-front-1696", "zzz"),
        Err(HubError::UnknownAction(action)) if action == "zzz"
    ));

    hub.set_connection(ConnectionStatus::Offline {
        reason: "test".into(),
    });
    assert!(matches!(
        hub.reply("frigate-front-1696", "disarm"),
        Err(HubError::Publish(_))
    ));
}

#[tokio::test]
async fn push_history_topic_feeds_alert_store() {
    let (hub, _publisher) = hub();
    assert_eq!(hub.push().history_topic(), HISTORY_TOPIC);

    // The broker's echo of the hub's own retained publish arrives live (retain = false).
    hub.ingest_message(HISTORY_TOPIC, &history_payload(), false);
    assert_eq!(hub.alerts().len(), 1);
    assert_eq!(hub.alerts()[0].id, "frigate-front-1696");
}

#[tokio::test]
async fn push_topics_are_not_discovery() {
    assert!(
        nidoo_core::registry::RegistryConfig::default_reserved().contains(&"push".to_owned()),
        "push is a reserved sub-tree of the discovery prefix"
    );
    let (hub, _publisher) = hub();
    hub.ingest_message(
        "nidoo/push/tokens/a",
        br#"{"token":"t","device_name":"Pixel","lang":"fr","updated_at":1}"#,
        true,
    );
    hub.ingest_message("nidoo/push/notification/flow", br#"{"title":"T"}"#, false);
    hub.ingest_message("nidoo/push/sensor/config", br#"{"name":"x"}"#, true);
    assert!(hub.ignored().is_empty(), "{:?}", hub.ignored());
    assert_eq!(hub.counts().0, 0);
}

#[tokio::test(start_paused = true)]
async fn notify_publishes_history_after_sync() {
    let (hub, mut publisher) = hub();
    hub.on_connected();
    hub.on_suback();
    tokio::time::sleep(Duration::from_secs(2)).await;
    tokio::task::yield_now().await;

    let id = hub
        .notify(
            "flow",
            &serde_json::json!({"v": 1, "id": "x", "title": "T"}),
        )
        .expect("accepted");
    assert_eq!(id, "x");

    let published = drain(&mut publisher);
    let last = published
        .iter()
        .rev()
        .find(|p| p.topic == "nidoo/push/history")
        .unwrap_or_else(|| panic!("a history publish in {published:?}"));
    assert!(last.retain);
    let list: serde_json::Value = serde_json::from_str(&last.payload).expect("json");
    assert_eq!(list[0]["id"], "x", "{list}");
    assert_eq!(list[0]["title"], "T");
}

/// A hub whose `[push] credentials` point at `path`.
fn hub_with_credentials(path: PathBuf) -> Arc<Hub> {
    let mut cfg = config();
    cfg.push.credentials = Some(path);
    let (tx, _rx) = mpsc::channel(64);
    Hub::new(&cfg, tx)
}

#[tokio::test]
async fn unreadable_credentials_keep_push_off() {
    let missing = std::env::temp_dir().join("nidoo-push-test-no-such-key.json");
    let hub = hub_with_credentials(missing);
    assert_eq!(hub.push().status().state, nidoo_push::PushState::Off);

    let dir = std::env::temp_dir().join(format!("nidoo-push-bad-key-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let bad = dir.join("key.json");
    std::fs::write(&bad, "not json").expect("write");
    let hub = hub_with_credentials(bad);
    assert_eq!(hub.push().status().state, nidoo_push::PushState::Off);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn readable_credentials_turn_push_on() {
    let dir = std::env::temp_dir().join(format!("nidoo-push-key-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let key = dir.join("key.json");
    std::fs::write(
        &key,
        r#"{"project_id":"p","client_email":"e@p.iam.gserviceaccount.com","private_key":"k","token_uri":"https://oauth2.googleapis.com/token"}"#,
    )
    .expect("write");
    let hub = hub_with_credentials(key);
    assert_eq!(hub.push().status().state, nidoo_push::PushState::Syncing);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn notify_rejects_an_invalid_payload() {
    let (hub, _publisher) = hub();
    assert!(hub.notify("flow", &serde_json::json!({"v": 1})).is_err());
}

#[tokio::test]
async fn alert_action_replies_on_new_responses_topic() {
    let (hub, mut publisher) = hub();
    hub.ingest(HISTORY_TOPIC, &history_payload());

    hub.reply("frigate-front-1696", "disarm")
        .expect("reply accepted");

    let published = publisher.recv().await.expect("a publish");
    assert_eq!(published.topic, hub.push().responses_topic());
    assert_eq!(published.topic, "nidoo/push/responses");
}

fn virtual_switch() -> VirtualSpec {
    VirtualSpec {
        kind: VirtualKind::Switch,
        name: "Force heating".to_owned(),
        min: None,
        max: None,
        step: None,
        unit: None,
        options: None,
        initial: None,
    }
}

const VIRTUAL_STATE: &str = "nidoo/virtual/force_heating";
const VIRTUAL_CONFIG: &str = "nidoo/switch/virtual/force_heating/config";

fn drain(publisher: &mut mpsc::Receiver<Publish>) -> Vec<Publish> {
    let mut out = Vec::new();
    while let Ok(publish) = publisher.try_recv() {
        out.push(publish);
    }
    out
}

#[tokio::test]
async fn creating_publishes_config_and_initial_off() {
    let (hub, mut publisher) = hub();
    let view = hub
        .set_virtual("force_heating", virtual_switch())
        .await
        .expect("created");
    assert_eq!(view.id, "virtual_force_heating");
    assert_eq!(view.kind, Kind::Switch);
    assert_eq!(view.state.value, StateValue::OnOff(false));

    let published = drain(&mut publisher);
    assert_eq!(published.len(), 2, "{published:?}");
    assert_eq!(published[0].topic, VIRTUAL_CONFIG);
    assert!(published[0].retain);
    assert_eq!(published[1].topic, VIRTUAL_STATE);
    assert_eq!(published[1].payload, "OFF");
    assert!(published[1].retain);
}

#[tokio::test]
async fn recreating_keeps_the_state() {
    let (hub, mut publisher) = hub();
    hub.set_virtual("force_heating", virtual_switch())
        .await
        .expect("created");
    hub.ingest(VIRTUAL_STATE, b"ON");
    drain(&mut publisher);

    let mut renamed = virtual_switch();
    renamed.name = "Force the heating".to_owned();
    let view = hub
        .set_virtual("force_heating", renamed)
        .await
        .expect("recreated");
    assert_eq!(view.label.as_deref(), Some("Force the heating"));
    assert_eq!(view.state.value, StateValue::OnOff(true));
    let published = drain(&mut publisher);
    assert_eq!(published.len(), 1, "{published:?}");
    assert_eq!(published[0].topic, VIRTUAL_CONFIG);
}

#[tokio::test]
async fn number_defaults_and_initial_override() {
    let (hub, mut publisher) = hub();
    let spec = VirtualSpec {
        kind: VirtualKind::Number,
        name: "Target".to_owned(),
        min: Some(10.0),
        max: Some(30.0),
        step: Some(0.5),
        unit: Some("°C".to_owned()),
        options: None,
        initial: None,
    };
    let view = hub
        .set_virtual("target", spec.clone())
        .await
        .expect("number");
    assert_eq!(view.kind, Kind::Number);
    let published = drain(&mut publisher);
    assert_eq!(published[0].topic, "nidoo/number/virtual/target/config");
    assert_eq!(published[1].topic, "nidoo/virtual/target");
    assert_eq!(published[1].payload, "10");

    let mut bad = spec.clone();
    bad.initial = Some(serde_json::json!(99));
    assert!(matches!(
        hub.set_virtual("other", bad).await,
        Err(HubError::Rejected(_))
    ));
    assert!(drain(&mut publisher).is_empty());
}

#[tokio::test]
async fn bad_virtual_id_is_rejected_without_publish() {
    let (hub, mut publisher) = hub();
    for id in ["Force", "", "a/b", &"x".repeat(49)] {
        assert!(
            matches!(
                hub.set_virtual(id, virtual_switch()).await,
                Err(HubError::Rejected(_))
            ),
            "{id}"
        );
    }
    assert!(drain(&mut publisher).is_empty());
}

#[tokio::test]
async fn virtual_switch_command_is_accepted_and_retained() {
    let (hub, mut publisher) = hub_with_timeout(500);
    hub.set_virtual("force_heating", virtual_switch())
        .await
        .expect("created");
    drain(&mut publisher);

    let echo_hub = Arc::clone(&hub);
    let echo = tokio::spawn(async move {
        let published = publisher.recv().await.expect("the command publish");
        assert_eq!(published.topic, VIRTUAL_STATE);
        assert_eq!(published.payload, "ON");
        assert!(published.retain, "a virtual command is retained");
        echo_hub.ingest(VIRTUAL_STATE, b"ON");
    });
    let response = hub
        .send_command(
            "virtual_force_heating",
            Command::TurnOn {
                brightness: None,
                color: None,
                effect: None,
            },
        )
        .await
        .expect("published");
    echo.await.expect("echo task");
    assert_confirmed_on(&response);
}

fn house_mode(options: &[&str]) -> VirtualSpec {
    VirtualSpec {
        kind: VirtualKind::Select,
        name: "House mode".to_owned(),
        min: None,
        max: None,
        step: None,
        unit: None,
        options: Some(options.iter().map(|o| (*o).to_owned()).collect()),
        initial: None,
    }
}

const MODE_STATE: &str = "nidoo/virtual/house_mode";
const MODE_CONFIG: &str = "nidoo/select/virtual/house_mode/config";

#[tokio::test]
async fn select_virtual_is_created_and_commanded() {
    let (hub, mut publisher) = hub_with_timeout(500);
    let view = hub
        .set_virtual("house_mode", house_mode(&["home", "away", "night"]))
        .await
        .expect("created");
    assert_eq!(view.kind, Kind::Select);
    assert_eq!(view.state.value, StateValue::Text("home".to_owned()));
    let published = drain(&mut publisher);
    assert_eq!(published.len(), 2, "{published:?}");
    assert_eq!(published[0].topic, MODE_CONFIG);
    assert!(published[0].retain);
    assert_eq!(published[1].topic, MODE_STATE);
    assert_eq!(published[1].payload, "home");
    assert!(published[1].retain);

    let echo_hub = Arc::clone(&hub);
    let echo = tokio::spawn(async move {
        let published = publisher.recv().await.expect("the command publish");
        assert_eq!(published.topic, MODE_STATE);
        assert_eq!(published.payload, "away");
        echo_hub.ingest(MODE_STATE, b"away");
        publisher
    });
    hub.send_command(
        "virtual_house_mode",
        Command::SelectOption {
            option: "away".to_owned(),
        },
    )
    .await
    .expect("published");
    let mut publisher = echo.await.expect("echo task");

    let result = hub
        .send_command(
            "virtual_house_mode",
            Command::SelectOption {
                option: "party".to_owned(),
            },
        )
        .await;
    assert!(matches!(result, Err(HubError::Command(_))), "{result:?}");
    assert!(drain(&mut publisher).is_empty());
}

#[tokio::test]
async fn select_options_change_resets_a_missing_state() {
    let (hub, mut publisher) = hub();
    hub.set_virtual("house_mode", house_mode(&["home", "away"]))
        .await
        .expect("created");
    hub.ingest(MODE_STATE, b"away");
    drain(&mut publisher);

    let view = hub
        .set_virtual("house_mode", house_mode(&["home", "night"]))
        .await
        .expect("redefined");
    assert_eq!(view.state.value, StateValue::Text("home".to_owned()));
    let published = drain(&mut publisher);
    assert_eq!(published.len(), 2, "{published:?}");
    assert_eq!(published[0].topic, MODE_CONFIG);
    assert_eq!(published[1].topic, MODE_STATE);
    assert_eq!(published[1].payload, "home");
    assert!(published[1].retain);

    hub.set_virtual("house_mode", house_mode(&["home", "night"]))
        .await
        .expect("redefined again");
    let published = drain(&mut publisher);
    assert_eq!(published.len(), 1, "{published:?}");
    assert_eq!(published[0].topic, MODE_CONFIG);
}

#[tokio::test]
async fn select_reset_goes_to_the_first_option_not_initial() {
    let (hub, mut publisher) = hub();
    hub.set_virtual("house_mode", house_mode(&["home", "away"]))
        .await
        .expect("created");
    hub.ingest(MODE_STATE, b"away");
    drain(&mut publisher);

    let mut spec = house_mode(&["home", "night"]);
    spec.initial = Some(serde_json::json!("night"));
    let view = hub
        .set_virtual("house_mode", spec)
        .await
        .expect("redefined");
    assert_eq!(view.state.value, StateValue::Text("home".to_owned()));
    let published = drain(&mut publisher);
    assert_eq!(published.len(), 2, "{published:?}");
    assert_eq!(published[1].topic, MODE_STATE);
    assert_eq!(published[1].payload, "home");
}

#[tokio::test]
async fn virtual_topics_are_not_ignored_discovery() {
    let (hub, _publisher) = hub();
    hub.ingest("nidoo/virtual/x", b"ON");
    hub.ingest("nidoo/virtual/x/extra/config", b"{}");
    assert!(hub.ignored().is_empty(), "{:?}", hub.ignored());
}

#[tokio::test]
async fn delete_virtual_clears_both_topics() {
    let (hub, mut publisher) = hub();
    hub.set_virtual("force_heating", virtual_switch())
        .await
        .expect("created");
    drain(&mut publisher);

    hub.delete_virtual("force_heating").await.expect("deleted");
    let published = drain(&mut publisher);
    let topics: Vec<&str> = published.iter().map(|p| p.topic.as_str()).collect();
    assert_eq!(topics, [VIRTUAL_CONFIG, VIRTUAL_STATE]);
    assert!(published.iter().all(|p| p.payload.is_empty() && p.retain));
    assert!(hub.entity_view("virtual_force_heating").is_none());

    assert!(matches!(
        hub.delete_virtual("force_heating").await,
        Err(HubError::UnknownEntity(_))
    ));
    // A real device is not a virtual entity.
    feed_switch(&hub);
    assert!(matches!(
        hub.delete_virtual(SWITCH_ID).await,
        Err(HubError::UnknownEntity(_))
    ));
}

#[tokio::test]
async fn virtual_button_has_no_state_and_no_retain() {
    let (hub, mut publisher) = hub();
    let spec = VirtualSpec {
        kind: VirtualKind::Button,
        name: "Boost".to_owned(),
        min: None,
        max: None,
        step: None,
        unit: None,
        options: None,
        initial: None,
    };
    let view = hub.set_virtual("boost", spec).await.expect("button");
    assert_eq!(view.kind, Kind::Button);
    let published = drain(&mut publisher);
    assert_eq!(published.len(), 1, "{published:?}");
    assert_eq!(published[0].topic, "nidoo/button/virtual/boost/config");
}

#[tokio::test]
async fn state_change_feeds_history() {
    let (hub, _publisher) = hub();
    feed_switch(&hub);
    let (history, mut rx) = History::detached(8, chrono_tz::UTC);
    hub.set_history(history);

    hub.ingest(&switch_state_topic(), br#"{"state":"ON"}"#);
    let got = tokio::time::timeout(Duration::from_millis(100), rx.recv())
        .await
        .expect("an incoming within 100 ms")
        .expect("open channel");
    assert_eq!(got.id, SWITCH_ID);
    assert_eq!(got.state.value, StateValue::OnOff(true));

    // Second time the same payload yields a StateReported, not StateChanged — no offer.
    hub.ingest(&switch_state_topic(), br#"{"state":"ON"}"#);
    assert!(matches!(
        rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn offer_never_blocks_when_full() {
    let (history, mut rx) = History::detached(1, chrono_tz::UTC);

    fn incoming(at: i64) -> Incoming {
        Incoming {
            at,
            id: "x".into(),
            state: nidoo_core::model::EntityState {
                value: StateValue::OnOff(true),
                attributes: serde_json::Map::new(),
                availability: nidoo_core::model::Availability::Online,
                last_update: Some((at / 1_000_000) as u64),
            },
            num: None,
        }
    }

    tokio::time::timeout(Duration::from_millis(10), async {
        history.offer(incoming(1));
        history.offer(incoming(2));
        history.offer(incoming(3));
    })
    .await
    .expect("offer never blocks");

    assert!(rx.try_recv().is_ok());
    assert!(matches!(
        rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
}

/// Feeds a hand-written generic entity (`unique_id` `u1`) and returns its id.
fn feed_generic(hub: &Hub, component: &str, config: serde_json::Value) -> String {
    let topic = format!("nidoo/{component}/u1/main/config");
    hub.ingest(&topic, config.to_string().as_bytes());
    let snapshot = hub.snapshot();
    snapshot
        .entities
        .iter()
        .find(|e| e.component == component)
        .unwrap_or_else(|| panic!("no {component} entity"))
        .id
        .clone()
}

fn control(name: &str, value: Option<serde_json::Value>) -> Command {
    Command::Control {
        control: name.to_owned(),
        value,
    }
}

#[tokio::test]
async fn retained_group_definition_is_loaded() {
    let (hub, _rx) = hub();
    let changes = hub.ingest(
        "nidoo/group/living/definition",
        br#"{"name":"Living lights","kind":"light","members":[]}"#,
    );
    assert!(changes.is_empty());
    assert!(hub.read_group_store().get("living").is_some());
    hub.ingest("nidoo/group/_migrated", b"1");
    assert!(hub.read_group_store().migrated());
}

fn groups_hub() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let mut cfg = config();
    cfg.groups.enabled = true;
    let (tx, rx) = mpsc::channel(64);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    feed_switch(&hub);
    (hub, rx)
}

fn switch_group() -> nidoo_api::GroupSpec {
    nidoo_api::GroupSpec {
        kind: Kind::Switch,
        name: "Stair".to_owned(),
        members: vec![SWITCH_ID.to_owned()],
        hide_members: true,
    }
}

#[tokio::test]
async fn scene_activate_is_accepted_without_echo() {
    let (hub, mut publisher) = hub_with_timeout(5_000);
    let id = feed_generic(
        &hub,
        "scene",
        serde_json::json!({"name": "Movie", "unique_id": "u1", "command_topic": "sc/set"}),
    );
    let started = std::time::Instant::now();
    let response = hub
        .send_command(&id, control("activate", None))
        .await
        .expect("accepted");
    assert!(response.accepted);
    assert_eq!(response.reason, None);
    assert_eq!(response.state, None);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(publisher.try_recv().expect("published").topic, "sc/set");
}

#[tokio::test]
async fn discovered_button_press_is_accepted_without_echo() {
    let (hub, mut publisher) = hub_with_timeout(5_000);
    let id = feed_generic(
        &hub,
        "button",
        serde_json::json!({"name": "Wake NAS", "unique_id": "u1",
                           "command_topic": "nas-waker/button/wake_nas/command"}),
    );
    let mut events = hub.subscribe();
    let started = std::time::Instant::now();
    let response = hub
        .send_command(&id, Command::Press)
        .await
        .expect("accepted");
    assert!(response.accepted, "{response:?}");
    assert_eq!(response.reason, None);
    assert_eq!(response.state, None);
    assert!(started.elapsed() < Duration::from_secs(1));
    let sent = publisher.try_recv().expect("published");
    assert_eq!(sent.topic, "nas-waker/button/wake_nas/command");
    assert_eq!(sent.payload, "PRESS");
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event, WsEvent::CommandFailed { .. }), "{event:?}");
    }
}

#[tokio::test]
async fn virtual_button_press_is_accepted_without_echo() {
    let (hub, mut publisher) = hub_with_timeout(5_000);
    let spec = VirtualSpec {
        kind: VirtualKind::Button,
        name: "Boost".to_owned(),
        min: None,
        max: None,
        step: None,
        unit: None,
        options: None,
        initial: None,
    };
    let view = hub.set_virtual("boost", spec).await.expect("button");
    drain(&mut publisher);
    let started = std::time::Instant::now();
    let response = hub
        .send_command(&view.id, Command::Press)
        .await
        .expect("accepted");
    assert!(response.accepted, "{response:?}");
    assert_eq!(response.state, None);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(publisher.try_recv().expect("published").payload, "PRESS");
}

#[tokio::test]
async fn lock_command_waits_for_echo() {
    let (hub, _publisher) = hub();
    let id = feed_generic(
        &hub,
        "lock",
        serde_json::json!({"name": "Door", "unique_id": "u1", "state_topic": "l/s",
                           "command_topic": "l/set"}),
    );
    let response = hub
        .send_command(&id, control("locked", Some(serde_json::json!(true))))
        .await
        .expect("mapped");
    assert!(!response.accepted);
}

fn secret_text_config() -> serde_json::Value {
    serde_json::json!({"name": "Code", "unique_id": "u1", "mode": "password",
                       "state_topic": "t/s", "command_topic": "t/set"})
}

#[tokio::test]
async fn secret_text_is_masked_in_state_changed_events() {
    let (hub, _publisher) = hub();
    let id = feed_generic(&hub, "text", secret_text_config());
    let mut events = hub.subscribe();
    hub.ingest("t/s", b"hunter2");
    loop {
        match next_event(&mut events).await {
            WsEvent::StateChanged { id: changed, state } if changed == id => {
                let StateValue::Generic(gs) = state.value else {
                    panic!("generic state");
                };
                assert_eq!(
                    gs.fields.get("value"),
                    Some(&nidoo_core::model::GenericValue::Text("••••".to_owned()))
                );
                break;
            }
            _ => {}
        }
    }
}

#[tokio::test]
async fn secret_text_never_reaches_history() {
    let (hub, _publisher) = hub();
    let (history, mut rx) = History::detached(8, chrono_tz::UTC);
    hub.set_history(history);
    feed_generic(&hub, "text", secret_text_config());
    hub.ingest("t/s", b"hunter2");
    assert!(matches!(
        rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn set_group_with_the_switch_off_is_refused_without_publish() {
    let (hub, mut rx) = hub();
    feed_switch(&hub);
    assert!(matches!(
        hub.set_group("stair", switch_group()).await,
        Err(HubError::Publish(_))
    ));
    assert!(drain(&mut rx).is_empty());
    assert!(matches!(hub.list_groups(), Err(HubError::Publish(_))));
    assert!(matches!(
        hub.delete_group("stair").await,
        Err(HubError::Publish(_))
    ));
}

#[tokio::test]
async fn type_table_is_published_retained_on_connect() {
    let (hub, mut publisher) = hub();
    hub.publish_types().await.expect("publish");
    let sent = publisher.try_recv().expect("a publish");
    assert_eq!(sent.topic, "nidoo/ui/_types");
    assert!(sent.retain);
    assert_eq!(sent.payload, nidoo_core::types::table_json());
}

#[tokio::test]
async fn type_table_echo_is_not_an_override() {
    let (hub, _publisher) = hub();
    let before = hub.snapshot();
    hub.ingest(
        "nidoo/ui/_types",
        nidoo_core::types::table_json().as_bytes(),
    );
    assert_eq!(hub.snapshot(), before);
}

#[tokio::test]
async fn set_group_rejects_bad_specs_without_publish() {
    let (hub, mut rx) = groups_hub();
    let mut sensor = switch_group();
    sensor.kind = Kind::Sensor;
    assert!(matches!(
        hub.set_group("stair", sensor).await,
        Err(HubError::Rejected(_))
    ));
    let mut nested = switch_group();
    nested.members = vec!["group_other".to_owned()];
    assert!(matches!(
        hub.set_group("stair", nested).await,
        Err(HubError::Rejected(_))
    ));
    let mut wrong_kind = switch_group();
    wrong_kind.kind = Kind::Light;
    assert!(matches!(
        hub.set_group("stair", wrong_kind).await,
        Err(HubError::Rejected(_))
    ));
    assert!(matches!(
        hub.set_group("BAD-id", switch_group()).await,
        Err(HubError::Rejected(_))
    ));
    assert!(drain(&mut rx).is_empty());
}

#[tokio::test]
async fn set_group_publishes_definition_config_availability_and_state() {
    let (hub, mut rx) = groups_hub();
    let view = hub
        .set_group("stair", switch_group())
        .await
        .expect("created");
    assert_eq!(view.id, "stair");
    assert_eq!(view.entity.id, "group_stair");
    assert_eq!(view.members.len(), 1);
    assert!(view.members[0].present && !view.members[0].mismatch);
    let published = drain(&mut rx);
    let topics: Vec<&str> = published.iter().map(|p| p.topic.as_str()).collect();
    assert_eq!(
        topics,
        [
            "nidoo/switch/group/stair/config",
            "nidoo/group/stair/avail",
            "nidoo/group/stair/definition",
            "nidoo/group/stair",
        ]
    );
    assert!(published.iter().all(|p| p.retain));
    assert_eq!(published[1].payload, "online");

    let again = hub
        .set_group("stair", switch_group())
        .await
        .expect("replaced");
    assert_eq!(again.id, "stair");
    let topics: Vec<String> = drain(&mut rx).into_iter().map(|p| p.topic).collect();
    assert!(
        !topics.contains(&"nidoo/group/stair".to_owned()),
        "{topics:?}"
    );
}

#[tokio::test]
async fn changing_the_kind_clears_the_old_config_topic() {
    let (hub, mut rx) = groups_hub();
    let mut spec = switch_group();
    spec.members.clear();
    hub.set_group("stair", spec.clone()).await.expect("created");
    drain(&mut rx);
    spec.kind = Kind::Light;
    hub.set_group("stair", spec).await.expect("replaced");
    let published = drain(&mut rx);
    assert!(
        published
            .iter()
            .any(|p| p.topic == "nidoo/switch/group/stair/config"
                && p.payload.is_empty()
                && p.retain),
        "{published:?}"
    );
    assert!(
        published
            .iter()
            .any(|p| p.topic == "nidoo/light/group/stair/config" && !p.payload.is_empty())
    );
}

#[tokio::test]
async fn delete_group_clears_the_four_topics_in_order() {
    let (hub, mut rx) = groups_hub();
    let spec = switch_group();
    hub.set_group("stair", spec.clone()).await.expect("created");
    drain(&mut rx);
    hub.delete_group("stair").await.expect("deleted");
    let published = drain(&mut rx);
    let topics: Vec<&str> = published.iter().map(|p| p.topic.as_str()).collect();
    assert_eq!(
        topics,
        [
            "nidoo/group/stair/definition",
            "nidoo/switch/group/stair/config",
            "nidoo/group/stair",
            "nidoo/group/stair/avail",
        ]
    );
    assert!(published.iter().all(|p| p.payload.is_empty() && p.retain));
    assert!(hub.entity_view("group_stair").is_none());
    assert!(matches!(
        hub.delete_group("nope").await,
        Err(HubError::UnknownEntity(_))
    ));
}

#[tokio::test]
async fn list_groups_names_the_flows_that_use_each_group() {
    let (hub, _rx) = groups_hub();
    let spec = switch_group();
    hub.set_group("stair", spec.clone()).await.expect("created");
    hub.ingest(
        "nidoo/automation/lamp",
        br#"{"id":"lamp","name":"Stair lamp","enabled":true,"nodes":[
            {"id":"t","type":"trigger.state","params":{"entity":"group_stair"}}],"edges":[]}"#,
    );
    hub.ingest(
        "nidoo/automation/other",
        br#"{"id":"other","name":"Other","enabled":true,"nodes":[
            {"id":"t","type":"trigger.state","params":{"entity":"somebody_else"}}],"edges":[]}"#,
    );
    let views = hub.list_groups().expect("enabled");
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].used_by_flows, ["Stair lamp"]);
    assert_eq!(views[0].name, "Stair");
}

#[tokio::test]
async fn list_groups_skips_groups_without_entities() {
    let (hub, _rx) = groups_hub();
    hub.ingest(
        "nidoo/group/ghost/definition",
        br#"{"name":"Ghost","kind":"switch","members":[]}"#,
    );
    let mut living = switch_group();
    living.members.clear();
    hub.set_group("living", living).await.expect("created");
    let views = hub.list_groups().expect("enabled");
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].id, "living");
}

const HALL_ID: &str = "0xaaaaaaaaaaaaaaaa_switch_zigbee2mqtt";

/// A second recorded switch, `rooms/hall/light`, made from the stair one.
fn feed_hall_switch(hub: &Hub) {
    let text = String::from_utf8(fixture(SWITCH_FIXTURE))
        .expect("utf8")
        .replace("0x847127fffe24e322", "0xaaaaaaaaaaaaaaaa")
        .replace("rooms/stair/light", "rooms/hall/light");
    hub.ingest(
        "nidoo/switch/0xaaaaaaaaaaaaaaaa/switch/config",
        text.as_bytes(),
    );
}

fn two_switch_group() -> nidoo_api::GroupSpec {
    nidoo_api::GroupSpec {
        kind: Kind::Switch,
        name: "Living".to_owned(),
        members: vec![SWITCH_ID.to_owned(), HALL_ID.to_owned()],
        hide_members: true,
    }
}

#[tokio::test]
async fn group_command_with_every_member_offline_is_rejected() {
    let (hub, mut rx) = groups_hub();
    feed_hall_switch(&hub);
    hub.set_group("living", two_switch_group())
        .await
        .expect("created");
    hub.ingest("zigbee2mqtt/bridge/state", br#"{"state":"offline"}"#);
    drain(&mut rx);

    let response = hub
        .send_command(
            "group_living",
            Command::TurnOn {
                brightness: None,
                color: None,
                effect: None,
            },
        )
        .await
        .expect("a response");
    assert!(!response.accepted);
    assert_eq!(response.reason, Some("no reachable member".to_owned()));
    assert!(
        drain(&mut rx)
            .iter()
            .all(|p| p.topic != "nidoo/group/living"),
        "the group state is untouched"
    );
}

#[tokio::test]
async fn member_state_echo_republishes_the_group_state() {
    let (hub, mut rx) = groups_hub();
    feed_hall_switch(&hub);
    hub.ingest(&switch_state_topic(), br#"{"state":"OFF"}"#);
    hub.ingest("zigbee2mqtt/rooms/hall/light", br#"{"state":"OFF"}"#);
    hub.set_group("living", two_switch_group())
        .await
        .expect("created");
    let initial = drain(&mut rx);
    let state = initial
        .iter()
        .find(|p| p.topic == "nidoo/group/living")
        .expect("initial state");
    assert_eq!(state.payload, r#"{"state":"OFF"}"#);

    hub.ingest(&switch_state_topic(), br#"{"state":"ON"}"#);
    let published = drain(&mut rx);
    let state = published
        .iter()
        .find(|p| p.topic == "nidoo/group/living")
        .expect("republished state");
    assert_eq!(state.payload, r#"{"state":"ON"}"#);
    assert!(state.retain);
    let view = hub.entity_view("group_living").expect("group entity");
    assert_eq!(view.state.value, StateValue::OnOff(true));
}

#[tokio::test]
async fn showas_drift_drops_the_member_from_fanout_but_keeps_it_mismatched() {
    let (hub, mut rx) = groups_hub();
    let mut patch = serde_json::Map::new();
    patch.insert("showAs".to_owned(), serde_json::Value::from("light"));
    hub.set_override(SWITCH_ID, &patch).await.expect("override");
    let spec = nidoo_api::GroupSpec {
        kind: Kind::Light,
        name: "Living".to_owned(),
        members: vec![SWITCH_ID.to_owned()],
        hide_members: true,
    };
    hub.set_group("living", spec.clone())
        .await
        .expect("created");

    let mut clear = serde_json::Map::new();
    clear.insert("showAs".to_owned(), serde_json::Value::Null);
    hub.set_override(SWITCH_ID, &clear).await.expect("override");
    drain(&mut rx);

    assert_eq!(hub.read_group_store().get("living"), Some(&spec));
    let views = hub.list_groups().expect("enabled");
    assert!(views[0].members[0].mismatch);

    let response = hub
        .send_command(
            "group_living",
            Command::TurnOn {
                brightness: None,
                color: None,
                effect: None,
            },
        )
        .await
        .expect("a response");
    assert!(!response.accepted);
    assert!(
        drain(&mut rx)
            .iter()
            .all(|p| p.topic != "zigbee2mqtt/rooms/stair/light/set"),
        "nothing reaches the drifted member"
    );
}

#[tokio::test]
async fn a_command_on_the_group_topic_fans_out_to_the_members() {
    let (hub, mut rx) = groups_hub();
    hub.set_group("stair", switch_group())
        .await
        .expect("created");
    drain(&mut rx);

    let changes = hub.ingest_message("nidoo/group/stair", br#"{"type":"turn_on"}"#, false);
    assert!(changes.is_empty());
    let publish = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("a publish")
        .expect("open");
    assert_eq!(publish.topic, "zigbee2mqtt/rooms/stair/light/set");
}

#[tokio::test]
async fn group_command_is_accepted_once_every_member_echoes() {
    let (hub, mut rx) = groups_hub();
    hub.set_group("stair", switch_group())
        .await
        .expect("created");
    drain(&mut rx);
    let echo = {
        let hub = Arc::clone(&hub);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            hub.ingest("zigbee2mqtt/rooms/stair/light", br#"{"state":"ON"}"#);
        })
    };
    let response = hub
        .send_command("group_stair", Command::TurnOff)
        .await
        .expect("a response");
    echo.await.expect("echo task");
    assert!(response.accepted, "{response:?}");
}

#[tokio::test(start_paused = true)]
async fn group_command_timeout_lists_missing_members_in_definition_order() {
    let (hub, mut rx) = groups_hub();
    feed_hall_switch(&hub);
    let spec = nidoo_api::GroupSpec {
        members: vec![HALL_ID.to_owned(), SWITCH_ID.to_owned()],
        ..two_switch_group()
    };
    hub.set_group("living", spec).await.expect("created");
    drain(&mut rx);

    let response = hub
        .send_command("group_living", Command::TurnOff)
        .await
        .expect("a response");
    assert!(!response.accepted);
    assert_eq!(
        response.reason,
        Some(format!("{HALL_ID}, {SWITCH_ID}: no echo"))
    );
}

#[tokio::test]
async fn group_command_refused_when_switch_is_off() {
    let (hub, mut rx) = hub();
    feed_switch(&hub);
    let spec = serde_json::to_vec(&switch_group()).expect("json");
    hub.ingest("nidoo/group/stair/definition", &spec);
    assert!(hub.read_group_store().get("stair").is_some());
    drain(&mut rx);

    let response = hub
        .send_command("group_stair", Command::TurnOff)
        .await
        .expect("a response");
    assert!(!response.accepted);
    assert_eq!(response.reason, Some("groups disabled".to_owned()));
    assert!(response.state.is_none());
    assert!(drain(&mut rx).is_empty(), "nothing reaches the members");
}

/// A recorded light or cover config made into entity `n`, placed in area `area` by override.
async fn feed_member(hub: &Hub, kind: Kind, n: u8, area: &str) -> String {
    let (fixture_name, old_id, old_path, component) = match kind {
        Kind::Light => (
            "discovery/light__0x00124b000b1355e6__light_3.json",
            "0x00124b000b1355e6",
            "miscellaneous/heater_mobile/3",
            "light",
        ),
        _ => (
            "discovery/cover__0x000d6ffffe9d44e4__cover.json",
            "0x000d6ffffe9d44e4",
            "rooms/office/shutter",
            "cover",
        ),
    };
    let hw = format!("0xbbbb00000000000{n}");
    let text = String::from_utf8(fixture(fixture_name))
        .expect("utf8")
        .replace(old_id, &hw)
        .replace(old_path, &format!("rooms/{area}/{component}{n}"));
    hub.ingest(
        &format!("nidoo/{component}/{hw}/{component}/config"),
        text.as_bytes(),
    );
    let id = hub
        .snapshot()
        .entities
        .into_iter()
        .find(|e| e.id.starts_with(&hw))
        .expect("entity")
        .id;
    let mut patch = serde_json::Map::new();
    patch.insert("area".to_owned(), serde_json::Value::from(area));
    hub.set_override(&id, &patch).await.expect("area override");
    id
}

fn migration_hub(enabled: bool) -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let mut cfg = config();
    cfg.groups.enabled = enabled;
    let (tx, rx) = mpsc::channel(64);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    (hub, rx)
}

async fn feed_living(hub: &Hub) {
    for n in 1..=2 {
        feed_member(hub, Kind::Light, n, "living").await;
        feed_member(hub, Kind::Cover, n + 2, "living").await;
    }
}

#[tokio::test]
async fn migration_on_empty_registry_defers_and_leaves_no_marker() {
    let (hub, mut rx) = migration_hub(true);
    let outcome = hub.run_group_migration_once().await;
    assert_eq!(
        outcome.skipped_reason,
        Some("house empty, retained sync not settled")
    );
    assert!(outcome.created.is_empty());
    assert!(drain(&mut rx).is_empty(), "no marker published");
    assert!(!hub.read_group_store().migrated());
}

#[tokio::test]
async fn migration_builds_light_and_cover_groups_once_marker_absent() {
    let (hub, mut rx) = migration_hub(true);
    feed_living(&hub).await;
    drain(&mut rx);
    let outcome = hub.run_group_migration_once().await;
    assert_eq!(outcome.skipped_reason, None);
    assert_eq!(outcome.created, ["cover_living", "light_living"]);
    let published = drain(&mut rx);
    let marker = published
        .iter()
        .find(|p| p.topic == "nidoo/group/_migrated")
        .expect("marker");
    assert!(marker.retain && !marker.payload.is_empty());
    assert!(hub.read_group_store().migrated());
    let views = hub.list_groups().expect("enabled");
    assert_eq!(views.len(), 2);
    for view in &views {
        let spec = hub.read_group_store().get(&view.id).cloned().expect("spec");
        assert!(spec.hide_members);
        assert_eq!(view.members.len(), 2);
    }
    let names: Vec<&str> = views.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["Shutters", "Lights"]);
}

#[tokio::test]
async fn migration_is_a_no_op_when_already_migrated() {
    let (hub, mut rx) = migration_hub(true);
    feed_living(&hub).await;
    hub.ingest("nidoo/group/_migrated", b"1");
    drain(&mut rx);
    let outcome = hub.run_group_migration_once().await;
    assert_eq!(outcome.skipped_reason, Some("already migrated"));
    assert!(outcome.created.is_empty());
    assert!(drain(&mut rx).is_empty(), "nothing published");
}

#[tokio::test]
async fn migration_refuses_when_switch_is_off() {
    let (hub, mut rx) = migration_hub(false);
    feed_living(&hub).await;
    drain(&mut rx);
    let outcome = hub.run_group_migration_once().await;
    assert_eq!(outcome.skipped_reason, Some("groups disabled"));
    assert!(drain(&mut rx).is_empty());
}

#[tokio::test]
async fn migration_honours_group_names() {
    let (hub, _rx) = migration_hub(true);
    feed_member(&hub, Kind::Light, 1, "living").await;
    feed_member(&hub, Kind::Light, 2, "living").await;
    hub.ingest(
        "nidoo/ui/_areas",
        br#"{"areas":[{"id":"living","name":"Living","groupNames":{"light":"Main lamps"}}]}"#,
    );
    let outcome = hub.run_group_migration_once().await;
    assert_eq!(outcome.created, ["light_living"]);
    let views = hub.list_groups().expect("enabled");
    assert_eq!(views[0].name, "Main lamps");
}

// ----- Frigate bridge -----

const FRIGATE_EVENT: &str = r#"{"type":"new","before":{},"after":{"id":"1696.5-abc","camera":"front_door","label":"person","has_clip":false,"false_positive":false,"entered_zones":[]}}"#;

/// A hub with the Frigate bridge on or off, its push sync already settled.
async fn frigate_hub(bridge: bool) -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let mut cfg = config();
    cfg.frigate.url = Some("http://192.168.1.20:5000".to_owned());
    cfg.frigate.bridge = bridge;
    let (tx, rx) = mpsc::channel(64);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    hub.on_connected();
    hub.on_suback();
    tokio::time::sleep(Duration::from_secs(2)).await;
    tokio::task::yield_now().await;
    (hub, rx)
}

/// The extra subscriptions of the bridges and watchdogs: without the alarm's documents and the
/// push responses topic and the broker's retained count, which are always subscribed.
fn bridge_filters(hub: &Hub) -> Vec<String> {
    hub.extra_filters()
        .into_iter()
        .filter(|f| {
            !f.starts_with("nidoo/alarm/")
                && f != "nidoo/push/responses"
                && f != nidoo::system::services::RETAINED_TOPIC
                && f != nidoo::sound::SPOKEN_TOPIC
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn frigate_event_becomes_history_alert_when_bridge_on() {
    let (hub, mut publisher) = frigate_hub(true).await;
    assert_eq!(bridge_filters(&hub), vec!["frigate/events".to_owned()]);

    hub.ingest_message("frigate/events", FRIGATE_EVENT.as_bytes(), false);

    let published = drain(&mut publisher);
    let last = published
        .iter()
        .rev()
        .find(|p| p.topic == "nidoo/push/history")
        .unwrap_or_else(|| panic!("a history publish in {published:?}"));
    let list: serde_json::Value = serde_json::from_str(&last.payload).expect("json");
    assert_eq!(list[0]["id"], "frigate-1696.5-abc", "{list}");
    assert_eq!(list[0]["title"], "Front Door");
    assert_eq!(list[0]["loc"]["key"], "frigate.object");
    assert_eq!(
        list[0]["image_url"],
        "http://192.168.1.20:5000/api/events/1696.5-abc/snapshot.jpg"
    );
}

#[tokio::test(start_paused = true)]
async fn frigate_events_ignored_when_bridge_off() {
    let (hub, mut publisher) = frigate_hub(false).await;
    assert!(bridge_filters(&hub).is_empty());

    hub.ingest_message("frigate/events", FRIGATE_EVENT.as_bytes(), false);

    let published = drain(&mut publisher);
    assert!(
        !published.iter().any(|p| p.payload.contains("frigate-")),
        "{published:?}"
    );
}

// ----- Watchdogs -----

/// A hub with the watchdogs on, its push sync already settled.
async fn watchdog_hub(acl_source: nidoo::config::AclSource) -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let mut cfg = config();
    cfg.watchdog.gateway.enabled = true;
    cfg.watchdog.acl.enabled = true;
    cfg.watchdog.acl.source = acl_source;
    let (tx, rx) = mpsc::channel(64);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    hub.on_connected();
    hub.on_suback();
    tokio::time::sleep(Duration::from_secs(2)).await;
    tokio::task::yield_now().await;
    (hub, rx)
}

/// The alert ids in the latest history publish, newest first.
fn history_ids(published: &[Publish]) -> Vec<String> {
    let Some(last) = published
        .iter()
        .rev()
        .find(|p| p.topic == "nidoo/push/history")
    else {
        return Vec::new();
    };
    let list: serde_json::Value = serde_json::from_str(&last.payload).expect("json");
    list.as_array()
        .expect("list")
        .iter()
        .map(|a| a["id"].as_str().expect("id").to_owned())
        .collect()
}

const GW_FAIL: &str = r#"{"status":"fail","exit_code":1,"timestamp":1700000000,"host":"hub-host"}"#;
const GW_OK: &str = r#"{"status":"ok","exit_code":0,"timestamp":1700000060,"host":"hub-host"}"#;
const DENIED: &str =
    "1791743566: Denied PUBLISH from Athena (d0, q0, r1, m0, 'athena/heartbeat', ... (207 bytes))";

#[tokio::test(start_paused = true)]
async fn watchdog_filters_follow_the_enabled_watchdogs() {
    use nidoo::config::AclSource;
    let (hub, _rx) = watchdog_hub(AclSource::Topic).await;
    assert_eq!(
        bridge_filters(&hub),
        vec![
            "encryption-gateway/health/+".to_owned(),
            "$SYS/broker/log/#".to_owned()
        ]
    );
    let (hub, _rx) = watchdog_hub(AclSource::File).await;
    assert_eq!(
        bridge_filters(&hub),
        vec!["encryption-gateway/health/+".to_owned()]
    );
    let (hub, _rx) = frigate_hub(false).await;
    assert!(bridge_filters(&hub).is_empty());
}

#[tokio::test(start_paused = true)]
async fn gateway_health_transitions_alert_even_when_retained_and_replays_stay_silent() {
    use nidoo::config::AclSource;
    let (hub, mut publisher) = watchdog_hub(AclSource::Topic).await;
    let topic = "encryption-gateway/health/gateway";

    // The retained replay on connect is news the first time ...
    hub.ingest_message(topic, GW_FAIL.as_bytes(), true);
    let ids = history_ids(&drain(&mut publisher));
    assert_eq!(ids, vec!["gw-health-gateway-fail-1700000000".to_owned()]);

    // ... and nothing when a reconnect replays it unchanged.
    hub.ingest_message(topic, GW_FAIL.as_bytes(), true);
    assert!(history_ids(&drain(&mut publisher)).is_empty());

    hub.ingest_message(topic, GW_OK.as_bytes(), false);
    let ids = history_ids(&drain(&mut publisher));
    assert_eq!(ids[0], "gw-health-gateway-ok-1700000060");
}

#[tokio::test(start_paused = true)]
async fn acl_log_topic_lines_become_system_alerts_and_cool_down() {
    use nidoo::config::AclSource;
    let (hub, mut publisher) = watchdog_hub(AclSource::Topic).await;

    hub.ingest_message("$SYS/broker/log/N", DENIED.as_bytes(), false);
    let ids = history_ids(&drain(&mut publisher));
    assert_eq!(ids.len(), 1, "{ids:?}");
    assert!(ids[0].starts_with("mqtt-acl-Athena-publish-"), "{ids:?}");

    hub.ingest_message("$SYS/broker/log/N", DENIED.as_bytes(), false);
    assert!(
        history_ids(&drain(&mut publisher)).is_empty(),
        "inside the cooldown"
    );

    // A retained replay and a topic outside the filter are no log lines.
    hub.ingest_message(
        "$SYS/broker/log/E",
        b"Denied SUBSCRIBE from B (q0, 'x/y')",
        true,
    );
    hub.ingest_message("other/log", b"Denied SUBSCRIBE from B (q0, 'x/y')", false);
    assert!(history_ids(&drain(&mut publisher)).is_empty());
}

#[tokio::test(start_paused = true)]
async fn acl_file_source_takes_lines_through_on_acl_line() {
    use nidoo::config::AclSource;
    let (hub, mut publisher) = watchdog_hub(AclSource::File).await;
    hub.ingest_message("$SYS/broker/log/N", DENIED.as_bytes(), false);
    assert!(
        history_ids(&drain(&mut publisher)).is_empty(),
        "topic ignored for a file source"
    );

    hub.on_acl_line(DENIED);
    let ids = history_ids(&drain(&mut publisher));
    assert!(ids[0].starts_with("mqtt-acl-Athena-publish-"), "{ids:?}");
}

#[test]
fn flow_notify_lists_mirror_the_push_service() {
    assert_eq!(
        nidoo_core::flow::NOTIFY_CHANNELS,
        nidoo_push::normalize::CHANNELS
    );
    assert_eq!(
        nidoo_core::flow::NOTIFY_SEVERITIES,
        nidoo_push::normalize::SEVERITIES
    );
}

/// A restored value is broadcast and stamped with its recorded time, but only while the
/// entity has not reported; a later retained value replaces it.
#[tokio::test]
async fn restore_recorded_states_broadcasts_once() {
    let (hub, _publisher) = hub();
    feed_switch(&hub);
    let mut events = hub.subscribe();

    let restored = hub.restore_recorded_states(vec![
        (
            SWITCH_ID.to_owned(),
            1_700_000_000_000_000,
            StateValue::OnOff(true),
        ),
        ("missing".to_owned(), 1, StateValue::OnOff(true)),
    ]);
    assert_eq!(restored, 1);
    match next_event(&mut events).await {
        WsEvent::StateChanged { id, state } => {
            assert_eq!(id, SWITCH_ID);
            assert_eq!(state.value, StateValue::OnOff(true));
            assert_eq!(state.last_update, Some(1_700_000_000));
        }
        other => panic!("expected StateChanged, got {other:?}"),
    }

    hub.ingest(
        &switch_state_topic(),
        &fixture("states/rooms__stair__light.json"),
    );
    let again = hub.restore_recorded_states(vec![(
        SWITCH_ID.to_owned(),
        1_700_000_001_000_000,
        StateValue::OnOff(true),
    )]);
    assert_eq!(again, 0);
}

async fn outage_hub(weather: bool, solar: bool) -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let mut cfg = config();
    cfg.weather.notify_unreachable = weather;
    cfg.solcast.notify_unreachable = solar;
    let (tx, mut rx) = mpsc::channel(64);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    hub.on_connected();
    hub.on_suback();
    tokio::time::sleep(Duration::from_secs(2)).await;
    tokio::task::yield_now().await;
    drain(&mut rx);
    (hub, rx)
}

#[tokio::test(start_paused = true)]
async fn a_service_down_for_half_an_hour_reaches_the_system_alerts_once() {
    use nidoo::outage::Service;
    let (hub, mut publisher) = outage_hub(true, false).await;
    assert!(hub.outage_watch_wanted());
    hub.service_failed_at(Service::Weather, 1_000, "timeout");
    hub.check_outages_at(1_000 + 29 * 60);
    assert!(history_ids(&drain(&mut publisher)).is_empty());

    hub.check_outages_at(1_000 + 30 * 60);
    assert_eq!(
        history_ids(&drain(&mut publisher)),
        vec!["service-weather-down-1000".to_owned()]
    );
    hub.check_outages_at(1_000 + 31 * 60);
    assert!(history_ids(&drain(&mut publisher)).is_empty());

    hub.service_succeeded_at(Service::Weather, 1_000 + 45 * 60);
    let ids = history_ids(&drain(&mut publisher));
    assert_eq!(ids[0], "service-weather-back-1000");
}

#[tokio::test(start_paused = true)]
async fn a_service_without_its_switch_never_alerts() {
    use nidoo::outage::Service;
    let (hub, mut publisher) = outage_hub(false, false).await;
    assert!(!hub.outage_watch_wanted());
    hub.service_failed_at(Service::Solar, 1_000, "API key refused");
    hub.check_outages_at(1_000 + 3 * 3600);
    hub.service_succeeded_at(Service::Solar, 1_000 + 4 * 3600);
    assert!(drain(&mut publisher).is_empty());
}

#[tokio::test]
async fn light_command_counts_one() {
    let (hub, _publisher) = hub();
    feed_switch(&hub);
    assert_eq!(hub.commands_sent(), 0);
    hub.send_command(SWITCH_ID, Command::TurnOff)
        .await
        .expect("published");
    assert_eq!(hub.commands_sent(), 1);
}

#[tokio::test]
async fn group_command_counts_each_member() {
    let (hub, mut rx) = groups_hub();
    feed_hall_switch(&hub);
    hub.set_group("living", two_switch_group())
        .await
        .expect("created");
    drain(&mut rx);
    let before = hub.commands_sent();
    hub.send_command("group_living", Command::TurnOff)
        .await
        .expect("a response");
    assert_eq!(hub.commands_sent() - before, 2);
}

#[tokio::test]
async fn sys_retained_count_is_captured_not_routed() {
    use nidoo::system::services::RETAINED_TOPIC;
    let (hub, _publisher) = hub();
    let changes = hub.ingest_message(RETAINED_TOPIC, b"2431", false);
    assert!(changes.is_empty());
    assert_eq!(hub.retained_count(), Some(2431));
    hub.ingest_message(RETAINED_TOPIC, b"not a number", false);
    assert_eq!(hub.retained_count(), Some(2431));
}

#[tokio::test]
async fn retained_unknown_until_seen() {
    let (hub, _publisher) = hub();
    assert_eq!(hub.retained_count(), None);
}

#[tokio::test]
async fn extra_filters_include_retained_topic() {
    let (hub, _publisher) = hub();
    assert!(
        hub.extra_filters()
            .contains(&nidoo::system::services::RETAINED_TOPIC.to_owned())
    );
}

#[test]
fn spoken_message_reaches_subscribers() {
    let (hub, _rx) = hub();
    assert!(
        hub.extra_filters()
            .contains(&nidoo::sound::SPOKEN_TOPIC.to_owned())
    );
    let mut spoken = hub.subscribe_spoken();
    let changes = hub.ingest_message(
        nidoo::sound::SPOKEN_TOPIC,
        br#"{"id":"clip","played":true}"#,
        false,
    );
    assert!(changes.is_empty());
    assert_eq!(
        spoken.try_recv().unwrap(),
        nidoo::sound::Spoken {
            id: "clip".into(),
            played: true
        }
    );
    assert!(hub.snapshot().entities.is_empty(), "not a device");
}

const PLAYER_ID: &str = "music_node_player";
const PLAYER_STATE: &str = "music/player/state";

/// A media player playing "A" by "Band", its last report at `position_at` 10:20:00.
fn feed_player(hub: &Hub) {
    let config = serde_json::json!({
        "name": "Music",
        "unique_id": PLAYER_ID,
        "state_topic": PLAYER_STATE,
        "command_topic": "music/player/set",
        "supported_features": ["play", "pause", "next", "previous", "play_media"],
        "device": {"identifiers": ["music_node"], "name": "Music server"}
    });
    hub.ingest(
        "nidoo/media_player/music_node/player/config",
        config.to_string().as_bytes(),
    );
    hub.ingest(PLAYER_STATE, &player_state("playing", "A", "10:20:00"));
}

fn player_state(state: &str, title: &str, at: &str) -> Vec<u8> {
    serde_json::json!({
        "state": state, "title": title, "artist": "Band",
        "duration_s": 200, "position_s": 0, "position_at": format!("2026-11-06T{at}Z")
    })
    .to_string()
    .into_bytes()
}

/// Sends `cmd` to the player and ingests `echo` as soon as the publish is out.
async fn media_with_echo(
    hub: &Arc<Hub>,
    mut publisher: mpsc::Receiver<Publish>,
    cmd: Command,
    echo: Vec<u8>,
) -> (nidoo_api::CommandResponse, Publish) {
    let echo_task = {
        let hub = Arc::clone(hub);
        tokio::spawn(async move {
            let published = publisher.recv().await.expect("the command publish");
            hub.ingest(PLAYER_STATE, &echo);
            published
        })
    };
    let response = hub
        .send_command(PLAYER_ID, cmd)
        .await
        .expect("mapped and published");
    (response, echo_task.await.expect("echo task"))
}

fn media(action: &str) -> Command {
    Command::MediaAction {
        action: action.to_owned(),
    }
}

#[tokio::test]
async fn media_next_is_confirmed_by_a_new_title() {
    let (hub, publisher) = hub_with_timeout(500);
    feed_player(&hub);
    let (response, published) = media_with_echo(
        &hub,
        publisher,
        media("next"),
        player_state("playing", "B", "10:21:00"),
    )
    .await;
    assert_eq!(published.topic, "music/player/set");
    assert_eq!(published.payload, r#"{"command":"next"}"#);
    assert!(response.accepted, "{response:?}");
    let StateValue::MediaPlayer(player) = response.state.expect("state").value else {
        panic!("a media player state");
    };
    assert_eq!(player.title.as_deref(), Some("B"));
}

/// Someone else paused while `next` waited: that report is not `next`'s echo.
#[tokio::test]
async fn media_next_ignores_a_report_that_changes_nothing_it_needs() {
    let (hub, publisher) = hub_with_timeout(100);
    feed_player(&hub);
    let (response, _) = media_with_echo(
        &hub,
        publisher,
        media("next"),
        player_state("paused", "A", "10:20:00"),
    )
    .await;
    assert!(!response.accepted, "{response:?}");
    assert_eq!(
        response.reason.as_deref(),
        Some("no state change within 100 ms")
    );
}

/// Pausing a paused player: the same retained report confirms it.
#[tokio::test]
async fn media_pause_on_a_paused_player_is_confirmed_by_the_same_report() {
    let (hub, publisher) = hub_with_timeout(500);
    feed_player(&hub);
    hub.ingest(PLAYER_STATE, &player_state("paused", "A", "10:20:00"));
    let (response, _) = media_with_echo(
        &hub,
        publisher,
        media("pause"),
        player_state("paused", "A", "10:20:00"),
    )
    .await;
    assert!(response.accepted, "{response:?}");
}

#[tokio::test]
async fn media_player_cannot_join_a_group() {
    let (hub, mut rx) = groups_hub();
    feed_player(&hub);
    let mut music = switch_group();
    music.kind = Kind::MediaPlayer;
    music.members = vec![PLAYER_ID.to_owned()];
    assert!(matches!(
        hub.set_group("music", music).await,
        Err(HubError::Rejected(_))
    ));
    assert!(drain(&mut rx).is_empty());
}
