#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_api::{FlowView, HomeSettings};
use nidoo_core::flow::wake_up::{WakeUp, to_flow};
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::wake_up_settings::WakeUpSettingsPage;
use nidoo_web::shell::Shell;
use nidoo_web::theme::Theme;
use serde_json::Value;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

const SETTINGS_URL: &str = "/api/v1/home/settings";

struct Mounted {
    /// Unmounts on drop, so the next test's router is the only one handling link clicks.
    _app: Box<dyn std::any::Any>,
    host: web_sys::Element,
    house: House,
}

fn managed_flow(id: &str, enabled: bool) -> FlowView {
    FlowView {
        definition: to_flow(id, "Weekday alarm", enabled, &WakeUp::default()),
        invalid: None,
        active_run: None,
        runs: Vec::new(),
    }
}

fn host_element() -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    host
}

fn mount_page(wake_up: bool, flows: Vec<FlowView>) -> Mounted {
    let host = host_element();
    web_sys::window()
        .unwrap()
        .history()
        .unwrap()
        .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/settings/wake-up"))
        .unwrap();
    let house = House::new();
    house.home_settings.set(HomeSettings {
        wake_up,
        ..Default::default()
    });
    house.flows.set(flows);
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(RwSignal::new(Lang::En));
        provide_context(RwSignal::new(Theme::Auto));
        nidoo_web::nav::install_nav();
        view! {
            <Router>
                <Routes fallback=|| "fallback">
                    <Route path=path!("/settings/wake-up") view=WakeUpSettingsPage />
                </Routes>
            </Router>
        }
    });
    Mounted {
        _app: Box::new(app),
        host,
        house,
    }
}

fn mount_shell() -> Mounted {
    let host = host_element();
    web_sys::window()
        .unwrap()
        .history()
        .unwrap()
        .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/"))
        .unwrap();
    let house = House::new();
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(RwSignal::new(Theme::Auto));
        provide_context(RwSignal::new(Lang::En));
        nidoo_web::nav::install_nav();
        view! {
            <Router>
                <Shell
                    climate_on=RwSignal::new(false)
                    alarm_armed=Signal::from(false)
                    solar_active=Signal::from(false)
                    alerts_badge=Signal::stored(None::<String>)
                >
                    <p>"child"</p>
                </Shell>
            </Router>
        }
    });
    Mounted {
        _app: Box::new(app),
        host,
        house,
    }
}

fn find(host: &web_sys::Element, selector: &str) -> Option<web_sys::HtmlElement> {
    host.query_selector(selector)
        .unwrap()
        .map(|e| e.unchecked_into())
}

fn one(host: &web_sys::Element, selector: &str) -> web_sys::HtmlElement {
    find(host, selector).unwrap_or_else(|| panic!("no {selector:?}"))
}

fn text(host: &web_sys::Element, selector: &str) -> String {
    one(host, selector).text_content().unwrap_or_default()
}

async fn settle() {
    tick().await;
    gloo_timers::future::TimeoutFuture::new(50).await;
    tick().await;
}

fn stored(wake_up: bool) -> String {
    serde_json::to_string(&HomeSettings {
        wake_up,
        ..Default::default()
    })
    .unwrap()
}

fn puts(stub: &FetchStub) -> Vec<Value> {
    stub.requests()
        .into_iter()
        .filter(|r| r.method == "PUT")
        .map(|r| serde_json::from_str(&r.body).unwrap())
        .collect()
}

#[wasm_bindgen_test]
async fn switching_on_saves_wake_up() {
    let stub = FetchStub::install();
    stub.reply("PUT", SETTINGS_URL, 200, &stored(true));
    let m = mount_page(false, Vec::new());
    settle().await;
    let switch = one(&m.host, ".wake-settings__switch");
    assert_eq!(
        switch.get_attribute("aria-checked").as_deref(),
        Some("false")
    );
    switch.click();
    let put = stub.wait_for("PUT").await.expect("a PUT");
    assert_eq!(put.url, SETTINGS_URL);
    let body: Value = serde_json::from_str(&put.body).unwrap();
    assert_eq!(body["wake_up"], true);
    settle().await;
    assert_eq!(
        switch.get_attribute("aria-checked").as_deref(),
        Some("true")
    );
    assert!(find(&m.host, ".wake-settings__banner").is_none());
    assert!(m.house.home_settings.get_untracked().wake_up);
}

