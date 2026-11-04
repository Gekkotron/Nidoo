#![cfg(target_arch = "wasm32")]
//! The Assistant page reads its replies aloud with the hub's speech: the header's speaker
//! switch (on by default, kept per browser), a reply spoken as it arrives, and ▶ on each reply.

use serde_json::{Value, json};
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

#[path = "support/assist_harness.rs"]
mod harness;
use harness::*;

const STATUS: &str = "/api/v1/assist/tts/status";
const SPEECH: &str = "/api/v1/assist/speech";
const SPEAK_KEY: &str = "nidoo.assist.speak";

fn storage() -> web_sys::Storage {
    web_sys::window().unwrap().local_storage().unwrap().unwrap()
}

/// A hub with a speech model in use, and an empty conversation.
fn speaking_hub() -> FetchStub {
    let _ = storage().remove_item(SPEAK_KEY);
    let _ = storage().remove_item("nidoo.assist.cleared");
    let stub = FetchStub::install();
    stub.reply("GET", TURNS, 200, r#"{"turns":[],"kept":true}"#);
    stub.reply(
        "GET",
        STATUS,
        200,
        r#"{"languages":[
              {"code":"en","model":"c--d--q8","voice":"default",
               "engine":{"state":"ready","model":"c--d--q8"}},
              {"code":"fr","model":"a--b--q8","voice":"zoe",
               "engine":{"state":"ready","model":"a--b--q8"}}],
            "token_set":false,"download":null,"speech_built":true}"#,
    );
    stub.reply(
        "POST",
        SPEECH,
        200,
        r#"{"id":"c1","url":"/api/v1/assist/speech/c1"}"#,
    );
    stub.reply(
        "POST",
        "/api/v1/assist",
        200,
        &turn("t1", "acted", "Salon allumé.", Value::Null).to_string(),
    );
    stub
}

fn spoken(stub: &FetchStub) -> Vec<Value> {
    stub.requests()
        .into_iter()
        .filter(|r| r.method == "POST" && r.url == SPEECH)
        .map(|r| serde_json::from_str(&r.body).unwrap())
        .collect()
}

#[wasm_bindgen_test]
async fn a_new_reply_is_spoken() {
    let stub = speaking_hub();
    let m = mount();
    let switch = wait_for(&m.host, ".assist__speak").await;
    assert_eq!(
        switch.get_attribute("aria-pressed").as_deref(),
        Some("true")
    );
    send(&m.host, "allume le salon");
    wait_for(&m.host, ".assist__reply").await;
    settle().await;
    assert_eq!(
        spoken(&stub),
        vec![json!({ "text": "Salon allumé.", "language": "fr", "voice": null })]
    );
}

#[wasm_bindgen_test]
async fn the_switch_turns_speech_off_and_is_remembered() {
    let stub = speaking_hub();
    let m = mount();
    wait_for(&m.host, ".assist__speak").await.click();
    settle().await;
    let switch = find(&m.host, ".assist__speak").unwrap();
    assert_eq!(
        switch.get_attribute("aria-pressed").as_deref(),
        Some("false")
    );
    assert_eq!(storage().get_item(SPEAK_KEY).unwrap().as_deref(), Some("0"));
    send(&m.host, "allume le salon");
    wait_for(&m.host, ".assist__reply").await;
    settle().await;
    assert!(spoken(&stub).is_empty(), "{:?}", stub.requests());
}

#[wasm_bindgen_test]
async fn play_speaks_that_reply_even_with_speech_off() {
    let stub = speaking_hub();
    storage().set_item(SPEAK_KEY, "0").unwrap();
    let m = mount();
    wait_for(&m.host, ".assist__speak").await;
    send(&m.host, "allume le salon");
    wait_for(&m.host, ".assist__play").await.click();
    settle().await;
    assert_eq!(
        spoken(&stub),
        vec![json!({ "text": "Salon allumé.", "language": "fr", "voice": null })]
    );
}

#[wasm_bindgen_test]
async fn no_speech_model_means_no_switch_and_no_play() {
    let _ = storage().remove_item(SPEAK_KEY);
    let _ = storage().remove_item("nidoo.assist.cleared");
    let stub = FetchStub::install();
    stub.reply("GET", TURNS, 200, r#"{"turns":[],"kept":true}"#);
    stub.reply(
        "GET",
        STATUS,
        200,
        r#"{"languages":[{"code":"fr","model":null,"voice":"zoe",
               "engine":{"state":"no_model"}}],
            "token_set":false,"download":null,"speech_built":true}"#,
    );
    stub.reply(
        "POST",
        "/api/v1/assist",
        200,
        &turn("t1", "acted", "Salon allumé.", Value::Null).to_string(),
    );
    let m = mount();
    wait_for(&m.host, ".assist__input").await;
    send(&m.host, "allume le salon");
    wait_for(&m.host, ".assist__reply").await;
    settle().await;
    assert!(find(&m.host, ".assist__speak").is_none());
    assert!(find(&m.host, ".assist__play").is_none());
    assert!(spoken(&stub).is_empty());
}

#[wasm_bindgen_test]
async fn the_header_tools_sit_together_after_the_hint() {
    let _stub = speaking_hub();
    let m = mount();
    wait_for(&m.host, ".assist__speak").await;
    let head = find(&m.host, ".assist__head").unwrap();
    let children = head.children();
    assert_eq!(children.length(), 2, "{}", head.outer_html());
    let tools = find(&m.host, ".assist__head > .assist__tools").unwrap();
    for part in [".assist__speak", ".assist__settings", ".assist__clear"] {
        assert!(
            tools.query_selector(part).unwrap().is_some(),
            "{part} not in the tools: {}",
            head.outer_html()
        );
    }
}

#[wasm_bindgen_test]
async fn a_hub_without_french_does_not_read_replies() {
    // Replies are French: a hub that speaks only English offers no switch.
    let _ = storage().remove_item(SPEAK_KEY);
    let _ = storage().remove_item("nidoo.assist.cleared");
    let stub = FetchStub::install();
    stub.reply("GET", TURNS, 200, r#"{"turns":[],"kept":true}"#);
    stub.reply(
        "GET",
        STATUS,
        200,
        r#"{"languages":[{"code":"en","model":"c--d--q8","voice":"default",
               "engine":{"state":"ready","model":"c--d--q8"}}],
            "token_set":false,"download":null,"speech_built":true}"#,
    );
    stub.reply(
        "POST",
        "/api/v1/assist",
        200,
        &turn("t1", "acted", "Salon allumé.", Value::Null).to_string(),
    );
    let m = mount();
    wait_for(&m.host, ".assist__input").await;
    send(&m.host, "allume le salon");
    wait_for(&m.host, ".assist__reply").await;
    settle().await;
    assert!(find(&m.host, ".assist__speak").is_none());
    assert!(spoken(&stub).is_empty());
}
