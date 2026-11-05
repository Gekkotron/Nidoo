//! `POST /api/v1/sound/speak` and `DELETE /api/v1/sound/satellites/{id}` against a hub without a broker and the fake tone synthesiser.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nidoo::assist::tts::tone::ToneLoader;
use nidoo::assist::tts::{Tts, TtsConfig};
use nidoo::config::Config;
use nidoo::http::router;
use nidoo::hub::Hub;
use nidoo_api::ConnectionStatus;
use nidoo_core::command::Publish;
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

struct Rig {
    _dir: tempfile::TempDir,
    addr: SocketAddr,
    client: reqwest::Client,
    rx: mpsc::Receiver<Publish>,
    hub: Arc<Hub>,
}

impl Rig {
    fn url(&self, path: &str) -> String {
        format!("http://{}/api/v1{path}", self.addr)
    }

    async fn send(&self, m: reqwest::Method, path: &str, body: Value) -> (StatusCode, Value) {
        let r = self
            .client
            .request(m, self.url(path))
            .json(&body)
            .send()
            .await
            .unwrap();
        (r.status(), r.json().await.unwrap_or(Value::Null))
    }

    async fn speak(&self, body: Value) -> (StatusCode, Value) {
        self.send(reqwest::Method::POST, "/sound/speak", body).await
    }

