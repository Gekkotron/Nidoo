#![cfg(target_arch = "wasm32")]
//! Leaving the Assistant while the hub is still answering leaves the app working. Alone in its
//! file: a link click is routed by the first router the page mounted (as `search_glass.rs`).

use serde_json::Value;
use wasm_bindgen_test::*;

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

#[path = "support/assist_harness.rs"]
mod harness;
use harness::*;

#[wasm_bindgen_test]
async fn leaving_mid_request_does_not_break_the_page() {
    let stub = FetchStub::install();
    stub.reply("GET", TURNS, 200, r#"{"turns":[],"kept":true}"#);
    let answer = turn("t1", "acted", "Salon allumé (4 lumières).", Value::Null);
    stub.reply_after("POST", "/api/v1/assist", 200, &answer.to_string(), 150);
    let m = mount_shell("/assistant");
    wait_for(&m.host, ".assist__input").await;
    send(&m.host, "allume le salon");
    settle().await;
    // Leave for the home page while the hub is still answering.
    find(&m.host, ".breadcrumb a[href=\"/\"]").unwrap().click();
    wait_for(&m.host, "#home").await;
    gloo_timers::future::TimeoutFuture::new(300).await;
    // The app still runs: the button opens the Assistant again.
    find(&m.host, ".assist-fab").unwrap().click();
    wait_for(&m.host, ".assist__input").await;
}
