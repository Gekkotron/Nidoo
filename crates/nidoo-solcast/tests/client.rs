//! The client against an axum stub that records what it was asked.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::get;
use nidoo_solcast::{ApiKey, Client, SolcastError};
use tokio::net::TcpListener;

#[derive(Default)]
struct Seen {
    path: String,
    query: String,
    auth: String,
}

#[derive(Clone)]
struct Stub {
    seen: Arc<Mutex<Seen>>,
    status: u16,
    body: String,
}

async fn handle(State(s): State<Stub>, uri: Uri, headers: HeaderMap) -> (StatusCode, String) {
    *s.seen.lock().unwrap() = Seen {
        path: uri.path().to_owned(),
        query: uri.query().unwrap_or_default().to_owned(),
        auth: headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned(),
    };
    (StatusCode::from_u16(s.status).unwrap(), s.body.clone())
}

async fn serve(status: u16, body: &str) -> (String, Arc<Mutex<Seen>>) {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let stub = Stub {
        seen: Arc::clone(&seen),
        status,
        body: body.to_owned(),
    };
    let app = Router::new().fallback(get(handle)).with_state(stub);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, seen)
}

fn fixture() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/forecasts.json"
    ))
    .unwrap()
}

#[tokio::test]
async fn sends_bearer_and_path_and_parses() {
    let (base, seen) = serve(200, &fixture()).await;
    let client = Client::new(base, ApiKey::new(" k\n".into())).unwrap();
    let periods = client.site_forecast("abcd").await.unwrap();
    assert_eq!(periods.len(), 48);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.path, "/rooftop_sites/abcd/forecasts");
    assert!(seen.query.contains("format=json"), "{}", seen.query);
    assert!(seen.query.contains("hours=192"), "{}", seen.query);
    assert_eq!(seen.auth, "Bearer k");
}

#[tokio::test]
async fn maps_401_403_429_500() {
    for (status, check) in [
        (401, "unauthorized"),
        (403, "unauthorized"),
        (429, "quota"),
        (500, "status"),
    ] {
        let (base, _) = serve(status, "{}").await;
        let client = Client::new(base, ApiKey::new("k".into())).unwrap();
        let err = client.site_forecast("abcd").await.unwrap_err();
        match (check, err) {
            ("unauthorized", SolcastError::Unauthorized) => {}
            ("quota", SolcastError::QuotaExceeded) => {}
            ("status", SolcastError::Status(500)) => {}
            (_, other) => panic!("{status}: {other:?}"),
        }
    }
}

#[tokio::test]
async fn key_never_in_debug_or_errors() {
    let key = ApiKey::new("s3cr3t".into());
    assert_eq!(format!("{key:?}"), "ApiKey(***)");

    let mut errors = Vec::new();
    for status in [401, 429, 500] {
        let (base, _) = serve(status, "{}").await;
        let client = Client::new(base, key.clone()).unwrap();
        errors.push(client.site_forecast("abcd").await.unwrap_err());
    }
    // Nothing listens on this port: a network error.
    let client = Client::new("http://127.0.0.1:1", key.clone()).unwrap();
    let net = client.site_forecast("abcd").await.unwrap_err();
    assert!(matches!(net, SolcastError::Network(_)), "{net:?}");
    errors.push(net);
    let (base, _) = serve(200, "not json").await;
    let client = Client::new(base, key).unwrap();
    errors.push(client.site_forecast("abcd").await.unwrap_err());

    for e in errors {
        assert!(!format!("{e}").contains("s3cr3t"), "{e}");
        assert!(!format!("{e:?}").contains("s3cr3t"), "{e:?}");
    }
}

#[tokio::test]
async fn garbage_body_is_parse_error() {
    let (base, _) = serve(200, "<html>nope</html>").await;
    let client = Client::new(base, ApiKey::new("k".into())).unwrap();
    let err = client.site_forecast("abcd").await.unwrap_err();
    assert!(matches!(err, SolcastError::Parse(_)), "{err:?}");
}
