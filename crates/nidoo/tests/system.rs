//! The system task through the hub: sensors and their availability, the restart push, the
//! service outage pushes and the in-memory view. The publisher channel is inspected.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use nidoo::config::{Config, HistoryConfig};
use nidoo::history::History;
use nidoo::hub::Hub;
use nidoo::setup::sections::Sections;
use nidoo::setup::{HubFile, Mode, RestartHook, SetupCtx};
use nidoo::system::heartbeat;
use nidoo::system::sample::Resources;
use nidoo_api::system::ids;
use nidoo_api::{ConnectionStatus, CpuView, MemView, ServiceState, SystemService};
use nidoo_core::command::Publish;
use tokio::sync::mpsc;

const T0: u64 = 1_800_000_000;

fn hub_with(notify: bool) -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let mut cfg = Config::default();
    cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
    cfg.system.notify = notify;
    let (tx, rx) = mpsc::channel(256);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    (hub, rx)
}

/// A hub whose push service is loaded, so a raised alert reaches `nidoo/push/history`.
async fn pushing_hub(notify: bool) -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let (hub, mut rx) = hub_with(notify);
    hub.on_connected();
    hub.on_suback();
    tokio::time::sleep(Duration::from_secs(2)).await;
    tokio::task::yield_now().await;
    drain(&mut rx);
    (hub, rx)
}

fn set_ctx(hub: &Hub, dir: &Path) {
    hub.set_setup(Arc::new(SetupCtx::new(
        dir.to_path_buf(),
        Mode::Run,
        HubFile::default(),
        None,
        Sections::default(),
        RestartHook::Record(Arc::default()),
    )));
}

fn drain(rx: &mut mpsc::Receiver<Publish>) -> Vec<Publish> {
    let mut out = Vec::new();
    while let Ok(p) = rx.try_recv() {
        out.push(p);
    }
    out
}

fn find<'a>(published: &'a [Publish], topic: &str) -> Option<&'a Publish> {
    published.iter().find(|p| p.topic == topic)
}

fn state(id: &str) -> String {
    format!("nidoo/system/{id}")
}

fn avail(id: &str) -> String {
    format!("nidoo/system/{id}/avail")
}

fn sensor_config(id: &str) -> String {
    format!("nidoo/sensor/nidoo_hub/{id}/config")
}

fn configs(published: &[Publish]) -> Vec<String> {
    let mut topics: Vec<String> = published
        .iter()
        .filter(|p| p.topic.ends_with("/config"))
        .map(|p| p.topic.clone())
        .collect();
    topics.sort();
    topics
}

/// The alerts in the latest history publish, newest first.
fn history(published: &[Publish]) -> Vec<serde_json::Value> {
    let Some(last) = published
        .iter()
        .rev()
        .find(|p| p.topic == "nidoo/push/history")
    else {
        return Vec::new();
    };
    let list: serde_json::Value = serde_json::from_str(&last.payload).expect("json");
    list.as_array().expect("list").clone()
}

fn resources(temperature_c: Option<f64>) -> Resources {
    Resources {
        cpu: Some(CpuView {
            server_pct: 12.5,
            hub_pct: 1.25,
        }),
        ram: Some(MemView {
            used_bytes: 2_000_000_000,
            total_bytes: 8_000_000_000,
            hub_bytes: 50_000_000,
        }),
        disk: Some((40_000_000_000, 100_000_000_000)),
        temperature_c,
    }
}

