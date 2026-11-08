#![cfg(target_arch = "wasm32")]
//! The speech input card: the model line follows the status, Download and Save call the hub,
//! a new key shows the secrets.yaml line, and replacing a key asks first.

use leptos::prelude::*;
use nidoo_web::components::toast::Toasts;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::speech_input::AssistListeningPage;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

const BASE: &str = "/api/v1/assist/stt";
const KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

fn status(installed: bool, has_key: bool) -> String {
    serde_json::json!({
        "built": true,
        "installed": installed,
        "engine": if installed { serde_json::json!({"state": "ready", "model": "parakeet-tdt-0.6b-v3-int8"}) } else { serde_json::json!({"state": "no_model"}) },
        "download": null,
        "settings": {"threads": 4, "gain": 4.0, "port": 4960, "listenMarginMs": 300},
        "hasKey": has_key,
        "portInUse": 4960
    })
    .to_string()
}

struct Mounted {
    _app: Box<dyn std::any::Any>,
    host: web_sys::Element,
}

fn mount() -> Mounted {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(Toasts::new());
        provide_context(RwSignal::new(Lang::En));
        view! { <AssistListeningPage /> }
    });
    Mounted {
        _app: Box::new(app),
        host,
    }
}

async fn settle() {
    leptos::task::tick().await;
    gloo_timers::future::TimeoutFuture::new(150).await;
    leptos::task::tick().await;
}

fn text(host: &web_sys::Element) -> String {
    host.text_content().unwrap_or_default()
}

fn click(host: &web_sys::Element, selector: &str) {
    host.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no {selector}"))
        .unchecked_into::<web_sys::HtmlElement>()
        .click();
}

fn sent(stub: &FetchStub, method: &str, url: &str) -> Vec<String> {
    stub.requests()
        .into_iter()
        .filter(|r| r.method == method && r.url == url)
        .map(|r| r.body)
        .collect()
}

#[wasm_bindgen_test]
async fn the_listening_page_holds_the_speech_input_card() {
    let stub = FetchStub::install();
    stub.reply("GET", &format!("{BASE}/status"), 200, &status(false, false));
    let m = mount();
    settle().await;
    let card = m
        .host
        .query_selector(".assist-set > .assist-set__card")
        .unwrap()
        .expect("the speech input card inside the page");
    assert!(
        card.text_content()
            .unwrap_or_default()
            .contains("Speech input")
    );
}

#[wasm_bindgen_test]
async fn not_installed_offers_the_download_with_its_size() {
    let stub = FetchStub::install();
    stub.reply("GET", &format!("{BASE}/status"), 200, &status(false, false));
    stub.reply("POST", &format!("{BASE}/download"), 202,
        r#"{"model":"parakeet-tdt-0.6b-v3-int8","state":"running","done":0,"total":0,"file":"","error":null}"#);
    let m = mount();
    settle().await;
    assert!(
        text(&m.host).contains("about 490 MB to download"),
        "{}",
        text(&m.host)
    );
    click(&m.host, ".speech-in__download");
    settle().await;
    assert_eq!(sent(&stub, "POST", &format!("{BASE}/download")).len(), 1);
}

#[wasm_bindgen_test]
async fn save_sends_the_new_threads() {
    let stub = FetchStub::install();
    stub.reply("GET", &format!("{BASE}/status"), 200, &status(true, false));
    stub.reply(
        "PUT",
        &format!("{BASE}/settings"),
        200,
        &status(true, false),
    );
    let m = mount();
    settle().await;
    assert!(text(&m.host).contains("Ready"));
    let input: web_sys::HtmlInputElement = m
        .host
        .query_selector(".speech-in__threads")
        .unwrap()
        .unwrap()
        .unchecked_into();
    input.set_value("2");
    input
        .dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();
    click(&m.host, ".speech-in__save");
    settle().await;
    let bodies = sent(&stub, "PUT", &format!("{BASE}/settings"));
    assert_eq!(bodies.len(), 1);
    let body: serde_json::Value = serde_json::from_str(&bodies[0]).unwrap();
    assert_eq!(body["threads"], 2);
    assert_eq!(body["listenMarginMs"], 300);
}

#[wasm_bindgen_test]
async fn a_new_key_shows_the_secrets_line() {
    let stub = FetchStub::install();
    stub.reply("GET", &format!("{BASE}/status"), 200, &status(true, false));
    stub.reply(
        "POST",
        &format!("{BASE}/key"),
        200,
        &format!(r#"{{"key":"{KEY}"}}"#),
    );
    let m = mount();
    settle().await;
    click(&m.host, ".speech-in__create");
    settle().await;
    assert!(
        text(&m.host).contains(&format!("voice_key: \"{KEY}\"")),
        "{}",
        text(&m.host)
    );
}

#[wasm_bindgen_test]
async fn replacing_the_key_asks_first() {
    let stub = FetchStub::install();
    stub.reply("GET", &format!("{BASE}/status"), 200, &status(true, true));
    stub.reply(
        "POST",
        &format!("{BASE}/key"),
        200,
        &format!(r#"{{"key":"{KEY}"}}"#),
    );
    let m = mount();
    settle().await;
    assert!(
        m.host
            .query_selector(".speech-in__create")
            .unwrap()
            .is_none()
    );
    assert!(
        text(&m.host).contains("shows a key only once"),
        "no way to read it back"
    );
    click(&m.host, ".speech-in__replace");
    settle().await;
    assert!(text(&m.host).contains("stop being heard"));
    assert!(
        sent(&stub, "POST", &format!("{BASE}/key")).is_empty(),
        "not yet"
    );
    click(&m.host, ".speech-in__keep");
    settle().await;
    assert!(!text(&m.host).contains("stop being heard"));
    click(&m.host, ".speech-in__replace");
    settle().await;
    click(&m.host, ".speech-in__confirm");
    settle().await;
    assert_eq!(sent(&stub, "POST", &format!("{BASE}/key")).len(), 1);
}
