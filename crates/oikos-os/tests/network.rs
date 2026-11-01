//! Network watches through the hub: set, delete, list and the retained definitions.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use oikos_api::{ConnectionStatus, NetworkWatchSpec, WatchState};
use oikos_core::command::Publish;
use oikos_os::config::Config;
use oikos_os::hub::{Hub, HubError};
use oikos_os::network::{Probe, Prober};
use tokio::sync::mpsc;

fn network_hub() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    hub_with(Config::default())
}

fn hub_with(mut cfg: Config) -> (Arc<Hub>, mpsc::Receiver<Publish>) {
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

fn spec(host: &str) -> NetworkWatchSpec {
    NetworkWatchSpec {
        name: "NAS".into(),
        host: host.into(),
        interval_s: 30,
        enabled: true,
        area: None,
        device: None,
    }
}

fn find<'a>(published: &'a [Publish], topic: &str) -> Option<&'a Publish> {
    published.iter().find(|p| p.topic == topic)
}

const VACUUM: &[u8] = br#"{"name":"Cleaner","unique_id":"vac_1_switch","command_topic":"vac/1/set","state_topic":"vac/1","device":{"identifiers":["vac_1"],"name":"Vacuum"}}"#;

fn feed_vacuum(hub: &Hub) {
    hub.ingest("oikos/switch/vac_1/switch/config", VACUUM);
}

#[tokio::test]
async fn set_publishes_definition_config_and_availability() {
    let (hub, mut rx) = network_hub();
    let view = hub
        .set_network_watch("nas", spec("nas.local"))
        .await
        .unwrap();
    let published = drain(&mut rx);
    assert!(
        find(&published, "oikos/network/nas/definition")
            .unwrap()
            .retain
    );
    assert!(
        find(&published, "oikos/binary_sensor/network/nas/config")
            .unwrap()
            .retain
    );
    assert_eq!(
        find(&published, "oikos/network/nas/avail").unwrap().payload,
        "online"
    );
    assert!(find(&published, "oikos/network/nas").is_none());
    assert_eq!(view.status.state, WatchState::Waiting);
    let listed = hub.list_network_watches().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].status.state, WatchState::Waiting);
    let entity = hub.entity_view("network_nas").expect("entity");
    assert_eq!(entity.device_class.as_deref(), Some("connectivity"));
}

#[tokio::test]
async fn paused_watch_is_published_offline() {
    let (hub, mut rx) = network_hub();
    let mut paused = spec("nas.local");
    paused.enabled = false;
    let view = hub.set_network_watch("nas", paused).await.unwrap();
    let published = drain(&mut rx);
    assert_eq!(
        find(&published, "oikos/network/nas/avail").unwrap().payload,
        "offline"
    );
    assert_eq!(view.status.state, WatchState::Paused);
    assert_eq!(
        hub.list_network_watches().unwrap()[0].status.state,
        WatchState::Paused
    );
}

#[tokio::test]
async fn device_watch_joins_the_device() {
    let (hub, mut rx) = network_hub();
    feed_vacuum(&hub);
    let mut s = spec("192.168.1.42");
    s.device = Some("vac_1".into());
    hub.set_network_watch("vac_1", s).await.unwrap();
    let published = drain(&mut rx);
    let config = find(&published, "oikos/binary_sensor/network/vac_1/config").unwrap();
    let config: serde_json::Value = serde_json::from_str(&config.payload).unwrap();
    assert_eq!(
        config["device"]["identifiers"],
        serde_json::json!(["vac_1"])
    );
}

#[tokio::test]
async fn second_watch_on_same_device_rejected() {
    let (hub, _rx) = network_hub();
    feed_vacuum(&hub);
    hub.ingest(
        "oikos/network/other/definition",
        br#"{"name":"Other","host":"192.168.1.43","device":"vac_1"}"#,
    );
    let mut s = spec("192.168.1.42");
    s.device = Some("vac_1".into());
    let err = hub.set_network_watch("vac_1", s).await.unwrap_err();
    assert!(matches!(err, HubError::Rejected(_)));
}

#[tokio::test]
async fn device_watch_id_must_match_device() {
    let (hub, _rx) = network_hub();
    feed_vacuum(&hub);
    let mut s = spec("192.168.1.42");
    s.device = Some("vac_1".into());
    let err = hub.set_network_watch("nas", s).await.unwrap_err();
    assert!(matches!(err, HubError::Rejected(_)));
}

#[tokio::test]
async fn unknown_device_rejected() {
    let (hub, _rx) = network_hub();
    let mut s = spec("192.168.1.42");
    s.device = Some("ghost".into());
    let err = hub.set_network_watch("ghost", s).await.unwrap_err();
    assert!(matches!(err, HubError::Rejected(_)));
}