#[tokio::test]
async fn tick_publishes_states_and_avail() {
    let (hub, mut rx) = hub_with(false);
    hub.tick_with(T0, resources(Some(45.0))).await;
    let published = drain(&mut rx);

    let cpu = find(&published, &state(ids::CPU)).expect("cpu state");
    assert_eq!(cpu.payload, "12.5");
    assert!(cpu.retain);
    assert_eq!(
        find(&published, &avail(ids::CPU))
            .expect("cpu avail")
            .payload,
        "online"
    );
    assert!(find(&published, &sensor_config(ids::CPU)).unwrap().retain);
    assert_eq!(find(&published, &state(ids::DEVICES)).unwrap().payload, "0");
    assert_eq!(
        find(&published, &state(ids::RAM_USED)).unwrap().payload,
        "2000"
    );
    assert_eq!(
        find(&published, &state(ids::DISK_USED)).unwrap().payload,
        "40.00"
    );
    assert_eq!(
        find(&published, &state(ids::STARTED))
            .unwrap()
            .payload
            .len(),
        "2026-01-01T00:00:00Z".len()
    );
    assert_eq!(find(&published, &state(ids::BROKER)).unwrap().payload, "ON");
    assert!(
        find(
            &published,
            "nidoo/binary_sensor/nidoo_hub/system_broker/config"
        )
        .is_some()
    );
    // Never measured (no history configured, no bridge state yet, no `$SYS` count): no
    // config and no state, only `offline` over whatever an earlier run left retained.
    for id in [ids::DATABASE, ids::ZIGBEE2MQTT, ids::RETAINED, ids::DB_SIZE] {
        assert!(
            !published
                .iter()
                .any(|p| p.topic.ends_with(&format!("{id}/config")) || p.topic == state(id)),
            "{id} published"
        );
        assert_eq!(find(&published, &avail(id)).unwrap().payload, "offline");
    }

    // The sensors' own configs joined the registry: only the entity count moves (devices
    // come from the Zigbee2MQTT device list).
    hub.tick_with(T0 + 60, resources(Some(45.0))).await;
    let mut moved: Vec<String> = drain(&mut rx).into_iter().map(|p| p.topic).collect();
    moved.sort();
    assert_eq!(moved, vec![state(ids::ENTITIES)]);
    // Nothing changed: nothing is published again.
    hub.tick_with(T0 + 120, resources(Some(45.0))).await;
    assert!(drain(&mut rx).is_empty());
}

#[tokio::test]
async fn tick_at_samples_the_machine() {
    let (hub, mut rx) = hub_with(false);
    hub.tick_at(T0).await;
    let published = drain(&mut rx);
    assert!(find(&published, &state(ids::DEVICES)).is_some());
    assert!(find(&published, &state(ids::RAM_USED)).is_some());
}

#[tokio::test]
async fn reconnect_republishes_configs() {
    let (hub, mut rx) = hub_with(false);
    hub.tick_with(T0, resources(Some(45.0))).await;
    let first = drain(&mut rx);
    let first_configs = configs(&first);
    assert!(first_configs.len() >= 10, "{first_configs:?}");

    // The broker drops: the tick runs, every publish is refused.
    hub.set_connection(ConnectionStatus::Connecting);
    hub.tick_with(T0 + 60, resources(Some(46.0))).await;
    assert!(drain(&mut rx).is_empty());

    hub.set_connection(ConnectionStatus::Online);
    hub.tick_with(T0 + 120, resources(Some(46.0))).await;
    let again = drain(&mut rx);
    assert_eq!(configs(&again), first_configs);
    assert_eq!(
        find(&again, &state(ids::TEMPERATURE)).unwrap().payload,
        "46.0"
    );
    assert_eq!(find(&again, &state(ids::CPU)).unwrap().payload, "12.5");
    assert_eq!(find(&again, &avail(ids::CPU)).unwrap().payload, "online");
}

#[tokio::test]
async fn vanished_temperature_goes_offline() {
    let (hub, mut rx) = hub_with(false);
    hub.tick_with(T0, resources(Some(45.0))).await;
    drain(&mut rx);
    hub.tick_with(T0 + 60, resources(None)).await;
    let published = drain(&mut rx);
    assert_eq!(
        find(&published, &avail(ids::TEMPERATURE)).unwrap().payload,
        "offline"
    );
    assert!(find(&published, &state(ids::TEMPERATURE)).is_none());
    assert_eq!(hub.system_view().temperature_c, None);

    hub.tick_with(T0 + 120, resources(Some(44.0))).await;
    let published = drain(&mut rx);
    assert_eq!(
        find(&published, &state(ids::TEMPERATURE)).unwrap().payload,
        "44.0"
    );
    assert_eq!(
        find(&published, &avail(ids::TEMPERATURE)).unwrap().payload,
        "online"
    );
}

#[tokio::test]
async fn never_measured_temperature_is_not_published() {
    let (hub, mut rx) = hub_with(false);
    hub.tick_with(T0, resources(None)).await;
    let published = drain(&mut rx);
    let temperature: Vec<&Publish> = published
        .iter()
        .filter(|p| p.topic.contains(ids::TEMPERATURE))
        .collect();
    assert_eq!(temperature.len(), 1, "{temperature:?}");
    assert_eq!(temperature[0].topic, avail(ids::TEMPERATURE));
    assert_eq!(temperature[0].payload, "offline");
    hub.tick_with(T0 + 60, resources(None)).await;
    assert!(
        !drain(&mut rx)
            .iter()
            .any(|p| p.topic.contains(ids::TEMPERATURE))
    );
}

