//! The alarm's settings, published panel entity and read view, driven without a broker.

use std::collections::BTreeSet;
use std::sync::Arc;

use nidoo::config::Config;
use nidoo::hub::{Hub, HubError};
use nidoo_alarm::doc::{
    AlarmSettings, ArmMode, Entry, Remote, RemoteAction, SensorKind, SensorRule, default_signals,
};
use nidoo_api::{ConnectionStatus, WsEvent};
use nidoo_core::command::Publish;
use nidoo_core::model::{Kind, StateValue};
use serde_json::json;
use tokio::sync::mpsc;

const CONFIG_TOPIC: &str = "oikos/alarm_control_panel/alarm/house/config";

fn hub() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let mut cfg = Config::default();
    cfg.topics.discovery_prefixes = vec!["oikos".to_owned()];
    let (tx, rx) = mpsc::channel(64);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    (hub, rx)
}

fn drain(rx: &mut mpsc::Receiver<Publish>) -> Vec<Publish> {
    let mut out = Vec::new();
    while let Ok(p) = rx.try_recv() {
        out.push(p);
    }
    out
}

fn contact(id: &str) -> serde_json::Value {
    json!({
        "name": id, "unique_id": id, "device_class": "door",
        "state_topic": format!("fx/{id}"),
        "availability_topic": format!("fx/{id}/avail"),
        "payload_on": "open", "payload_off": "closed",
        "device": {"identifiers": [format!("dev_{id}")], "name": id}
    })
}

fn feed(hub: &Hub, component: &str, id: &str, config: serde_json::Value) {
    hub.ingest(
        &format!("oikos/{component}/{id}/{id}/config"),
        config.to_string().as_bytes(),
    );
}

/// Two door contacts, a motion sensor, a siren switch, a remote and a tracker.
fn house(hub: &Hub) {
    feed(hub, "binary_sensor", "front_door", contact("front_door"));
    feed(hub, "binary_sensor", "back_door", contact("back_door"));
    let mut motion = contact("hall_motion");
    motion["device_class"] = json!("motion");
    feed(hub, "binary_sensor", "hall_motion", motion);
    feed(
        hub,
        "switch",
        "siren_plug",
        json!({"name": "Siren", "unique_id": "siren_plug", "state_topic": "fx/siren",
               "availability_topic": "fx/siren/avail", "command_topic": "fx/siren/set"}),
    );
    feed(
        hub,
        "sensor",
        "hall_temp",
        json!({"name": "Temp", "unique_id": "hall_temp", "state_topic": "fx/temp"}),
    );
    feed(
        hub,
        "device_tracker",
        "phone_a",
        json!({"name": "Phone", "unique_id": "phone_a", "state_topic": "fx/phone_a",
               "availability_topic": "fx/phone_a/avail",
               "payload_home": "home", "payload_not_home": "not_home"}),
    );
    feed(
        hub,
        "device_automation",
        "keyfob",
        json!({"automation_type": "trigger", "type": "action", "subtype": "arm",
               "payload": "arm", "topic": "fx/keyfob/action",
               "device": {"identifiers": ["dev_keyfob"], "name": "Keyfob"}}),
    );
}

fn entity_id(hub: &Hub, fragment: &str) -> String {
    hub.snapshot()
        .entities
        .into_iter()
        .find(|e| e.id.contains(fragment))
        .unwrap_or_else(|| panic!("no entity {fragment}"))
        .id
}

fn enabled_with(sensor: &str) -> AlarmSettings {
    AlarmSettings {
        enabled: true,
        sensors: vec![SensorRule {
            entity: sensor.to_owned(),
            kind: SensorKind::Opening,
            modes: BTreeSet::from([ArmMode::Away]),
            entry: Entry::Instant,
        }],
        ..AlarmSettings::default()
    }
}

#[tokio::test]
async fn disabled_by_default_publishes_nothing() {
    let (hub, mut rx) = hub();
    assert!(!hub.alarm_view().enabled);
    assert_eq!(hub.alarm_view().state, "disarmed");
    assert!(
        !drain(&mut rx)
            .iter()
            .any(|p| p.topic.contains("alarm_control_panel"))
    );
    assert!(
        hub.extra_filters()
            .contains(&"oikos/alarm/settings".to_owned())
    );
    assert!(
        hub.extra_filters()
            .contains(&"oikos/alarm/state".to_owned())
    );
    assert!(
        hub.extra_filters()
            .contains(&"oikos/alarm/panel/set".to_owned())
    );
}

#[tokio::test]
async fn enable_publishes_entity() {
    let (hub, mut rx) = hub();
    house(&hub);
    let mut events = hub.subscribe();
    let sensor = entity_id(&hub, "front_door");
    hub.set_alarm_settings(enabled_with(&sensor)).await.unwrap();

    let published = drain(&mut rx);
    let config = published
        .iter()
        .find(|p| p.topic == CONFIG_TOPIC)
        .expect("config published");
    assert!(config.retain);
    let doc: serde_json::Value = serde_json::from_str(&config.payload).unwrap();
    assert_eq!(
        doc["supported_features"],
        json!(["arm_away", "arm_home", "arm_night"])
    );
    assert_eq!(doc["code_arm_required"], json!(false));
    assert_eq!(doc["unique_id"], json!("oikos_alarm_house"));
    assert!(
        published
            .iter()
            .any(|p| p.topic == "oikos/alarm/settings" && p.retain)
    );
    // The runtime is persisted before the panel state is published (R6).
    let at = |topic: &str| published.iter().position(|p| p.topic == topic).unwrap();
    assert!(at("oikos/alarm/state") < at("oikos/alarm/panel"));
    assert_eq!(
        published
            .iter()
            .find(|p| p.topic == "oikos/alarm/panel")
            .unwrap()
            .payload,
        "disarmed"
    );

    let view = hub.entity_view("oikos_alarm_house").expect("panel entity");
    assert_eq!(view.kind, Kind::AlarmPanel);
    assert_eq!(view.state.value, StateValue::Alarm("disarmed".to_owned()));
    assert!(hub.alarm_view().enabled);

    let mut saw = false;
    while let Ok(event) = events.try_recv() {
        saw |= matches!(event, WsEvent::AlarmChanged);
    }
    assert!(saw, "AlarmChanged was broadcast");
}

#[tokio::test]
async fn disable_removes_entity_keeps_settings() {
    let (hub, mut rx) = hub();
    house(&hub);
    let sensor = entity_id(&hub, "front_door");
    let mut settings = enabled_with(&sensor);
    hub.set_alarm_settings(settings.clone()).await.unwrap();
    assert!(hub.entity_view("oikos_alarm_house").is_some());
    drain(&mut rx);

    settings.enabled = false;
    hub.set_alarm_settings(settings).await.unwrap();
    let published = drain(&mut rx);
    let removal = published.iter().find(|p| p.topic == CONFIG_TOPIC).unwrap();
    assert!(removal.retain && removal.payload.is_empty());
    assert!(hub.entity_view("oikos_alarm_house").is_none());
    assert_eq!(hub.alarm_settings().sensors.len(), 1);
    assert!(!hub.alarm_view().enabled);
}

