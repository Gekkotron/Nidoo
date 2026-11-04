//! The speech management routes over HTTP, against a fake Hugging Face server and a fake
//! synthesiser.

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::fake_hf::{FakeRepo, serve};
use futures_util::stream;
use nidoo::assist::tts::engine::NoSpeechLoader;
use nidoo::assist::tts::store::MAX_WEIGHTS_BYTES;
use nidoo::assist::tts::tone::ToneLoader;
use nidoo::assist::tts::voices::{VOICE_CATALOG_REPO, VOICE_CATALOG_REV};
use nidoo::assist::tts::{Tts, TtsConfig};
use nidoo::config::Config;
use nidoo::http::router;
use nidoo::hub::Hub;
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

const REPO: &str = "gradium/phonon-test";

const VOICES_JSONL: &str = concat!(
    "{\"filename\":\"voices/AAA.safetensors\",\"name\":\"Noemie\",\"voice_id\":\"AAA\",\"language\":\"fr\"}\n",
    "{\"filename\":\"voices/BBB.safetensors\",\"name\":\"Bob\",\"voice_id\":\"BBB\",\"language\":\"en\"}\n",
);

fn voice_catalog() -> FakeRepo {
    FakeRepo::new(
        VOICE_CATALOG_REPO,
        VOICE_CATALOG_REV,
        vec![
            ("voices.jsonl", VOICES_JSONL.as_bytes()),
            ("voices/AAA.safetensors", b"voicebytes"),
        ],
    )
}

fn repo() -> FakeRepo {
    FakeRepo::new(
        REPO,
        "c0ffee",
        vec![
            ("model.q8.gguf", b"0123456789abcdef"),
            (
                "config.json",
                b"{\"languages\":[\"fr\"],\"model_id\":\"Phonon Test\"}",
            ),
            ("tokenizer.json", b"{}"),
        ],
    )
}

struct Rig {
    dir: tempfile::TempDir,
    addr: SocketAddr,
    client: reqwest::Client,
    base: String,
}

impl Rig {
    fn url(&self, path: &str) -> String {
        format!("http://{}/api/v1/assist/tts{path}", self.addr)
    }