#[tokio::test]
async fn delete_clears_only_network_topics() {
    let (hub, mut rx) = network_hub();
    feed_vacuum(&hub);
    let mut s = spec("192.168.1.42");
    s.device = Some("vac_1".into());
    hub.set_network_watch("vac_1", s).await.unwrap();
    drain(&mut rx);
    hub.delete_network_watch("vac_1").await.unwrap();
    let published = drain(&mut rx);
    let mut topics: Vec<&str> = published.iter().map(|p| p.topic.as_str()).collect();
    topics.sort_unstable();
    assert_eq!(
        topics,
        [
            "oikos/binary_sensor/network/vac_1/config",
            "oikos/network/vac_1",
            "oikos/network/vac_1/avail",
            "oikos/network/vac_1/definition",
        ]
    );
    assert!(published.iter().all(|p| p.payload.is_empty() && p.retain));
    assert!(hub.list_network_watches().unwrap().is_empty());
    assert!(hub.entity_view("network_vac_1").is_none());
    assert!(
        hub.snapshot()
            .entities
            .iter()
            .any(|e| e.device_id.as_deref() == Some("vac_1"))
    );
}

#[tokio::test]
async fn delete_unknown_watch_is_unknown_entity() {
    let (hub, _rx) = network_hub();
    let err = hub.delete_network_watch("nas").await.unwrap_err();
    assert!(matches!(err, HubError::UnknownEntity(_)));
}

#[tokio::test]
async fn retained_definition_is_loaded() {
    let (hub, _rx) = network_hub();
    hub.ingest(
        "oikos/network/nas/definition",
        br#"{"name":"NAS","host":"nas.local"}"#,
    );
    let listed = hub.list_network_watches().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "nas");
    assert_eq!(listed[0].spec.host, "nas.local");
}

#[tokio::test]
async fn disabled_feature_is_503() {
    let mut cfg = Config::default();
    cfg.network.enabled = false;
    let (hub, _rx) = hub_with(cfg);
    let err = hub
        .set_network_watch("nas", spec("nas.local"))
        .await
        .unwrap_err();
    assert!(matches!(err, HubError::Publish(_)));
    assert!(matches!(
        hub.list_network_watches().unwrap_err(),
        HubError::Publish(_)
    ));
}

#[tokio::test]
async fn standalone_area_is_written_as_override() {
    let (hub, _rx) = network_hub();
    let mut s = spec("nas.local");
    s.area = Some("office".into());
    hub.set_network_watch("nas", s).await.unwrap();
    let entity = hub.entity_view("network_nas").unwrap();
    assert_eq!(entity.ui.area.as_deref(), Some("office"));
}

/// Answers from a script, then `Miss`; counts every ping.
struct FakeProber {
    script: Mutex<VecDeque<Probe>>,
    calls: AtomicUsize,
}