#[tokio::test]
async fn enable_seeds_runtime_from_live_trackers() {
    let (hub, mut rx) = hub();
    house(&hub);
    hub.ingest("fx/phone_a", b"home");
    let sensor = entity_id(&hub, "front_door");
    let tracker = entity_id(&hub, "phone_a");
    let mut settings = enabled_with(&sensor);
    settings.presence.trackers = vec![tracker.clone()];
    hub.set_alarm_settings(settings).await.unwrap();
    let state = drain(&mut rx)
        .into_iter()
        .find(|p| p.topic == "oikos/alarm/state")
        .unwrap();
    let doc: serde_json::Value = serde_json::from_str(&state.payload).unwrap();
    assert_eq!(doc["trackers"][&tracker], json!("home"));
    assert_eq!(doc["auto_arm_ready"], json!(true));
}

#[tokio::test]
async fn rejects_unknown_or_wrong_kind_entities() {
    let (hub, _rx) = hub();
    house(&hub);
    let unknown = enabled_with("no_such_entity");
    assert!(matches!(
        hub.set_alarm_settings(unknown).await,
        Err(HubError::Rejected(_))
    ));
    let switch = entity_id(&hub, "siren_plug");
    assert!(matches!(
        hub.set_alarm_settings(enabled_with(&switch)).await,
        Err(HubError::Rejected(_))
    ));
    let sensor = entity_id(&hub, "front_door");
    let mut bad_remote = enabled_with(&sensor);
    bad_remote.remotes = vec![Remote {
        entity: entity_id(&hub, "hall_temp"),
        signals: default_signals(),
    }];
    assert!(matches!(
        hub.set_alarm_settings(bad_remote).await,
        Err(HubError::Rejected(_))
    ));
    let mut bad_tracker = enabled_with(&sensor);
    bad_tracker.presence.trackers = vec![sensor.clone()];
    assert!(matches!(
        hub.set_alarm_settings(bad_tracker).await,
        Err(HubError::Rejected(_))
    ));
    assert!(!hub.alarm_view().enabled);
}

#[tokio::test]
async fn accepts_siren_switch_remote_and_tracker() {
    let (hub, _rx) = hub();
    house(&hub);
    let mut settings = enabled_with(&entity_id(&hub, "front_door"));
    settings.sirens = vec![nidoo_alarm::doc::Siren::Entity {
        entity: entity_id(&hub, "siren_plug"),
        chirp_ms: None,
    }];
    settings.remotes = vec![Remote {
        entity: entity_id(&hub, "keyfob"),
        signals: [("arm".to_owned(), RemoteAction::Away)].into(),
    }];
    settings.presence.trackers = vec![entity_id(&hub, "phone_a")];
    hub.set_alarm_settings(settings).await.unwrap();
}

