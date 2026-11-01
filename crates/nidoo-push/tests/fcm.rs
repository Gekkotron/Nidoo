use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use futures_util::future::BoxFuture;
use nidoo_push::Priority;
use nidoo_push::fcm::{
    FcmTransport, HttpFcm, Outcome, SendResult, ServiceAccount, classify, send_data,
};
use url::Url;

#[test]
fn classify_unregistered_is_invalid() {
    let body = r#"{"error":{"status":"NOT_FOUND","details":[{"errorCode":"UNREGISTERED"}]}}"#;
    assert_eq!(classify(404, body), Outcome::Invalid);
}

#[test]
fn classify_invalid_argument_about_token_is_invalid() {
    let body = r#"{"error":{"status":"INVALID_ARGUMENT","message":"The registration token is not a valid FCM registration token"}}"#;
    assert_eq!(classify(400, body), Outcome::Invalid);
}

#[test]
fn classify_invalid_argument_about_message_is_message() {
    let body = r#"{"error":{"status":"INVALID_ARGUMENT","message":"Message is too big"}}"#;
    assert_eq!(
        classify(400, body),
        Outcome::Message("Message is too big".into())
    );
}

#[test]
fn classify_server_error_is_transient() {
    assert!(matches!(classify(503, "oops"), Outcome::Transient(_)));
}

#[test]
fn classify_success_is_sent() {
    assert_eq!(classify(200, "{}"), Outcome::Sent);
}

struct Fake;
impl FcmTransport for Fake {
    fn send<'a>(
        &'a self,
        token: &'a str,
        _d: &'a BTreeMap<String, String>,
        _p: Priority,
        _t: u32,
    ) -> BoxFuture<'a, Outcome> {
        Box::pin(async move {
            match token {
                "ok" => Outcome::Sent,
                "dead" => Outcome::Invalid,
                "big" => Outcome::Message("too big".into()),
                _ => Outcome::Transient("x".into()),
            }
        })
    }
}

#[tokio::test]
async fn send_data_splits_tokens_by_outcome() {
    let tokens: Vec<String> = ["ok", "dead", "big", "flaky"].map(String::from).to_vec();
    let r: SendResult = send_data(&Fake, &BTreeMap::new(), &tokens, Priority::High, 60).await;
    assert_eq!(r.sent, 1);
    assert_eq!(r.invalid, vec!["dead"]);
    assert_eq!(r.message_errors, 1);
    assert_eq!(r.last_message_error, "too big");
    assert_eq!(r.transient, vec!["flaky"]);
}

#[derive(Default)]
struct Rec {
    token_requests: AtomicUsize,
    fcm: Mutex<Vec<(HeaderMap, serde_json::Value)>>,
    token_status: Mutex<u16>,
    fcm_statuses: Mutex<Vec<u16>>,
}

async fn token_ep(State(s): State<Arc<Rec>>) -> (StatusCode, String) {
    let n = s.token_requests.fetch_add(1, Ordering::SeqCst) + 1;
    let st = *s.token_status.lock().unwrap();
    if st != 200 {
        return (StatusCode::from_u16(st).unwrap(), "{}".into());
    }
    (
        StatusCode::OK,
        format!(r#"{{"access_token":"at-{n}","expires_in":3600}}"#),
    )
}

async fn fcm_ep(
    State(s): State<Arc<Rec>>,
    h: HeaderMap,
    axum::Json(b): axum::Json<serde_json::Value>,
) -> (StatusCode, String) {
    s.fcm.lock().unwrap().push((h, b));
    let st = {
        let mut v = s.fcm_statuses.lock().unwrap();
        if v.is_empty() { 200 } else { v.remove(0) }
    };
    (StatusCode::from_u16(st).unwrap(), "{}".into())
}

async fn setup(token_status: u16, fcm_statuses: Vec<u16>) -> (HttpFcm, Arc<Rec>) {
    let rec = Arc::new(Rec::default());
    *rec.token_status.lock().unwrap() = token_status;
    *rec.fcm_statuses.lock().unwrap() = fcm_statuses;
    let app = Router::new()
        .route("/token", post(token_ep))
        .route("/v1/projects/p/messages:send", post(fcm_ep))
        .with_state(rec.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    let mut acct = ServiceAccount::load(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/test-sa.json"
    )))
    .unwrap();
    acct.token_uri = format!("{base}token");
    (HttpFcm::new(acct, Url::parse(&base).unwrap()), rec)
}

fn data() -> BTreeMap<String, String> {
    BTreeMap::from([("k".to_string(), "v".to_string())])
}

#[tokio::test]
async fn http_fcm_posts_data_only_message() {
    let (f, rec) = setup(200, vec![]).await;
    let out = f.send("tok-1", &data(), Priority::High, 43200).await;
    assert_eq!(out, Outcome::Sent);
    let reqs = rec.fcm.lock().unwrap();
    let (h, b) = &reqs[0];
    assert_eq!(h["authorization"], "Bearer at-1");
    let m = &b["message"];
    assert_eq!(m["token"], "tok-1");
    assert_eq!(m["data"]["k"], "v");
    assert_eq!(m["android"]["priority"], "high");
    assert_eq!(m["android"]["ttl"], "43200s");
    assert!(m.get("notification").is_none());
}

#[tokio::test]
async fn access_token_is_cached_until_near_expiry() {
    let (f, rec) = setup(200, vec![]).await;
    f.send("a", &data(), Priority::Normal, 60).await;
    f.send("b", &data(), Priority::Normal, 60).await;
    assert_eq!(rec.token_requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unauthorized_drops_the_cached_token_and_is_transient() {
    let (f, rec) = setup(200, vec![401]).await;
    let out = f.send("a", &data(), Priority::High, 60).await;
    assert!(matches!(out, Outcome::Transient(_)));
    assert_eq!(
        f.send("a", &data(), Priority::High, 60).await,
        Outcome::Sent
    );
    assert_eq!(rec.token_requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn oauth_failure_is_transient() {
    let (f, _rec) = setup(500, vec![]).await;
    let out = f.send("a", &data(), Priority::High, 60).await;
    assert!(matches!(out, Outcome::Transient(_)));
}
