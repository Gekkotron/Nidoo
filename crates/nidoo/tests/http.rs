//! The HTTP API, the WebSocket stream and the embedded web UI, served on an ephemeral port
//! over a hub driven by fixtures (no broker).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use nidoo::config::Config;
use nidoo::http::router;
use nidoo::hub::Hub;
use nidoo_api::ConnectionStatus;
use nidoo_core::command::Publish;
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A recorded ZBMINI switch: the `rooms/stair/light` device.
const SWITCH_FIXTURE: &str = "discovery/switch__0x847127fffe24e322__switch.json";
const SWITCH_ID: &str = "0x847127fffe24e322_switch_zigbee2mqtt";
const SWITCH_TOPIC: &str = "nidoo/switch/0x847127fffe24e322/switch/config";
/// The same device's link-quality sensor: it reads the same state topic as the switch.
const LINKQUALITY_FIXTURE: &str = "discovery/sensor__0x847127fffe24e322__linkquality.json";
const LINKQUALITY_ID: &str = "0x847127fffe24e322_linkquality_zigbee2mqtt";
const LINKQUALITY_TOPIC: &str = "nidoo/sensor/0x847127fffe24e322/linkquality/config";

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

/// The switch's `state_topic`, from its discovery config.
fn switch_state_topic() -> String {
    let config: Value = serde_json::from_slice(&fixture(SWITCH_FIXTURE)).expect("switch config");
    config["state_topic"]
        .as_str()
        .expect("state_topic")
        .to_owned()
}

/// The LED and the motion sensor the reference flow talks to.
const LED_TOPIC: &str = "nidoo/light/0x60a423fffe94d1c6/light/config";
const LED_FIXTURE: &str = "discovery/light__0x60a423fffe94d1c6__light.json";
const LED_ID: &str = "0x60a423fffe94d1c6_light_zigbee2mqtt";
const MOTION_TOPIC: &str = "nidoo/binary_sensor/0x00158d008b5a2d65/occupancy/config";
const MOTION_FIXTURE: &str = "discovery/binary_sensor__0x00158d008b5a2d65__occupancy.json";
const MOTION_ID: &str = "0x00158d008b5a2d65_occupancy_zigbee2mqtt";

/// The living-room reference flow, as the UI would send it.
fn reference_flow() -> Value {
    json!({
        "id": "living", "name": "Living LED on motion", "enabled": true, "mode": "restart",
        "nodes": [
            { "id": "motion", "type": "trigger.state", "params": { "entity": MOTION_ID } },
            { "id": "on", "type": "command",
              "params": { "entity": LED_ID, "command": { "type": "turn_on" } } }
        ],
        "edges": [ { "from": "motion", "to": "on" } ]
    })
}

struct Server {
    hub: Arc<Hub>,
    publisher: mpsc::Receiver<Publish>,
    addr: SocketAddr,
    client: reqwest::Client,
}

impl Server {
    /// A hub with the switch discovered, served on `127.0.0.1:0`. Its broker link is still
    /// `Connecting`: a test that writes calls [`Server::online`] first.
    async fn start() -> Server {
        Server::start_with(config()).await
    }

    /// Like [`Server::start`], with the LED and the motion sensor discovered too and the broker
    /// link up, so flows can be written.
    async fn start_with_flows() -> Server {
        let server = Server::start().await;
        server.hub.ingest(LED_TOPIC, &fixture(LED_FIXTURE));
        server.hub.ingest(MOTION_TOPIC, &fixture(MOTION_FIXTURE));
        server.online();
        server
    }

    /// Like [`Server::start`], with its own configuration.
    async fn start_with(cfg: Config) -> Server {
        let (tx, publisher) = mpsc::channel(64);
        let hub = Hub::new(&cfg, tx);
        hub.ingest(SWITCH_TOPIC, &fixture(SWITCH_FIXTURE));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(
            axum::serve(
                listener,
                router(Arc::clone(&hub), nidoo::setup::SetupCtx::inert()),
            )
            .into_future(),
        );
        Server {
            hub,
            publisher,
            addr,
            client: reqwest::Client::new(),
        }
    }

    /// Marks the broker link up, so the hub's writers publish.
    fn online(&self) {
        self.hub.set_connection(ConnectionStatus::Online);
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    async fn get(&self, path: &str) -> reqwest::Response {
        self.client.get(self.url(path)).send().await.expect("GET")
    }

    async fn ws(&self) -> Ws {
        let (stream, _) = tokio_tungstenite::connect_async(format!("ws://{}/ws", self.addr))
            .await
            .expect("websocket");
        stream
    }
}

/// The next text frame of `ws`, parsed.
async fn next_json(ws: &mut Ws) -> Value {
    let frame = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("a frame within 2 s")
        .expect("open stream")
        .expect("a frame");
    match frame {
        Message::Text(text) => serde_json::from_str(text.as_str()).expect("json frame"),
        other => panic!("expected a text frame, got {other:?}"),
    }
}

#[tokio::test]
async fn health_and_snapshot() {
    let server = Server::start().await;

    let health = server.get("/health").await;
    assert_eq!(health.status(), StatusCode::OK);
    let health: Value = health.json().await.expect("json");
    assert_eq!(health["status"], "ok");
    assert_eq!(health["connection"]["type"], "connecting");
    assert_eq!(health["entities"], 1);

    let snapshot = server.get("/api/v1/snapshot").await;
    assert_eq!(snapshot.status(), StatusCode::OK);
    assert_eq!(
        snapshot
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
    let snapshot: Value = snapshot.json().await.expect("json");
    assert_eq!(snapshot["connection"]["type"], "connecting");
    assert_eq!(snapshot["entities"].as_array().map(Vec::len), Some(1));
    assert_eq!(snapshot["entities"][0]["id"], SWITCH_ID);
    assert_eq!(snapshot["entities"][0]["kind"], "switch");

    let missing = server.get("/api/v1/nope").await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let missing: Value = missing.json().await.expect("json error body");
    assert!(missing["error"].is_string());
}

#[tokio::test]
async fn alarm_test_chirp_without_that_siren_is_422() {
    let server = Server::start().await;

    let response = server
        .client
        .post(server.url("/api/v1/alarm/test-chirp"))
        .json(&json!({"siren": 0}))
        .send()
        .await
        .expect("POST");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body: Value = response.json().await.expect("json");
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|e| e.contains("no siren"))
    );
}

#[tokio::test]
async fn command_unknown_entity_404() {
    let server = Server::start().await;

    let response = server
        .client
        .post(server.url("/api/v1/entities/nope/command"))
        .json(&json!({"type": "turn_off"}))
        .send()
        .await
        .expect("POST");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body: Value = response.json().await.expect("json");
    assert_eq!(body["error"], "unknown entity `nope`");
}

