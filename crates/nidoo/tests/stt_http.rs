//! The speech input routes under `/api/v1/assist/stt`.

use std::net::SocketAddr;

use nidoo::assist::stt::Stt;
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
        format!("http://{}/api/v1/assist/stt{path}", self.addr)
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> (StatusCode, Value) {
        let resp = req.send().await.unwrap();
        let status = resp.status();
        let text = resp.text().await.unwrap();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.send(self.client.get(self.url(path))).await
    }
}

async fn rig(with_stt: bool) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let (tx, _rx) = mpsc::channel(8);
    let hub = Hub::new(&Config::default(), tx);
    if with_stt {
        hub.set_stt(Stt::start(dir.path().join("stt")));
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, router(hub, nidoo::setup::SetupCtx::inert())).into_future());
    Rig {
        _dir: dir,
        addr,
        client: reqwest::Client::new(),
    }
}

#[tokio::test]
async fn status_without_model() {
    let r = rig(true).await;
    let (s, v) = r.get("/status").await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["installed"], false);
    assert_eq!(v["hasKey"], false);
    assert_eq!(v["portInUse"], 4960);
    assert_eq!(
        v["settings"],
        json!({"threads": 4, "gain": 4.0, "port": 4960, "listenMarginMs": 300})
    );
    assert_eq!(v["engine"]["state"], "no_model");
}

#[tokio::test]
async fn put_settings_422() {
    let r = rig(true).await;
    let bad = json!({"threads": 0, "gain": 4.0, "port": 4960, "listenMarginMs": 300});
    let (s, v) = r.send(r.client.put(r.url("/settings")).json(&bad)).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    let good = json!({"threads": 2, "gain": 6.0, "port": 5000, "listenMarginMs": 500});
    let (s, v) = r.send(r.client.put(r.url("/settings")).json(&good)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["settings"], good);
    assert_eq!(v["portInUse"], 4960, "a new port applies after a restart");
}

#[tokio::test]
async fn key_404_then_created() {
    let r = rig(true).await;
    let (s, _) = r.get("/key").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, v) = r.send(r.client.post(r.url("/key"))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let key = v["key"].as_str().unwrap().to_owned();
    assert_eq!(key.len(), 64);
    let (s, v) = r.get("/key").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["key"], key.as_str());
    let (_, status) = r.get("/status").await;
    assert_eq!(status["hasKey"], true);
}

#[tokio::test]
async fn cancel_without_download_is_404() {
    let r = rig(true).await;
    let (s, _) = r.send(r.client.delete(r.url("/download"))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn hub_without_stt_answers_409() {
    let r = rig(false).await;
    let (s, v) = r.get("/status").await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(v["error"], "speech input is not set up on this hub");
}
