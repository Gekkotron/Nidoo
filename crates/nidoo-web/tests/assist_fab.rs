#![cfg(target_arch = "wasm32")]
//! The floating button opens the Assistant. Alone in its file: a link click is routed by the
//! first router the page mounted, so it must be the only test here (as `search_glass.rs`).

use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

#[path = "support/assist_harness.rs"]
mod harness;
use harness::*;

#[wasm_bindgen_test]
async fn fab_on_home_links_to_assistant() {
    let stub = FetchStub::install();
    stub.reply("GET", TURNS, 200, r#"{"turns":[],"kept":true}"#);
    let m = mount_shell("/");
    let fab = wait_for(&m.host, ".assist-fab").await;
    assert_eq!(fab.get_attribute("href").as_deref(), Some("/assistant"));
    assert_eq!(
        fab.get_attribute("aria-label").as_deref(),
        Some("Open the assistant")
    );
    fab.click();
    settle().await;
    assert_eq!(
        web_sys::window().unwrap().location().pathname().unwrap(),
        "/assistant"
    );
    wait_for(&m.host, ".assist__input").await;
    assert!(find(&m.host, ".assist-fab").is_none());
}