#[tokio::test]
async fn command_offline_entity_422_and_timeout_200() {
    let mut server = Server::start().await;
    server.online();
    server
        .hub
        .ingest("zigbee2mqtt/bridge/state", br#"{"state":"offline"}"#);

    let response = server
        .client
        .post(server.url(&format!("/api/v1/entities/{SWITCH_ID}/command")))
        .json(&json!({"type": "turn_off"}))
        .send()
        .await
        .expect("POST");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body: Value = response.json().await.expect("json");
    assert_eq!(body["error"], "rejected: entity is offline");
    assert!(server.publisher.try_recv().is_err());

    server
        .hub
        .ingest("zigbee2mqtt/bridge/state", br#"{"state":"online"}"#);
    let response = server
        .client
        .post(server.url(&format!("/api/v1/entities/{SWITCH_ID}/command")))
        .json(&json!({"type": "turn_on"}))
        .send()
        .await
        .expect("POST");
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.expect("json");
    assert_eq!(body["accepted"], false);
    assert_eq!(body["reason"], "no state change within 50 ms");
    let published = server.publisher.try_recv().expect("the command publish");
    assert_eq!(published.topic, "zigbee2mqtt/rooms/stair/light/set");
    assert_eq!(published.payload, "ON");
}

#[tokio::test]
async fn patch_override_then_snapshot_reflects_it() {
    let mut server = Server::start().await;
    server.online();

    let response = server
        .client
        .patch(server.url(&format!("/api/v1/entities/{SWITCH_ID}/override")))
        .json(&json!({"showAs": "light", "favorite": true}))
        .send()
        .await
        .expect("PATCH");
    assert_eq!(response.status(), StatusCode::OK);
    let view: Value = response.json().await.expect("json");
    assert_eq!(view["id"], SWITCH_ID);
    assert_eq!(view["kind"], "light");
    assert_eq!(view["ui"]["favorite"], true);
    let published = server.publisher.try_recv().expect("the override publish");
    assert_eq!(published.topic, format!("nidoo/ui/{SWITCH_ID}"));
    assert!(published.retain);

    let snapshot: Value = server
        .get("/api/v1/snapshot")
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(snapshot["entities"][0]["kind"], "light");
    assert_eq!(snapshot["entities"][0]["ui"]["favorite"], true);

    let response = server
        .client
        .delete(server.url(&format!("/api/v1/entities/{SWITCH_ID}/override")))
        .send()
        .await
        .expect("DELETE");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let published = server.publisher.try_recv().expect("the clearing publish");
    assert_eq!(published.payload, "");
    assert!(published.retain);
    let snapshot: Value = server
        .get("/api/v1/snapshot")
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(snapshot["entities"][0]["kind"], "switch");
    assert_eq!(snapshot["entities"][0]["ui"]["favorite"], Value::Null);

    let response = server
        .client
        .delete(server.url("/api/v1/entities/nope/override"))
        .send()
        .await
        .expect("DELETE");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// While the broker link is down (here: still connecting) a write is refused with 503 and
/// nothing is queued, instead of being accepted and lost.
#[tokio::test]
async fn writes_while_broker_offline_503() {
    let mut server = Server::start().await;

    let response = server
        .client
        .patch(server.url(&format!("/api/v1/entities/{SWITCH_ID}/override")))
        .json(&json!({"favorite": true}))
        .send()
        .await
        .expect("PATCH");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = response.json().await.expect("json");
    assert_eq!(body["error"], "publish: broker offline");

    let response = server
        .client
        .post(server.url(&format!("/api/v1/entities/{SWITCH_ID}/command")))
        .json(&json!({"type": "turn_on"}))
        .send()
        .await
        .expect("POST");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(server.publisher.try_recv().is_err(), "nothing was queued");
    let snapshot: Value = server
        .get("/api/v1/snapshot")
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(snapshot["entities"][0]["ui"]["favorite"], Value::Null);
}

#[tokio::test]
async fn invalid_patch_400() {
    let server = Server::start().await;

    let response = server
        .client
        .patch(server.url(&format!("/api/v1/entities/{SWITCH_ID}/override")))
        .json(&json!({"nope": true}))
        .send()
        .await
        .expect("PATCH");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.expect("json");
    assert_eq!(body["error"], "unknown override field `nope`");

    let response = server
        .client
        .patch(server.url("/api/v1/entities/nope/override"))
        .json(&json!({"favorite": true}))
        .send()
        .await
        .expect("PATCH");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn put_areas_and_list_ignored() {
    let mut server = Server::start().await;
    server.online();

    let response = server
        .client
        .put(server.url("/api/v1/areas"))
        .json(&json!({"areas": [{"id": "stair", "name": "Stair", "order": 2}]}))
        .send()
        .await
        .expect("PUT");
    assert_eq!(response.status(), StatusCode::OK);
    let areas: Value = response.json().await.expect("json");
    let stair = areas
        .as_array()
        .expect("an array")
        .iter()
        .find(|a| a["id"] == "stair")
        .expect("the declared area");
    assert_eq!(stair["name"], "Stair");
    assert_eq!(stair["order"], 2);
    let published = server.publisher.try_recv().expect("the areas publish");
    assert_eq!(published.topic, "nidoo/ui/_areas");
    assert!(published.retain);

    let ignored = server.get("/api/v1/ignored").await;
    assert_eq!(ignored.status(), StatusCode::OK);
    let ignored: Value = ignored.json().await.expect("json");
    assert!(ignored.is_array());
}

#[tokio::test]
async fn patch_area_renames_and_keeps_the_app_fields() {
    let mut server = Server::start().await;
    server.online();
    server.hub.ingest(
        "nidoo/ui/_areas",
        br#"{"areas":[{"id":"dehors","name":"Dehors","order":7,"groupNames":{"cover":"Volets"}}]}"#,
    );

    let response = server
        .client
        .patch(server.url("/api/v1/areas/dehors"))
        .json(&json!({"name": "  Outdoor "}))
        .send()
        .await
        .expect("PATCH");
    assert_eq!(response.status(), StatusCode::OK);
    let areas: Value = response.json().await.expect("json");
    let dehors = areas
        .as_array()
        .expect("an array")
        .iter()
        .find(|a| a["id"] == "dehors")
        .expect("the renamed area");
    assert_eq!(dehors["name"], "Outdoor");
    let published = server.publisher.try_recv().expect("the areas publish");
    assert_eq!(published.topic, "nidoo/ui/_areas");
    assert_eq!(
        published.payload,
        r#"{"areas":[{"id":"dehors","name":"Outdoor","order":7,"groupNames":{"cover":"Volets"}}]}"#
    );

    for (path, name, status) in [
        (
            "/api/v1/areas/dehors",
            " ",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "/api/v1/areas/Unassigned",
            "Nowhere",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ("/api/v1/areas/attic", "Attic", StatusCode::NOT_FOUND),
    ] {
        let response = server
            .client
            .patch(server.url(path))
            .json(&json!({"name": name}))
            .send()
            .await
            .expect("PATCH");
        assert_eq!(response.status(), status, "{path} {name:?}");
    }
    assert!(
        server.publisher.try_recv().is_err(),
        "a refused rename publishes nothing"
    );
}

async fn patch_group(server: &Server, path: &str, body: Value) -> reqwest::Response {
    server
        .client
        .patch(server.url(path))
        .json(&body)
        .send()
        .await
        .expect("PATCH")
}

#[tokio::test]
async fn rename_keeps_group_fields() {
    let mut server = Server::start().await;
    server.online();
    server.hub.ingest(
        "nidoo/ui/_areas",
        br#"{"areas":[{"id":"dehors","name":"Dehors","groupNames":{"cover":"Volets"}}]}"#,
    );

    let response = patch_group(&server, "/api/v1/areas/dehors", json!({"name": "Outdoor"})).await;
    assert_eq!(response.status(), StatusCode::OK);
    let published = server.publisher.try_recv().expect("the rename publish");
    assert_eq!(
        published.payload,
        r#"{"areas":[{"id":"dehors","name":"Outdoor","groupNames":{"cover":"Volets"}}]}"#
    );
}

#[tokio::test]
async fn ws_sends_snapshot_then_events() {
    let server = Server::start().await;
    let mut ws = server.ws().await;

    let first = next_json(&mut ws).await;
    assert_eq!(first["type"], "snapshot");
    assert_eq!(first["entities"][0]["id"], SWITCH_ID);

    server
        .hub
        .ingest(&switch_state_topic(), br#"{"state":"ON"}"#);
    let event = next_json(&mut ws).await;
    assert_eq!(event["type"], "state_changed");
    assert_eq!(event["id"], SWITCH_ID);
    assert_eq!(
        event["state"]["value"],
        json!({"type": "on_off", "value": true})
    );

    // The same report again changes nothing and is not pushed; the next frame is the OFF.
    server
        .hub
        .ingest(&switch_state_topic(), br#"{"state":"ON"}"#);
    server
        .hub
        .ingest(&switch_state_topic(), br#"{"state":"OFF"}"#);
    let event = next_json(&mut ws).await;
    assert_eq!(event["type"], "state_changed");
    assert_eq!(
        event["state"]["value"],
        json!({"type": "on_off", "value": false})
    );

    ws.close(None).await.expect("close");
}

/// The test runtime is single-threaded, so the client cannot be served while the test floods
/// the hub: the broadcast receiver lags and the stream recovers with a fresh snapshot. The
/// events still queued when it lagged are older than that snapshot and must not follow it:
/// the first frame after it is the one for a message ingested afterwards.
#[tokio::test]
async fn ws_lag_sends_fresh_snapshot() {
    let server = Server::start().await;
    server
        .hub
        .ingest(LINKQUALITY_TOPIC, &fixture(LINKQUALITY_FIXTURE));
    let mut ws = server.ws().await;
    assert_eq!(next_json(&mut ws).await["type"], "snapshot");

    // The flood only ever changes the switch; the sensor's key is absent from every payload.
    let topic = switch_state_topic();
    for i in 0..1100 {
        let payload: &[u8] = if i % 2 == 0 {
            br#"{"state":"ON"}"#
        } else {
            br#"{"state":"OFF"}"#
        };
        server.hub.ingest(&topic, payload);
    }

    let recovered = next_json(&mut ws).await;
    assert_eq!(recovered["type"], "snapshot");
    let switch = recovered["entities"]
        .as_array()
        .expect("entities")
        .iter()
        .find(|e| e["id"] == SWITCH_ID)
        .expect("the switch");
    assert_eq!(
        switch["state"]["value"],
        json!({"type": "on_off", "value": false})
    );

    server.hub.ingest(&topic, br#"{"linkquality":42}"#);
    let event = next_json(&mut ws).await;
    assert_eq!(
        (event["type"].as_str(), event["id"].as_str()),
        (Some("state_changed"), Some(LINKQUALITY_ID)),
        "no event older than the snapshot may follow it: {event}"
    );
    assert_eq!(
        event["state"]["value"],
        json!({"type": "number", "value": 42.0})
    );
}

#[tokio::test]
async fn cors_is_open_to_any_origin() {
    let server = Server::start().await;

    let cors = server
        .client
        .get(server.url("/api/v1/snapshot"))
        .header("origin", "http://localhost:8081")
        .send()
        .await
        .expect("GET");
    assert_eq!(
        cors.headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("*")
    );
}

// ----- push alerts -----

const HISTORY_TOPIC: &str = "nidoo/push/history";
const ALERT_ENTRY: &str = r#"{"v":1,"id":"frigate-front-1696","channel":"camera","severity":"warning","title":"Front Door","body":"Person in Porch","actions":[{"id":"disarm","label":"Disarm"}],"silent":false,"sent_at":1760880000,"source":"frigate"}"#;

fn history_payload() -> Vec<u8> {
    format!("[{ALERT_ENTRY}]").into_bytes()
}

#[tokio::test]
async fn alerts_route_and_ws_event() {
    let server = Server::start().await;
    let mut ws = server.ws().await;
    let snapshot = next_json(&mut ws).await;
    assert_eq!(snapshot["type"], "snapshot");
    assert_eq!(snapshot["alerts"], json!([]));

    server.hub.ingest(HISTORY_TOPIC, &history_payload());

    let event = next_json(&mut ws).await;
    assert_eq!(event["type"], "alerts_changed");
    assert_eq!(event["alerts"][0]["id"], "frigate-front-1696");

    let response = server.get("/api/v1/alerts").await;
    assert_eq!(response.status(), StatusCode::OK);
    let alerts: Value = response.json().await.expect("json");
    assert_eq!(alerts.as_array().map(Vec::len), Some(1));
    assert_eq!(alerts[0]["channel"], "camera");
}

#[tokio::test]
async fn push_status_reports_off_without_credentials() {
    let server = Server::start().await;

    let response = server.get("/api/v1/push").await;
    assert_eq!(response.status(), StatusCode::OK);
    let status: Value = response.json().await.expect("json");
    assert_eq!(status, json!({"state": "off", "installs": 0, "queued": 0}));
}

#[tokio::test]
async fn areas_changed_reaches_the_socket() {
    let server = Server::start().await;
    server.online();
    let mut ws = server.ws().await;
    next_json(&mut ws).await;

    let response = server
        .client
        .put(server.url("/api/v1/areas"))
        .json(&json!({"areas": [{"id": "stair", "name": "Stair", "order": 2}]}))
        .send()
        .await
        .expect("PUT");
    assert_eq!(response.status(), StatusCode::OK);

    let event = next_json(&mut ws).await;
    assert_eq!(event["type"], "areas_changed");
    assert!(event["areas"].is_array(), "got {event}");
}

#[tokio::test]
async fn alert_action_publishes_and_404s() {
    let mut server = Server::start().await;
    server.online();
    server.hub.ingest(HISTORY_TOPIC, &history_payload());

    let response = server
        .client
        .post(server.url("/api/v1/alerts/frigate-front-1696/actions/disarm"))
        .send()
        .await
        .expect("POST");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let published = server.publisher.recv().await.expect("a publish");
    assert_eq!(published.topic, "nidoo/push/responses");

    let response = server
        .client
        .post(server.url("/api/v1/alerts/nope/actions/disarm"))
        .send()
        .await
        .expect("POST");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body: Value = response.json().await.expect("json");
    assert!(body["error"].as_str().unwrap_or_default().contains("nope"));

    let response = server
        .client
        .post(server.url("/api/v1/alerts/frigate-front-1696/actions/zzz"))
        .send()
        .await
        .expect("POST");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn alert_action_refused_offline() {
    let server = Server::start().await;
    server.hub.ingest(HISTORY_TOPIC, &history_payload());

    let response = server
        .client
        .post(server.url("/api/v1/alerts/frigate-front-1696/actions/disarm"))
        .send()
        .await
        .expect("POST");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// ----- Frigate media proxy -----

mod fake_frigate {
    //! A Frigate stand-in: `/api/login` hands out `frigate_token=tok<N>` for user `u` /
    //! password `p` (N counts the logins); the snapshot wants the latest token, and when
    //! `expire_used` is set a token is accepted only once. `/api/config` lists the cameras
    //! `front_door`, `garden` and a disabled `old`; their frames want the token too. Every
    //! request path is logged.

    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::Router;
    use axum::body::Body;
    use axum::extract::{RawQuery, Request, State};
    use axum::http::{HeaderMap, StatusCode, header};
    use axum::middleware::{self, Next};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use serde_json::Value;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    pub struct Fake {
        pub addr: SocketAddr,
        pub logins: Arc<AtomicUsize>,
        state: Arc<FakeState>,
    }

    struct FakeState {
        logins: Arc<AtomicUsize>,
        expire_used: bool,
        used: Mutex<Vec<String>>,
        config_reads: AtomicUsize,
        camera_config_reads: AtomicUsize,
        frame_queries: Mutex<Vec<String>>,
        paths: Mutex<Vec<String>>,
    }

    impl Fake {
        pub async fn start(expire_used: bool) -> Fake {
            let logins = Arc::new(AtomicUsize::new(0));
            let state = Arc::new(FakeState {
                logins: Arc::clone(&logins),
                expire_used,
                used: Mutex::new(Vec::new()),
                config_reads: AtomicUsize::new(0),
                camera_config_reads: AtomicUsize::new(0),
                frame_queries: Mutex::new(Vec::new()),
                paths: Mutex::new(Vec::new()),
            });
            let app = Router::new()
                .route("/api/login", post(login))
                .route("/api/events/1/snapshot.jpg", get(snapshot))
                .route("/api/events/1/clip.mp4", get(snapshot))
                .route("/api/events/1/thumbnail.jpg", get(snapshot))
                .route("/api/events/redirect/snapshot.jpg", get(redirect_to_config))
                .route("/api/config/raw", get(config_raw))
                .route("/api/config", get(camera_config))
                .route("/api/front_door/latest.jpg", get(latest))
                .route("/api/garden/latest.jpg", get(latest))
                .route("/api/front_door", get(mjpeg))
                .layer(middleware::from_fn_with_state(Arc::clone(&state), log_path))
                .with_state(Arc::clone(&state));
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
            let addr = listener.local_addr().expect("addr");
            tokio::spawn(axum::serve(listener, app).into_future());
            Fake {
                addr,
                logins,
                state,
            }
        }

        pub fn origin(&self) -> String {
            format!("http://{}", self.addr)
        }

        pub fn snapshot_url(&self) -> String {
            format!("{}/api/events/1/snapshot.jpg", self.origin())
        }

        pub async fn tokens_used(&self) -> usize {
            self.state.used.lock().await.len()
        }

        /// How many times `/api/config/raw` was read (it never should be).
        pub fn config_reads(&self) -> usize {
            self.state.config_reads.load(Ordering::SeqCst)
        }

        /// How many times `/api/config` (the camera list) was read.
        pub fn camera_config_reads(&self) -> usize {
            self.state.camera_config_reads.load(Ordering::SeqCst)
        }

        /// The query strings of the `latest.jpg` requests, in order.
        pub async fn frame_queries(&self) -> Vec<String> {
            self.state.frame_queries.lock().await.clone()
        }

        /// Every request path, in order.
        pub async fn paths(&self) -> Vec<String> {
            self.state.paths.lock().await.clone()
        }
    }

    async fn log_path(
        State(state): State<Arc<FakeState>>,
        request: Request,
        next: Next,
    ) -> Response {
        state
            .paths
            .lock()
            .await
            .push(request.uri().path().to_owned());
        next.run(request).await
    }

    async fn camera_config(State(state): State<Arc<FakeState>>) -> Response {
        state.camera_config_reads.fetch_add(1, Ordering::SeqCst);
        let config = r#"{"mqtt":{"password":"secret"},"cameras":{"front_door":{"enabled":true},"garden":{},"old":{"enabled":false}}}"#;
        ([(header::CONTENT_TYPE, "application/json")], config).into_response()
    }

    async fn latest(
        State(state): State<Arc<FakeState>>,
        RawQuery(query): RawQuery,
        headers: HeaderMap,
    ) -> Response {
        state
            .frame_queries
            .lock()
            .await
            .push(query.unwrap_or_default());
        snapshot(State(state), headers).await
    }

    async fn mjpeg() -> Response {
        Response::builder()
            .header(
                header::CONTENT_TYPE,
                "multipart/x-mixed-replace; boundary=frame",
            )
            .body(Body::from(
                "--frame\r\nContent-Type: image/jpeg\r\n\r\nJPEG\r\n",
            ))
            .expect("response")
    }

    async fn redirect_to_config() -> Response {
        Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, "/api/config/raw")
            .body(Body::empty())
            .expect("response")
    }

    async fn config_raw(State(state): State<Arc<FakeState>>) -> Response {
        state.config_reads.fetch_add(1, Ordering::SeqCst);
        (StatusCode::OK, "mqtt:\n  password: secret\n").into_response()
    }

    async fn login(State(state): State<Arc<FakeState>>, body: String) -> Response {
        let value: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        if value["user"] != "u" || value["password"] != "p" {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let n = state.logins.fetch_add(1, Ordering::SeqCst) + 1;
        Response::builder()
            .status(StatusCode::OK)
            .header(header::SET_COOKIE, "other=1; Path=/")
            .header(
                header::SET_COOKIE,
                format!("frigate_token=tok{n}; Path=/; HttpOnly"),
            )
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .expect("response")
    }

    async fn snapshot(State(state): State<Arc<FakeState>>, headers: HeaderMap) -> Response {
        let wanted = format!("tok{}", state.logins.load(Ordering::SeqCst));
        let cookie = headers
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        let presented = cookie
            .split(';')
            .map(str::trim)
            .find_map(|c| c.strip_prefix("frigate_token="))
            .unwrap_or_default()
            .to_owned();
        if presented.is_empty() || presented != wanted {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        {
            let mut used = state.used.lock().await;
            if state.expire_used && used.contains(&presented) {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            used.push(presented);
        }
        let mut builder = Response::builder()
            .header(header::CONTENT_TYPE, "image/jpeg")
            .header(header::ACCEPT_RANGES, "bytes");
        if headers.contains_key(header::RANGE) {
            builder = builder
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_RANGE, "bytes 0-3/4");
        } else {
            builder = builder.status(StatusCode::OK);
        }
        builder.body(Body::from("JPEG")).expect("response")
    }
}

fn frigate_config(fake: &fake_frigate::Fake, with_login: bool) -> Config {
    let mut cfg = config();
    cfg.frigate.url = Some(fake.origin());
    if with_login {
        cfg.frigate.user = Some("u".into());
        cfg.frigate.password = Some("p".into());
    }
    cfg
}

#[tokio::test]
async fn media_proxy_logs_in_and_streams() {
    let fake = fake_frigate::Fake::start(false).await;
    let server = Server::start_with(frigate_config(&fake, true)).await;

    let response = server
        .get(&format!("/api/v1/media?url={}", fake.snapshot_url()))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("image/jpeg")
    );
    assert_eq!(response.text().await.expect("body"), "JPEG");
    assert_eq!(fake.logins.load(std::sync::atomic::Ordering::SeqCst), 1);

    // A second fetch reuses the cached token.
    let response = server
        .get(&format!("/api/v1/media?url={}", fake.snapshot_url()))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(fake.logins.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn media_proxy_passes_range_through() {
    let fake = fake_frigate::Fake::start(false).await;
    let server = Server::start_with(frigate_config(&fake, true)).await;

    let response = server
        .client
        .get(server.url(&format!("/api/v1/media?url={}", fake.snapshot_url())))
        .header("range", "bytes=0-3")
        .send()
        .await
        .expect("GET");

    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        response
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok()),
        Some("bytes 0-3/4")
    );
    assert_eq!(
        response
            .headers()
            .get("accept-ranges")
            .and_then(|v| v.to_str().ok()),
        Some("bytes")
    );
}

#[tokio::test]
async fn media_proxy_relogs_once_on_401() {
    let fake = fake_frigate::Fake::start(true).await;
    let server = Server::start_with(frigate_config(&fake, true)).await;
    let path = format!("/api/v1/media?url={}", fake.snapshot_url());

    assert_eq!(server.get(&path).await.status(), StatusCode::OK);
    // tok1 is now spent: the next fetch gets a 401, the hub logs in again and retries once.
    assert_eq!(server.get(&path).await.status(), StatusCode::OK);
    assert_eq!(fake.logins.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(fake.tokens_used().await, 2);
}

#[tokio::test]
async fn media_proxy_without_login_sends_no_cookie() {
    let fake = fake_frigate::Fake::start(false).await;
    let server = Server::start_with(frigate_config(&fake, false)).await;

    let response = server
        .get(&format!("/api/v1/media?url={}", fake.snapshot_url()))
        .await;

    // The fake insists on a cookie, so the upstream 401 is passed through untouched.
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(fake.logins.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn media_proxy_refuses_other_origins_and_missing_url() {
    let fake = fake_frigate::Fake::start(false).await;
    let server = Server::start_with(frigate_config(&fake, true)).await;

    let response = server
        .get("/api/v1/media?url=http://elsewhere:5000/api/events/1/snapshot.jpg")
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = server.get("/api/v1/media").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = server.get("/api/v1/media?url=nope").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Without a [frigate] section there is nothing to proxy.
    let plain = Server::start().await;
    let response = plain
        .get(&format!("/api/v1/media?url={}", fake.snapshot_url()))
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn alerts_carry_proxied_media_urls() {
    let fake = fake_frigate::Fake::start(false).await;
    let server = Server::start_with(frigate_config(&fake, true)).await;
    let entry = format!(
        r#"[{{"v":1,"id":"cam","channel":"camera","title":"Front","sent_at":1,"image_url":"{}"}}]"#,
        fake.snapshot_url()
    );

    server.hub.ingest(HISTORY_TOPIC, entry.as_bytes());

    let alerts: Value = server
        .get("/api/v1/alerts")
        .await
        .json()
        .await
        .expect("json");
    let image = alerts[0]["image_url"].as_str().expect("image_url");
    assert!(image.starts_with("/api/v1/media?url="), "{image}");
    assert!(image.contains("snapshot.jpg"), "{image}");
}

#[tokio::test]
async fn media_proxy_allows_only_event_media() {
    let fake = fake_frigate::Fake::start(false).await;
    let server = Server::start_with(frigate_config(&fake, true)).await;

    for path in [
        "/api/config/raw",
        "/api/front/latest.jpg",
        "/api/events/1/snapshot.jpg/../../config/raw",
        "/api/events//snapshot.jpg",
        "/api/events/1/other.bin",
    ] {
        let response = server
            .get(&format!("/api/v1/media?url={}{path}", fake.origin()))
            .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
    }
    // Still fine: the three event media kinds, with or without a query.
    for path in [
        "/api/events/1/snapshot.jpg",
        "/api/events/1/snapshot.jpg?h=112",
        "/api/events/1/clip.mp4",
        "/api/events/1/thumbnail.jpg",
    ] {
        let response = server
            .get(&format!("/api/v1/media?url={}{path}", fake.origin()))
            .await;
        assert_ne!(response.status(), StatusCode::FORBIDDEN, "{path}");
    }
    assert_eq!(fake.logins.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn media_proxy_does_not_follow_redirects() {
    let fake = fake_frigate::Fake::start(false).await;
    let server = Server::start_with(frigate_config(&fake, true)).await;

    // The fake answers this event id with a redirect to its config; the hub must hand the
    // redirect status back instead of following it.
    let response = server
        .get(&format!(
            "/api/v1/media?url={}/api/events/redirect/snapshot.jpg",
            fake.origin()
        ))
        .await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(fake.config_reads(), 0);
}

#[tokio::test]
async fn media_proxy_gives_up_quickly_on_an_unreachable_frigate() {
    // 10.255.255.1 is unroutable from a LAN: the connect hangs until a timeout fires.
    let mut cfg = config();
    cfg.frigate.url = Some("http://10.255.255.1:8971".into());
    cfg.frigate.user = Some("u".into());
    cfg.frigate.password = Some("p".into());
    cfg.frigate.connect_timeout_ms = 300;
    let server = Server::start_with(cfg).await;

    let started = std::time::Instant::now();
    let response = server
        .get("/api/v1/media?url=http://10.255.255.1:8971/api/events/1/snapshot.jpg")
        .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "took {:?}",
        started.elapsed()
    );
}

fn live_config(fake: &fake_frigate::Fake) -> Config {
    let mut cfg = frigate_config(fake, true);
    cfg.frigate.live = true;
    cfg
}

fn header_of(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

#[tokio::test]
async fn cameras_lists_enabled_frigate_cameras() {
    let fake = fake_frigate::Fake::start(false).await;
    let server = Server::start_with(live_config(&fake)).await;

    let body: Value = server
        .get("/api/v1/cameras")
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(
        body,
        json!({"live": true, "cameras": [
            {"id": "front_door", "name": "Front Door"},
            {"id": "garden", "name": "Garden"},
        ]})
    );
    server.get("/api/v1/cameras").await;
    assert_eq!(fake.camera_config_reads(), 1);
}

#[tokio::test]
async fn cameras_without_frigate_is_empty() {
    let server = Server::start().await;
    let response = server.get("/api/v1/cameras").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.expect("json");
    assert_eq!(body, json!({"live": false, "cameras": []}));
}

#[tokio::test]
async fn camera_frames_need_live() {
    let fake = fake_frigate::Fake::start(false).await;
    let server = Server::start_with(frigate_config(&fake, true)).await;

    for path in [
        "/api/v1/cameras/front_door/latest.jpg",
        "/api/v1/cameras/front_door/stream",
    ] {
        assert_eq!(
            server.get(path).await.status(),
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
    assert!(fake.frame_queries().await.is_empty());
    assert!(
        !fake
            .paths()
            .await
            .iter()
            .any(|p| p.starts_with("/api/front_door")),
        "{:?}",
        fake.paths().await
    );
}

#[tokio::test]
async fn camera_latest_passes_through_with_login() {
    let fake = fake_frigate::Fake::start(false).await;
    let server = Server::start_with(live_config(&fake)).await;

    let response = server
        .get("/api/v1/cameras/front_door/latest.jpg?h=720")
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header_of(&response, "content-type").as_deref(),
        Some("image/jpeg")
    );
    assert_eq!(
        header_of(&response, "cache-control").as_deref(),
        Some("no-store")
    );
    assert_eq!(response.text().await.expect("body"), "JPEG");

    let response = server.get("/api/v1/cameras/garden/latest.jpg?h=5000").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        fake.frame_queries().await,
        ["h=720&height=720", "h=1080&height=1080"]
    );
}

#[tokio::test]
async fn camera_stream_is_multipart() {
    let fake = fake_frigate::Fake::start(false).await;
    let server = Server::start_with(live_config(&fake)).await;

    let response = server.get("/api/v1/cameras/front_door/stream?h=360").await;
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = header_of(&response, "content-type").unwrap_or_default();
    assert!(
        content_type.starts_with("multipart/x-mixed-replace"),
        "{content_type}"
    );
    assert_eq!(
        header_of(&response, "cache-control").as_deref(),
        Some("no-store")
    );
}

#[tokio::test]
async fn unknown_or_odd_camera_ids_never_reach_frigate() {
    let fake = fake_frigate::Fake::start(false).await;
    let server = Server::start_with(live_config(&fake)).await;

    for id in ["nope", "..%2Fconfig", "a%20b", "old"] {
        for file in ["latest.jpg", "stream"] {
            let path = format!("/api/v1/cameras/{id}/{file}");
            assert_eq!(
                server.get(&path).await.status(),
                StatusCode::NOT_FOUND,
                "{path}"
            );
        }
    }
    let paths = fake.paths().await;
    assert!(
        paths
            .iter()
            .all(|p| p == "/api/login" || p == "/api/config"),
        "{paths:?}"
    );
}

#[tokio::test]
async fn frigate_down_lists_nothing_and_frames_fail() {
    let closed = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = closed.local_addr().expect("addr");
    drop(closed);
    let mut cfg = config();
    cfg.frigate.url = Some(format!("http://{addr}"));
    cfg.frigate.live = true;
    let server = Server::start_with(cfg).await;

    let response = server.get("/api/v1/cameras").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.expect("json");
    assert_eq!(body, json!({"live": true, "cameras": []}));

    let response = server.get("/api/v1/cameras/front_door/latest.jpg").await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn a_frigate_failure_is_remembered_briefly() {
    // A Frigate that accepts connections and hangs up at once: every read fails fast.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let accepted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    {
        let accepted = Arc::clone(&accepted);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                accepted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                drop(stream);
            }
        });
    }
    let mut cfg = config();
    cfg.frigate.url = Some(format!("http://{addr}"));
    cfg.frigate.live = true;
    let server = Server::start_with(cfg).await;

    for _ in 0..4 {
        let response = server.get("/api/v1/cameras/front_door/latest.jpg").await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }
    let body: Value = server
        .get("/api/v1/cameras")
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(body["cameras"], json!([]));
    assert_eq!(accepted.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_failed_frame_fetch_spares_frigate_briefly() {
    // A Frigate whose list answers but whose frame request drops the connection (the handler
    // panics), as when Frigate goes down after the list was read.
    use axum::routing::get;
    let frames = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let app = {
        let frames = Arc::clone(&frames);
        axum::Router::new()
            .route(
                "/api/config",
                get(|| async { r#"{"cameras":{"front_door":{}}}"# }),
            )
            .route(
                "/api/front_door/latest.jpg",
                get(move || {
                    frames.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    async { panic!("connection lost") as &'static str }
                }),
            )
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(axum::serve(listener, app).into_future());
    let mut cfg = config();
    cfg.frigate.url = Some(format!("http://{addr}"));
    cfg.frigate.live = true;
    let server = Server::start_with(cfg).await;

    for _ in 0..4 {
        let response = server.get("/api/v1/cameras/front_door/latest.jpg").await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }
    assert_eq!(frames.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn put_flow_then_get_and_list() {
    let server = Server::start_with_flows().await;

    let put = server
        .client
        .put(server.url("/api/v1/flows/living"))
        .json(&reference_flow())
        .send()
        .await
        .expect("PUT");
    assert_eq!(put.status(), StatusCode::OK);
    let put: Value = put.json().await.expect("json");
    assert_eq!(put["flow"]["definition"]["name"], "Living LED on motion");

    let one = server.get("/api/v1/flows/living").await;
    assert_eq!(one.status(), StatusCode::OK);
    let one: Value = one.json().await.expect("json");
    assert_eq!(one["definition"]["name"], "Living LED on motion");

    let list: Value = server
        .get("/api/v1/flows")
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(list.as_array().map(Vec::len), Some(1));

    let missing = server.get("/api/v1/flows/nope").await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let missing: Value = missing.json().await.expect("json");
    assert!(missing["error"].as_str().unwrap().contains("unknown flow"));
}

#[tokio::test]
async fn put_flow_invalid_422_and_id_mismatch_422() {
    let server = Server::start_with_flows().await;

    let mut bad = reference_flow();
    bad["nodes"][1]["params"]["entity"] = json!("no_such_entity");
    let res = server
        .client
        .put(server.url("/api/v1/flows/living"))
        .json(&bad)
        .send()
        .await
        .expect("PUT");
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let res = server
        .client
        .put(server.url("/api/v1/flows/other"))
        .json(&reference_flow())
        .send()
        .await
        .expect("PUT");
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let res: Value = res.json().await.expect("json");
    assert!(
        res["error"].as_str().unwrap().starts_with("id mismatch"),
        "{res}"
    );

    // A body without an id takes the path's.
    let mut anonymous = reference_flow();
    anonymous.as_object_mut().unwrap().remove("id");
    let res = server
        .client
        .put(server.url("/api/v1/flows/living"))
        .json(&anonymous)
        .send()
        .await
        .expect("PUT");
    assert_eq!(res.status(), StatusCode::OK);
    let res: Value = res.json().await.expect("json");
    assert_eq!(res["flow"]["definition"]["id"], "living");
}

#[tokio::test]
async fn flows_offline_503() {
    let server = Server::start_with_flows().await;
    server.hub.set_connection(ConnectionStatus::Connecting);
    let res = server
        .client
        .put(server.url("/api/v1/flows/living"))
        .json(&reference_flow())
        .send()
        .await
        .expect("PUT");
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn delete_flow_204_then_404() {
    let server = Server::start_with_flows().await;
    server
        .client
        .put(server.url("/api/v1/flows/living"))
        .json(&reference_flow())
        .send()
        .await
        .expect("PUT");

    let del = server
        .client
        .delete(server.url("/api/v1/flows/living"))
        .send()
        .await
        .expect("DELETE");
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
    let del = server
        .client
        .delete(server.url("/api/v1/flows/living"))
        .send()
        .await
        .expect("DELETE");
    assert_eq!(del.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn enabled_and_run_endpoints() {
    let server = Server::start_with_flows().await;
    server
        .client
        .put(server.url("/api/v1/flows/living"))
        .json(&reference_flow())
        .send()
        .await
        .expect("PUT");

    let post = |path: &'static str, body: Value| {
        let req = server.client.post(server.url(path)).json(&body);
        async move { req.send().await.expect("POST") }
    };

    let res = post("/api/v1/flows/living/enabled", json!({"enabled": false})).await;
    assert_eq!(res.status(), StatusCode::OK);
    let res: Value = res.json().await.expect("json");
    assert_eq!(res["definition"]["enabled"], false);

    let res = post("/api/v1/flows/living/run", json!({})).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let res: Value = res.json().await.expect("json");
    assert!(res["error"].as_str().unwrap().contains("flow is disabled"));

    let res = post("/api/v1/flows/living/enabled", json!({"enabled": true})).await;
    assert_eq!(res.status(), StatusCode::OK);

    // The engine is not started here, so the run stays `running`.
    let res = post("/api/v1/flows/living/run", json!({})).await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let res: Value = res.json().await.expect("json");
    assert_eq!(res["status"], "running");

    let res = post("/api/v1/flows/nope/run", json!({})).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn post_flow_run_starts_from_a_manual_card() {
    let server = Server::start_with_flows().await;
    let mut flow = reference_flow();
    flow["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "id": "test_on", "type": "trigger.manual", "params": { "value": true } }));
    flow["edges"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "from": "test_on", "to": "on" }));
    let res = server
        .client
        .put(server.url("/api/v1/flows/living"))
        .json(&flow)
        .send()
        .await
        .expect("PUT");
    assert_eq!(res.status(), StatusCode::OK);

    let post = |body: Value| {
        let req = server
            .client
            .post(server.url("/api/v1/flows/living/run"))
            .json(&body);
        async move { req.send().await.expect("POST") }
    };

    let res = post(json!({ "node": "motion" })).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let res: Value = res.json().await.expect("json");
    assert!(
        res["error"]
            .as_str()
            .unwrap()
            .contains("not a manual trigger")
    );

    let res = post(json!({ "node": "test_on" })).await;
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let res: Value = res.json().await.expect("json");
    assert_eq!(res["status"], "running");
    assert_eq!(res["trigger"]["value"], json!(true), "{res}");
}

#[tokio::test]
async fn post_flow_run_accepts_an_empty_or_missing_body() {
    let server = Server::start_with_flows().await;
    server
        .client
        .put(server.url("/api/v1/flows/living"))
        .json(&reference_flow())
        .send()
        .await
        .expect("PUT");
    let res = server
        .client
        .post(server.url("/api/v1/flows/living/run"))
        .send()
        .await
        .expect("POST");
    assert_eq!(res.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn post_flow_area_sets_clears_and_rejects() {
    let server = Server::start_with_flows().await;
    server
        .client
        .put(server.url("/api/v1/flows/living"))
        .json(&reference_flow())
        .send()
        .await
        .expect("PUT flow");
    let areas = server
        .client
        .put(server.url("/api/v1/areas"))
        .json(&json!({"areas": [{"id": "living", "name": "Living room"}]}))
        .send()
        .await
        .expect("PUT areas");
    assert_eq!(areas.status(), StatusCode::OK);

    let post = |path: &'static str, body: Value| {
        let req = server.client.post(server.url(path)).json(&body);
        async move { req.send().await.expect("POST") }
    };

    let res = post("/api/v1/flows/living/area", json!({"area": "living"})).await;
    assert_eq!(res.status(), StatusCode::OK);
    let res: Value = res.json().await.expect("json");
    assert_eq!(res["definition"]["area"], "living");

    let res = post("/api/v1/flows/living/area", json!({"area": null})).await;
    assert_eq!(res.status(), StatusCode::OK);
    let res: Value = res.json().await.expect("json");
    assert_eq!(res["definition"]["area"], Value::Null);

    let res = post("/api/v1/flows/living/area", json!({"area": "ghost"})).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let res = post("/api/v1/flows/nope/area", json!({"area": null})).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // A missing (or misspelt) key is refused rather than read as "clear the room".
    let res = post("/api/v1/flows/living/area", json!({"area": "living"})).await;
    assert_eq!(res.status(), StatusCode::OK);
    for body in [json!({}), json!({"aera": null})] {
        let res = post("/api/v1/flows/living/area", body.clone()).await;
        assert!(res.status().is_client_error(), "{body}: {}", res.status());
        let text = res.text().await.expect("text");
        assert!(text.contains("area"), "{text}");
    }
    let flow: Value = server
        .client
        .get(server.url("/api/v1/flows/living"))
        .send()
        .await
        .expect("GET flow")
        .json()
        .await
        .expect("json");
    assert_eq!(flow["definition"]["area"], "living");
}

#[tokio::test]
async fn ws_carries_flows_changed() {
    let server = Server::start_with_flows().await;
    let mut ws = server.ws().await;
    let first = next_json(&mut ws).await;
    assert_eq!(first["type"], "snapshot");
    assert_eq!(first["flows"].as_array().map(Vec::len), Some(0));

    server
        .client
        .put(server.url("/api/v1/flows/living"))
        .json(&reference_flow())
        .send()
        .await
        .expect("PUT");
    let event = next_json(&mut ws).await;
    assert_eq!(event["type"], "flows_changed");
    assert_eq!(event["flows"].as_array().map(Vec::len), Some(1));

    ws.close(None).await.expect("close");
}

#[tokio::test]
async fn schedule_preview_lists_three_ticks() {
    let server = Server::start().await;
    let response = server
        .get("/api/v1/schedule?cron=*/10%20*%20*%20*%20*")
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.expect("json");
    assert!(body["timezone"].as_str().is_some_and(|z| !z.is_empty()));
    let next = body["next"].as_array().expect("next");
    assert_eq!(next.len(), 3);
    for tick in next {
        let tick = tick.as_str().expect("string");
        chrono::DateTime::parse_from_rfc3339(tick).expect("RFC 3339 with offset");
    }
    let capped: Value = server
        .get("/api/v1/schedule?cron=*/10%20*%20*%20*%20*&count=50")
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(capped["next"].as_array().expect("next").len(), 10);
}

#[tokio::test]
async fn schedule_preview_rejects_bad_cron() {
    let server = Server::start().await;
    let response = server.get("/api/v1/schedule?cron=61%20*%20*%20*%20*").await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body: Value = response.json().await.expect("json");
    assert!(body["error"].as_str().is_some_and(|e| !e.is_empty()));
}

#[tokio::test]
async fn http_put_virtual_round_trips() {
    let mut server = Server::start().await;
    server.online();
    let url = server.url("/api/v1/virtual/force_heating");
    let res = server
        .client
        .put(&url)
        .json(&json!({ "kind": "switch", "name": "Force heating" }))
        .send()
        .await
        .expect("PUT");
    assert_eq!(res.status(), StatusCode::OK);
    let view: Value = res.json().await.expect("json");
    assert_eq!(view["id"], "virtual_force_heating");
    assert_eq!(view["kind"], "switch");
    let snapshot: Value = server
        .get("/api/v1/snapshot")
        .await
        .json()
        .await
        .expect("json");
    assert!(
        snapshot["entities"]
            .as_array()
            .expect("entities")
            .iter()
            .any(|e| e["id"] == "virtual_force_heating")
    );

    let bad = server
        .client
        .put(server.url("/api/v1/virtual/Bad"))
        .json(&json!({ "kind": "switch", "name": "x" }))
        .send()
        .await
        .expect("PUT");
    assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);

    while server.publisher.try_recv().is_ok() {}
    let del = server.client.delete(&url).send().await.expect("DELETE");
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
    let topics: Vec<String> = std::iter::from_fn(|| server.publisher.try_recv().ok())
        .map(|p| p.topic)
        .collect();
    assert_eq!(
        topics,
        [
            "nidoo/switch/virtual/force_heating/config",
            "nidoo/virtual/force_heating"
        ]
    );
    let again = server.client.delete(&url).send().await.expect("DELETE");
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn http_put_virtual_offline_is_503() {
    let server = Server::start().await;
    let res = server
        .client
        .put(server.url("/api/v1/virtual/x"))
        .json(&json!({ "kind": "button", "name": "x" }))
        .send()
        .await
        .expect("PUT");
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn history_routes_without_history_are_503() {
    let server = Server::start().await;
    let res = server
        .get("/api/v1/history/series?entity=x&range=24h")
        .await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = res.json().await.expect("json");
    assert_eq!(body, json!({"error": "history_unavailable"}));
}

#[tokio::test]
async fn history_bad_params_are_400() {
    let server = Server::start().await;
    let (history, _rx) = nidoo::history::History::detached(8, chrono_tz::UTC);
    server.hub.set_history(history);

    let res = server.get("/api/v1/history/series?entity=x&range=2d").await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body: Value = res.json().await.expect("json");
    assert_eq!(body, json!({"error": "bad_request"}));

    let res = server.get("/api/v1/history/last_change").await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let res = server
        .get("/api/v1/history/daily?entity=x&days=8&kind=power")
        .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn main_control_patch_rejects_unknown_control() {
    let mut server = Server::start().await;
    server.online();
    server.hub.ingest(
        "nidoo/lock/0x00158d0001a2b3c4/lock/config",
        &fixture("discovery/lock__0x00158d0001a2b3c4__lock.json"),
    );
    let lock_id = "0x00158d0001a2b3c4_lock_zigbee2mqtt";
    let client = server.client.clone();
    let lock_url = server.url(&format!("/api/v1/entities/{lock_id}/override"));
    let patch = |body: Value| client.patch(&lock_url).json(&body).send();

    let response = patch(json!({"mainControl": "warp"})).await.expect("PATCH");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(server.publisher.try_recv().is_err());

    let response = patch(json!({"mainControl": "none"})).await.expect("PATCH");
    assert_eq!(response.status(), StatusCode::OK);
    let response = patch(json!({"mainControl": "locked"}))
        .await
        .expect("PATCH");
    assert_eq!(response.status(), StatusCode::OK);
    let view: Value = response.json().await.expect("json");
    assert_eq!(view["ui"]["mainControl"], "locked");

    let response = patch(json!({"mainControl": "locked"}))
        .await
        .expect("PATCH");
    assert_eq!(response.status(), StatusCode::OK);
    // A switch announces no generic controls, so anything but "none" is refused.
    let response = server
        .client
        .patch(server.url(&format!("/api/v1/entities/{SWITCH_ID}/override")))
        .json(&json!({"mainControl": "locked"}))
        .send()
        .await
        .expect("PATCH");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

const CAMERA_TOPIC: &str = "nidoo/camera/porch/config";
const CAMERA_ID: &str = "porch_camera";

fn camera_config() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "unique_id": CAMERA_ID,
        "name": "Porch",
        "topic": "cam/stream",
    }))
    .expect("json")
}

#[tokio::test]
async fn camera_bytes_are_served_by_the_picture_route() {
    let server = Server::start().await;
    server.hub.ingest(CAMERA_TOPIC, &camera_config());
    server
        .hub
        .ingest_message("cam/stream", &[0xFF, 0xD8, 1, 2, 3], false);

    let response = server
        .get(&format!("/api/v1/entities/{CAMERA_ID}/picture"))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("image/jpeg")
    );
    assert_eq!(
        response
            .headers()
            .get("x-content-type-options")
            .and_then(|v| v.to_str().ok()),
        Some("nosniff")
    );
    assert_eq!(
        response.bytes().await.expect("body").as_ref(),
        &[0xFF, 0xD8, 1, 2, 3]
    );
}

#[tokio::test]
async fn unknown_entity_picture_is_404() {
    let server = Server::start().await;
    let response = server.get("/api/v1/entities/no_such_id/picture").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn oversized_picture_is_dropped() {
    let server = Server::start().await;
    server.hub.ingest(CAMERA_TOPIC, &camera_config());
    let big = vec![0u8; 6 * 1024 * 1024];
    server.hub.ingest_message("cam/stream", &big, false);

    let response = server
        .get(&format!("/api/v1/entities/{CAMERA_ID}/picture"))
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

fn groups_config() -> Config {
    let mut cfg = config();
    cfg.groups.enabled = true;
    cfg
}

#[tokio::test]
async fn http_groups_list_is_empty_then_503_when_off() {
    let server = Server::start_with(groups_config()).await;
    let res = server.get("/api/v1/groups").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.json::<Value>().await.expect("json"), json!([]));

    let off = Server::start().await;
    assert_eq!(
        off.get("/api/v1/groups").await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn http_put_and_delete_group() {
    let server = Server::start_with(groups_config()).await;
    server.online();
    let url = server.url("/api/v1/groups/living");
    let res = server
        .client
        .put(&url)
        .json(&json!({ "kind": "switch", "name": "Living", "members": [] }))
        .send()
        .await
        .expect("PUT");
    assert_eq!(res.status(), StatusCode::OK);
    let view: Value = res.json().await.expect("json");
    assert_eq!(view["id"], "living");
    assert_eq!(view["hideMembers"], true);
    assert_eq!(view["usedByFlows"], json!([]));

    let bad = server
        .client
        .put(server.url("/api/v1/groups/BAD-id"))
        .json(&json!({ "kind": "switch", "name": "x", "members": [] }))
        .send()
        .await
        .expect("PUT");
    assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let missing = server
        .client
        .delete(server.url("/api/v1/groups/ghost"))
        .send()
        .await
        .expect("DELETE");
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

fn lights_off_ambiance(id: &str) -> Value {
    json!({
        "id": id,
        "name": "Night",
        "actions": [{ "type": "category", "what": "lights", "to": "off" }]
    })
}

#[tokio::test]
async fn ambiances_crud_round_trip() {
    let server = Server::start().await;
    server.online();
    let url = server.url("/api/v1/ambiances/night");

    let listed: Value = server
        .get("/api/v1/ambiances")
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(listed, json!([]));

    // The id is omitted: the path supplies it.
    let res = server
        .client
        .put(&url)
        .json(&json!({ "name": "Night", "actions": [] }))
        .send()
        .await
        .expect("PUT");
    assert_eq!(res.status(), StatusCode::OK);
    let view: Value = res.json().await.expect("json");
    assert_eq!(view["id"], "night");
    assert_eq!(view["ambiance"]["id"], "night");

    let listed: Value = server
        .get("/api/v1/ambiances")
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(listed.as_array().map(Vec::len), Some(1));

    let deleted = server.client.delete(&url).send().await.expect("DELETE");
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let again = server.client.delete(&url).send().await.expect("DELETE");
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn put_ambiance_path_body_mismatch_is_422() {
    let server = Server::start().await;
    server.online();
    let res = server
        .client
        .put(server.url("/api/v1/ambiances/night"))
        .json(&lights_off_ambiance("day"))
        .send()
        .await
        .expect("PUT");
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn apply_unknown_ambiance_is_404() {
    let server = Server::start().await;
    server.online();
    let res = server
        .client
        .post(server.url("/api/v1/ambiances/ghost/apply"))
        .send()
        .await
        .expect("POST");
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn apply_returns_report_json() {
    let server = Server::start().await;
    server.online();
    let put = server
        .client
        .put(server.url("/api/v1/ambiances/night"))
        .json(&lights_off_ambiance("night"))
        .send()
        .await
        .expect("PUT");
    assert_eq!(put.status(), StatusCode::OK);

    let res = server
        .client
        .post(server.url("/api/v1/ambiances/night/apply"))
        .send()
        .await
        .expect("POST");
    assert_eq!(res.status(), StatusCode::OK);
    let report: Value = res.json().await.expect("json");
    assert_eq!(report["ambiance"], "night");
    assert!(report["started_at"].is_u64(), "{report}");
    // The fixture house has no light: nothing was attempted.
    assert_eq!(report["actions"][0]["outcome"], "skipped");
}

const DAIKIN_TOPIC: &str = "nidoo/climate/daikin_192_168_1_150/climate/config";
const DAIKIN_FIXTURE: &str = "discovery/climate__daikin_192_168_1_150__climate.json";

/// A server with `[climate] enabled`, the broker link up and one Daikin placed in `living`.
async fn climate_server() -> Server {
    let mut cfg = config();
    cfg.climate.enabled = true;
    let server = Server::start_with(cfg).await;
    server.hub.ingest(DAIKIN_TOPIC, &fixture(DAIKIN_FIXTURE));
    server.online();
    let id = server
        .hub
        .snapshot()
        .entities
        .into_iter()
        .find(|e| e.id.starts_with("daikin_192_168_1_150"))
        .expect("daikin")
        .id;
    let mut patch = serde_json::Map::new();
    patch.insert("area".to_owned(), Value::from("living"));
    server
        .hub
        .set_override(&id, &patch)
        .await
        .expect("area override");
    server
}

#[tokio::test]
async fn climate_disabled_is_503() {
    let server = Server::start().await;
    assert_eq!(
        server.get("/api/v1/climate").await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let res = server
        .client
        .put(server.url("/api/v1/climate/zones/living"))
        .json(&json!({}))
        .send()
        .await
        .expect("PUT");
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = res.json().await.expect("json");
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|e| e.contains("climate disabled"))
    );
}

#[tokio::test]
async fn climate_get_view() {
    let server = climate_server().await;
    let res = server.get("/api/v1/climate").await;
    assert_eq!(res.status(), StatusCode::OK);
    let view: Value = res.json().await.expect("json");
    assert_eq!(view["enabled"], true);
    assert_eq!(view["zones"][0]["zone"], "living");

    let decisions = server
        .get("/api/v1/climate/zones/living/decisions?limit=5")
        .await;
    assert_eq!(decisions.status(), StatusCode::OK);
    let log: Value = decisions.json().await.expect("json");
    assert!(log["recent"].is_array() && log["older"].as_array().is_some_and(Vec::is_empty));
    let missing = server.get("/api/v1/climate/zones/ghost/decisions").await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn climate_put_zone_rejects_invalid() {
    let server = climate_server().await;
    let url = server.url("/api/v1/climate/zones/living");
    let bad = server
        .client
        .put(&url)
        .json(&json!({"manual_hold_max_min": 0}))
        .send()
        .await
        .expect("PUT");
    assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let ok = server
        .client
        .put(&url)
        .json(&json!({"state": "active"}))
        .send()
        .await
        .expect("PUT");
    assert_eq!(ok.status(), StatusCode::NO_CONTENT);
    let unknown = server
        .client
        .put(server.url("/api/v1/climate/zones/ghost"))
        .json(&json!({}))
        .send()
        .await
        .expect("PUT");
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn climate_room_eco_put_and_delete() {
    let server = climate_server().await;
    let url = server.url("/api/v1/climate/zones/living/eco");
    let put = |body: Value| {
        let req = server.client.put(&url).json(&body);
        async move { req.send().await.expect("PUT").status() }
    };
    assert_eq!(
        put(json!({"until": 4_000_000_000u64})).await,
        StatusCode::NO_CONTENT
    );
    let view: Value = server
        .client
        .get(server.url("/api/v1/climate"))
        .send()
        .await
        .expect("GET")
        .json()
        .await
        .expect("json");
    assert_eq!(view["zones"][0]["eco"]["until"], 4_000_000_000u64);
    assert_eq!(
        put(json!({"until": 1})).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        put(json!({"until": 4_000_000_000u64, "x": 1})).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let deleted = server.client.delete(&url).send().await.expect("DELETE");
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn climate_override_minutes_and_until_exclusive() {
    let server = climate_server().await;
    let url = server.url("/api/v1/climate/zones/living/override");
    let post = |body: Value| {
        let req = server.client.post(&url).json(&body);
        async move { req.send().await.expect("POST").status() }
    };
    // A pending zone does not follow overrides: 422, not a silent no-op.
    let pending = server
        .client
        .post(&url)
        .json(&json!({"setpoint": 21.0, "minutes": 30}))
        .send()
        .await
        .expect("POST");
    assert_eq!(pending.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body: Value = pending.json().await.expect("json");
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|e| e.contains("zone is not active")),
        "{body}"
    );
    let active = server
        .client
        .put(server.url("/api/v1/climate/zones/living"))
        .json(&json!({"state": "active"}))
        .send()
        .await
        .expect("PUT");
    assert_eq!(active.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        post(json!({"setpoint": 21.0, "minutes": 30, "until": 4_000_000_000u64})).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(json!({"setpoint": 21.0})).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(json!({"minutes": 30})).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        post(json!({"setpoint": 21.0, "minutes": 30})).await,
        StatusCode::NO_CONTENT
    );
    let view: Value = server
        .get("/api/v1/climate")
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(view["zones"][0]["overrides"][0]["origin"], "user");
    let cleared = server.client.delete(&url).send().await.expect("DELETE");
    assert_eq!(cleared.status(), StatusCode::NO_CONTENT);
    let view: Value = server
        .get("/api/v1/climate")
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(
        view["zones"][0]["overrides"].as_array().map(Vec::len),
        Some(0)
    );
}

#[tokio::test]
async fn climate_approve_without_proposal_is_rejected() {
    let server = climate_server().await;
    for action in ["approve", "reject"] {
        let res = server
            .client
            .post(server.url(&format!("/api/v1/climate/zones/living/proposal/{action}")))
            .send()
            .await
            .expect("POST");
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY, "{action}");
    }
}

#[tokio::test]
async fn climate_approve_body_is_parsed_like_any_json_body() {
    let server = climate_server().await;
    let url = server.url("/api/v1/climate/zones/living/proposal/approve");
    let post = |body: &'static str| {
        let req = server.client.post(&url).body(body);
        async move { req.send().await.expect("POST").status() }
    };
    // The body is read before the proposal is looked up.
    assert_eq!(post("{").await, StatusCode::BAD_REQUEST);
    assert_eq!(
        post(r#"{"heat": 5}"#).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn get_alarm_returns_view() {
    let server = Server::start().await;
    let res = server.get("/api/v1/alarm").await;
    assert_eq!(res.status(), StatusCode::OK);
    let body: Value = res.json().await.expect("json");
    assert_eq!(body["enabled"], false);
    assert_eq!(body["entity_id"], "nidoo_alarm_house");
}

#[tokio::test]
async fn put_alarm_settings_validates() {
    let server = Server::start().await;
    server.online();
    let url = server.url("/api/v1/alarm/settings");
    let bad = server
        .client
        .put(&url)
        .json(&json!({"siren_s": 5}))
        .send()
        .await
        .expect("PUT");
    assert_eq!(bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let ok = server
        .client
        .put(&url)
        .json(&json!({"enabled": false, "siren_s": 120}))
        .send()
        .await
        .expect("PUT");
    assert_eq!(ok.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn http_network_round_trips() {
    let server = Server::start().await;
    server.online();
    let url = server.url("/api/v1/network/nas");
    let res = server
        .client
        .put(&url)
        .json(&json!({ "name": "NAS", "host": "nas.local" }))
        .send()
        .await
        .expect("PUT");
    assert_eq!(res.status(), StatusCode::OK);
    let view: Value = res.json().await.expect("json");
    assert_eq!(view["entity"], "network_nas");

    let list: Value = server
        .get("/api/v1/network")
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(list.as_array().map(Vec::len), Some(1));
    assert_eq!(list[0]["id"], "nas");

    let gone = server.client.delete(&url).send().await.expect("DELETE");
    assert_eq!(gone.status(), StatusCode::NO_CONTENT);
    let again = server.client.delete(&url).send().await.expect("DELETE");
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn http_network_bad_interval_is_422() {
    let server = Server::start().await;
    server.online();
    let res = server
        .client
        .put(server.url("/api/v1/network/nas"))
        .json(&json!({ "name": "NAS", "host": "nas.local", "intervalS": 5 }))
        .send()
        .await
        .expect("PUT");
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