    /// Uploads a model speaking French and makes it the house language.
    async fn make_tts_ready(&self) {
        let (_, v) = self
            .send(reqwest::Method::POST, "/assist/tts/uploads", json!({}))
            .await;
        let id = v["id"].as_str().unwrap().to_owned();
        let files: [(&str, &[u8]); 3] = [
            ("config.json", br#"{"languages":["fr"],"model_id":"M"}"#),
            ("tokenizer.json", b"{}"),
            ("model.q8.gguf", b"weights"),
        ];
        for (name, body) in files {
            let r = self
                .client
                .put(self.url(&format!("/assist/tts/uploads/{id}/files/{name}")))
                .body(body.to_vec())
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::NO_CONTENT);
        }
        let (s, m) = self
            .send(
                reqwest::Method::POST,
                &format!("/assist/tts/uploads/{id}/finish"),
                json!({}),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{m}");
        let (s, _) = self
            .send(
                reqwest::Method::PUT,
                "/assist/tts/languages/fr",
                json!({"model": m["id"]}),
            )
            .await;
        assert_eq!(s, StatusCode::OK);
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let st: Value = self
                .client
                .get(self.url("/assist/tts/status"))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if st["languages"][0]["engine"]["state"] == "ready" {
                return;
            }
            assert!(Instant::now() < end, "never ready: {st}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// The messages published so far on `topic`.
    fn published(&mut self, topic: &str) -> Vec<Publish> {
        let mut out = Vec::new();
        while let Ok(p) = self.rx.try_recv() {
            if p.topic == topic {
                out.push(p);
            }
        }
        out
    }
}

/// A satellite as `nidoo-sound` announces it, allowed and connected, in `area`.
fn satellite(hub: &Hub, sid: &str, area: &str) {
    let device = format!("nidoo_sound_{sid}");
    for (component, object) in [
        ("number", "volume"),
        ("switch", "mute"),
        ("switch", "allowed"),
        ("binary_sensor", "connected"),
    ] {
        let mut config = json!({
            "name": object,
            "unique_id": format!("{device}_{object}"),
            "state_topic": format!("nidoo-sound/{sid}/{object}"),
            "payload_on": "ON",
            "payload_off": "OFF",
            "availability_topic": "nidoo-sound/status",
            "device": { "identifiers": [device], "name": sid },
        });
        if component != "binary_sensor" {
            config["command_topic"] = json!(format!("nidoo-sound/{sid}/{object}/set"));
        }
        hub.ingest(
            &format!("nidoo/{component}/{device}/{object}/config"),
            config.to_string().as_bytes(),
        );
        hub.ingest(
            &format!("nidoo/ui/{device}_{object}"),
            json!({ "area": area }).to_string().as_bytes(),
        );
    }
    hub.ingest(&format!("nidoo-sound/{sid}/allowed"), b"ON");
    hub.ingest(&format!("nidoo-sound/{sid}/connected"), b"ON");
}

/// A hub online with satellites `a` (kitchen) and `b` (office); speech set up when `tts`.
async fn rig(tts: bool) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let (tx, rx) = mpsc::channel(256);
    let cfg = Config::default();
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    hub.ingest(
        "nidoo/ui/_areas",
        br#"{"areas":[{"id":"kitchen","name":"Kitchen"},{"id":"office","name":"Office"}]}"#,
    );
    satellite(&hub, "a", "kitchen");
    satellite(&hub, "b", "office");
    if tts {
        hub.set_tts(Tts::start(TtsConfig {
            root: dir.path().join("tts"),
            hf_base: "http://127.0.0.1:1".to_owned(),
            loader: Arc::new(ToneLoader),
        }));
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(
        axum::serve(
            listener,
            router(hub.clone(), nidoo::setup::SetupCtx::inert()),
        )
        .into_future(),
    );
    let rig = Rig {
        _dir: dir,
        addr,
        client: reqwest::Client::new(),
        rx,
        hub,
    };
    if tts {
        rig.make_tts_ready().await;
    }
    rig
}

#[tokio::test]
async fn speak_route_rejects_room_and_satellite_together() {
    let r = rig(false).await;
    let (s, v) = r
        .speak(json!({"text": "hello", "room": "kitchen", "satellite": "a"}))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(v["error"].is_string(), "{v}");
}

#[tokio::test]
async fn speak_route_without_tts_is_409() {
    let r = rig(false).await;
    let (s, v) = r.speak(json!({"text": "hello"})).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"], "speech is not set up on this hub");
}

#[tokio::test]
async fn speak_route_unknown_room_is_422() {
    let r = rig(true).await;
    let (s, v) = r.speak(json!({"text": "hello", "room": "attic"})).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}

#[tokio::test]
async fn speak_route_no_satellite_is_409() {
    let r = rig(true).await;
    let (s, v) = r.speak(json!({"text": "hello", "satellite": "zzz"})).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"], "this satellite cannot speak now");
}

#[tokio::test]
async fn speak_route_empty_text_is_422() {
    let r = rig(true).await;
    let (s, v) = r.speak(json!({"text": "  "})).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}

#[tokio::test]
async fn speak_route_publishes_the_speak_message() {
    let mut r = rig(true).await;
    let (s, v) = r.speak(json!({"text": "bonjour", "room": "Kitchen"})).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    assert_eq!(v, json!({"players": 1}));
    let sent = r.published("nidoo-sound/speak");
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(!sent[0].retain);
    let payload: Value = serde_json::from_str(&sent[0].payload).unwrap();
    assert_eq!(payload["players"], json!(["a"]));
    let id = payload["id"].as_str().unwrap();
    assert_eq!(
        payload["url"].as_str().unwrap(),
        format!("/api/v1/assist/speech/{id}.wav")
    );

    let (s, v) = r.speak(json!({"text": "bonjour"})).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    assert_eq!(v, json!({"players": 2}));
    let sent = r.published("nidoo-sound/speak");
    let payload: Value = serde_json::from_str(&sent[0].payload).unwrap();
    assert_eq!(payload["players"], json!(["a", "b"]));
}

#[tokio::test]
async fn remove_route_asks_the_sound_service_and_clears_the_rooms() {
    let mut r = rig(false).await;
    r.published("");
    let (s, v) = r
        .send(reqwest::Method::DELETE, "/sound/satellites/a", Value::Null)
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let all: Vec<Publish> = std::iter::from_fn(|| r.rx.try_recv().ok()).collect();
    let sent: Vec<&Publish> = all
        .iter()
        .filter(|p| p.topic == "nidoo-sound/a/remove/set")
        .collect();
    assert_eq!(sent.len(), 1, "{all:?}");
    assert!(!sent[0].retain);
    // Its rooms go too: a board that comes back starts like a first install.
    let mut cleared = Vec::new();
    for p in &all {
        if p.topic.starts_with("nidoo/ui/nidoo_sound_") {
            assert!(p.retain && p.payload.is_empty(), "{p:?}");
            cleared.push(p.topic.clone());
        }
    }
    cleared.sort();
    assert_eq!(
        cleared,
        [
            "nidoo/ui/nidoo_sound_a_allowed",
            "nidoo/ui/nidoo_sound_a_connected",
            "nidoo/ui/nidoo_sound_a_mute",
            "nidoo/ui/nidoo_sound_a_volume",
        ]
    );
}

#[tokio::test]
async fn remove_route_unknown_satellite_is_404() {
    let mut r = rig(false).await;
    let (s, v) = r
        .send(
            reqwest::Method::DELETE,
            "/sound/satellites/zzz",
            Value::Null,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    assert!(r.published("nidoo-sound/zzz/remove/set").is_empty());
}

#[tokio::test]
async fn remove_route_needs_the_sound_service() {
    let mut r = rig(false).await;
    r.hub.ingest("nidoo-sound/status", b"offline");
    let (s, v) = r
        .send(reqwest::Method::DELETE, "/sound/satellites/a", Value::Null)
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"], "the sound service is offline");
    assert!(r.published("nidoo-sound/a/remove/set").is_empty());
}