#[tokio::test(start_paused = true)]
async fn unexpected_boot_pushes_when_notify_on() {
    let (hub, mut rx) = pushing_hub(true).await;
    let dir = tempfile::tempdir().unwrap();
    heartbeat::write_heartbeat(dir.path(), nidoo::clock::now() - 600).unwrap();
    set_ctx(&hub, dir.path());

    let task = hub.start_system().expect("a setup context");
    task.abort();
    let alerts = history(&drain(&mut rx));
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    let alert = &alerts[0];
    assert!(
        alert["id"].as_str().unwrap().starts_with("hub-restart-"),
        "{alert}"
    );
    assert_eq!(alert["title"], "Hub restarted");
    let body = alert["body"].as_str().unwrap();
    assert!(
        body == "The hub restarted after an unexpected stop (down 10 min)."
            || body == "The hub restarted after an unexpected stop (down 11 min).",
        "{body}"
    );
    let view = hub.system_view();
    assert_eq!(view.restarts.len(), 1);
    assert_eq!(view.restarts[0].stop, nidoo_api::StopKind::Unexpected);

    // Later ticks never push the restart again.
    hub.tick_with(T0, resources(None)).await;
    hub.tick_with(T0 + 60, resources(None)).await;
    assert!(history(&drain(&mut rx)).is_empty());
}

#[tokio::test(start_paused = true)]
async fn unexpected_boot_with_notify_off_is_silent() {
    let (hub, mut rx) = pushing_hub(false).await;
    let dir = tempfile::tempdir().unwrap();
    heartbeat::write_heartbeat(dir.path(), nidoo::clock::now() - 600).unwrap();
    set_ctx(&hub, dir.path());
    hub.start_system().expect("a setup context").abort();
    assert!(history(&drain(&mut rx)).is_empty());
}

#[tokio::test(start_paused = true)]
async fn corrupt_heartbeat_pushes_an_unknown_downtime() {
    let (hub, mut rx) = pushing_hub(true).await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(heartbeat::FILE), b"{\"heart").unwrap();
    set_ctx(&hub, dir.path());
    hub.start_system().expect("a setup context").abort();
    let alerts = history(&drain(&mut rx));
    assert_eq!(
        alerts[0]["body"],
        "The hub restarted after an unexpected stop (down an unknown time)."
    );
}

#[tokio::test(start_paused = true)]
async fn clean_boot_never_pushes() {
    let (hub, mut rx) = pushing_hub(true).await;
    let dir = tempfile::tempdir().unwrap();
    let now = nidoo::clock::now();
    heartbeat::write_heartbeat(dir.path(), now - 600).unwrap();
    heartbeat::write_clean_stop(dir.path(), now - 590).unwrap();
    set_ctx(&hub, dir.path());
    hub.start_system().expect("a setup context").abort();
    assert!(history(&drain(&mut rx)).is_empty());
    assert_eq!(
        hub.system_view().restarts[0].stop,
        nidoo_api::StopKind::Clean
    );
}

#[tokio::test(start_paused = true)]
async fn first_boot_never_pushes() {
    let (hub, mut rx) = pushing_hub(true).await;
    let dir = tempfile::tempdir().unwrap();
    set_ctx(&hub, dir.path());
    hub.start_system().expect("a setup context").abort();
    assert!(history(&drain(&mut rx)).is_empty());
    assert_eq!(
        hub.system_view().restarts[0].stop,
        nidoo_api::StopKind::First
    );
}

#[test]
fn start_without_a_setup_context_is_none() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let (hub, _rx) = hub_with(true);
        assert!(hub.start_system().is_none());
    });
}

