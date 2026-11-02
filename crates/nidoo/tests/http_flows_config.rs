//! `PUT /api/v1/flows/{id}` wraps its answer with the Paris-fallback warning, and
//! `GET /api/v1/config/flows` reports the house point, over a hub served on an ephemeral port.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use nidoo::config::Config;
use nidoo::http::router;
use nidoo::hub::Hub;
use nidoo_api::{ConnectionStatus, FlowWriteOutcome, FlowsConfigView};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

const MOTION_TOPIC: &str = "nidoo/binary_sensor/0x00158d008b5a2d65/occupancy/config";
const MOTION_FIXTURE: &str = "discovery/binary_sensor__0x00158d008b5a2d65__occupancy.json";
const MOTION_ID: &str = "0x00158d008b5a2d65_occupancy_zigbee2mqtt";

struct App {
    addr: SocketAddr,
    client: reqwest::Client,
    _hub: Arc<Hub>,
}

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../nidoo-core/tests/fixtures/"
    ));
    std::fs::read(path.join(name)).expect("fixture")
}

async fn test_app(point: Option<(f64, f64)>) -> App {
    let mut cfg = Config::default();
    cfg.commands.confirm_timeout_ms = 50;
    cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
    cfg.flows.latitude = point.map(|p| p.0);
    cfg.flows.longitude = point.map(|p| p.1);
    let (tx, mut publisher) = mpsc::channel(64);
    tokio::spawn(async move { while publisher.recv().await.is_some() {} });
    let hub = Hub::new(&cfg, tx);
    hub.ingest(MOTION_TOPIC, &fixture(MOTION_FIXTURE));
    hub.set_connection(ConnectionStatus::Online);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(
        axum::serve(
            listener,
            router(Arc::clone(&hub), nidoo::setup::SetupCtx::inert()),
        )
        .into_future(),
    );
    App {
        addr,
        client: reqwest::Client::new(),
        _hub: hub,
    }
}

async fn test_app_without_coords() -> App {
    test_app(None).await
}

async fn test_app_with_point(lat: f64, lon: f64) -> App {
    test_app(Some((lat, lon))).await
}

async fn put_raw(app: &App, path: &str, body: &Value) -> Value {
    let response = app
        .client
        .put(format!("http://{}{path}", app.addr))
        .json(body)
        .send()
        .await
        .expect("PUT");
    assert!(response.status().is_success(), "{}", response.status());
    response.json().await.expect("json")
}

async fn put_json<T: DeserializeOwned>(app: &App, path: &str, body: &Value) -> T {
    serde_json::from_value(put_raw(app, path, body).await).expect("typed body")
}

async fn get_json<T: DeserializeOwned>(app: &App, path: &str) -> T {
    app.client
        .get(format!("http://{}{path}", app.addr))
        .send()
        .await
        .expect("GET")
        .json()
        .await
        .expect("typed body")
}

fn sun_flow_json(id: &str) -> Value {
    json!({ "id": id, "name": "S", "nodes": [{
        "id": "ev", "type": "trigger.sun", "params": { "which": "sunset" }
    }] })
}

#[tokio::test]
async fn put_flow_wraps_with_warning_when_coords_absent_and_flow_uses_sun() {
    let app = test_app_without_coords().await;
    let out: FlowWriteOutcome = put_json(&app, "/api/v1/flows/s", &sun_flow_json("s")).await;
    assert!(
        out.warning
            .as_deref()
            .is_some_and(|w| w.contains("Paris fallback"))
    );
    assert_eq!(out.flow.definition.id, "s");

    let plain = json!({ "id": "p", "name": "P", "nodes": [{
        "id": "t", "type": "trigger.state", "params": { "entity": MOTION_ID }
    }] });
    let out: FlowWriteOutcome = put_json(&app, "/api/v1/flows/p", &plain).await;
    assert!(out.warning.is_none());
}

#[tokio::test]
async fn put_flow_warns_for_an_is_dark_flow_without_coords() {
    let app = test_app_without_coords().await;
    let dark = json!({ "id": "d", "name": "D", "nodes": [
        { "id": "t", "type": "trigger.state", "params": { "entity": MOTION_ID } },
        { "id": "c", "type": "if", "params": { "op": "is_dark" } }
    ], "edges": [{ "from": "t", "to": "c" }] });
    let out: FlowWriteOutcome = put_json(&app, "/api/v1/flows/d", &dark).await;
    assert!(out.warning.is_some());
}

#[tokio::test]
async fn put_flow_no_warning_when_coords_set() {
    let app = test_app_with_point(45.75, 4.85).await;
    let out: FlowWriteOutcome = put_json(&app, "/api/v1/flows/s", &sun_flow_json("s")).await;
    assert!(out.warning.is_none());
}

#[tokio::test]
async fn warning_key_is_always_present_on_the_wire() {
    let app = test_app_with_point(45.75, 4.85).await;
    let raw = put_raw(&app, "/api/v1/flows/s", &sun_flow_json("s")).await;
    assert!(raw.get("warning").is_some());
    assert_eq!(raw["warning"], json!(null));
}

#[tokio::test]
async fn get_flows_config_reports_fallback_active() {
    let app = test_app_without_coords().await;
    let v: FlowsConfigView = get_json(&app, "/api/v1/config/flows").await;
    assert_eq!(v.latitude, None);
    assert_eq!(v.longitude, None);
    assert!(v.fallback_active);
    let app = test_app_with_point(45.75, 4.85).await;
    let v: FlowsConfigView = get_json(&app, "/api/v1/config/flows").await;
    assert_eq!(v.latitude, Some(45.75));
    assert!(!v.fallback_active);
}