#[wasm_bindgen_test]
async fn switching_on_rolls_back_when_the_hub_refuses() {
    let stub = FetchStub::install();
    stub.reply("PUT", SETTINGS_URL, 500, r#"{"error":"nope"}"#);
    let m = mount_page(false, Vec::new());
    settle().await;
    one(&m.host, ".wake-settings__switch").click();
    stub.wait_for("PUT").await.expect("a PUT");
    settle().await;
    assert!(!m.house.home_settings.get_untracked().wake_up);
    assert_eq!(
        one(&m.host, ".wake-settings__switch")
            .get_attribute("aria-checked")
            .as_deref(),
        Some("false")
    );
}

#[wasm_bindgen_test]
async fn switching_off_asks_before_saving() {
    let stub = FetchStub::install();
    stub.reply("PUT", SETTINGS_URL, 200, &stored(false));
    let m = mount_page(
        true,
        vec![
            managed_flow("wake-up-1", true),
            managed_flow("wake-up-2", false),
        ],
    );
    settle().await;
    one(&m.host, ".wake-settings__switch").click();
    settle().await;
    assert_eq!(
        text(&m.host, ".wake-settings__banner strong"),
        "1 active alarm will be turned off."
    );
    assert!(
        puts(&stub).is_empty(),
        "nothing is saved before the confirm"
    );
    assert!(m.house.home_settings.get_untracked().wake_up);

    // Cancel keeps it on and closes the question.
    one(&m.host, ".wake-settings__cancel").click();
    settle().await;
    assert!(find(&m.host, ".wake-settings__banner").is_none());
    assert!(puts(&stub).is_empty());

    one(&m.host, ".wake-settings__switch").click();
    settle().await;
    one(&m.host, ".wake-settings__confirm").click();
    stub.wait_for("PUT").await.expect("a PUT");
    settle().await;
    let sent = puts(&stub);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["wake_up"], false);
    assert!(find(&m.host, ".wake-settings__banner").is_none());
    assert!(!m.house.home_settings.get_untracked().wake_up);
}

#[wasm_bindgen_test]
async fn switching_off_with_no_active_alarm_saves_at_once() {
    let stub = FetchStub::install();
    stub.reply("PUT", SETTINGS_URL, 200, &stored(false));
    let m = mount_page(true, vec![managed_flow("wake-up-1", false)]);
    settle().await;
    one(&m.host, ".wake-settings__switch").click();
    let put = stub.wait_for("PUT").await.expect("a PUT");
    let body: Value = serde_json::from_str(&put.body).unwrap();
    assert_eq!(body["wake_up"], false);
    assert!(find(&m.host, ".wake-settings__banner").is_none());
}

#[wasm_bindgen_test]
async fn page_says_when_no_pinson_is_connected() {
    let _stub = FetchStub::install();
    let m = mount_page(true, Vec::new());
    settle().await;
    assert_eq!(
        text(&m.host, ".wake-settings__status"),
        "No media player connected"
    );
    assert_eq!(
        text(&m.host, ".wake-settings__link"),
        "Open the Wake-up page"
    );
}

#[wasm_bindgen_test]
async fn sidebar_shows_wake_up_only_while_it_is_on() {
    let _stub = FetchStub::install();
    let m = mount_shell();
    settle().await;
    assert!(find(&m.host, ".sidebar a[href=\"/wake-up\"]").is_none());
    m.house.home_settings.update(|s| s.wake_up = true);
    settle().await;
    assert_eq!(text(&m.host, ".sidebar a[href=\"/wake-up\"]"), "Wake-up");
    m.house.home_settings.update(|s| s.wake_up = false);
    settle().await;
    assert!(find(&m.host, ".sidebar a[href=\"/wake-up\"]").is_none());
}