#[tokio::test]
async fn invalid_retained_settings_keep_previous() {
    let (hub, _rx) = hub();
    let good = json!({"enabled": false, "siren_s": 30}).to_string();
    hub.ingest_message("oikos/alarm/settings", good.as_bytes(), true);
    assert_eq!(hub.alarm_settings().siren_s, 30);
    assert!(hub.alarm_view().settings_invalid.is_none());
    hub.ingest_message("oikos/alarm/settings", br#"{"siren_s": 5}"#, true);
    assert!(hub.alarm_view().settings_invalid.is_some());
    assert_eq!(hub.alarm_settings().siren_s, 30);
}

#[tokio::test]
async fn view_warns_about_missing_entities() {
    let (hub, _rx) = hub();
    let doc = serde_json::to_string(&enabled_with("ghost_door")).unwrap();
    hub.ingest_message("oikos/alarm/settings", doc.as_bytes(), true);
    let warnings = hub.alarm_view().warnings;
    assert!(
        warnings.iter().any(|w| w.contains("ghost_door")),
        "{warnings:?}"
    );
}

#[tokio::test]
async fn alarm_documents_are_not_devices() {
    let (hub, _rx) = hub();
    let changes = hub.ingest_message("oikos/alarm/panel/set", b"ARM_AWAY", false);
    assert!(changes.is_empty());
    hub.ingest_message("oikos/alarm/state", b"not json", true);
    assert!(hub.snapshot().entities.is_empty());
}

/// A hub with the engine running and the alarm armed-capable: the door is closed, the engine
/// has settled, and the hub's own retained publishes are echoed back as the broker would.
struct Rig {
    hub: Arc<Hub>,
    rx: mpsc::Receiver<Publish>,
    /// Everything the hub published since the last `clear`.
    log: Vec<Publish>,
    door: String,
    task: tokio::task::JoinHandle<()>,
}

impl Rig {
    async fn new() -> Rig {
        Rig::with(|_, _| {}).await
    }

    async fn with(edit: impl FnOnce(&Hub, &mut AlarmSettings)) -> Rig {
        let (hub, rx) = hub();
        house(&hub);
        let door = entity_id(&hub, "front_door");
        hub.ingest("fx/front_door", b"closed");
        let mut settings = enabled_with(&door);
        edit(&hub, &mut settings);
        hub.set_alarm_settings(settings).await.unwrap();
        // The push service accepts alerts once its token map has settled.
        hub.on_connected();
        hub.on_suback();
        let task = hub.start_alarm();
        let mut rig = Rig {
            hub,
            rx,
            log: Vec::new(),
            door,
            task,
        };
        // The engine reaches its boot settle before the clock moves: it reads the wall clock
        // again when it resumes, and the paused clock never moves that.
        rig.pump().await;
        rig.advance(6).await;
        rig
    }

    /// Lets the engine run, then echoes what the hub published back to it.
    async fn pump(&mut self) {
        for _ in 0..20 {
            tokio::task::yield_now().await;
            for p in drain(&mut self.rx) {
                self.hub
                    .ingest_message(&p.topic, p.payload.as_bytes(), p.retain);
                // The siren switch echoes its command, as a device would.
                if p.topic == "fx/siren/set" {
                    self.hub
                        .ingest_message("fx/siren", p.payload.as_bytes(), false);
                }
                self.log.push(p);
            }
        }
    }

    async fn advance(&mut self, secs: u64) {
        tokio::time::advance(std::time::Duration::from_secs(secs)).await;
        self.pump().await;
    }

    fn state(&self) -> String {
        self.hub.alarm_view().state
    }

    /// The payloads published on `topic` since the last `clear`.
    fn sent(&self, topic: &str) -> Vec<String> {
        self.log
            .iter()
            .filter(|p| p.topic == topic)
            .map(|p| p.payload.clone())
            .collect()
    }

    fn clear(&mut self) {
        self.log.clear();
    }

    /// The alarm's alerts, newest first.
    fn alarm_alerts(&self) -> Vec<nidoo_api::AlertView> {
        self.hub
            .alerts()
            .into_iter()
            .filter(|a| a.id.starts_with("alarm-"))
            .collect()
    }

    /// The critical trigger alerts, newest first (no armed/disarmed note, no offline warning).
    fn trigger_alerts(&self) -> Vec<nidoo_api::AlertView> {
        self.alarm_alerts()
            .into_iter()
            .filter(|a| is_alert(a, "critical"))
            .collect()
    }

    fn panel(&self) -> StateValue {
        self.hub
            .entity_view("oikos_alarm_house")
            .unwrap()
            .state
            .value
    }

    async fn command(&mut self, cmd: nidoo_core::model::Command) -> nidoo_api::CommandResponse {
        let hub = Arc::clone(&self.hub);
        let reply = hub.send_command("oikos_alarm_house", cmd).await.unwrap();
        self.pump().await;
        reply
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn arm(mode: nidoo_core::model::AlarmMode) -> nidoo_core::model::Command {
    nidoo_core::model::Command::Arm { mode, code: None }
}

#[tokio::test(start_paused = true)]
async fn send_command_arms_and_disarms() {
    use nidoo_core::model::{AlarmMode, Command};
    let mut rig = Rig::new().await;
    let reply = rig.command(arm(AlarmMode::Away)).await;
    assert!(reply.accepted, "{reply:?}");
    assert_eq!(rig.state(), "arming");
    assert_eq!(rig.panel(), StateValue::Alarm("arming".to_owned()));
    let reply = rig.command(Command::Disarm { code: None }).await;
    assert!(reply.accepted);
    assert_eq!(rig.state(), "disarmed");
    assert_eq!(rig.panel(), StateValue::Alarm("disarmed".to_owned()));
}

#[tokio::test(start_paused = true)]
async fn vacation_mode_refused() {
    let mut rig = Rig::new().await;
    let reply = rig
        .command(arm(nidoo_core::model::AlarmMode::Vacation))
        .await;
    assert!(!reply.accepted);
    assert!(reply.reason.is_some());
    assert_eq!(rig.state(), "disarmed");
}

#[tokio::test(start_paused = true)]
async fn commands_are_refused_while_disabled() {
    use nidoo_core::model::AlarmMode;
    let (hub, _rx) = hub();
    let reply = hub
        .send_command("oikos_alarm_house", arm(AlarmMode::Away))
        .await
        .unwrap();
    assert!(!reply.accepted);
    assert_eq!(reply.reason.as_deref(), Some("alarm disabled"));
}

#[tokio::test(start_paused = true)]
async fn raw_mqtt_command_arms() {
    let mut rig = Rig::new().await;
    // A retained command is a replay, never a press.
    rig.hub
        .ingest_message("oikos/alarm/panel/set", b"ARM_NIGHT", true);
    rig.pump().await;
    assert_eq!(rig.state(), "disarmed");
    rig.hub
        .ingest_message("oikos/alarm/panel/set", b"ARM_NIGHT", false);
    rig.pump().await;
    assert_eq!(rig.state(), "arming");
}

#[tokio::test(start_paused = true)]
async fn replayed_states_never_trigger() {
    use nidoo_core::model::AlarmMode;
    let mut rig = Rig::new().await;
    rig.command(arm(AlarmMode::Away)).await;
    rig.advance(61).await;
    assert_eq!(rig.state(), "armed_away");

    rig.hub.set_connection(ConnectionStatus::Offline {
        reason: "test".to_owned(),
    });
    rig.pump().await;
    rig.hub.set_connection(ConnectionStatus::Online);
    rig.pump().await;
    // The broker replays the retained contact as open: no previous value is known.
    rig.hub.ingest_message("fx/front_door", b"open", true);
    rig.pump().await;
    assert_eq!(rig.state(), "armed_away");
    rig.advance(10).await;
    assert_eq!(rig.state(), "armed_away");
}

#[tokio::test(start_paused = true)]
async fn door_open_triggers_via_registry() {
    use nidoo_core::model::AlarmMode;
    let mut rig = Rig::new().await;
    rig.command(arm(AlarmMode::Away)).await;
    rig.advance(61).await;
    assert_eq!(rig.state(), "armed_away");

    rig.hub.ingest_message("fx/front_door", b"open", false);
    rig.pump().await;
    assert_eq!(rig.state(), "triggered");
    assert_eq!(rig.panel(), StateValue::Alarm("triggered".to_owned()));
    let attrs = rig
        .hub
        .entity_view("oikos_alarm_house")
        .unwrap()
        .state
        .attributes;
    assert_eq!(attrs["cause"], json!([rig.door]), "{attrs:?}");
}

#[tokio::test(start_paused = true)]
async fn runtime_persisted_retained() {
    use nidoo_alarm::machine::{Phase, Runtime};
    use nidoo_core::model::AlarmMode;
    let mut rig = Rig::new().await;
    let hub = Arc::clone(&rig.hub);
    let reply = hub
        .send_command("oikos_alarm_house", arm(AlarmMode::Away))
        .await
        .unwrap();
    assert!(reply.accepted);
    let seen = drain(&mut rig.rx);
    let last = seen
        .iter()
        .rev()
        .find(|p| p.topic == "oikos/alarm/state")
        .expect("runtime published");
    assert!(last.retain);
    let rt: Runtime = serde_json::from_str(&last.payload).unwrap();
    assert!(matches!(rt.phase, Phase::Arming { .. }));
}

#[tokio::test(start_paused = true)]
async fn remote_arm_after_idle_gets_a_fresh_exit_delay() {
    let mut rig = Rig::with(|hub, settings| {
        settings.remotes = vec![Remote {
            entity: entity_id(hub, "keyfob"),
            signals: [("arm".to_owned(), RemoteAction::Away)].into(),
        }];
    })
    .await;
    rig.advance(1000).await;
    rig.hub.ingest_message("fx/keyfob/action", b"arm", false);
    rig.pump().await;
    assert_eq!(rig.state(), "arming");
}

#[tokio::test(start_paused = true)]
async fn boot_settle_backlog_is_not_replayed_against_the_seed() {
    use nidoo_alarm::machine::{Phase, Runtime};
    let (hub, mut rx) = hub();
    house(&hub);
    let door = entity_id(&hub, "front_door");
    hub.ingest("fx/front_door", b"closed");
    hub.ingest_message(
        "oikos/alarm/settings",
        serde_json::to_string(&enabled_with(&door))
            .unwrap()
            .as_bytes(),
        true,
    );
    let armed = Runtime {
        phase: Phase::Armed {
            mode: ArmMode::Away,
        },
        ..Runtime::default()
    };
    hub.ingest_message(
        "oikos/alarm/state",
        serde_json::to_string(&armed).unwrap().as_bytes(),
        true,
    );
    let task = hub.start_alarm();
    // Within the settle: a stale replay says open, then the live state says closed.
    hub.ingest_message("fx/front_door", b"open", true);
    hub.ingest_message("fx/front_door", b"closed", false);
    for _ in 0..3 {
        tokio::time::advance(std::time::Duration::from_secs(3)).await;
        for _ in 0..20 {
            tokio::task::yield_now().await;
            drain(&mut rx);
        }
    }
    assert_eq!(hub.alarm_view().state, "armed_away");
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn alarm_changed_during_an_outage_does_not_reseed() {
    use nidoo_core::model::AlarmMode;
    let mut rig = Rig::new().await;
    rig.command(arm(AlarmMode::Away)).await;
    rig.advance(61).await;
    assert_eq!(rig.state(), "armed_away");

    rig.hub.set_connection(ConnectionStatus::Offline {
        reason: "test".to_owned(),
    });
    rig.pump().await;
    // A settings document arriving while the link is down announces AlarmChanged.
    let mut settings = rig.hub.alarm_settings();
    settings.siren_s = 60;
    let doc = serde_json::to_string(&settings).unwrap();
    rig.hub
        .ingest_message("oikos/alarm/settings", doc.as_bytes(), true);
    rig.pump().await;
    rig.hub.set_connection(ConnectionStatus::Online);
    rig.pump().await;
    rig.hub.ingest_message("fx/front_door", b"open", true);
    rig.pump().await;
    assert_eq!(rig.state(), "armed_away");
}

const SIREN_SET: &str = "fx/siren/set";
const MQTT_SIREN: &str = "dev/siren/set";

/// A switch siren and a raw MQTT siren with a chirp.
fn with_sirens(hub: &Hub, settings: &mut AlarmSettings) {
    settings.sirens = vec![
        nidoo_alarm::doc::Siren::Entity {
            entity: entity_id(hub, "siren_plug"),
            chirp_ms: None,
        },
        nidoo_alarm::doc::Siren::Mqtt {
            mqtt: nidoo_alarm::doc::MqttSiren {
                topic: MQTT_SIREN.to_owned(),
                on: "ON".to_owned(),
                off: "OFF".to_owned(),
                chirp: Some("BEEP".to_owned()),
            },
        },
    ];
}

/// Armed away with both sirens, the door closed.
async fn armed_rig() -> Rig {
    let mut rig = Rig::with(with_sirens).await;
    rig.command(arm(nidoo_core::model::AlarmMode::Away)).await;
    rig.advance(61).await;
    assert_eq!(rig.state(), "armed_away");
    rig.clear();
    rig
}

async fn trip(rig: &mut Rig) {
    rig.hub.ingest_message("fx/front_door", b"open", false);
    rig.pump().await;
    assert_eq!(rig.state(), "triggered");
}

fn is_alert(alert: &nidoo_api::AlertView, severity: &str) -> bool {
    serde_json::to_value(alert.severity).unwrap() == severity
}

#[tokio::test(start_paused = true)]
async fn trigger_rings_both_siren_kinds_and_pushes() {
    let mut rig = armed_rig().await;
    trip(&mut rig).await;
    assert_eq!(rig.sent(SIREN_SET), ["ON"]);
    assert_eq!(rig.sent(MQTT_SIREN), ["ON"]);
    let alerts = rig.trigger_alerts();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    let alert = &alerts[0];
    assert_eq!(alert.channel, nidoo_api::AlertChannel::Alarm);
    assert!(is_alert(alert, "critical"));
    assert_eq!(alert.title, "Alarm");
    assert!(alert.body.ends_with("opened"), "{}", alert.body);
    let actions: Vec<(&str, &str)> = alert
        .actions
        .iter()
        .map(|a| (a.id.as_str(), a.label.as_str()))
        .collect();
    assert_eq!(actions, [("disarm", "Disarm")]);
    assert!(!alert.resolved);
}

#[tokio::test(start_paused = true)]
async fn ring_stops_after_siren_s() {
    let mut rig = armed_rig().await;
    trip(&mut rig).await;
    rig.clear();
    rig.advance(180).await;
    assert_eq!(rig.sent(SIREN_SET), ["OFF"]);
    assert_eq!(rig.sent(MQTT_SIREN), ["OFF"]);
    assert_eq!(rig.state(), "armed_away");
    let alerts = rig.trigger_alerts();
    assert_eq!(alerts.len(), 1);
    assert!(!alerts[0].resolved, "the alert stays until a disarm");
}

#[tokio::test(start_paused = true)]
async fn arming_chirps() {
    let mut rig = Rig::with(with_sirens).await;
    rig.clear();
    rig.command(arm(nidoo_core::model::AlarmMode::Away)).await;
    assert!(rig.sent(MQTT_SIREN).is_empty(), "no chirp at the command");
    rig.advance(61).await;
    assert_eq!(rig.state(), "armed_away");
    assert_eq!(rig.sent(MQTT_SIREN), ["BEEP"]);
    assert!(rig.sent(SIREN_SET).is_empty(), "the switch has no chirp_ms");
}

#[tokio::test]
async fn test_chirp_publishes_chirp() {
    let (hub, mut rx) = hub();
    house(&hub);
    let mut settings = AlarmSettings::default();
    with_sirens(&hub, &mut settings);
    settings.sirens.push(nidoo_alarm::doc::Siren::Mqtt {
        mqtt: nidoo_alarm::doc::MqttSiren {
            topic: "dev/quiet/set".to_owned(),
            on: "ON".to_owned(),
            off: "OFF".to_owned(),
            chirp: None,
        },
    });
    // The alarm stays disabled: the chirp is a setup aid.
    hub.set_alarm_settings(settings).await.unwrap();
    drain(&mut rx);

    hub.alarm_test_chirp(1).unwrap();
    let sent = drain(&mut rx);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(
        (sent[0].topic.as_str(), sent[0].payload.as_str()),
        (MQTT_SIREN, "BEEP")
    );

    for index in [0, 2, 9] {
        let err = hub.alarm_test_chirp(index).unwrap_err();
        assert!(matches!(err, HubError::Rejected(_)), "{index}: {err:?}");
    }
    assert!(drain(&mut rx).is_empty());
}

#[tokio::test(start_paused = true)]
async fn test_chirp_entity_siren_pulses_on_then_off() {
    let mut rig = Rig::with(|hub, settings| {
        settings.sirens = vec![nidoo_alarm::doc::Siren::Entity {
            entity: entity_id(hub, "siren_plug"),
            chirp_ms: Some(500),
        }];
    })
    .await;
    rig.clear();
    rig.hub.alarm_test_chirp(0).unwrap();
    rig.pump().await;
    assert_eq!(rig.sent(SIREN_SET), ["ON"], "on right away");
    rig.advance(1).await;
    assert_eq!(rig.sent(SIREN_SET), ["ON", "OFF"], "off after chirp_ms");
}

#[tokio::test(start_paused = true)]
async fn test_chirp_reports_a_refused_publish() {
    let rig = Rig::with(with_sirens).await;
    rig.hub.set_connection(ConnectionStatus::Offline {
        reason: "test".to_owned(),
    });
    let err = rig.hub.alarm_test_chirp(1).unwrap_err();
    assert!(matches!(err, HubError::Publish(_)), "{err:?}");
}

#[tokio::test(start_paused = true)]
async fn push_disarm_reply_disarms() {
    let mut rig = armed_rig().await;
    trip(&mut rig).await;
    rig.clear();
    let id = rig.trigger_alerts()[0].id.clone();
    let reply = json!({"id": id, "action": "disarm", "install_id": "x", "ts": 1}).to_string();
    // A retained reply is a replay, never a press.
    rig.hub
        .ingest_message("oikos/push/responses", reply.as_bytes(), true);
    rig.pump().await;
    assert_eq!(rig.state(), "triggered");
    rig.hub
        .ingest_message("oikos/push/responses", reply.as_bytes(), false);
    rig.pump().await;
    assert_eq!(rig.state(), "disarmed");
    assert_eq!(rig.sent(SIREN_SET), ["OFF"]);
    assert_eq!(rig.sent(MQTT_SIREN), ["OFF"]);
    assert!(rig.trigger_alerts().iter().all(|a| a.resolved));
}

#[tokio::test(start_paused = true)]
async fn other_replies_are_ignored() {
    let mut rig = armed_rig().await;
    trip(&mut rig).await;
    let id = rig.trigger_alerts()[0].id.clone();
    for reply in [
        json!({"id": id, "action": "snooze"}),
        json!({"id": "frigate-1", "action": "disarm"}),
    ] {
        rig.hub
            .ingest_message("oikos/push/responses", reply.to_string().as_bytes(), false);
    }
    rig.pump().await;
    assert_eq!(rig.state(), "triggered");
}

#[tokio::test(start_paused = true)]
async fn failing_siren_does_not_block_others() {
    let mut rig = Rig::with(|hub, settings| {
        with_sirens(hub, settings);
        hub.ingest("fx/siren/avail", b"offline");
    })
    .await;
    rig.command(arm(nidoo_core::model::AlarmMode::Away)).await;
    rig.advance(61).await;
    rig.clear();
    trip(&mut rig).await;
    assert!(rig.sent(SIREN_SET).is_empty());
    assert_eq!(rig.sent(MQTT_SIREN), ["ON"]);
    let alerts = rig.trigger_alerts();
    assert!(
        alerts[0].body.contains("Siren failed"),
        "{}",
        alerts[0].body
    );
}

#[tokio::test(start_paused = true)]
async fn open_window_at_arm_is_in_the_armed_push() {
    let mut rig = Rig::with(|hub, settings| {
        settings.sensors.push(SensorRule {
            entity: entity_id(hub, "back_door"),
            kind: SensorKind::Opening,
            modes: BTreeSet::from([ArmMode::Away]),
            entry: Entry::Instant,
        });
        hub.ingest("fx/back_door", b"open");
    })
    .await;
    rig.command(arm(nidoo_core::model::AlarmMode::Away)).await;
    rig.advance(61).await;
    assert_eq!(rig.state(), "armed_away");
    let alerts = rig.alarm_alerts();
    assert_eq!(
        alerts.len(),
        1,
        "one push, not an armed one and a note: {alerts:?}"
    );
    assert!(is_alert(&alerts[0], "info"));
    assert_eq!(
        alerts[0].body,
        "Armed away. back_door open, watched for closing"
    );
}

#[tokio::test(start_paused = true)]
async fn arming_and_disarming_push_with_sound() {
    use nidoo_core::model::{AlarmMode, Command};
    let mut rig = Rig::new().await;
    rig.command(arm(AlarmMode::Night)).await;
    assert!(
        rig.alarm_alerts().is_empty(),
        "no push while the exit delay runs"
    );
    rig.advance(61).await;
    assert_eq!(rig.state(), "armed_night");
    let alerts = rig.alarm_alerts();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert!(is_alert(&alerts[0], "info"));
    assert!(!alerts[0].silent);
    assert_eq!(alerts[0].channel, nidoo_api::AlertChannel::Alarm);
    assert_eq!(alerts[0].body, "Armed night");
    assert!(alerts[0].actions.is_empty());

    rig.command(Command::Disarm { code: None }).await;
    let alerts = rig.alarm_alerts();
    assert_eq!(alerts.len(), 2, "{alerts:?}");
    assert_eq!(alerts[0].body, "Disarmed");
    assert!(!alerts[0].silent);
    assert_ne!(alerts[0].id, alerts[1].id, "a log, not a replaced alert");

    // Disarming an alarm that is already disarmed says nothing.
    rig.command(Command::Disarm { code: None }).await;
    assert_eq!(rig.alarm_alerts().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn cancelled_exit_delay_pushes_disarmed_only() {
    use nidoo_core::model::{AlarmMode, Command};
    let mut rig = Rig::new().await;
    rig.command(arm(AlarmMode::Away)).await;
    rig.command(Command::Disarm { code: None }).await;
    let bodies: Vec<String> = rig.alarm_alerts().into_iter().map(|a| a.body).collect();
    assert_eq!(bodies, ["Disarmed"]);
}

#[tokio::test(start_paused = true)]
async fn offline_sensor_warns_once_and_resolves() {
    let mut rig = armed_rig().await;
    let offline = format!("alarm-offline-{}", rig.door);
    for _ in 0..2 {
        rig.hub
            .ingest_message("fx/front_door/avail", b"offline", false);
        rig.pump().await;
    }
    let warnings: Vec<_> = rig
        .alarm_alerts()
        .into_iter()
        .filter(|a| a.id == offline)
        .collect();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(is_alert(&warnings[0], "warning"));
    assert_eq!(warnings[0].body, "Alarm sensor offline: front_door");
    assert!(!warnings[0].resolved);

    rig.hub
        .ingest_message("fx/front_door/avail", b"online", false);
    rig.pump().await;
    let warning = rig
        .alarm_alerts()
        .into_iter()
        .find(|a| a.id == offline)
        .unwrap();
    assert!(warning.resolved);
    // A new outage is a new warning.
    rig.hub
        .ingest_message("fx/front_door/avail", b"offline", false);
    rig.pump().await;
    let warning = rig
        .alarm_alerts()
        .into_iter()
        .find(|a| a.id == offline)
        .unwrap();
    assert!(!warning.resolved);
    // Disarming resolves it.
    rig.command(nidoo_core::model::Command::Disarm { code: None })
        .await;
    let warning = rig
        .alarm_alerts()
        .into_iter()
        .find(|a| a.id == offline)
        .unwrap();
    assert!(warning.resolved);
}

/// A hub whose retained settings and runtime are in place before the engine boots.
fn boot_hub(
    edit: impl FnOnce(&Hub, &mut AlarmSettings),
    runtime: impl FnOnce(&Hub) -> nidoo_alarm::machine::Runtime,
) -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let (hub, rx) = hub();
    house(&hub);
    hub.ingest("fx/front_door", b"closed");
    let mut settings = enabled_with(&entity_id(&hub, "front_door"));
    edit(&hub, &mut settings);
    hub.ingest_message(
        "oikos/alarm/settings",
        serde_json::to_string(&settings).unwrap().as_bytes(),
        true,
    );
    hub.ingest_message(
        "oikos/alarm/state",
        serde_json::to_string(&runtime(&hub)).unwrap().as_bytes(),
        true,
    );
    hub.on_connected();
    hub.on_suback();
    (hub, rx)
}

#[tokio::test(start_paused = true)]
async fn tracker_change_while_down_disarms_on_boot() {
    use nidoo_alarm::machine::{Phase, Presence, Runtime};
    let (hub, mut rx) = boot_hub(
        |hub, settings| {
            settings.presence.trackers = vec![entity_id(hub, "phone_a")];
            hub.ingest("fx/phone_a", b"home");
        },
        |hub| Runtime {
            phase: Phase::Armed {
                mode: ArmMode::Away,
            },
            trackers: [(entity_id(hub, "phone_a"), Presence::Away)].into(),
            ..Runtime::default()
        },
    );
    let task = hub.start_alarm();
    for _ in 0..3 {
        tokio::time::advance(std::time::Duration::from_secs(3)).await;
        for _ in 0..20 {
            tokio::task::yield_now().await;
            drain(&mut rx);
        }
    }
    assert_eq!(hub.alarm_view().state, "disarmed");
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn restart_resumes_ringing() {
    use nidoo_alarm::machine::{Phase, Runtime};
    let now = nidoo::clock::now();
    let runtime = Runtime {
        phase: Phase::Triggered {
            back_to: Some(ArmMode::Away),
            until: now + 100,
        },
        cause: vec!["x".to_owned()],
        ..Runtime::default()
    };
    let (hub, rx) = boot_hub(with_sirens, |_| runtime);
    let task = hub.start_alarm();
    let mut rig = Rig {
        door: entity_id(&hub, "front_door"),
        hub,
        rx,
        log: Vec::new(),
        task,
    };
    // Let the engine reach its boot settle before the clock moves.
    rig.pump().await;
    rig.advance(6).await;
    assert_eq!(rig.state(), "triggered");
    assert_eq!(rig.sent(SIREN_SET), ["ON"]);
    assert_eq!(rig.sent(MQTT_SIREN), ["ON"]);
    rig.clear();
    rig.advance(100).await;
    assert_eq!(rig.sent(SIREN_SET), ["OFF"]);
    assert_eq!(rig.sent(MQTT_SIREN), ["OFF"]);
    assert_eq!(rig.state(), "armed_away");
}

#[tokio::test(start_paused = true)]
async fn disable_while_ringing_silences() {
    let mut rig = armed_rig().await;
    trip(&mut rig).await;
    rig.clear();
    let mut settings = rig.hub.alarm_settings();
    settings.enabled = false;
    // The switch siren has no echo here, so its off command times out; it is still sent.
    rig.hub.set_alarm_settings(settings).await.unwrap();
    rig.pump().await;
    assert_eq!(rig.sent(SIREN_SET), ["OFF"]);
    assert_eq!(rig.sent(MQTT_SIREN), ["OFF"]);
    assert!(rig.trigger_alerts().iter().all(|a| a.resolved));
    let removal = rig.log.iter().find(|p| p.topic == CONFIG_TOPIC).unwrap();
    assert!(removal.retain && removal.payload.is_empty());
    assert_eq!(rig.state(), "disarmed");
}

#[tokio::test(start_paused = true)]
async fn climate_replies_still_routed() {
    let mut cfg = Config::default();
    cfg.topics.discovery_prefixes = vec!["oikos".to_owned()];
    cfg.climate.enabled = true;
    let (tx, mut rx) = mpsc::channel(64);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    house(&hub);
    hub.set_alarm_settings(enabled_with(&entity_id(&hub, "front_door")))
        .await
        .unwrap();
    drain(&mut rx);
    // Neither an alarm id nor a pending proposal: ignored, and the alarm is untouched.
    for reply in [
        br#"{"id":"climate-proposal-p-1","action":"approve"}"#.as_slice(),
        br#"{"id":"alarm-1-0","action":"disarm"}"#.as_slice(),
    ] {
        assert!(
            hub.ingest_message("oikos/push/responses", reply, false)
                .is_empty()
        );
    }
    tokio::task::yield_now().await;
    assert_eq!(hub.alarm_view().state, "disarmed");
}

#[tokio::test(start_paused = true)]
async fn unresponsive_siren_delays_neither_alert_nor_disarm() {
    let mut rig = Rig::with(|hub, settings| {
        with_sirens(hub, settings);
        // Online, but the switch never echoes its command.
        hub.ingest("fx/siren/avail", b"online");
    })
    .await;
    rig.hub.ingest_message("fx/siren", b"OFF", false);
    rig.command(arm(nidoo_core::model::AlarmMode::Away)).await;
    rig.advance(61).await;
    rig.clear();
    rig.hub.ingest_message("fx/front_door", b"open", false);
    // Pump without echoing the siren command: only the alarm's own topics.
    let hub = Arc::clone(&rig.hub);
    let pump = |rig: &mut Rig| {
        for p in drain(&mut rig.rx) {
            hub.ingest_message(&p.topic, p.payload.as_bytes(), p.retain);
            rig.log.push(p);
        }
    };
    for _ in 0..20 {
        tokio::task::yield_now().await;
        pump(&mut rig);
    }
    tokio::time::advance(std::time::Duration::from_millis(2100)).await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
        pump(&mut rig);
    }
    assert_eq!(rig.state(), "triggered");
    let alerts = rig.trigger_alerts();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert!(
        alerts[0].body.contains("(no confirmation)"),
        "{}",
        alerts[0].body
    );
    assert_eq!(rig.sent(MQTT_SIREN), ["ON"]);

    // A disarm right away completes without waiting for the siren either.
    let hub2 = Arc::clone(&rig.hub);
    let disarm = tokio::spawn(async move {
        hub2.send_command(
            "oikos_alarm_house",
            nidoo_core::model::Command::Disarm { code: None },
        )
        .await
    });
    for _ in 0..20 {
        tokio::task::yield_now().await;
        pump(&mut rig);
    }
    tokio::time::advance(std::time::Duration::from_millis(2100)).await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
        pump(&mut rig);
    }
    assert!(disarm.await.unwrap().unwrap().accepted);
    assert_eq!(rig.state(), "disarmed");
}

/// A rig whose watched door is a *delayed* opening in away mode; `silent` is a second delayed
/// door that has never published a state.
async fn delayed_rig() -> (Rig, String) {
    let mut silent = String::new();
    let rig = Rig::with(|hub, settings| {
        silent = entity_id(hub, "back_door");
        settings.sensors[0].entry = Entry::Delayed;
        settings.sensors.push(SensorRule {
            entity: silent.clone(),
            kind: SensorKind::Opening,
            modes: BTreeSet::from([ArmMode::Away]),
            entry: Entry::Delayed,
        });
    })
    .await;
    (rig, silent)
}

#[tokio::test(start_paused = true)]
async fn delayed_door_with_known_state_goes_pending() {
    use nidoo_core::model::AlarmMode;
    let (mut rig, _) = delayed_rig().await;
    rig.command(arm(AlarmMode::Away)).await;
    rig.advance(61).await;
    assert_eq!(rig.state(), "armed_away");
    rig.hub.ingest_message("fx/front_door", b"open", false);
    rig.pump().await;
    assert_eq!(rig.state(), "pending");
}

#[tokio::test(start_paused = true)]
async fn first_ever_open_of_a_stateless_sensor_is_watched_for_closing() {
    use nidoo_core::model::AlarmMode;
    let (mut rig, silent) = delayed_rig().await;
    rig.command(arm(AlarmMode::Away)).await;
    rig.advance(61).await;
    assert_eq!(rig.state(), "armed_away");
    assert!(rig.hub.entity_view(&silent).is_some());
    // Live (non-retained) first publication of `open` on a sensor never seen before: it may
    // have been open for hours, so it is found open, not opened.
    rig.hub.ingest_message("fx/back_door", b"open", false);
    rig.pump().await;
    assert_eq!(rig.state(), "armed_away");
    assert_eq!(
        rig.hub.alarm_view().open_at_arm,
        std::slice::from_ref(&silent)
    );
    assert!(rig.alarm_alerts().iter().all(|a| !is_alert(a, "critical")));
    // Closing it is the event, per its entry: delayed goes pending.
    rig.hub.ingest_message("fx/back_door", b"closed", false);
    rig.pump().await;
    assert_eq!(rig.state(), "pending");
}

/// Armed away from a retained runtime, a second (instant) window that has not reported yet.
#[tokio::test(start_paused = true)]
async fn restart_with_an_open_window_does_not_trigger() {
    use nidoo_alarm::machine::{Phase, Runtime};
    let mut window = String::new();
    let (hub, rx) = boot_hub(
        |hub, settings| {
            window = entity_id(hub, "back_door");
            settings.sensors.push(SensorRule {
                entity: window.clone(),
                kind: SensorKind::Opening,
                modes: BTreeSet::from([ArmMode::Away]),
                entry: Entry::Instant,
            });
        },
        |_| Runtime {
            phase: Phase::Armed {
                mode: ArmMode::Away,
            },
            ..Runtime::default()
        },
    );
    let task = hub.start_alarm();
    let mut rig = Rig {
        door: entity_id(&hub, "front_door"),
        hub,
        rx,
        log: Vec::new(),
        task,
    };
    rig.pump().await;
    rig.advance(6).await;
    assert_eq!(rig.state(), "armed_away");
    // Zigbee2MQTT publishes non-retained: the window's first report after the boot is `open`.
    rig.hub.ingest_message("fx/back_door", b"open", false);
    rig.pump().await;
    assert_eq!(
        rig.state(),
        "armed_away",
        "an open window is not a new opening"
    );
    assert_eq!(
        rig.hub.alarm_view().open_at_arm,
        std::slice::from_ref(&window)
    );
    rig.hub.ingest_message("fx/back_door", b"closed", false);
    rig.pump().await;
    assert_eq!(rig.state(), "triggered", "closing it is the event");
}

/// Tens of events the engine never reads: more than the broadcast channel holds.
fn flood(hub: &Hub) {
    for n in 0..1100 {
        hub.ingest_message("fx/temp", n.to_string().as_bytes(), false);
    }
}

#[tokio::test(start_paused = true)]
async fn lagged_events_are_treated_like_a_reconnect() {
    use nidoo_core::model::AlarmMode;
    let mut rig = Rig::new().await;
    rig.command(arm(AlarmMode::Away)).await;
    rig.advance(61).await;
    assert_eq!(rig.state(), "armed_away");
    // The engine does not run between these calls: the flood overruns its receiver, and the
    // door's opening comes last, so it is still delivered after the lag.
    flood(&rig.hub);
    rig.hub.ingest_message("fx/front_door", b"open", false);
    rig.pump().await;
    assert_eq!(
        rig.state(),
        "armed_away",
        "what was known before the lag is not compared with what follows it"
    );
    // Once settled, the engine is synced again and a change counts.
    rig.advance(6).await;
    rig.hub.ingest_message("fx/front_door", b"closed", false);
    rig.hub.ingest_message("fx/front_door", b"open", false);
    rig.pump().await;
    assert_eq!(rig.state(), "triggered");
}

#[tokio::test(start_paused = true)]
async fn late_retained_runtime_is_not_overwritten() {
    use nidoo_alarm::machine::{Phase, Runtime};
    let (hub, rx) = hub();
    house(&hub);
    let door = entity_id(&hub, "front_door");
    hub.ingest("fx/front_door", b"closed");
    hub.ingest_message(
        "oikos/alarm/settings",
        serde_json::to_string(&enabled_with(&door))
            .unwrap()
            .as_bytes(),
        true,
    );
    hub.on_connected();
    hub.on_suback();
    let task = hub.start_alarm();
    let mut rig = Rig {
        door,
        hub,
        rx,
        log: Vec::new(),
        task,
    };
    rig.pump().await;
    // The settle is over and the runtime has not arrived.
    rig.advance(10).await;
    let armed = Runtime {
        phase: Phase::Armed {
            mode: ArmMode::Away,
        },
        ..Runtime::default()
    };
    rig.hub.ingest_message(
        "oikos/alarm/state",
        serde_json::to_string(&armed).unwrap().as_bytes(),
        true,
    );
    rig.advance(5).await;
    assert_eq!(rig.state(), "armed_away");
    rig.advance(40).await;
    assert_eq!(rig.state(), "armed_away");
}

#[tokio::test(start_paused = true)]
async fn missing_runtime_is_taken_as_empty_after_the_wait() {
    let (hub, rx) = hub();
    house(&hub);
    let door = entity_id(&hub, "front_door");
    hub.ingest_message(
        "oikos/alarm/settings",
        serde_json::to_string(&enabled_with(&door))
            .unwrap()
            .as_bytes(),
        true,
    );
    hub.on_connected();
    hub.on_suback();
    let task = hub.start_alarm();
    let mut rig = Rig {
        door,
        hub,
        rx,
        log: Vec::new(),
        task,
    };
    rig.pump().await;
    rig.advance(6).await;
    assert!(rig.sent("oikos/alarm/state").is_empty(), "still waiting");
    rig.advance(31).await;
    assert_eq!(rig.state(), "disarmed");
    assert!(
        !rig.sent(CONFIG_TOPIC).is_empty(),
        "the entity is published"
    );
}

/// A presence tracker that is offline reads as unknown, whatever it last reported.
#[tokio::test(start_paused = true)]
async fn offline_tracker_blocks_auto_arm() {
    let mut rig = Rig::with(|hub, settings| {
        settings.presence.trackers = vec![entity_id(hub, "phone_a")];
        settings.presence.arm = true;
        settings.presence.arm_after_s = 60;
        // Someone is home when the alarm is switched on, so presence may arm once they leave.
        hub.ingest("fx/phone_a", b"home");
    })
    .await;
    // The phone's last word is `not_home`, then it goes offline with no one else around.
    rig.hub.ingest_message("fx/phone_a", b"not_home", false);
    rig.pump().await;
    rig.hub
        .ingest_message("fx/phone_a/avail", b"offline", false);
    rig.pump().await;
    rig.advance(600).await;
    assert_eq!(rig.state(), "disarmed", "an offline phone is not away");
}

#[tokio::test(start_paused = true)]
async fn tracker_stays_away_when_online_so_auto_arm_works() {
    let mut rig = Rig::with(|hub, settings| {
        settings.presence.trackers = vec![entity_id(hub, "phone_a")];
        settings.presence.arm = true;
        settings.presence.arm_after_s = 60;
        hub.ingest("fx/phone_a", b"home");
    })
    .await;
    rig.hub.ingest_message("fx/phone_a", b"not_home", false);
    rig.pump().await;
    rig.advance(120).await;
    assert_eq!(rig.state(), "armed_away");
}

#[tokio::test(start_paused = true)]
async fn saving_settings_while_ringing_silences_a_dropped_siren() {
    let mut rig = armed_rig().await;
    trip(&mut rig).await;
    rig.clear();
    let mut settings = rig.hub.alarm_settings();
    settings
        .sirens
        .retain(|s| matches!(s, nidoo_alarm::doc::Siren::Entity { .. }));
    rig.hub.set_alarm_settings(settings).await.unwrap();
    rig.pump().await;
    assert_eq!(
        rig.sent(MQTT_SIREN),
        ["OFF"],
        "the dropped siren is switched off"
    );
    assert!(rig.sent(SIREN_SET).is_empty(), "the kept one keeps ringing");
    assert_eq!(rig.state(), "triggered");
}

#[tokio::test(start_paused = true)]
async fn disabling_while_ringing_switches_off_the_previous_sirens_too() {
    let mut rig = armed_rig().await;
    trip(&mut rig).await;
    rig.clear();
    let mut settings = rig.hub.alarm_settings();
    settings.enabled = false;
    settings
        .sirens
        .retain(|s| matches!(s, nidoo_alarm::doc::Siren::Entity { .. }));
    rig.hub.set_alarm_settings(settings).await.unwrap();
    rig.pump().await;
    assert_eq!(rig.sent(MQTT_SIREN), ["OFF"]);
    assert_eq!(rig.sent(SIREN_SET), ["OFF"]);
}

#[tokio::test(start_paused = true)]
async fn test_chirp_is_refused_while_ringing() {
    let mut rig = armed_rig().await;
    trip(&mut rig).await;
    rig.clear();
    let err = rig.hub.alarm_test_chirp(1).unwrap_err();
    match err {
        HubError::Rejected(reason) => assert!(reason.contains("ringing"), "{reason}"),
        other => panic!("{other:?}"),
    }
    rig.pump().await;
    assert!(rig.sent(MQTT_SIREN).is_empty(), "nothing was published");
}

#[tokio::test(start_paused = true)]
async fn lag_read_while_offline_still_resyncs_on_reconnect() {
    use nidoo_core::model::AlarmMode;
    let mut rig = Rig::new().await;
    rig.command(arm(AlarmMode::Away)).await;
    rig.advance(61).await;
    assert_eq!(rig.state(), "armed_away");
    // The link drops and the flood pushes the drop out of the engine's receiver: the engine
    // reads the lag while the link is still down.
    rig.hub.set_connection(ConnectionStatus::Offline {
        reason: "test".to_owned(),
    });
    flood(&rig.hub);
    rig.pump().await;
    rig.hub.set_connection(ConnectionStatus::Online);
    rig.pump().await;
    rig.advance(6).await;
    rig.hub.ingest_message("fx/front_door", b"open", false);
    rig.pump().await;
    assert_eq!(rig.state(), "triggered", "the engine is synced again");
}

#[tokio::test(start_paused = true)]
async fn tracker_change_lost_to_a_lag_is_reconciled() {
    use nidoo_core::model::AlarmMode;
    let mut rig = Rig::with(|hub, settings| {
        settings.presence.trackers = vec![entity_id(hub, "phone_a")];
        hub.ingest("fx/phone_a", b"not_home");
    })
    .await;
    rig.command(arm(AlarmMode::Away)).await;
    rig.advance(61).await;
    assert_eq!(rig.state(), "armed_away");
    // The homecoming is the oldest event, so the flood drops it before the engine reads it.
    rig.hub.ingest_message("fx/phone_a", b"home", false);
    flood(&rig.hub);
    rig.pump().await;
    rig.advance(6).await;
    assert_eq!(rig.state(), "disarmed");
}

#[tokio::test(start_paused = true)]
async fn commands_are_refused_until_the_engine_has_started() {
    use nidoo_core::model::AlarmMode;
    let (hub, _rx) = hub();
    house(&hub);
    hub.set_alarm_settings(enabled_with(&entity_id(&hub, "front_door")))
        .await
        .unwrap();
    let reply = hub
        .send_command("oikos_alarm_house", arm(AlarmMode::Away))
        .await
        .unwrap();
    assert!(!reply.accepted);
    assert_eq!(reply.reason.as_deref(), Some("alarm starting"));
    assert_eq!(hub.alarm_view().state, "disarmed");
}
