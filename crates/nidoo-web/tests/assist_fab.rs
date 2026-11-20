#![cfg(target_arch = "wasm32")]
//! The chat panel's link opens the Assistant page. Alone in its file: a link click is routed by
//! the first router the page mounted, so it must be the only test here (as `search_glass.rs`).

use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

#[path = "support/assist_harness.rs"]
mod harness;
use harness::*;

#[wasm_bindgen_test]
async fn the_panel_link_opens_the_assistant_page() {
    let stub = FetchStub::install();
    stub.reply("GET", TURNS, 200, r#"{"turns":[],"kept":true}"#);
    let m = mount_shell("/");
    wait_for(&m.host, ".assist-fab").await.click();
    let link = wait_for(&m.host, ".assist-panel .assist__full").await;
    assert_eq!(link.get_attribute("href").as_deref(), Some("/assistant"));
    assert_eq!(
        link.get_attribute("aria-label").as_deref(),
        Some("Open the full page")
    );
    link.click();
    settle().await;
    assert_eq!(
        web_sys::window().unwrap().location().pathname().unwrap(),
        "/assistant"
    );
    // The page itself, with its trace; the button and its panel are gone.
    wait_for(&m.host, ".assist__trace").await;
    assert!(find(&m.host, ".assist-fab").is_none());
    assert!(find(&m.host, ".assist-panel").is_none());
    // Back on another page the button returns, its panel closed.
    find(&m.host, ".breadcrumb a[href=\"/\"]").unwrap().click();
    wait_for(&m.host, "#home").await;
    wait_for(&m.host, ".assist-fab").await;
    assert!(find(&m.host, ".assist-panel").is_none());
}