impl FakeProber {
    fn new(script: Vec<Probe>) -> Arc<FakeProber> {
        Arc::new(FakeProber {
            script: Mutex::new(script.into()),
            calls: AtomicUsize::new(0),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Prober for FakeProber {
    fn probe<'a>(&'a self, _host: &'a str, _timeout: Duration) -> BoxFuture<'a, Probe> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let next = self
            .script
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Probe::Miss);
        Box::pin(async move { next })
    }
}

const REPLY: Probe = Probe::Reply(Duration::from_millis(3));

/// Lets the paused clock run for `secs` seconds, giving the loops time to act each second.
async fn run_for(secs: u64) {
    for _ in 0..secs {
        tokio::time::advance(Duration::from_secs(1)).await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    }
}

fn payloads(published: &[Publish], topic: &str) -> Vec<String> {
    published
        .iter()
        .filter(|p| p.topic == topic)
        .map(|p| p.payload.clone())
        .collect()
}

#[tokio::test(start_paused = true)]
async fn three_misses_publish_off_once() {
    let (hub, mut rx) = network_hub();
    let prober = FakeProber::new(vec![REPLY, Probe::Miss, Probe::Miss, Probe::Miss]);
    let _task = hub.start_network_with(prober.clone()).unwrap();
    hub.set_network_watch("nas", spec("nas.local"))
        .await
        .unwrap();
    run_for(200).await;
    let published = drain(&mut rx);
    assert_eq!(payloads(&published, "oikos/network/nas"), ["ON", "OFF"]);
    let listed = hub.list_network_watches().unwrap();
    assert_eq!(listed[0].status.state, WatchState::Disconnected);
}

#[tokio::test(start_paused = true)]
async fn restart_publishes_nothing_before_the_first_verdict() {
    let (hub, mut rx) = network_hub();
    let prober = FakeProber::new(vec![Probe::Miss, Probe::Miss]);
    let _task = hub.start_network_with(prober.clone()).unwrap();
    hub.set_network_watch("nas", spec("nas.local"))
        .await
        .unwrap();
    while prober.calls() < 2 {
        run_for(1).await;
    }
    run_for(1).await;
    assert_eq!(prober.calls(), 2);
    let published = drain(&mut rx);
    assert!(payloads(&published, "oikos/network/nas").is_empty());
}

#[tokio::test(start_paused = true)]
async fn cannot_ping_touches_only_availability() {
    let (hub, mut rx) = network_hub();
    let prober = FakeProber::new(vec![REPLY, Probe::CannotPing("no socket".into()), REPLY]);
    let _task = hub.start_network_with(prober.clone()).unwrap();
    hub.set_network_watch("nas", spec("nas.local"))
        .await
        .unwrap();
    drain(&mut rx);
    let mut states = Vec::new();
    let mut avails = Vec::new();
    let mut saw_cannot_ping = false;
    for _ in 0..100 {
        run_for(1).await;
        let published = drain(&mut rx);
        states.extend(payloads(&published, "oikos/network/nas"));
        avails.extend(payloads(&published, "oikos/network/nas/avail"));
        saw_cannot_ping |=
            hub.list_network_watches().unwrap()[0].status.state == WatchState::CannotPing;
    }
    assert_eq!(states, ["ON"]);
    assert_eq!(avails, ["online", "offline", "online"]);
    assert!(saw_cannot_ping);
}

#[tokio::test(start_paused = true)]
async fn pausing_stops_the_loop_and_marks_offline() {
    let (hub, mut rx) = network_hub();
    let prober = FakeProber::new(vec![REPLY; 50]);
    let _task = hub.start_network_with(prober.clone()).unwrap();
    hub.set_network_watch("nas", spec("nas.local"))
        .await
        .unwrap();
    run_for(125).await;
    assert!(prober.calls() >= 3);
    let mut paused = spec("nas.local");
    paused.enabled = false;
    hub.set_network_watch("nas", paused).await.unwrap();
    run_for(2).await;
    drain(&mut rx);
    let calls = prober.calls();
    run_for(150).await;
    assert_eq!(prober.calls(), calls);
    let published = drain(&mut rx);
    assert!(payloads(&published, "oikos/network/nas").is_empty());
    let listed = hub.list_network_watches().unwrap();
    assert_eq!(listed[0].status.state, WatchState::Paused);
}

#[tokio::test(start_paused = true)]
async fn deleting_stops_the_loop() {
    let (hub, _rx) = network_hub();
    let prober = FakeProber::new(vec![REPLY; 50]);
    let _task = hub.start_network_with(prober.clone()).unwrap();
    hub.set_network_watch("nas", spec("nas.local"))
        .await
        .unwrap();
    run_for(65).await;
    hub.delete_network_watch("nas").await.unwrap();
    run_for(2).await;
    let calls = prober.calls();
    run_for(150).await;
    assert_eq!(prober.calls(), calls);
}

#[tokio::test(start_paused = true)]
async fn interval_change_restarts_the_loop() {
    let (hub, _rx) = network_hub();
    let prober = FakeProber::new(vec![REPLY; 100]);
    let _task = hub.start_network_with(prober.clone()).unwrap();
    hub.set_network_watch("nas", spec("nas.local"))
        .await
        .unwrap();
    run_for(10).await;
    let mut faster = spec("nas.local");
    faster.interval_s = 10;
    hub.set_network_watch("nas", faster).await.unwrap();
    run_for(2).await;
    let before = prober.calls();
    run_for(100).await;
    assert!(prober.calls() - before >= 8, "{}", prober.calls() - before);
}

#[tokio::test(start_paused = true)]
async fn failed_publish_is_retried() {
    let (hub, mut rx) = network_hub();
    let prober = FakeProber::new(vec![REPLY, Probe::Miss, Probe::Miss]);
    let _task = hub.start_network_with(prober.clone()).unwrap();
    hub.set_network_watch("nas", spec("nas.local"))
        .await
        .unwrap();
    while prober.calls() < 2 {
        run_for(1).await;
    }
    hub.set_connection(ConnectionStatus::Offline {
        reason: "test".into(),
    });
    while prober.calls() < 4 {
        run_for(1).await;
    }
    run_for(1).await;
    let during = drain(&mut rx);
    assert!(!payloads(&during, "oikos/network/nas").contains(&"OFF".to_owned()));
    hub.set_connection(ConnectionStatus::Online);
    run_for(35).await;
    let after = drain(&mut rx);
    assert_eq!(payloads(&after, "oikos/network/nas"), ["OFF"]);
}
