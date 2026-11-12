//! Shared by the Assistant browser tests: mounting the page alone or inside the real shell, a
//! fetch-stubbed hub, and DOM helpers. Pulled in with
//! `#[path = "support/assist_harness.rs"] mod harness;` next to `fetch_stub.rs`.
#![allow(dead_code)]

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

use super::fetch_stub::FetchStub;

/// The turns the page asks for on load; gloo-net leaves a trailing `&` on a query in the URL.
pub const TURNS: &str = "/api/v1/assist/turns?limit=50&";

pub struct Mounted {
    pub _app: Box<dyn std::any::Any>,
    pub host: web_sys::Element,
}

/// The page in English.
pub fn mount() -> Mounted {
    mount_in(Lang::En)
}

/// The page in `lang`, which is also the language its replies are spoken in.
pub fn mount_in(lang: Lang) -> Mounted {
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
        provide_context(RwSignal::new(lang));
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

pub async fn settle() {
    tick().await;
    gloo_timers::future::TimeoutFuture::new(50).await;
    tick().await;
}

pub fn find(host: &web_sys::Element, selector: &str) -> Option<web_sys::HtmlElement> {
    host.query_selector(selector)
        .unwrap()
        .map(|e| e.unchecked_into())
}

pub fn all(host: &web_sys::Element, selector: &str) -> Vec<web_sys::HtmlElement> {
    let nodes = host.query_selector_all(selector).unwrap();
    (0..nodes.length())
        .map(|i| nodes.item(i).unwrap().unchecked_into())
        .collect()
}

pub async fn wait_for(host: &web_sys::Element, selector: &str) -> web_sys::HtmlElement {
    for _ in 0..80 {
        if let Some(e) = find(host, selector) {
            settle().await;
            return e;
        }
        gloo_timers::future::TimeoutFuture::new(25).await;
    }
    panic!("waited for {selector:?}: {}", host.inner_html());
}

pub fn text(host: &web_sys::Element) -> String {
    host.text_content().unwrap_or_default()
}

pub fn turn(id: &str, outcome: &str, reply: &str, question: Value) -> Value {
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
pub fn send(host: &web_sys::Element, sentence: &str) {
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

pub fn posts(stub: &FetchStub) -> Vec<Value> {
    stub.requests()
        .into_iter()
        .filter(|r| r.method == "POST" && r.url == "/api/v1/assist")
        .map(|r| serde_json::from_str(&r.body).unwrap())
        .collect()
}

/// The real shell around a plain home page and the Assistant page, at `url`.
pub fn mount_shell(url: &str) -> Mounted {
    use nidoo_web::shell::Shell;
    use nidoo_web::theme::Theme;
    let window = web_sys::window().unwrap();
    let document = window.document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    window
        .history()
        .unwrap()
        .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(url))
        .unwrap();
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(House::new());
        provide_context(Toasts::new());
        provide_context(RwSignal::new(Theme::Auto));
        provide_context(RwSignal::new(Lang::En));
        provide_context(nidoo_web::commands::Pending::new());
        provide_context(nidoo_web::components::confirm::Confirming::new());
        nidoo_web::nav::install_nav();
        let badge: Signal<Option<String>> = Signal::stored(None);
        view! {
            <Router>
                <Shell
                    climate_on=Signal::from(false)
                    alarm_armed=Signal::from(false)
                    solar_active=Signal::from(false)
                    alerts_badge=badge
                >
                    <Routes fallback=|| ()>
                        <Route path=path!("/") view=|| view! { <p id="home">"home"</p> } />
                        <Route path=path!("/assistant") view=AssistantScreen />
                    </Routes>
                </Shell>
            </Router>
        }
    });
    Mounted {
        _app: Box::new(app),
        host,
    }
}
