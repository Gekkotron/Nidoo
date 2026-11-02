//! The ambiance store and CRUD on a hub driven without a broker.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nidoo::config::Config;
use nidoo::hub::{Hub, HubError};
use nidoo_api::{
    Ambiance, AmbianceAction, ApplyReport, Category, CategoryAction, ClimateAction,
    ConnectionStatus, DeviceAction, FlowAction, FlowDo, Outcome, Target, WsEvent,
};
use nidoo_core::command::Publish;
use nidoo_core::model::{Area, AreasDocument, Command};
use tokio::sync::mpsc;

const SWITCH_FIXTURE: &str = "discovery/switch__0x847127fffe24e322__switch.json";
const SWITCH_ID: &str = "0x847127fffe24e322_switch_zigbee2mqtt";
const SWITCH_TOPIC: &str = "nidoo/switch/0x847127fffe24e322/switch/config";

fn fixture(name: &str) -> Vec<u8> {
    let dir = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../nidoo-core/tests/fixtures/"
    ));
    std::fs::read(dir.join(name)).expect("fixture")
}

fn config() -> Config {
    let mut cfg = Config::default();
    cfg.commands.confirm_timeout_ms = 50;
    cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
    cfg
}

fn hub() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let (tx, rx) = mpsc::channel(64);
    let hub = Hub::new(&config(), tx);
    hub.set_connection(ConnectionStatus::Online);
    (hub, rx)
}

fn ambiance(id: &str, actions: Vec<AmbianceAction>) -> Ambiance {
    Ambiance {
        id: id.to_owned(),
        name: id.to_owned(),
        subtitle: None,
        icon: None,
        pinned: false,
        order: 1,
        actions,
        extra: Default::default(),
    }
}

fn category(area: Option<&str>, except: Vec<String>) -> AmbianceAction {
    AmbianceAction::Category(CategoryAction {
        what: Category::Lights,
        to: Target::Off,
        brightness: None,
        area: area.map(str::to_owned),
        except,
    })
}

fn rejected(result: Result<nidoo_api::AmbianceView, HubError>) -> String {
    match result {
        Err(HubError::Rejected(why)) => why,
        other => panic!("expected Rejected, got {other:?}"),
    }
}

const VALID: &str = r#"{"id":"a","name":"A","actions":[]}"#;

#[tokio::test]
async fn retained_payload_is_listed_and_empty_payload_deletes() {
    let (hub, _rx) = hub();
    hub.ingest("nidoo/ui/ambiances/a", VALID.as_bytes());
    let list = hub.ambiances();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, "a");
    assert!(list[0].ambiance.is_some() && list[0].invalid.is_none());
    assert!(hub.ambiance("a").is_some());

    hub.ingest("nidoo/ui/ambiances/a", b"");
    assert!(hub.ambiances().is_empty());
    assert!(hub.ambiance("a").is_none());
}

