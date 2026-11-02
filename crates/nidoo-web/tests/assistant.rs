#![cfg(target_arch = "wasm32")]
//! The Assistant page: the conversation, the answer chips, the trace panel and Faux, over a
//! stubbed hub.

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::assistant::AssistantScreen;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

/// gloo-net leaves a trailing `&` on a query written in the URL.
const TURNS: &str = "/api/v1/assist/turns?limit=50&";

struct Mounted {
    _app: Box<dyn std::any::Any>,
    host: web_sys::Element,
}

fn mount() -> Mounted {
    let window = web_sys::window().unwrap();
    let document = window.document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    window
        .history()
        .unwrap()
        .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/assistant"))
        .unwrap();
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(House::new());
        provide_context(Toasts::new());
        provide_context(RwSignal::new(Lang::En));
        nidoo_web::nav::install_nav();
        view! {
            <Router>
                <Routes fallback=|| "fallback">
                    <Route path=path!("/assistant") view=AssistantScreen />
                </Routes>
            </Router>
        }
    });
    Mounted {
        _app: Box::new(app),
        host,
    }
}

async fn settle() {
    tick().await;
    gloo_timers::future::TimeoutFuture::new(50).await;
    tick().await;
}

fn find(host: &web_sys::Element, selector: &str) -> Option<web_sys::HtmlElement> {
    host.query_selector(selector)
        .unwrap()
        .map(|e| e.unchecked_into())
}

fn all(host: &web_sys::Element, selector: &str) -> Vec<web_sys::HtmlElement> {
    let nodes = host.query_selector_all(selector).unwrap();
    (0..nodes.length())
        .map(|i| nodes.item(i).unwrap().unchecked_into())
        .collect()
}

async fn wait_for(host: &web_sys::Element, selector: &str) -> web_sys::HtmlElement {
    for _ in 0..80 {
        if let Some(e) = find(host, selector) {
            settle().await;
            return e;
        }
        gloo_timers::future::TimeoutFuture::new(25).await;
    }
    panic!("waited for {selector:?}: {}", host.inner_html());
}

fn text(host: &web_sys::Element) -> String {
    host.text_content().unwrap_or_default()
}

fn turn(id: &str, outcome: &str, reply: &str, question: Value) -> Value {
    json!({
        "id": id,
        "at": 1800000000u64,
        "client": "c",
        "text": "mets le salon à 25",
        "simulate": false,
        "outcome": outcome,
        "reply": reply,
        "question": question,
        "trace": {
            "words": ["mets", "le", "salon", "a", "25"],
            "intent": "set_temp",
            "room": "salon",
            "roomHow": "mots : salon",
            "entities": ["climate_salon"],
            "value": "19 − 0,5 = 18,5 °C",
            "gate": "hors de 16–23 °C : demander",
            "actions": [{ "target": "salon", "what": "consigne 25 °C", "result": "ok" }],
            "elapsedUs": 120
        },
        "wrong": false
    })
}

/// Types `sentence` in the field and presses Enter.
fn send(host: &web_sys::Element, sentence: &str) {
    let input: web_sys::HtmlInputElement = find(host, ".assist__input").unwrap().unchecked_into();
    input.set_value(sentence);
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    input
        .dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &init).unwrap())
        .unwrap();
    let key = web_sys::KeyboardEventInit::new();
    key.set_key("Enter");
    key.set_bubbles(true);
    input
        .dispatch_event(
            &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &key).unwrap(),
        )
        .unwrap();
}

fn posts(stub: &FetchStub) -> Vec<Value> {
    stub.requests()
        .into_iter()
        .filter(|r| r.method == "POST" && r.url == "/api/v1/assist")
        .map(|r| serde_json::from_str(&r.body).unwrap())
        .collect()
}

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
