//! The speech address over HTTP, against the fake tone synthesiser.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nidoo::assist::tts::tone::{BlockingLoader, Release, ToneLoader};
use nidoo::assist::tts::{Tts, TtsConfig};
use nidoo::config::Config;
use nidoo::http::router;
use nidoo::hub::Hub;
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

struct Rig {
    _dir: tempfile::TempDir,
    addr: SocketAddr,
    client: reqwest::Client,
}

impl Rig {
    fn url(&self, path: &str) -> String {
        format!("http://{}/api/v1/assist{path}", self.addr)
    }

    async fn post_speech(&self, body: Value) -> (StatusCode, Value) {
        let r = self
            .client
            .post(self.url("/speech"))
            .json(&body)
            .send()
            .await
            .unwrap();
        (r.status(), r.json().await.unwrap_or(Value::Null))
    }

    async fn get_url(&self, url: &str) -> reqwest::Response {
        self.client
            .get(format!("http://{}{url}", self.addr))
            .send()
            .await
            .unwrap()
    }

    async fn install_and_activate(&self) {
        let (s, v) = self
            .json(reqwest::Method::POST, "/tts/uploads", json!({}))
            .await;
        assert_eq!(s, StatusCode::OK);
        let id = v["id"].as_str().unwrap().to_owned();
        let files: [(&str, &[u8]); 3] = [
            ("config.json", br#"{"languages":["fr"],"model_id":"M"}"#),
            ("tokenizer.json", b"{}"),
            ("model.q8.gguf", b"weights"),
        ];
        for (name, body) in files {
            let r = self
                .client
                .put(self.url(&format!("/tts/uploads/{id}/files/{name}")))
                .body(body.to_vec())
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::NO_CONTENT);
        }
        let (s, m) = self
            .json(
                reqwest::Method::POST,
                &format!("/tts/uploads/{id}/finish"),
                json!({}),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{m}");
        let (s, _) = self
            .json(
                reqwest::Method::PUT,
                "/tts/languages/fr",
                json!({"model": m["id"]}),
            )
            .await;
        assert_eq!(s, StatusCode::OK);
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            let st: Value = self
                .client
                .get(self.url("/tts/status"))
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

    async fn json(&self, m: reqwest::Method, path: &str, body: Value) -> (StatusCode, Value) {
        let r = self
            .client
            .request(m, self.url(path))
            .json(&body)
            .send()
            .await
            .unwrap();
        (r.status(), r.json().await.unwrap_or(Value::Null))
    }
}

async fn rig_with(loader: Arc<dyn nidoo::assist::tts::engine::SpeakerLoader>, ready: bool) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let (tx, _rx) = mpsc::channel(8);
    let mut cfg = Config::default();
    cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
    let hub = Hub::new(&cfg, tx);
    hub.set_tts(Tts::start(TtsConfig {
        root: dir.path().join("tts"),
        hf_base: "http://127.0.0.1:1".to_owned(),
        loader,
    }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, router(hub, nidoo::setup::SetupCtx::inert())).into_future());
    let rig = Rig {
        _dir: dir,
        addr,
        client: reqwest::Client::new(),
    };
    if ready {
        rig.install_and_activate().await;
    }
    rig
}

async fn ready_rig() -> Rig {
    rig_with(Arc::new(ToneLoader), true).await
}

const FULL: usize = 44 + 2 * 2_400 * 2;

#[tokio::test]
async fn speech_streams_a_wav() {
    let r = ready_rig().await;
    let (s, t) = r.post_speech(json!({"text": "un deux"})).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let url = t["url"].as_str().unwrap();
    assert_eq!(
        url,
        format!("/api/v1/assist/speech/{}.wav", t["id"].as_str().unwrap())
    );
    let resp = r.get_url(url).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()["content-type"], "audio/wav");
    assert_eq!(resp.headers()["cache-control"], "no-store");
    let body = resp.bytes().await.unwrap();
    assert_eq!(body.len(), FULL);
    assert_eq!(&body[0..4], b"RIFF");
}

#[tokio::test]
async fn a_second_fetch_gets_the_whole_clip() {
    let r = ready_rig().await;
    let (_, t) = r.post_speech(json!({"text": "un deux"})).await;
    let url = t["url"].as_str().unwrap();
    let first = r.get_url(url).await.bytes().await.unwrap();
    let second = r
        .client
        .get(format!("http://{}{url}", r.addr))
        .header("Range", "bytes=100-")
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    let second = second.bytes().await.unwrap();
    assert_eq!(first.len(), FULL);
    assert_eq!(&second[8..40], &first[8..40]);
    assert_eq!(&second[44..], &first[44..]);
    assert_eq!(&second[40..44], &9_600u32.to_le_bytes());
    assert_eq!(&second[4..8], &(36u32 + 9_600).to_le_bytes());
}

#[tokio::test]
async fn expired_or_unknown_is_404() {
    let r = ready_rig().await;
    tokio::time::pause();
    let (_, t) = r.post_speech(json!({"text": "un"})).await;
    let url = t["url"].as_str().unwrap().to_owned();
    assert_eq!(r.get_url(&url).await.status(), StatusCode::OK);
    tokio::time::advance(Duration::from_secs(61)).await;
    let (s, _) = r.post_speech(json!({"text": "deux"})).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(r.get_url(&url).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        r.get_url("/api/v1/assist/speech/nope.wav").await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        r.get_url("/api/v1/assist/speech/nope").await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn speech_without_a_ready_model_is_409() {
    let r = rig_with(Arc::new(ToneLoader), false).await;
    let (s, v) = r.post_speech(json!({"text": "bonjour"})).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
}

#[tokio::test]
async fn empty_text_is_422() {
    let r = ready_rig().await;
    let (s, _) = r.post_speech(json!({"text": "  \n "})).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn text_over_500_chars_is_422() {
    let r = ready_rig().await;
    let (s, _) = r.post_speech(json!({"text": "é".repeat(501)})).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _) = r.post_speech(json!({"text": "é".repeat(500)})).await;
    assert_eq!(s, StatusCode::CREATED);
}

#[tokio::test]
async fn queue_full_is_503() {
    let (loader, release): (BlockingLoader, Release) = BlockingLoader::new();
    let r = rig_with(Arc::new(loader), true).await;
    let (s, _) = r.post_speech(json!({"text": "un"})).await;
    assert_eq!(s, StatusCode::CREATED);
    let started = {
        let release = release.clone();
        tokio::task::spawn_blocking(move || release.wait_started(1))
            .await
            .unwrap()
    };
    assert!(started);
    for _ in 0..4 {
        let (s, _) = r.post_speech(json!({"text": "un"})).await;
        assert_eq!(s, StatusCode::CREATED);
    }
    let (s, _) = r.post_speech(json!({"text": "un"})).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    release.release_all();
}

#[tokio::test]
async fn the_text_is_not_in_the_url() {
    let r = ready_rig().await;
    let (_, t) = r.post_speech(json!({"text": "bonjour"})).await;
    assert!(!t["url"].as_str().unwrap().contains("bonjour"));
    assert_eq!(t["id"].as_str().unwrap().len(), 32);
}
