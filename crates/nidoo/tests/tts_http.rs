//! The speech management routes over HTTP, against a fake Hugging Face server and a fake
//! synthesiser.

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::fake_hf::{FakeRepo, serve};
use futures_util::stream;
use nidoo::assist::tts::engine::{NoSpeechLoader, SpeakerLoader};
use nidoo::assist::tts::store::MAX_WEIGHTS_BYTES;
use nidoo::assist::tts::tone::{RecordingLoader, ToneLoader};
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
const BILINGUAL: &str = "gradium/phonon-bilingual";

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
            ("voices/BBB.safetensors", b"voicebytes"),
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

fn repo_bilingual() -> FakeRepo {
    FakeRepo::new(
        BILINGUAL,
        "b111",
        vec![
            ("model.q8.gguf", b"0123456789abcdef"),
            (
                "config.json",
                b"{\"languages\":[\"fr\",\"en\"],\"model_id\":\"Phonon Two\"}",
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

    /// Waits until the house language's engine reaches `engine_state`.
    async fn status_until(&self, engine_state: &str) -> Value {
        self.language_until(0, engine_state).await
    }

    /// Waits until the language at `index` reaches `engine_state`.
    async fn language_until(&self, index: usize, engine_state: &str) -> Value {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let (_, s) = self.get("/status").await;
            if s["languages"][index]["engine"]["state"] == engine_state {
                return s;
            }
            assert!(Instant::now() < end, "never reached {engine_state}: {s}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Speaks French with `model` (adds or changes the `fr` language).
    async fn activate(&self, model: &str) -> (StatusCode, Value) {
        self.set_language("fr", json!({"model": model})).await
    }

    async fn set_language(&self, code: &str, body: Value) -> (StatusCode, Value) {
        self.send(reqwest::Method::PUT, &format!("/languages/{code}"), body)
            .await
    }

    async fn delete(&self, path: &str) -> StatusCode {
        self.client
            .delete(self.url(path))
            .send()
            .await
            .unwrap()
            .status()
    }

    async fn install(&self) -> String {
        self.install_repo(REPO).await
    }

    async fn install_repo(&self, repo: &str) -> String {
        let (s, v) = self
            .send(reqwest::Method::POST, "/inspect", json!({"url": repo}))
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let (s, v) = self
            .send(
                reqwest::Method::POST,
                "/downloads",
                json!({"repo": repo, "revision": v["revision"], "weights": "model.q8.gguf"}),
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
    start_tts_with(root, base, Arc::new(ToneLoader))
}

fn start_tts_with(
    root: std::path::PathBuf,
    base: &str,
    loader: Arc<dyn SpeakerLoader>,
) -> Arc<Tts> {
    Tts::start(TtsConfig {
        root,
        hf_base: base.to_owned(),
        loader,
    })
}

async fn rig_with(with_tts: bool, prepare: impl FnOnce(&std::path::Path)) -> Rig {
    rig_full(with_tts, prepare, Arc::new(ToneLoader)).await
}

async fn rig_full(
    with_tts: bool,
    prepare: impl FnOnce(&std::path::Path),
    loader: Arc<dyn SpeakerLoader>,
) -> Rig {
    let (base, _log) = serve(vec![repo(), repo_bilingual(), voice_catalog()]).await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("tts");
    prepare(&root);
    let (tx, _rx) = mpsc::channel(8);
    let mut cfg = Config::default();
    cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
    let hub = Hub::new(&cfg, tx);
    if with_tts {
        hub.set_tts(start_tts_with(root, &base, loader));
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
    assert_eq!(v["languages"], json!([]));
    assert_eq!(v["token_set"], false);
}

#[tokio::test]
async fn download_then_activate_then_ready() {
    let r = rig().await;
    let id = r.install().await;
    let (_, models) = r.get("/models").await;
    assert_eq!(models.as_array().unwrap().len(), 1);
    assert_eq!(models[0]["id"], id);
    assert_eq!(models[0]["languages"], json!(["fr"]));
    let (s, v) = r.activate(&id).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let st = r.status_until("ready").await;
    assert_eq!(st["languages"][0]["model"], id);
    let (_, models) = r.get("/models").await;
    assert_eq!(models[0]["used_by"], json!(["fr"]));
}

#[tokio::test]
async fn state_survives_a_restart() {
    let r = rig().await;
    let id = r.install().await;
    r.activate(&id).await;
    r.status_until("ready").await;
    let again = start_tts(r.root(), &r.base).await;
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let st = again.status();
        let first = st.languages.first();
        if first.is_some_and(
            |l| matches!(&l.engine, nidoo_api::EngineState::Ready { model } if *model == id),
        ) {
            let first = first.unwrap();
            assert_eq!(first.code, "fr");
            assert_eq!(first.voice, "default");
            break;
        }
        assert!(Instant::now() < end, "not ready: {st:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn removing_a_used_model_leaves_languages_without_one() {
    let r = rig().await;
    let id = r.install().await;
    r.activate(&id).await;
    r.status_until("ready").await;
    let resp = r
        .client
        .delete(r.url(&format!("/models/{id}")))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    let (_, st) = r.get("/status").await;
    assert_eq!(st["languages"][0]["code"], "fr");
    assert!(st["languages"][0]["model"].is_null());
    assert_eq!(st["languages"][0]["engine"]["state"], "no_model");
    let (_, models) = r.get("/models").await;
    assert!(models.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn activate_unknown_is_404() {
    let r = rig().await;
    let (s, _) = r.activate("nope").await;
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
    let id = r.install().await;
    let (s, v) = r
        .set_language("fr", json!({"model": id, "voice": "default"}))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    r.status_until("ready").await;
    let (s, _) = r
        .set_language("fr", json!({"model": id, "voice": "nobody"}))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, st) = r.get("/status").await;
    assert_eq!(
        st["languages"][0]["voice"], "default",
        "a refused voice changes nothing"
    );
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
    assert_eq!(st["languages"][0]["model"], "broken");
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
    assert!(tts.status().languages.is_empty());
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
    r.activate(&id).await;
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
        .set_language("fr", json!({"model": id, "voice": "Noemie"}))
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
    assert_eq!(status["languages"][0]["voice"], "Noemie");
    assert!(r.root().join("voices/AAA.safetensors").is_file());
}

#[tokio::test]
async fn a_catalog_voice_can_be_heard_without_choosing_it() {
    let r = rig().await;
    let id = r.install().await;
    r.activate(&id).await;
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

#[tokio::test]
async fn languages_start_empty_and_speech_is_409() {
    let r = rig().await;
    let (_, st) = r.get("/status").await;
    assert_eq!(st["languages"], json!([]));
    let resp = r
        .client
        .post(format!("http://{}/api/v1/assist/speech", r.addr))
        .json(&json!({"text": "Bonjour"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn add_two_languages_on_one_model_loads_one_engine() {
    let r = rig().await;
    let id = r.install_repo(BILINGUAL).await;
    for code in ["fr", "en"] {
        let (s, v) = r.set_language(code, json!({"model": id})).await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    r.language_until(0, "ready").await;
    let st = r.language_until(1, "ready").await;
    assert_eq!(st["languages"][0]["model"], id);
    assert_eq!(st["languages"][1]["model"], id);
    let (_, models) = r.get("/models").await;
    assert_eq!(models[0]["used_by"], json!(["fr", "en"]));
}

#[tokio::test]
async fn switching_one_of_two_shared_languages_keeps_the_engine() {
    let r = rig().await;
    let both = r.install_repo(BILINGUAL).await;
    let fr_only = r.install().await;
    r.set_language("fr", json!({"model": both})).await;
    r.set_language("en", json!({"model": both, "voice": "Bob"}))
        .await;
    r.language_until(1, "ready").await;
    let (s, v) = r.set_language("en", json!({"model": fr_only})).await;
    // The French-only model does not speak English.
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    let (s, v) = r.set_language("fr", json!({"model": fr_only})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let st = r.language_until(0, "ready").await;
    assert_eq!(st["languages"][0]["model"], fr_only);
    assert_eq!(st["languages"][1]["model"], both);
    assert_eq!(st["languages"][1]["engine"]["state"], "ready");
    assert_eq!(st["languages"][1]["voice"], "Bob");
}

#[tokio::test]
async fn a_model_that_does_not_speak_the_code_is_422() {
    let r = rig().await;
    let id = r.install().await;
    let (s, _) = r.set_language("de", json!({"model": id})).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _) = r.set_language("français", json!({"model": id})).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, v) = r.set_language("FR", json!({"model": id})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["languages"][0]["code"], "fr");
}

#[tokio::test]
async fn house_moves_first_and_survives_restart() {
    let r = rig().await;
    let id = r.install_repo(BILINGUAL).await;
    r.set_language("fr", json!({"model": id})).await;
    r.set_language("en", json!({"model": id})).await;
    let (s, v) = r
        .send(
            reqwest::Method::PUT,
            "/languages/house",
            json!({"code": "en"}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["languages"][0]["code"], "en");
    let (s, _) = r
        .send(
            reqwest::Method::PUT,
            "/languages/house",
            json!({"code": "de"}),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let again = start_tts(r.root(), &r.base).await;
    let codes: Vec<_> = again
        .status()
        .languages
        .into_iter()
        .map(|l| l.code)
        .collect();
    assert_eq!(codes, ["en", "fr"]);
}

#[tokio::test]
async fn removing_a_language_unloads_its_model_when_unused() {
    let r = rig().await;
    let id = r.install_repo(BILINGUAL).await;
    r.set_language("fr", json!({"model": id})).await;
    r.set_language("en", json!({"model": id})).await;
    r.language_until(1, "ready").await;
    assert_eq!(r.delete("/languages/fr").await, StatusCode::OK);
    let (_, st) = r.get("/status").await;
    assert_eq!(
        st["languages"][0]["code"], "en",
        "the next one is the house language"
    );
    assert_eq!(st["languages"][0]["engine"]["state"], "ready");
    assert_eq!(r.delete("/languages/en").await, StatusCode::OK);
    assert_eq!(r.delete("/languages/en").await, StatusCode::NOT_FOUND);
    let (_, models) = r.get("/models").await;
    assert_eq!(models[0]["used_by"], json!([]));
}

#[tokio::test]
async fn old_state_boots_as_one_language_with_its_voice() {
    // A hub from before languages: one active model and a chosen catalog voice.
    let r = rig().await;
    let id = r.install().await;
    std::fs::write(
        r.root().join("state.json"),
        json!({"active": id, "voice": "Noemie"}).to_string(),
    )
    .unwrap();
    let again = start_tts(r.root(), &r.base).await;
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let st = again.status();
        assert_eq!(st.languages.len(), 1, "{st:?}");
        assert_eq!(st.languages[0].code, "fr");
        assert_eq!(st.languages[0].voice, "Noemie");
        if r.root().join("voices/AAA.safetensors").is_file() {
            break;
        }
        assert!(Instant::now() < end, "the chosen voice was never fetched");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let saved = std::fs::read_to_string(r.root().join("state.json")).unwrap();
    assert!(
        !saved.contains("\"active\""),
        "saved in the new shape: {saved}"
    );
}

#[tokio::test]
async fn available_lists_codes_not_yet_used() {
    let r = rig().await;
    let fr_only = r.install().await;
    let both = r.install_repo(BILINGUAL).await;
    let (_, v) = r.get("/languages/available").await;
    let mut fr_models: Vec<String> = serde_json::from_value(v[1]["models"].clone()).unwrap();
    fr_models.sort();
    let mut want = vec![fr_only.clone(), both.clone()];
    want.sort();
    assert_eq!(v[0]["code"], "en");
    assert_eq!(v[0]["models"], json!([both]));
    assert_eq!(v[1]["code"], "fr");
    assert_eq!(fr_models, want);
    r.set_language("fr", json!({"model": fr_only})).await;
    let (_, v) = r.get("/languages/available").await;
    assert_eq!(v, json!([{"code": "en", "models": [both]}]));
}

impl Rig {
    async fn speak(&self, body: Value) -> (StatusCode, Value) {
        let resp = self
            .client
            .post(format!("http://{}/api/v1/assist/speech", self.addr))
            .json(&body)
            .send()
            .await
            .unwrap();
        (resp.status(), resp.json().await.unwrap_or(Value::Null))
    }
}

fn names(voices: &Value) -> Vec<String> {
    voices["voices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn a_voice_of_another_language_is_refused() {
    let r = rig().await;
    let id = r.install().await;
    r.activate(&id).await;
    r.status_until("ready").await;
    let (s, _) = r.speak(json!({"text": "Bonjour", "voice": "Bob"})).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!r.root().join("voices/BBB.safetensors").exists());
}

#[tokio::test]
async fn voices_follow_the_requested_language() {
    let r = rig().await;
    let id = r.install_repo(BILINGUAL).await;
    r.set_language("fr", json!({"model": id})).await;
    r.set_language("en", json!({"model": id})).await;
    r.language_until(1, "ready").await;
    let (s, en) = r.get("/voices?language=en").await;
    assert_eq!(s, StatusCode::OK, "{en}");
    assert!(names(&en).contains(&"Bob".to_string()), "{en}");
    assert!(!names(&en).contains(&"Noemie".to_string()), "{en}");
    let (_, house) = r.get("/voices").await;
    assert!(names(&house).contains(&"Noemie".to_string()), "{house}");
    assert!(!names(&house).contains(&"Bob".to_string()), "{house}");
    let (s, _) = r.get("/voices?language=de").await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn a_named_catalog_voice_speaks_on_its_first_use() {
    // Registering a voice is a message to the engine thread: the speech asking for it must
    // still get it, even when the thread has not taken the voice in yet.
    let loader = RecordingLoader {
        add_voice_delay: Duration::from_millis(200),
        ..RecordingLoader::default()
    };
    let voices = loader.voices.clone();
    let r = rig_full(true, |_| {}, Arc::new(loader)).await;
    let id = r.install().await;
    r.activate(&id).await;
    r.status_until("ready").await;
    let (s, t) = r.speak(json!({"text": "Bonjour", "voice": "Noemie"})).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let url = t["url"].as_str().unwrap();
    r.client
        .get(format!("http://{}{url}", r.addr))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(*voices.lock().unwrap(), [Some("Noemie".to_string())]);
}
