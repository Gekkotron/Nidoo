#![cfg(target_arch = "wasm32")]
//! The floating button's chat panel: it opens over the page, talks to the hub for real, and
//! closes with the button or Escape. No link is clicked here (see `assist_fab.rs`).

use serde_json::Value;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

#[path = "support/assist_harness.rs"]
mod harness;
use harness::*;

const SIMULATE_KEY: &str = "nidoo.assist.simulate";

fn storage() -> web_sys::Storage {
    web_sys::window().unwrap().local_storage().unwrap().unwrap()
}

#[wasm_bindgen_test]
async fn the_button_opens_and_closes_the_panel_over_the_page() {
    let stub = FetchStub::install();
    stub.reply("GET", TURNS, 200, r#"{"turns":[],"kept":true}"#);
    let m = mount_shell("/");
    let fab = wait_for(&m.host, ".assist-fab").await;
    assert_eq!(
        fab.get_attribute("aria-label").as_deref(),
        Some("Open the assistant")
    );
    assert_eq!(fab.get_attribute("aria-expanded").as_deref(), Some("false"));
    assert!(find(&m.host, ".assist-panel").is_none());

    fab.click();
    wait_for(&m.host, ".assist-panel .assist__input").await;
    // Still on the page it was opened from.
    assert_eq!(
        web_sys::window().unwrap().location().pathname().unwrap(),
        "/"
    );
    assert!(find(&m.host, "#home").is_some());
    let fab = find(&m.host, ".assist-fab").unwrap();
    assert_eq!(
        fab.get_attribute("aria-label").as_deref(),
        Some("Close the assistant")
    );
    assert_eq!(fab.get_attribute("aria-expanded").as_deref(), Some("true"));
    // The conversation only: what the page adds stays on the page.
    assert!(find(&m.host, ".assist__trace").is_none());
    assert!(find(&m.host, ".assist__simulate").is_none());
    assert!(find(&m.host, ".assist__clear").is_none());
    let panel = find(&m.host, ".assist-panel").unwrap();
    assert!(
        panel
            .text_content()
            .unwrap()
            .contains("Lights, shutters, heating setpoints"),
        "{}",
        panel.inner_html()
    );

    fab.click();
    settle().await;
    assert!(find(&m.host, ".assist-panel").is_none());
    assert_eq!(
        find(&m.host, ".assist-fab")
            .unwrap()
            .get_attribute("aria-label")
            .as_deref(),
        Some("Open the assistant")
    );
}

#[wasm_bindgen_test]
async fn the_panel_acts_for_real_even_when_the_page_simulates() {
    storage().set_item(SIMULATE_KEY, "1").unwrap();
    let stub = FetchStub::install();
    stub.reply("GET", TURNS, 200, r#"{"turns":[],"kept":true}"#);
    let answer = turn("t1", "acted", "Salon allumé (4 lumières).", Value::Null);
    stub.reply("POST", "/api/v1/assist", 200, &answer.to_string());
    let m = mount_shell("/");
    wait_for(&m.host, ".assist-fab").await.click();
    wait_for(&m.host, ".assist-panel .assist__input").await;
    send(&m.host, "allume le salon");
    let reply = wait_for(&m.host, ".assist-panel .assist__reply").await;
    assert_eq!(reply.text_content().unwrap(), "Salon allumé (4 lumières).");
    let sent = posts(&stub);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["text"], "allume le salon");
    assert_eq!(sent[0]["simulate"], false);
    // The hint made room for the conversation, and the page's switch is as it was.
    assert!(find(&m.host, ".assist__empty").is_none());
    assert_eq!(
        storage().get_item(SIMULATE_KEY).unwrap().as_deref(),
        Some("1")
    );
    let _ = storage().remove_item(SIMULATE_KEY);
    let _ = storage().remove_item("nidoo.assist.history");
}

#[wasm_bindgen_test]
async fn escape_closes_the_panel_and_focuses_the_button() {
    let stub = FetchStub::install();
    stub.reply("GET", TURNS, 200, r#"{"turns":[],"kept":true}"#);
    let m = mount_shell("/");
    wait_for(&m.host, ".assist-fab").await.click();
    wait_for(&m.host, ".assist-panel .assist__input").await;
    let key = web_sys::KeyboardEventInit::new();
    key.set_key("Escape");
    key.set_bubbles(true);
    find(&m.host, ".assist__input")
        .unwrap()
        .dispatch_event(
            &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &key).unwrap(),
        )
        .unwrap();
    settle().await;
    assert!(find(&m.host, ".assist-panel").is_none());
    let focused = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .active_element()
        .unwrap();
    assert!(focused.class_list().contains("assist-fab"));
}

/// The turns list as the stylesheet makes it in the panel: a short box that scrolls.
fn scrolling_turns() -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let style = document.create_element("style").unwrap();
    style.set_text_content(Some(
        ".assist-panel .assist__turns{display:block;height:120px;overflow-y:auto;margin:0;padding:0}\
         .assist-panel .assist__turn{display:block;height:60px}",
    ));
    document.head().unwrap().append_child(&style).unwrap();
    style
}

#[wasm_bindgen_test]
async fn the_newest_turn_is_scrolled_into_view() {
    let style = scrolling_turns();
    let stub = FetchStub::install();
    let past: Vec<Value> = (0..6)
        .map(|i| turn(&format!("p{i}"), "acted", "C'est fait.", Value::Null))
        .collect();
    let list = serde_json::json!({ "turns": past, "kept": true });
    stub.reply("GET", TURNS, 200, &list.to_string());
    let answer = turn("t1", "acted", "Salon allumé (4 lumières).", Value::Null);
    stub.reply("POST", "/api/v1/assist", 200, &answer.to_string());
    let m = mount_shell("/");
    wait_for(&m.host, ".assist-fab").await.click();
    wait_for(&m.host, ".assist-panel .assist__turn").await;
    gloo_timers::future::TimeoutFuture::new(100).await;
    let turns = find(&m.host, ".assist-panel .assist__turns").unwrap();
    // Opening shows the end of the conversation.
    assert!(turns.scroll_top() > 0, "on open: {}", turns.scroll_top());

    // Scrolled back up, a sentence and its reply come into view.
    turns.set_scroll_top(0);
    send(&m.host, "allume le salon");
    for _ in 0..80 {
        if all(&m.host, ".assist-panel .assist__turn").len() == 7 {
            break;
        }
        gloo_timers::future::TimeoutFuture::new(25).await;
    }
    gloo_timers::future::TimeoutFuture::new(100).await;
    assert_eq!(all(&m.host, ".assist-panel .assist__turn").len(), 7);
    let last = all(&m.host, ".assist-panel .assist__turn").pop().unwrap();
    let (list, turn) = (
        turns.get_bounding_client_rect(),
        last.get_bounding_client_rect(),
    );
    assert!(
        turn.top() >= list.top() - 1.0 && turn.bottom() <= list.bottom() + 1.0,
        "the new turn is outside the list: {}..{} in {}..{}",
        turn.top(),
        turn.bottom(),
        list.top(),
        list.bottom()
    );
    style.remove();
    let _ = storage().remove_item("nidoo.assist.history");
}