#[tokio::test]
async fn invalid_payload_is_kept_with_reason() {
    let (hub, _rx) = hub();
    hub.ingest("nidoo/ui/ambiances/a", br#"{"id":"a","name":""}"#);
    let list = hub.ambiances();
    assert_eq!(list.len(), 1);
    assert!(list[0].invalid.is_some(), "{list:?}");
    assert!(list[0].ambiance.is_none());
}

#[tokio::test]
async fn topic_id_mismatch_is_invalid() {
    let (hub, _rx) = hub();
    hub.ingest(
        "nidoo/ui/ambiances/a",
        br#"{"id":"b","name":"B","actions":[]}"#,
    );
    let list = hub.ambiances();
    assert_eq!(list[0].id, "a");
    let reason = list[0].invalid.as_deref().expect("invalid");
    assert!(reason.contains("does not match the topic"), "{reason}");
}

#[tokio::test]
async fn set_ambiance_publishes_retained_and_lists_it() {
    let (hub, mut rx) = hub();
    let view = hub
        .set_ambiance(ambiance("a", vec![]))
        .await
        .expect("set_ambiance");
    assert_eq!(view.id, "a");
    let published = rx.try_recv().expect("a publish");
    assert_eq!(published.topic, "nidoo/ui/ambiances/a");
    assert!(published.retain);
    assert_eq!(hub.ambiances().len(), 1);
}

#[tokio::test]
async fn set_ambiance_rejects_unknown_entity_area_flow() {
    let (hub, _rx) = hub();
    let device = AmbianceAction::Device(DeviceAction {
        entity: "nope".to_owned(),
        command: Command::Toggle,
    });
    assert!(rejected(hub.set_ambiance(ambiance("a", vec![device])).await).contains("nope"));

    let why = rejected(
        hub.set_ambiance(ambiance("a", vec![category(Some("nowhere"), vec![])]))
            .await,
    );
    assert!(why.contains("action 0") && why.contains("nowhere"), "{why}");

    let flow = AmbianceAction::Flow(FlowAction {
        flow: "ghost".to_owned(),
        action: FlowDo::Run,
    });
    assert!(rejected(hub.set_ambiance(ambiance("a", vec![flow])).await).contains("ghost"));

    let why = rejected(
        hub.set_ambiance(ambiance(
            "a",
            vec![category(None, vec!["ghost".to_owned()])],
        ))
        .await,
    );
    assert!(why.contains("except"), "{why}");
    assert!(hub.ambiances().is_empty());
}

#[tokio::test]
async fn set_ambiance_resolves_area_name_to_id() {
    let (hub, _rx) = hub();
    hub.set_areas(AreasDocument {
        areas: vec![Area {
            id: "lounge".to_owned(),
            name: "Lounge".to_owned(),
            ..Area::default()
        }],
    })
    .await
    .expect("areas");
    let view = hub
        .set_ambiance(ambiance("a", vec![category(Some("Lounge"), vec![])]))
        .await
        .expect("set_ambiance");
    let stored = view.ambiance.expect("valid");
    match &stored.actions[0] {
        AmbianceAction::Category(c) => assert_eq!(c.area.as_deref(), Some("lounge")),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn set_ambiance_accepts_a_known_device_and_rejects_climate_while_engine_off() {
    let (hub, _rx) = hub();
    hub.ingest(SWITCH_TOPIC, &fixture(SWITCH_FIXTURE));
    let device = AmbianceAction::Device(DeviceAction {
        entity: SWITCH_ID.to_owned(),
        command: Command::Toggle,
    });
    hub.set_ambiance(ambiance("a", vec![device]))
        .await
        .expect("a known switch");

    let climate = AmbianceAction::Climate(ClimateAction {
        clear: true,
        ..ClimateAction::default()
    });
    let why = rejected(hub.set_ambiance(ambiance("b", vec![climate])).await);
    assert!(why.contains("the climate engine is off"), "{why}");
}

#[tokio::test]
async fn delete_unknown_is_404_kind() {
    let (hub, _rx) = hub();
    match hub.delete_ambiance("ghost").await {
        Err(HubError::UnknownAmbiance(id)) => assert_eq!(id, "ghost"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn delete_publishes_an_empty_retained_payload() {
    let (hub, mut rx) = hub();
    hub.set_ambiance(ambiance("a", vec![])).await.expect("set");
    rx.try_recv().expect("set publish");
    hub.delete_ambiance("a").await.expect("delete");
    let published = rx.try_recv().expect("delete publish");
    assert_eq!(published.topic, "nidoo/ui/ambiances/a");
    assert!(published.retain && published.payload.is_empty());
    assert!(hub.ambiances().is_empty());
}

#[tokio::test]
async fn ingest_broadcasts_ambiances_changed() {
    let (hub, _rx) = hub();
    let mut events = hub.subscribe();
    hub.ingest("nidoo/ui/ambiances/a", VALID.as_bytes());
    let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .expect("an event within 1 s")
        .expect("open channel");
    match event {
        WsEvent::AmbiancesChanged { ambiances } => {
            assert_eq!(ambiances.len(), 1);
            assert_eq!(ambiances[0].id, "a");
        }
        other => panic!("{other:?}"),
    }
    // An identical echo changes nothing, so nothing is broadcast.
    hub.ingest("nidoo/ui/ambiances/a", VALID.as_bytes());
    assert!(events.try_recv().is_err());
}

// Applying an ambiance.

const LED_FIXTURE: &str = "discovery/light__0x60a423fffe94d1c6__light.json";
const LED_TOPIC: &str = "nidoo/light/0x60a423fffe94d1c6/light/config";
const LED_ID: &str = "0x60a423fffe94d1c6_light_zigbee2mqtt";
const LED_STATE_TOPIC: &str = "zigbee2mqtt/rooms/living/led_ceiling";
const LED_COMMAND_TOPIC: &str = "zigbee2mqtt/rooms/living/led_ceiling/set";
const TV_FIXTURE: &str = "discovery/light__0x9035eafffec07012__light.json";
const TV_TOPIC: &str = "nidoo/light/0x9035eafffec07012/light/config";
const TV_ID: &str = "0x9035eafffec07012_light_zigbee2mqtt";
const TV_STATE_TOPIC: &str = "zigbee2mqtt/rooms/living/led_tv";

fn hub_with_timeout(ms: u64) -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let mut cfg = config();
    cfg.commands.confirm_timeout_ms = ms;
    let (tx, rx) = mpsc::channel(64);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    (hub, rx)
}

/// A hub that knows the ceiling LED, currently `on`.
fn led_hub(ms: u64, on: bool) -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let (hub, rx) = hub_with_timeout(ms);
    hub.ingest(LED_TOPIC, &fixture(LED_FIXTURE));
    set_led(&hub, on);
    (hub, rx)
}

fn set_led(hub: &Hub, on: bool) {
    let state: &[u8] = if on {
        br#"{"state":"ON","brightness":100}"#
    } else {
        br#"{"state":"OFF","brightness":100}"#
    };
    hub.ingest(LED_STATE_TOPIC, state);
}

/// Plays the broker: records the topic of every publish and echoes the LED's new state, and
/// the switch's too when `echo_switch`. Anything else gets no echo.
fn echo(
    mut publisher: mpsc::Receiver<Publish>,
    hub: Arc<Hub>,
    echo_switch: bool,
) -> Arc<Mutex<Vec<String>>> {
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&log);
    tokio::spawn(async move {
        while let Some(publish) = publisher.recv().await {
            sink.lock().expect("log").push(publish.topic.clone());
            if publish.topic == LED_COMMAND_TOPIC {
                let on = serde_json::from_str::<serde_json::Value>(&publish.payload)
                    .is_ok_and(|v| v["state"] == "ON");
                set_led(&hub, on);
            } else if echo_switch && publish.topic.ends_with("/stair/light/set") {
                hub.ingest(
                    "zigbee2mqtt/rooms/stair/light",
                    format!(r#"{{"state":"{}"}}"#, publish.payload).as_bytes(),
                );
            }
        }
    });
    log
}

fn published(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    log.lock().expect("log").clone()
}

/// Stores the ambiance as a retained payload, skipping the writer's checks.
fn save(hub: &Hub, a: &Ambiance) {
    let payload = serde_json::to_vec(a).expect("serialize");
    hub.ingest(&format!("nidoo/ui/ambiances/{}", a.id), &payload);
}

fn lights(to: Target) -> AmbianceAction {
    AmbianceAction::Category(CategoryAction {
        what: Category::Lights,
        to,
        brightness: None,
        area: None,
        except: vec![],
    })
}

fn device(entity: &str, command: Command) -> AmbianceAction {
    AmbianceAction::Device(DeviceAction {
        entity: entity.to_owned(),
        command,
    })
}

fn turn_on() -> Command {
    Command::TurnOn {
        brightness: None,
        color: None,
        effect: None,
    }
}

fn outcomes(report: &ApplyReport) -> Vec<Outcome> {
    report.actions.iter().map(|a| a.outcome).collect()
}

#[tokio::test]
async fn already_off_light_is_not_commanded() {
    let (hub, rx) = led_hub(50, false);
    let log = echo(rx, Arc::clone(&hub), false);
    save(&hub, &ambiance("night", vec![lights(Target::Off)]));

    let report = hub.apply_ambiance("night").await.expect("apply");

    assert_eq!(report.ambiance, "night");
    assert_eq!(outcomes(&report), [Outcome::Ok]);
    assert_eq!(report.actions[0].total, 1);
    assert!(report.actions[0].failed.is_empty() && report.actions[0].skipped.is_empty());
    tokio::task::yield_now().await;
    assert!(published(&log).is_empty(), "{:?}", published(&log));
}

#[tokio::test]
async fn on_light_is_turned_off_and_reported_ok() {
    let (hub, rx) = led_hub(50, true);
    let log = echo(rx, Arc::clone(&hub), false);
    save(&hub, &ambiance("night", vec![lights(Target::Off)]));

    let report = hub.apply_ambiance("night").await.expect("apply");

    assert_eq!(outcomes(&report), [Outcome::Ok]);
    assert_eq!(report.actions[0].total, 1);
    assert_eq!(published(&log), [LED_COMMAND_TOPIC]);
}

#[tokio::test]
async fn silent_device_is_failed_and_next_action_still_runs() {
    let (hub, rx) = led_hub(50, true);
    hub.ingest(SWITCH_TOPIC, &fixture(SWITCH_FIXTURE));
    let _log = echo(rx, Arc::clone(&hub), false);
    save(
        &hub,
        &ambiance(
            "mix",
            vec![device(SWITCH_ID, turn_on()), lights(Target::Off)],
        ),
    );

    let report = hub.apply_ambiance("mix").await.expect("apply");

    assert_eq!(outcomes(&report), [Outcome::Failed, Outcome::Ok]);
    let failed = &report.actions[0].failed;
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].id, SWITCH_ID);
    assert!(failed[0].reason.contains("no state change"), "{failed:?}");
}

#[tokio::test]
async fn offline_member_is_skipped_rest_ok() {
    let (hub, rx) = led_hub(50, true);
    // The TV light reports its availability on a topic of its own.
    let mut config: serde_json::Value = serde_json::from_slice(&fixture(TV_FIXTURE)).expect("json");
    config["availability"][0]["topic"] = "test/tv_availability".into();
    hub.ingest(TV_TOPIC, config.to_string().as_bytes());
    hub.ingest(TV_STATE_TOPIC, br#"{"state":"ON","brightness":100}"#);
    hub.ingest("test/tv_availability", br#"{"state":"offline"}"#);
    let log = echo(rx, Arc::clone(&hub), false);
    save(&hub, &ambiance("night", vec![lights(Target::Off)]));

    let report = hub.apply_ambiance("night").await.expect("apply");

    assert_eq!(outcomes(&report), [Outcome::Ok]);
    let action = &report.actions[0];
    assert_eq!(action.total, 2);
    assert_eq!(action.skipped.len(), 1);
    assert_eq!(action.skipped[0].id, TV_ID);
    assert_eq!(action.skipped[0].reason, "offline");
    assert!(action.failed.is_empty());
    assert_eq!(published(&log), [LED_COMMAND_TOPIC]);
}

#[tokio::test]
async fn vanished_entity_is_skipped_not_fatal() {
    let (hub, rx) = led_hub(50, true);
    hub.ingest(SWITCH_TOPIC, &fixture(SWITCH_FIXTURE));
    let _log = echo(rx, Arc::clone(&hub), false);
    save(
        &hub,
        &ambiance(
            "mix",
            vec![device(SWITCH_ID, turn_on()), lights(Target::Off)],
        ),
    );
    hub.ingest(SWITCH_TOPIC, b"");

    let report = hub.apply_ambiance("mix").await.expect("apply");

    assert_eq!(outcomes(&report), [Outcome::Skipped, Outcome::Ok]);
    assert_eq!(
        report.actions[0].reason.as_deref(),
        Some("no longer exists")
    );
}

#[tokio::test]
async fn flow_enable_action_enables_flow() {
    let (hub, rx) = led_hub(50, true);
    hub.ingest(
        "nidoo/binary_sensor/0x00158d008b5a2d65/occupancy/config",
        &fixture("discovery/binary_sensor__0x00158d008b5a2d65__occupancy.json"),
    );
    let _log = echo(rx, Arc::clone(&hub), false);
    let flow = serde_json::json!({
        "id": "living", "name": "Living", "enabled": false,
        "nodes": [
            { "id": "motion", "type": "trigger.state",
              "params": { "entity": "0x00158d008b5a2d65_occupancy_zigbee2mqtt" } },
            { "id": "on", "type": "command",
              "params": { "entity": LED_ID, "command": { "type": "turn_on" } } }
        ],
        "edges": [{ "from": "motion", "to": "on" }]
    });
    hub.ingest("nidoo/automation/living", flow.to_string().as_bytes());
    assert!(!hub.flow("living").expect("flow").definition.enabled);
    let action = AmbianceAction::Flow(FlowAction {
        flow: "living".to_owned(),
        action: FlowDo::Enable,
    });
    save(&hub, &ambiance("a", vec![action]));

    let report = hub.apply_ambiance("a").await.expect("apply");

    assert_eq!(outcomes(&report), [Outcome::Ok]);
    assert!(hub.flow("living").expect("flow").definition.enabled);
}

#[tokio::test]
async fn climate_action_skipped_when_engine_off() {
    let (hub, _rx) = hub_with_timeout(50);
    // Saving would be refused while the engine is off, so the payload is written directly.
    let climate = AmbianceAction::Climate(ClimateAction {
        clear: true,
        ..ClimateAction::default()
    });
    save(&hub, &ambiance("a", vec![climate]));

    let report = hub.apply_ambiance("a").await.expect("apply");

    assert_eq!(outcomes(&report), [Outcome::Skipped]);
    assert_eq!(
        report.actions[0].reason.as_deref(),
        Some("the climate engine is off")
    );
}

#[tokio::test]
async fn second_apply_while_running_is_refused() {
    let (hub, rx) = led_hub(500, true);
    let _log = echo(rx, Arc::clone(&hub), false);
    // The switch never echoes, so the first run waits out the whole timeout.
    hub.ingest(SWITCH_TOPIC, &fixture(SWITCH_FIXTURE));
    save(&hub, &ambiance("slow", vec![device(SWITCH_ID, turn_on())]));

    let first = {
        let hub = Arc::clone(&hub);
        tokio::spawn(async move { hub.apply_ambiance("slow").await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(
        hub.apply_ambiance("slow").await,
        Err(HubError::Rejected("already running".to_owned()))
    );
    let report = first.await.expect("task").expect("first apply");
    assert_eq!(outcomes(&report), [Outcome::Failed]);
}

#[tokio::test]
async fn lock_released_after_failed_run() {
    let (hub, rx) = led_hub(50, true);
    hub.ingest(SWITCH_TOPIC, &fixture(SWITCH_FIXTURE));
    let _log = echo(rx, Arc::clone(&hub), false);
    save(&hub, &ambiance("a", vec![device(SWITCH_ID, turn_on())]));

    let first = hub.apply_ambiance("a").await.expect("first");
    assert_eq!(outcomes(&first), [Outcome::Failed]);
    let second = hub.apply_ambiance("a").await.expect("second runs again");
    assert_eq!(outcomes(&second), [Outcome::Failed]);
}

#[tokio::test]
async fn apply_broadcasts_report() {
    let (hub, rx) = led_hub(50, true);
    let _log = echo(rx, Arc::clone(&hub), false);
    save(&hub, &ambiance("night", vec![lights(Target::Off)]));
    let mut events = hub.subscribe();

    let report = hub.apply_ambiance("night").await.expect("apply");

    let mut seen = None;
    while let Ok(event) = events.try_recv() {
        if let WsEvent::AmbianceApplied { report } = event {
            seen = Some(report);
        }
    }
    assert_eq!(seen, Some(report));
}

#[tokio::test]
async fn apply_refused_while_broker_offline() {
    let (hub, mut rx) = led_hub(50, true);
    save(&hub, &ambiance("night", vec![lights(Target::Off)]));
    hub.set_connection(ConnectionStatus::Offline {
        reason: "test".to_owned(),
    });

    assert_eq!(
        hub.apply_ambiance("night").await,
        Err(HubError::Publish("broker offline".to_owned()))
    );
    assert!(rx.try_recv().is_err(), "nothing may be published");
}

#[tokio::test]
async fn apply_unknown_or_invalid_ambiance_is_refused() {
    let (hub, _rx) = hub_with_timeout(50);
    assert_eq!(
        hub.apply_ambiance("ghost").await,
        Err(HubError::UnknownAmbiance("ghost".to_owned()))
    );
    hub.ingest("nidoo/ui/ambiances/bad", br#"{"id":"bad","name":""}"#);
    assert!(matches!(
        hub.apply_ambiance("bad").await,
        Err(HubError::Rejected(_))
    ));
}

// Climate actions on an engine that is on.

fn climate_hub() -> Arc<Hub> {
    let mut cfg = config();
    cfg.climate.enabled = true;
    let (tx, rx) = mpsc::channel(64);
    // Nothing here reads the publishes.
    std::mem::forget(rx);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    hub
}

async fn set_zones(hub: &Hub, zones: &[(&str, nidoo_climate::doc::ZoneState)]) {
    hub.set_areas(AreasDocument {
        areas: zones
            .iter()
            .map(|(id, _)| Area {
                id: (*id).to_owned(),
                name: (*id).to_owned(),
                ..Area::default()
            })
            .collect(),
    })
    .await
    .expect("areas");
    for (zone, state) in zones {
        hub.set_climate_zone(
            zone,
            nidoo_climate::doc::ZoneSettings {
                state: *state,
                ..Default::default()
            },
        )
        .expect("zone settings");
    }
}

fn overrides_of(hub: &Hub, zone: &str) -> usize {
    hub.climate_view()
        .expect("climate on")
        .zones
        .into_iter()
        .find(|z| z.zone == zone)
        .map_or(0, |z| z.overrides.len())
}

fn hold(zone: Option<&str>) -> AmbianceAction {
    AmbianceAction::Climate(ClimateAction {
        zone: zone.map(str::to_owned),
        setpoint: Some(serde_json::json!(19.0)),
        hours: Some(2),
        ..ClimateAction::default()
    })
}

#[tokio::test]
async fn climate_hold_and_clear_on_an_active_zone() {
    use nidoo_climate::doc::ZoneState;
    let hub = climate_hub();
    set_zones(&hub, &[("living", ZoneState::Active)]).await;
    let clear = AmbianceAction::Climate(ClimateAction {
        zone: Some("living".to_owned()),
        clear: true,
        ..ClimateAction::default()
    });
    save(&hub, &ambiance("hold", vec![hold(Some("living"))]));
    save(&hub, &ambiance("clear", vec![clear]));

    let report = hub.apply_ambiance("hold").await.expect("hold");
    assert_eq!(outcomes(&report), [Outcome::Ok]);
    assert_eq!(report.actions[0].total, 1);
    assert_eq!(overrides_of(&hub, "living"), 1);

    let report = hub.apply_ambiance("clear").await.expect("clear");
    assert_eq!(outcomes(&report), [Outcome::Ok]);
    assert_eq!(overrides_of(&hub, "living"), 0);
}

#[tokio::test]
async fn climate_every_zone_reaches_only_active_zones() {
    use nidoo_climate::doc::ZoneState;
    let hub = climate_hub();
    set_zones(
        &hub,
        &[("living", ZoneState::Active), ("attic", ZoneState::Pending)],
    )
    .await;
    save(&hub, &ambiance("all", vec![hold(None)]));

    let report = hub.apply_ambiance("all").await.expect("apply");

    assert_eq!(outcomes(&report), [Outcome::Ok]);
    assert_eq!(report.actions[0].total, 1);
    assert!(report.actions[0].failed.is_empty());
    assert_eq!(overrides_of(&hub, "living"), 1);
    assert_eq!(overrides_of(&hub, "attic"), 0);
}

// Final-review fixes.

#[tokio::test]
async fn bridge_switch_and_z2m_group_are_not_category_members() {
    let (hub, rx) = hub_with_timeout(50);
    hub.ingest(
        "nidoo/switch/1221051039810110150109113116116_0x00124b0022983b3e/permit_join/config",
        &fixture(
            "discovery/switch__1221051039810110150109113116116_0x00124b0022983b3e__permit_join.json",
        ),
    );
    hub.ingest(
        "nidoo/light/1221051039810110150109113116116_1/light/config",
        &fixture("discovery/light__1221051039810110150109113116116_1__light.json"),
    );
    // Typed as a plug, so only the bridge rule keeps it out.
    hub.ingest(
        "nidoo/ui/bridge_0x00124b0022983b3e_permit_join_zigbee2mqtt",
        br#"{"showAs":"plug"}"#,
    );
    let log = echo(rx, Arc::clone(&hub), false);
    let plugs = AmbianceAction::Category(CategoryAction {
        what: Category::Plugs,
        to: Target::Off,
        brightness: None,
        area: None,
        except: vec![],
    });
    save(&hub, &ambiance("night", vec![plugs, lights(Target::Off)]));

    let report = hub.apply_ambiance("night").await.expect("apply");

    assert_eq!(outcomes(&report), [Outcome::Skipped, Outcome::Skipped]);
    assert!(report.actions.iter().all(|a| a.total == 0), "{report:?}");
    tokio::task::yield_now().await;
    assert!(published(&log).is_empty(), "{:?}", published(&log));
}

/// Waits for the next `AmbianceApplied` broadcast.
async fn next_applied(events: &mut tokio::sync::broadcast::Receiver<WsEvent>) -> ApplyReport {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let WsEvent::AmbianceApplied { report } = events.recv().await.expect("open") {
                return report;
            }
        }
    })
    .await
    .expect("an AmbianceApplied within 2 s")
}

#[tokio::test]
async fn dropped_apply_still_finishes_and_reports() {
    let (hub, rx) = led_hub(200, true);
    hub.ingest(SWITCH_TOPIC, &fixture(SWITCH_FIXTURE));
    let _log = echo(rx, Arc::clone(&hub), false);
    // The switch never echoes, so the run outlives the caller's patience.
    save(
        &hub,
        &ambiance("a", vec![device(SWITCH_ID, turn_on()), lights(Target::Off)]),
    );
    let mut events = hub.subscribe();

    let dropped = tokio::time::timeout(Duration::from_millis(20), hub.apply_ambiance("a")).await;
    assert!(dropped.is_err(), "the caller gave up first");

    let report = next_applied(&mut events).await;
    assert_eq!(report.ambiance, "a");
    assert_eq!(outcomes(&report), [Outcome::Failed, Outcome::Ok]);
    // The run is over, so the lock is free again.
    let again = hub.apply_ambiance("a").await.expect("runs again");
    assert_eq!(again.actions.len(), 2);
}

#[tokio::test]
async fn resave_with_an_unchanged_action_on_a_vanished_entity_is_accepted() {
    let (hub, _rx) = hub();
    hub.ingest(SWITCH_TOPIC, &fixture(SWITCH_FIXTURE));
    let mut a = ambiance("a", vec![device(SWITCH_ID, turn_on())]);
    hub.set_ambiance(a.clone()).await.expect("first save");
    hub.ingest(SWITCH_TOPIC, b"");

    a.pinned = !a.pinned;
    hub.set_ambiance(a.clone())
        .await
        .expect("an unchanged action is not checked again");

    let mut changed = a.clone();
    changed.actions[0] = device(SWITCH_ID, Command::TurnOff);
    assert!(rejected(hub.set_ambiance(changed).await).contains(SWITCH_ID));

    a.actions.push(device("ghost", Command::TurnOff));
    let why = rejected(hub.set_ambiance(a).await);
    assert!(why.contains("action 1") && why.contains("ghost"), "{why}");
}

#[tokio::test]
async fn every_target_offline_is_said_once_for_the_action() {
    let (hub, rx) = hub_with_timeout(50);
    let mut config: serde_json::Value = serde_json::from_slice(&fixture(TV_FIXTURE)).expect("json");
    config["availability"][0]["topic"] = "test/tv_availability".into();
    hub.ingest(TV_TOPIC, config.to_string().as_bytes());
    hub.ingest(TV_STATE_TOPIC, br#"{"state":"ON","brightness":100}"#);
    hub.ingest("test/tv_availability", br#"{"state":"offline"}"#);
    let log = echo(rx, Arc::clone(&hub), false);
    save(
        &hub,
        &ambiance(
            "night",
            vec![lights(Target::Off), device(TV_ID, Command::TurnOff)],
        ),
    );

    let report = hub.apply_ambiance("night").await.expect("apply");

    assert_eq!(outcomes(&report), [Outcome::Skipped, Outcome::Skipped]);
    for action in &report.actions {
        assert_eq!(action.reason.as_deref(), Some("every target is offline"));
        assert_eq!(action.skipped.len(), 1);
    }
    tokio::task::yield_now().await;
    assert!(published(&log).is_empty(), "{:?}", published(&log));
}

#[tokio::test]
async fn category_in_a_vanished_area_is_skipped() {
    let (hub, rx) = led_hub(50, true);
    let log = echo(rx, Arc::clone(&hub), false);
    save(
        &hub,
        &ambiance("night", vec![category(Some("gone_room"), vec![])]),
    );

    let report = hub.apply_ambiance("night").await.expect("apply");

    assert_eq!(outcomes(&report), [Outcome::Skipped]);
    assert_eq!(
        report.actions[0].reason.as_deref(),
        Some("no longer exists")
    );
    tokio::task::yield_now().await;
    assert!(published(&log).is_empty(), "{:?}", published(&log));
}

#[tokio::test]
async fn a_large_category_does_not_overflow_the_publish_queue() {
    let (hub, mut publisher) = hub_with_timeout(500);
    let base: serde_json::Value = serde_json::from_slice(&fixture(LED_FIXTURE)).expect("json");
    for n in 0..80 {
        let mut config = base.clone();
        config["unique_id"] = format!("bulk{n}_light").into();
        config["device"]["identifiers"] = serde_json::json!([format!("bulk{n}")]);
        config["state_topic"] = format!("test/bulk{n}").into();
        config["command_topic"] = format!("test/bulk{n}/set").into();
        hub.ingest(
            &format!("nidoo/light/bulk{n}/light/config"),
            config.to_string().as_bytes(),
        );
        hub.ingest(&format!("test/bulk{n}"), br#"{"state":"ON"}"#);
    }
    // The broker: every light echoes the state it was sent.
    let broker = Arc::clone(&hub);
    tokio::spawn(async move {
        while let Some(publish) = publisher.recv().await {
            if let Some(state) = publish.topic.strip_suffix("/set") {
                broker.ingest(state, publish.payload.as_bytes());
            }
        }
    });
    save(&hub, &ambiance("night", vec![lights(Target::Off)]));

    let report = hub.apply_ambiance("night").await.expect("apply");

    let action = &report.actions[0];
    assert_eq!(action.total, 80);
    assert!(action.failed.is_empty(), "{:?}", action.failed);
    assert_eq!(outcomes(&report), [Outcome::Ok]);
}

#[tokio::test]
async fn set_ambiance_refuses_an_alarm_code() {
    let (hub, _rx) = hub();
    hub.ingest(
        "nidoo/alarm_control_panel/jeedom/house_alarm/config",
        &fixture("discovery/alarm_control_panel__jeedom__house_alarm.json"),
    );
    let arm = device(
        "jeedom_house_alarm",
        Command::Arm {
            mode: nidoo_core::model::AlarmMode::Away,
            code: Some("1234".to_owned()),
        },
    );
    let why = rejected(hub.set_ambiance(ambiance("a", vec![arm])).await);
    assert!(why.contains("alarm codes are not allowed"), "{why}");
    let disarm = device(
        "jeedom_house_alarm",
        Command::Disarm {
            code: Some("1234".to_owned()),
        },
    );
    let why = rejected(hub.set_ambiance(ambiance("a", vec![disarm])).await);
    assert!(why.contains("alarm codes are not allowed"), "{why}");
    assert!(hub.ambiances().is_empty());
}
