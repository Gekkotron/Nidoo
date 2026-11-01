//! Network watches through the hub: set, delete, list and the retained definitions.

use std::sync::Arc;

use oikos_api::{ConnectionStatus, NetworkWatchSpec, WatchState};
use oikos_core::command::Publish;
use oikos_os::config::Config;
use oikos_os::hub::{Hub, HubError};
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