#[tokio::test(start_paused = true)]
async fn zigbee2mqtt_down_five_minutes_pushes_once() {
    let (hub, mut rx) = pushing_hub(true).await;
    hub.ingest("zigbee2mqtt/bridge/state", br#"{"state":"offline"}"#);
    hub.tick_with(T0, resources(None)).await;
    hub.tick_with(T0 + 4 * 60, resources(None)).await;
    assert!(history(&drain(&mut rx)).is_empty());
    hub.tick_with(T0 + 5 * 60, resources(None)).await;
    let alerts = history(&drain(&mut rx));
    assert_eq!(alerts[0]["id"], format!("service-zigbee2mqtt-down-{T0}"));
    hub.tick_with(T0 + 6 * 60, resources(None)).await;
    assert!(history(&drain(&mut rx)).is_empty());

    hub.ingest("zigbee2mqtt/bridge/state", br#"{"state":"online"}"#);
    hub.tick_with(T0 + 7 * 60, resources(None)).await;
    let alerts = history(&drain(&mut rx));
    assert_eq!(alerts[0]["id"], format!("service-zigbee2mqtt-back-{T0}"));
}

#[tokio::test]
async fn system_view_reports_unknown_retained_before_sys() {
    let (hub, _rx) = hub_with(false);
    let view = hub.system_view();
    assert_eq!(view.retained, None);
    assert_eq!(view.cpu, None);
    for service in &view.services {
        assert_eq!(service.state, ServiceState::Unknown);
        assert_eq!(service.since, None);
    }
    assert_eq!(
        view.services.iter().map(|s| s.service).collect::<Vec<_>>(),
        vec![
            SystemService::Broker,
            SystemService::Zigbee2mqtt,
            SystemService::Database
        ]
    );
    assert_eq!(view.uptime_pct, 100.0);

    hub.ingest_message("$SYS/broker/retained messages/count", b"42", false);
    hub.tick_with(T0, resources(None)).await;
    let view = hub.system_view();
    assert_eq!(view.retained, Some(42));
    assert_eq!(view.cpu.unwrap().server_pct, 12.5);
    let broker = &view.services[0];
    assert_eq!((broker.state, broker.since), (ServiceState::Up, Some(T0)));
    // The database is not configured: unknown, and never "since".
    let database = &view.services[2];
    assert_eq!(
        (database.state, database.since),
        (ServiceState::Unknown, None)
    );
}

#[tokio::test]
async fn broker_reads_unknown_before_the_first_connack() {
    let (tx, _rx) = mpsc::channel(256);
    let hub = Hub::new(&Config::default(), tx);
    hub.tick_with(T0, resources(None)).await;
    let broker = &hub.system_view().services[0];
    assert_eq!((broker.state, broker.since), (ServiceState::Unknown, None));

    hub.set_connection(ConnectionStatus::Offline {
        reason: "connection refused".into(),
    });
    hub.tick_with(T0 + 60, resources(None)).await;
    let broker = &hub.system_view().services[0];
    assert_eq!(
        (broker.state, broker.since),
        (ServiceState::Down, Some(T0 + 60))
    );
}

fn with_history(hub: &Hub, url: &str) {
    let cfg = HistoryConfig {
        url: url.to_owned(),
        ..HistoryConfig::default()
    };
    hub.set_history(History::start(&cfg, hub.zone()).expect("a lazy pool"));
}

#[tokio::test]
async fn unreachable_database_reads_down() {
    let (hub, mut rx) = hub_with(false);
    with_history(&hub, "postgres://u:p@127.0.0.1:1/x");
    let dir = tempfile::tempdir().unwrap();
    set_ctx(&hub, dir.path());
    hub.start_system().expect("a setup context").abort();

    hub.tick_with(T0, resources(None)).await;
    let published = drain(&mut rx);
    assert_eq!(
        find(&published, &state(ids::DATABASE)).unwrap().payload,
        "OFF"
    );
    assert!(
        find(
            &published,
            "nidoo/binary_sensor/nidoo_hub/system_database/config"
        )
        .is_some()
    );
    assert_eq!(
        find(&published, &avail(ids::DB_SIZE)).unwrap().payload,
        "offline"
    );
    let view = hub.system_view();
    let database = &view.services[2];
    assert_eq!(
        (database.state, database.since),
        (ServiceState::Down, Some(T0))
    );
    // The start waits in memory for the database.
    assert_eq!(view.restarts.len(), 1);
    assert_eq!(view.restarts[0].stop, nidoo_api::StopKind::First);
}

#[tokio::test]
async fn database_tick_logs_the_start_and_reports_its_size() {
    let Some(tdb) = common::db::fresh_db().await else {
        return;
    };
    let (hub, mut rx) = hub_with(false);
    with_history(&hub, &tdb.url);
    let dir = tempfile::tempdir().unwrap();
    set_ctx(&hub, dir.path());
    hub.start_system().expect("a setup context").abort();

    let now = nidoo::clock::now();
    hub.tick_with(now, resources(None)).await;
    let published = drain(&mut rx);
    assert_eq!(
        find(&published, &state(ids::DATABASE)).unwrap().payload,
        "ON"
    );
    assert!(find(&published, &state(ids::DB_SIZE)).is_some());
    let view = hub.system_view();
    assert_eq!(view.services[2].state, ServiceState::Up);
    assert!(view.disk.unwrap().db_bytes.unwrap() > 0);
    assert_eq!(view.restarts.len(), 1);
    let logged = tdb.db.restarts_since(0).await.unwrap();
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].stop, nidoo_api::StopKind::First);
    tdb.drop_db().await;
}
