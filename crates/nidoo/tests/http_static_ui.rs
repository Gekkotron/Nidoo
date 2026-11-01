//! The embedded web UI: the index, the SPA fallback and the JSON 404 under `/api`.
//! Needs `trunk build` in `crates/nidoo-web` for the embedded-page assertions.

use nidoo::config::Config;
use nidoo::http::router;
use nidoo::hub::Hub;
use reqwest::StatusCode;
use reqwest::header::{
    ACCEPT_ENCODING, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_TYPE, HeaderMap, VARY,
};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// Serves a broker-less hub on an ephemeral port and fetches `path`.
async fn get(path: &str) -> (StatusCode, Option<String>, String) {
    let (status, headers, body) = fetch(path).await;
    let content_type = headers
        .get(CONTENT_TYPE)
        .map(|v| v.to_str().expect("content type").to_owned());
    (status, content_type, body)
}

async fn fetch(path: &str) -> (StatusCode, HeaderMap, String) {
    let (status, headers, body) = fetch_bytes(path, None).await;
    (status, headers, String::from_utf8_lossy(&body).into_owned())
}

/// Like [`fetch`], sending `accept_encoding` when given and returning the body undecoded.
async fn fetch_bytes(
    path: &str,
    accept_encoding: Option<&str>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let (tx, _publisher) = mpsc::channel(8);
    let hub = Hub::new(&Config::default(), tx);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(axum::serve(listener, router(hub, nidoo::setup::SetupCtx::inert())).into_future());
    let mut request = reqwest::Client::new().get(format!("http://{addr}{path}"));
    if let Some(accept) = accept_encoding {
        request = request.header(ACCEPT_ENCODING, accept);
    }
    let response = request.send().await.expect("GET");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.bytes().await.expect("body").to_vec();
    (status, headers, body)
}

#[tokio::test]
async fn root_returns_the_embedded_index_or_a_missing_page() {
    let (status, _, body) = get("/").await;
    let built = status == StatusCode::OK && body.contains("<!doctype html>");
    let missing = status == StatusCode::SERVICE_UNAVAILABLE && body.contains("has not been built");
    assert!(built || missing, "unexpected response: {status} {body}");
}

#[tokio::test]
async fn unknown_path_falls_back_to_index() {
    let (root_status, _, root_body) = get("/").await;
    let (status, content_type, body) = get("/entity/0xabc").await;
    assert_eq!(status, root_status);
    assert_eq!(body, root_body);
    assert!(content_type.is_some_and(|t| t.starts_with("text/html")));
}

#[tokio::test]
async fn api_fallback_is_not_html() {
    let (status, content_type, _) = get("/api/v1/does-not-exist").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(content_type.is_some_and(|t| t.contains("json")));
}

#[tokio::test]
async fn index_is_never_cached_and_hashed_assets_are_immutable() {
    let (status, headers, body) = fetch("/").await;
    if status != StatusCode::OK {
        return; // dist not built: nothing to cache
    }
    assert_eq!(headers[CACHE_CONTROL], "no-cache");
    let asset = body
        .split('"')
        .find(|s| s.ends_with(".wasm") || s.ends_with(".js"))
        .expect("a script or wasm reference in index.html");
    let (status, headers, _) = fetch(asset).await;
    assert_eq!(status, StatusCode::OK, "{asset}");
    assert_eq!(
        headers[CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
}

#[tokio::test]
async fn wasm_is_sent_gzipped_only_to_browsers_that_accept_it() {
    let (status, _, index) = fetch("/").await;
    if status != StatusCode::OK {
        return; // dist not built
    }
    let wasm = index
        .split('"')
        .find(|s| s.ends_with(".wasm"))
        .expect("a wasm reference in index.html");
    let (status, headers, plain) = fetch_bytes(wasm, Some("identity")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers.get(CONTENT_ENCODING).is_none());
    assert!(plain.starts_with(b"\0asm"));

    let (status, headers, body) = fetch_bytes(wasm, Some("gzip, deflate")).await;
    assert_eq!(status, StatusCode::OK);
    match headers.get(CONTENT_ENCODING) {
        // A debug `trunk build` writes no compressed copies.
        None => assert_eq!(body, plain),
        Some(coding) => {
            assert_eq!(coding, "gzip");
            assert_eq!(headers[VARY], "accept-encoding");
            assert!(body.starts_with(&[0x1f, 0x8b]), "gzip magic");
            assert!(body.len() < plain.len());
        }
    }
}