    fn root(&self) -> std::path::PathBuf {
        self.dir.path().join("tts")
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        let r = self.client.get(self.url(path)).send().await.unwrap();
        (r.status(), r.json().await.unwrap_or(Value::Null))
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

    async fn status_until(&self, engine_state: &str) -> Value {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let (_, s) = self.get("/status").await;
            if s["engine"]["state"] == engine_state {
                return s;
            }
            assert!(Instant::now() < end, "never reached {engine_state}: {s}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn install(&self) -> String {
        let (s, v) = self
            .send(reqwest::Method::POST, "/inspect", json!({"url": REPO}))
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let (s, v) = self
            .send(
                reqwest::Method::POST,
                "/downloads",
                json!({"repo": REPO, "revision": v["revision"], "weights": "model.q8.gguf"}),
            )
            .await;
        assert_eq!(s, StatusCode::ACCEPTED, "{v}");
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let (_, d) = self.get("/downloads/current").await;
            if d["state"] == "done" {
                return d["model"].as_str().unwrap().to_owned();
            }
            assert!(d["state"] == "running" && Instant::now() < end, "{d}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

async fn start_tts(root: std::path::PathBuf, base: &str) -> Arc<Tts> {
    Tts::start(TtsConfig {
        root,
        hf_base: base.to_owned(),
        loader: Arc::new(ToneLoader),
    })
}

async fn rig_with(with_tts: bool, prepare: impl FnOnce(&std::path::Path)) -> Rig {
    let (base, _log) = serve(vec![repo(), voice_catalog()]).await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("tts");
    prepare(&root);
    let (tx, _rx) = mpsc::channel(8);
    let mut cfg = Config::default();
    cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
    let hub = Hub::new(&cfg, tx);
    if with_tts {
        hub.set_tts(start_tts(root, &base).await);
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, router(hub, nidoo::setup::SetupCtx::inert())).into_future());
    Rig {
        dir,
        addr,
        client: reqwest::Client::new(),
        base,
    }
}

async fn rig() -> Rig {
    rig_with(true, |_| {}).await
}

#[tokio::test]
async fn status_with_nothing_installed() {
    let r = rig().await;
    let (s, v) = r.get("/status").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["engine"]["state"], "no_model");
    assert_eq!(v["token_set"], false);
    assert_eq!(v["voice"], "default");
}

#[tokio::test]
async fn download_then_activate_then_ready() {
    let r = rig().await;
    let id = r.install().await;
    let (_, models) = r.get("/models").await;
    assert_eq!(models.as_array().unwrap().len(), 1);
    assert_eq!(models[0]["id"], id);
    assert_eq!(models[0]["french"], true);
    let (s, _) = r
        .send(reqwest::Method::PUT, "/active", json!({"model": id}))
        .await;
    assert_eq!(s, StatusCode::OK);
    let st = r.status_until("ready").await;
    assert_eq!(st["active"], id);
}

#[tokio::test]
async fn state_survives_a_restart() {
    let r = rig().await;
    let id = r.install().await;
    r.send(reqwest::Method::PUT, "/active", json!({"model": id}))
        .await;
    r.status_until("ready").await;
    let again = start_tts(r.root(), &r.base).await;
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let st = again.status();
        if matches!(&st.engine, nidoo_api::EngineState::Ready { model } if *model == id) {
            assert_eq!(st.active.as_deref(), Some(id.as_str()));
            assert_eq!(st.voice, "default");
            break;
        }
        assert!(Instant::now() < end, "not ready: {st:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn remove_active_unloads() {
    let r = rig().await;
    let id = r.install().await;
    r.send(reqwest::Method::PUT, "/active", json!({"model": id}))
        .await;
    r.status_until("ready").await;
    let resp = r
        .client
        .delete(r.url(&format!("/models/{id}")))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    let (_, st) = r.get("/status").await;
    assert_eq!(st["engine"]["state"], "no_model");
    assert!(st["active"].is_null());
    let (_, models) = r.get("/models").await;
    assert!(models.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn activate_unknown_is_404() {
    let r = rig().await;
    let (s, _) = r
        .send(reqwest::Method::PUT, "/active", json!({"model": "nope"}))
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn token_is_write_only() {
    let r = rig().await;
    let mut bodies = Vec::new();
    let resp = r
        .client
        .put(r.url("/token"))
        .json(&json!({"token": "hf_test"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    bodies.push(resp.text().await.unwrap());
    for path in ["/status", "/models", "/voices"] {
        let resp = r.client.get(r.url(path)).send().await.unwrap();
        bodies.push(resp.text().await.unwrap());
    }
    let (_, st) = r.get("/status").await;
    assert_eq!(st["token_set"], true);
    // A failing call must not leak it either.
    let resp = r
        .client
        .post(r.url("/inspect"))
        .json(&json!({"url": "https://example.com/a/b"}))
        .send()
        .await
        .unwrap();
    bodies.push(resp.text().await.unwrap());
    for b in bodies {
        assert!(!b.contains("hf_test"), "token leaked in: {b}");
    }
    let resp = r
        .client
        .put(r.url("/token"))
        .json(&json!({"token": null}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    let (_, st) = r.get("/status").await;
    assert_eq!(st["token_set"], false);
}

fn model_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        (
            "config.json",
            br#"{"languages":["fr"],"model_id":"My Model"}"#.to_vec(),
        ),
        ("tokenizer.json", b"{}".to_vec()),
        ("model.q8.gguf", b"weights".to_vec()),
    ]
}

async fn put_file(r: &Rig, id: &str, name: &str, body: Vec<u8>) -> StatusCode {
    r.client
        .put(r.url(&format!("/uploads/{id}/files/{name}")))
        .body(body)
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn upload_round_trip() {
    let r = rig().await;
    let (s, v) = r.send(reqwest::Method::POST, "/uploads", json!({})).await;
    assert_eq!(s, StatusCode::OK);
    let id = v["id"].as_str().unwrap().to_owned();
    assert_eq!(id.len(), 32);
    for (name, body) in model_files() {
        assert_eq!(put_file(&r, &id, name, body).await, StatusCode::NO_CONTENT);
    }
    let (s, m) = r
        .send(
            reqwest::Method::POST,
            &format!("/uploads/{id}/finish"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{m}");
    assert_eq!(m["source"], "upload");
    assert_eq!(m["id"], "upload--my-model--q8");
    let (_, models) = r.get("/models").await;
    assert_eq!(models[0]["source"], "upload");
}

fn staging_dirs(r: &Rig) -> usize {
    std::fs::read_dir(r.root().join("models"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(".staging-"))
        .count()
}

#[tokio::test]
async fn upload_refuses_unknown_names_and_oversize() {
    let r = rig().await;
    let (_, v) = r.send(reqwest::Method::POST, "/uploads", json!({})).await;
    let id = v["id"].as_str().unwrap().to_owned();
    assert_eq!(
        put_file(&r, &id, "evil.sh", b"x".to_vec()).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(staging_dirs(&r), 1);
    // A weight stream over the cap, never held whole.
    let chunks = (MAX_WEIGHTS_BYTES / (1 << 20)) + 2;
    let body =
        reqwest::Body::wrap_stream(stream::iter((0..chunks).map(|_| {
            Ok::<_, std::convert::Infallible>(bytes::Bytes::from(vec![0u8; 1 << 20]))
        })));
    let status = r
        .client
        .put(r.url(&format!("/uploads/{id}/files/model.q8.gguf")))
        .body(body)
        .send()
        .await
        .map(|resp| resp.status());
    assert_eq!(status.unwrap(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(staging_dirs(&r), 0);
}

#[tokio::test]
async fn upload_finish_without_config_is_422() {
    let r = rig().await;
    let (_, v) = r.send(reqwest::Method::POST, "/uploads", json!({})).await;
    let id = v["id"].as_str().unwrap().to_owned();
    assert_eq!(
        put_file(&r, &id, "model.q8.gguf", b"w".to_vec()).await,
        StatusCode::NO_CONTENT
    );
    let (s, _) = r
        .send(
            reqwest::Method::POST,
            &format!("/uploads/{id}/finish"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(staging_dirs(&r), 0);
}

#[tokio::test]
async fn other_routes_keep_the_default_body_limit() {
    let r = rig().await;
    let big = json!({"url": "a".repeat(3 * 1024 * 1024)});
    // The server answers from the headers and may close while the body is still going out,
    // so a send that fails is retried.
    for _ in 0..5 {
        if let Ok(resp) = r.client.post(r.url("/inspect")).json(&big).send().await {
            assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
            return;
        }
    }
    panic!("the server never answered");
}

#[tokio::test]
async fn hub_without_tts_answers_409() {
    let r = rig_with(false, |_| {}).await;
    let (s, v) = r.get("/status").await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(v["error"], "speech is not set up on this hub");
}

#[tokio::test]
async fn set_voice_rules() {
    let r = rig().await;
    let (s, _) = r
        .send(reqwest::Method::PUT, "/voice", json!({"voice": "someone"}))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, v) = r
        .send(reqwest::Method::PUT, "/voice", json!({"voice": "default"}))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let id = r.install().await;
    r.send(reqwest::Method::PUT, "/active", json!({"model": id}))
        .await;
    r.status_until("ready").await;
    let (s, _) = r
        .send(reqwest::Method::PUT, "/voice", json!({"voice": "nobody"}))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn boot_with_a_broken_active_model_still_serves() {
    let r = rig_with(true, |root| {
        let dir = root.join("models").join("broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fail"), b"").unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            json!({
                "id": "broken", "repo": null, "revision": null, "weights": "model.q8.gguf",
                "files": [], "licence": null, "languages": ["fr"], "source": "upload",
                "installed_at": 1
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(root.join("state.json"), r#"{"active":"broken"}"#).unwrap();
    })
    .await;
    let st = r.status_until("error").await;
    assert_eq!(st["active"], "broken");
    let resp = r
        .client
        .post(format!("http://{}/api/v1/assist", r.addr))
        .json(&json!({"client": "page-1", "text": "allume le salon", "simulate": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_build_without_speech_still_answers() {
    let (base, _) = serve(vec![]).await;
    let dir = tempfile::tempdir().unwrap();
    let tts = Tts::start(TtsConfig {
        root: dir.path().join("tts"),
        hf_base: base,
        loader: Arc::new(NoSpeechLoader),
    });
    assert_eq!(tts.status().voice, "default");
}

async fn open_upload(r: &Rig) -> (StatusCode, Value) {
    r.send(reqwest::Method::POST, "/uploads", json!({})).await
}

#[tokio::test]
async fn a_disconnect_mid_body_frees_the_next_upload() {
    use tokio::io::AsyncWriteExt;
    let r = rig().await;
    let (_, v) = open_upload(&r).await;
    let id = v["id"].as_str().unwrap().to_owned();
    let mut sock = tokio::net::TcpStream::connect(r.addr).await.unwrap();
    let head = format!(
        "PUT /api/v1/assist/tts/uploads/{id}/files/model.q8.gguf HTTP/1.1\r\nHost: x\r\nContent-Length: 1000000\r\n\r\n"
    );
    sock.write_all(head.as_bytes()).await.unwrap();
    sock.write_all(&[0u8; 1000]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    // While the write runs, a second open is refused.
    assert_eq!(open_upload(&r).await.0, StatusCode::CONFLICT);
    drop(sock);
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let (s, v) = open_upload(&r).await;
        if s == StatusCode::OK {
            assert_ne!(v["id"], id.as_str());
            break;
        }
        assert!(Instant::now() < end, "still blocked: {s}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(staging_dirs(&r), 1);
}

#[tokio::test]
async fn an_abandoned_upload_is_replaced_by_the_next() {
    let r = rig().await;
    let (_, v) = open_upload(&r).await;
    let old = v["id"].as_str().unwrap().to_owned();
    assert_eq!(
        put_file(&r, &old, "tokenizer.json", b"{}".to_vec()).await,
        StatusCode::NO_CONTENT
    );
    let (s, v) = open_upload(&r).await;
    assert_eq!(s, StatusCode::OK);
    assert_ne!(v["id"], old.as_str());
    assert_eq!(staging_dirs(&r), 1);
    assert_eq!(
        put_file(&r, &old, "config.json", b"{}".to_vec()).await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn aborting_an_upload_removes_its_files_and_ends_the_session() {
    let r = rig().await;
    let (_, v) = open_upload(&r).await;
    let id = v["id"].as_str().unwrap().to_owned();
    assert_eq!(
        put_file(&r, &id, "tokenizer.json", b"{}".to_vec()).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(staging_dirs(&r), 1);
    let resp = r
        .client
        .delete(r.url(&format!("/uploads/{id}")))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(staging_dirs(&r), 0, "the staging folder is gone");
    let (s, _) = r
        .send(
            reqwest::Method::POST,
            &format!("/uploads/{id}/finish"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_catalog_voice_can_be_chosen_end_to_end() {
    let r = rig().await;
    let id = r.install().await;
    r.send(reqwest::Method::PUT, "/active", json!({"model": id}))
        .await;
    r.status_until("ready").await;
    let (_, before) = r.get("/voices").await;
    let noemie = before["voices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "Noemie")
        .expect("the French catalog voice is offered");
    assert_eq!(noemie["on_disk"], false);
    assert!(
        before["voices"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["name"] != "Bob"),
        "a voice in another language is not offered"
    );
    let (s, v) = r
        .send(reqwest::Method::PUT, "/voice", json!({"voice": "Noemie"}))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, voices) = r.get("/voices").await;
    assert_eq!(voices["chosen"], "Noemie");
    assert_eq!(voices["fell_back"], false);
    let chosen = voices["voices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "Noemie")
        .unwrap();
    assert_eq!(chosen["on_disk"], true);
    let (_, status) = r.get("/status").await;
    assert_eq!(status["voice"], "Noemie");
    assert!(r.root().join("voices/AAA.safetensors").is_file());
}

#[tokio::test]
async fn a_catalog_voice_can_be_heard_without_choosing_it() {
    let r = rig().await;
    let id = r.install().await;
    r.send(reqwest::Method::PUT, "/active", json!({"model": id}))
        .await;
    r.status_until("ready").await;
    let speech = format!("http://{}/api/v1/assist/speech", r.addr);
    let post = |body: Value| {
        let (client, speech) = (r.client.clone(), speech.clone());
        async move {
            let resp = client.post(speech).json(&body).send().await.unwrap();
            (
                resp.status(),
                resp.json::<Value>().await.unwrap_or(Value::Null),
            )
        }
    };
    let (s, v) = post(json!({"text": "Bonjour", "voice": "Noemie"})).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert!(r.root().join("voices/AAA.safetensors").is_file());
    let (_, voices) = r.get("/voices").await;
    assert_eq!(
        voices["chosen"], "default",
        "hearing a voice does not choose it"
    );
    let (s, _) = post(json!({"text": "Bonjour", "voice": "nobody"})).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}
