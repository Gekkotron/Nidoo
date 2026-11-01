//! The home settings document: the retained topic, the hub's writer, REST and MCP.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use nidoo::config::Config;
use nidoo::http::router;
use nidoo::hub::Hub;
use nidoo_api::{ConnectionStatus, HomeSettings, VirtualKind, VirtualSpec, WsEvent};
use nidoo_core::command::Publish;
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::StreamableHttpClientTransport;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

const HOME_TOPIC: &str = "oikos/ui/_home";
const TEMPERATURE_TOPIC: &str = "oikos/sensor/0x00124b0008fd6568/temperature_1/config";
const TEMPERATURE_FIXTURE: &str = "discovery/sensor__0x00124b0008fd6568__temperature_1.json";

fn fixture(name: &str) -> Vec<u8> {
    let dir = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../nidoo-core/tests/fixtures/"
    ));
    std::fs::read(dir.join(name)).expect("fixture")
}

fn hub() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let mut cfg = Config::default();
    cfg.topics.discovery_prefixes = vec!["oikos".to_owned()];
    let (tx, rx) = mpsc::channel(64);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    (hub, rx)
}

fn eco(value: f64) -> HomeSettings {
    HomeSettings {
        eco_setpoint: Some(value),
        ..HomeSettings::default()
    }
}

fn select_spec() -> VirtualSpec {
    VirtualSpec {
        kind: VirtualKind::Select,
        name: "House mode".to_owned(),
        min: None,
        max: None,
        step: None,
        unit: None,
        options: Some(vec!["home".to_owned(), "away".to_owned()]),
        initial: None,
    }
}

async fn serve(hub: &Arc<Hub>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(
        axum::serve(
            listener,
            router(Arc::clone(hub), nidoo::setup::SetupCtx::inert()),
        )
        .into_future(),
    );
    addr
}

#[tokio::test]
async fn defaults_when_nothing_retained() {
    let (hub, _rx) = hub();
    assert_eq!(hub.home_settings(), HomeSettings::default());
}

#[tokio::test]
async fn set_publishes_retained_applies_and_broadcasts() {
    let (hub, mut rx) = hub();
    let mut events = hub.subscribe();
    let stored = hub.set_home_settings(eco(15.0)).expect("stored");
    assert_eq!(stored, eco(15.0));
    assert_eq!(hub.home_settings(), eco(15.0));

    let publish = rx.try_recv().expect("one publish");
    assert!(rx.try_recv().is_err(), "exactly one publish");
    assert_eq!(publish.topic, HOME_TOPIC);
    assert!(publish.retain);
    let doc: HomeSettings = serde_json::from_str(&publish.payload).expect("json document");
    assert_eq!(doc, eco(15.0));

    match events.try_recv().expect("an event") {
        WsEvent::HomeSettingsChanged { settings } => assert_eq!(settings, eco(15.0)),
        other => panic!("unexpected event {other:?}"),
    }
}

#[tokio::test]
async fn offline_broker_refuses_the_write() {
    let (hub, _rx) = hub();
    hub.set_connection(ConnectionStatus::Connecting);
    assert!(hub.set_home_settings(eco(15.0)).is_err());
    assert_eq!(hub.home_settings(), HomeSettings::default());
}

#[tokio::test]
async fn retained_document_is_applied_on_ingest() {
    let (hub, _rx) = hub();
    let ignored = hub.ignored();
    hub.ingest(HOME_TOPIC, br#"{"eco_setpoint":16.5}"#);
    assert_eq!(hub.home_settings().eco_setpoint, Some(16.5));
    assert!(hub.entity_view("_home").is_none());
    assert_eq!(hub.ignored(), ignored);

    hub.ingest(HOME_TOPIC, b"");
    assert_eq!(hub.home_settings(), HomeSettings::default());
}

#[tokio::test]
async fn invalid_retained_document_keeps_previous() {
    let (hub, _rx) = hub();
    hub.ingest(HOME_TOPIC, br#"{"eco_setpoint":16.5}"#);
    hub.ingest(HOME_TOPIC, b"not json");
    assert_eq!(hub.home_settings().eco_setpoint, Some(16.5));
}

#[tokio::test]
async fn rest_get_put_roundtrip_and_422() {
    let (hub, _rx) = hub();
    let addr = serve(&hub).await;
    let client = reqwest::Client::new();
    let url = format!("http://{addr}/api/v1/home/settings");

    let put = client
        .put(&url)
        .json(&json!({"eco_setpoint": 15}))
        .send()
        .await
        .expect("put");
    assert_eq!(put.status(), 200);
    let body: HomeSettings = put.json().await.expect("body");
    assert_eq!(body, eco(15.0));
    let got: HomeSettings = client
        .get(&url)
        .send()
        .await
        .expect("get")
        .json()
        .await
        .expect("body");
    assert_eq!(got, eco(15.0));

    let bad = client
        .put(&url)
        .json(&json!({"eco_setpoint": 50}))
        .send()
        .await
        .expect("put");
    assert_eq!(bad.status(), 422);
    let err: Value = bad.json().await.expect("body");
    assert!(
        err["error"]
            .as_str()
            .unwrap_or_default()
            .contains("eco_setpoint"),
        "{err}"
    );

    hub.set_virtual("house_mode", select_spec())
        .await
        .expect("select");
    let ok = client
        .put(&url)
        .json(&json!({"mode_entity": "virtual_house_mode"}))
        .send()
        .await
        .expect("put");
    assert_eq!(ok.status(), 200);

    hub.set_connection(ConnectionStatus::Connecting);
    let offline = client
        .put(&url)
        .json(&json!({"eco_setpoint": 16}))
        .send()
        .await
        .expect("put");
    assert_eq!(offline.status(), 503);
}

#[tokio::test]
async fn mcp_set_and_get_home_settings() {
    let (hub, _rx) = hub();
    hub.ingest(TEMPERATURE_TOPIC, &fixture(TEMPERATURE_FIXTURE));
    let indoor = hub
        .snapshot()
        .entities
        .iter()
        .find(|e| e.device_class.as_deref() == Some("temperature"))
        .expect("temperature sensor")
        .id
        .clone();
    let addr = serve(&hub).await;
    let transport = StreamableHttpClientTransport::from_uri(format!("http://{addr}/mcp"));
    let client = ().serve(transport).await.expect("mcp initialize");

    let call = |name: &'static str, args: Value| {
        let client = &client;
        async move {
            let Value::Object(map) = args else {
                panic!("arguments must be an object")
            };
            client
                .call_tool(CallToolRequestParams::new(name).with_arguments(map))
                .await
                .expect("call")
        }
    };

    let set = call(
        "set_home_settings",
        json!({"settings": {"eco_setpoint": 16.5, "indoor_entity": indoor}}),
    )
    .await;
    assert_ne!(set.is_error, Some(true), "{set:?}");
    let got = call("get_home_settings", json!({})).await;
    let doc = got.structured_content.expect("structured");
    assert_eq!(doc["eco_setpoint"], 16.5);
    assert_eq!(doc["indoor_entity"], indoor.as_str());

    let refused = call(
        "set_home_settings",
        json!({"settings": {"eco_setpoint": 99}}),
    )
    .await;
    assert_eq!(refused.is_error, Some(true));
    assert_eq!(hub.home_settings().eco_setpoint, Some(16.5));
    client.cancel().await.expect("close");
}
