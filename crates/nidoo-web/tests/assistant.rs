#![cfg(target_arch = "wasm32")]
//! The Assistant page: the conversation, the answer chips, the trace panel and Faux, and where
//! the floating button shows, over a stubbed hub.

use serde_json::{Value, json};
use wasm_bindgen_test::*;

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

#[path = "support/assist_harness.rs"]
mod harness;
use harness::*;

#[wasm_bindgen_test]
async fn assistant_route_renders() {
    let stub = FetchStub::install();
    stub.reply("GET", TURNS, 200, r#"{"turns":[],"kept":false}"#);
    let m = mount();
    wait_for(&m.host, ".assist__input").await;
    settle().await;
    assert!(
        text(&m.host).contains("History is not kept"),
        "{} {:?}",
        text(&m.host),
        stub.requests()
    );
    assert!(find(&m.host, ".assist__simulate input").is_some());
}

#[wasm_bindgen_test]
async fn the_gear_opens_the_assistant_group_in_settings() {
    let stub = FetchStub::install();
    stub.reply("GET", TURNS, 200, r#"{"turns":[],"kept":false}"#);
    let m = mount();
    wait_for(&m.host, ".assist__settings").await;
    let gear = find(&m.host, ".assist__settings").unwrap();
    assert_eq!(
        gear.get_attribute("href").as_deref(),
        Some("/settings#assistant")
    );
}

#[wasm_bindgen_test]
async fn sending_shows_reply() {
    let stub = FetchStub::install();
    stub.reply("GET", TURNS, 200, r#"{"turns":[],"kept":true}"#);
    let answer = turn(
        "t1",
        "acted",
        "Salon réglé à 20 °C (avant 19 °C).",
        Value::Null,
    );
    stub.reply("POST", "/api/v1/assist", 200, &answer.to_string());
    let m = mount();
    wait_for(&m.host, ".assist__input").await;
    send(&m.host, "mets le salon à 20");
    wait_for(&m.host, ".assist__reply").await;
    assert!(text(&m.host).contains("Salon réglé à 20 °C"));
    let sent = posts(&stub);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["text"], "mets le salon à 20");
    assert_eq!(sent[0]["simulate"], false);
    assert!(!sent[0]["client"].as_str().unwrap().is_empty());
    assert!(!text(&m.host).contains("History is not kept"));
}

#[wasm_bindgen_test]
async fn confirm_chips_send_yes() {
    let stub = FetchStub::install();
    stub.reply("GET", TURNS, 200, r#"{"turns":[],"kept":true}"#);
    let asked = turn(
        "t1",
        "confirming",
        "Passer le Salon à 25 °C ?",
        json!({ "pending": "p1", "kind": "confirm", "choices": [] }),
    );
    stub.reply("POST", "/api/v1/assist", 200, &asked.to_string());
    let m = mount();
    wait_for(&m.host, ".assist__input").await;
    send(&m.host, "mets le salon à 25");
    wait_for(&m.host, ".assist__chip").await;
    let chips = all(&m.host, ".assist__chip");
    let labels: Vec<String> = chips.iter().map(|c| c.text_content().unwrap()).collect();
    assert_eq!(labels, ["Yes", "No"]);
    chips[0].click();
    settle().await;
    settle().await;
    let sent = posts(&stub);
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(sent[1]["reply"], json!({ "type": "yes", "pending": "p1" }));
    assert!(sent[1]["text"].is_null());
}

#[wasm_bindgen_test]
async fn choice_chips_send_the_index() {
    let stub = FetchStub::install();
    stub.reply("GET", TURNS, 200, r#"{"turns":[],"kept":true}"#);
    let asked = turn(
        "t1",
        "asked",
        "Dans quelle pièce ? Salon ou Bureau.",
        json!({ "pending": "p2", "kind": "choice", "choices": ["Salon", "Bureau"] }),
    );
    stub.reply("POST", "/api/v1/assist", 200, &asked.to_string());
    let m = mount();
    wait_for(&m.host, ".assist__input").await;
    send(&m.host, "mets le chauffage à 20");
    wait_for(&m.host, ".assist__chip").await;
    let chips = all(&m.host, ".assist__chip");
    assert_eq!(chips[1].text_content().unwrap(), "Bureau");
    chips[1].click();
    settle().await;
    settle().await;
    let sent = posts(&stub);
    assert_eq!(
        sent[1]["reply"],
        json!({ "type": "choice", "pending": "p2", "choice": 1 })
    );
}

#[wasm_bindgen_test]
async fn an_expired_question_shows_no_chips() {
    let stub = FetchStub::install();
    let mut old = turn(
        "t0",
        "confirming",
        "Passer le Salon à 25 °C ?",
        json!({ "pending": "p0", "kind": "confirm", "choices": [] }),
    );
    // Asked long ago: the hub forgot it.
    old["at"] = json!(1000u64);
    stub.reply(
        "GET",
        TURNS,
        200,
        &json!({ "turns": [old], "kept": true }).to_string(),
    );
    let m = mount();
    wait_for(&m.host, ".assist__reply").await;
    settle().await;
    assert!(
        find(&m.host, ".assist__chip").is_none(),
        "{}",
        text(&m.host)
    );
}

#[wasm_bindgen_test]
async fn chips_go_when_the_hub_refuses_the_question() {
    let stub = FetchStub::install();
    stub.reply("GET", TURNS, 200, r#"{"turns":[],"kept":true}"#);
    let asked = turn(
        "t1",
        "confirming",
        "Passer le Salon à 25 °C ?",
        json!({ "pending": "p1", "kind": "confirm", "choices": [] }),
    );
    stub.reply("POST", "/api/v1/assist", 200, &asked.to_string());
    let m = mount();
    wait_for(&m.host, ".assist__input").await;
    send(&m.host, "mets le salon à 25");
    wait_for(&m.host, ".assist__chip").await;
    // The hub no longer knows the question: the next post is refused.
    stub.reply(
        "POST",
        "/api/v1/assist",
        422,
        r#"{"error":"question expired"}"#,
    );
    all(&m.host, ".assist__chip")[0].click();
    settle().await;
    settle().await;
    assert!(
        find(&m.host, ".assist__chip").is_none(),
        "{}",
        text(&m.host)
    );
}

#[wasm_bindgen_test]
async fn trace_panel_shows_value() {
    let stub = FetchStub::install();
    let past = turn("t0", "acted", "Salon réglé à 18,5 °C.", Value::Null);
    stub.reply(
        "GET",
        TURNS,
        200,
        &json!({ "turns": [past], "kept": true }).to_string(),
    );
    let m = mount();
    wait_for(&m.host, ".assist__trace dd").await;
    let panel = find(&m.host, ".assist__trace")
        .unwrap()
        .text_content()
        .unwrap();
    assert!(panel.contains("19 − 0,5 = 18,5 °C"), "{panel}");
    assert!(panel.contains("hors de 16–23 °C"), "{panel}");
    assert!(panel.contains("consigne 25 °C"), "{panel}");
    assert!(panel.contains("mots : salon"), "{panel}");
}

#[wasm_bindgen_test]
async fn faux_marks_turn() {
    let stub = FetchStub::install();
    let past = turn("t0", "acted", "Salon réglé à 18,5 °C.", Value::Null);
    stub.reply(
        "GET",
        TURNS,
        200,
        &json!({ "turns": [past], "kept": true }).to_string(),
    );
    stub.reply("POST", "/api/v1/assist/turns/t0/wrong", 204, "");
    let m = mount();
    let button = wait_for(&m.host, ".assist__wrong").await;
    assert_eq!(button.text_content().unwrap(), "Wrong");
    button.click();
    settle().await;
    settle().await;
    assert!(
        stub.requests()
            .iter()
            .any(|r| r.method == "POST" && r.url == "/api/v1/assist/turns/t0/wrong")
    );
    let button = find(&m.host, ".assist__wrong").unwrap();
    assert_eq!(button.text_content().unwrap(), "Marked wrong");
    assert_eq!(
        button.get_attribute("aria-pressed").as_deref(),
        Some("true")
    );
}

// The floating button.

#[wasm_bindgen_test]
async fn fab_hidden_on_assistant() {
    let stub = FetchStub::install();
    stub.reply("GET", TURNS, 200, r#"{"turns":[],"kept":true}"#);
    let m = mount_shell("/assistant");
    wait_for(&m.host, ".assist__input").await;
    assert!(find(&m.host, ".assist-fab").is_none());
}

#[wasm_bindgen_test]
async fn fab_hidden_while_the_drawer_is_out() {
    let stub = FetchStub::install();
    stub.reply("GET", TURNS, 200, r#"{"turns":[],"kept":true}"#);
    let m = mount_shell("/");
    wait_for(&m.host, ".assist-fab").await;
    find(&m.host, ".content-header__menu").unwrap().click();
    settle().await;
    assert!(find(&m.host, ".assist-fab").is_none());
}
