#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use oikos_api::{AreaView, WsEvent};
use oikos_web::house::House;
use oikos_web::i18n::Lang;
use oikos_web::screens::network_settings::NetworkSettingsPage;
use oikos_web::screens::settings::SettingsScreen;
use oikos_web::theme::Theme;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

/// A watch as the hub lists it.
fn watch(id: &str, name: &str, host: &str, device: Option<&str>, state: &str, since: u64) -> Value {
    json!({
        "id": id,
        "spec": {
            "name": name,
            "host": host,
            "intervalS": 60,
            "enabled": true,
            "area": null,
            "device": device,
        },
        "entity": format!("network_{id}"),
        "status": {"state": state, "since": since, "lastReply": null, "rttMs": null},
    })
}

fn now() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

fn two_watches() -> String {
    json!([
        watch(
            "roborock",
            "Roborock",
            "192.168.1.42",
            Some("roborock"),
            "connected",
            now()
        ),
        watch(
            "nas",
            "NAS",
            "nas.local",
            None,
            "disconnected",
            now() - 14 * 60
        ),
    ])
    .to_string()
}

struct Mounted {
    _app: Box<dyn std::any::Any>,
    host: web_sys::Element,
}

fn mount(path_: &str) -> Mounted {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    web_sys::window()
        .unwrap()
        .history()
        .unwrap()
        .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(path_))
        .unwrap();
    let house = House::new();
    house.apply(WsEvent::AreasChanged {
        areas: vec![AreaView {
            id: "office".into(),
            name: "Office".into(),
            icon: None,
            order: None,
            entity_count: 0,
            primary_count: 0,
            group_names: Default::default(),
            group_favorites: Default::default(),
        }],
    });
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(RwSignal::new(Lang::En));
        provide_context(RwSignal::new(Theme::Auto));
        oikos_web::nav::install_nav();
        view! {
            <Router>
                <Routes fallback=|| "fallback">
                    <Route path=path!("/settings") view=SettingsScreen />
                    <Route path=path!("/settings/network-presence") view=NetworkSettingsPage />
                </Routes>
            </Router>
        }
    });
    Mounted {
        _app: Box::new(app),
        host,
    }
}

fn all(host: &web_sys::Element, selector: &str) -> Vec<web_sys::HtmlElement> {
    let nodes = host.query_selector_all(selector).unwrap();
    (0..nodes.length())
        .map(|i| nodes.item(i).unwrap().unchecked_into())
        .collect()
}

fn texts(host: &web_sys::Element, selector: &str) -> Vec<String> {
    all(host, selector)
        .iter()
        .map(|e| e.text_content().unwrap_or_default())
        .collect()
}

async fn settle() {
    tick().await;
    gloo_timers::future::TimeoutFuture::new(50).await;
    tick().await;
}

/// Waits until `selector` matches at least `n` elements, then lets the handlers attach.
async fn wait_for(host: &web_sys::Element, selector: &str, n: usize) {
    for _ in 0..80 {
        if all(host, selector).len() >= n {
            break;
        }
        gloo_timers::future::TimeoutFuture::new(25).await;
    }
    let found = all(host, selector).len();
    assert!(found >= n, "waited for {n} x {selector:?}, found {found}");
    settle().await;
}

/// The button inside `scope` whose text is `label`.
fn button(scope: &web_sys::Element, label: &str) -> web_sys::HtmlElement {
    all(scope, "button")
        .into_iter()
        .find(|b| b.text_content().unwrap_or_default().trim() == label)
        .unwrap_or_else(|| panic!("no {label:?} button in {:?}", scope.text_content()))
}

/// Types `value` into the field of the label whose caption is `caption`.
fn type_into(scope: &web_sys::Element, caption: &str, value: &str) {
    let label = all(scope, "label.climate-field")
        .into_iter()
        .find(|l| l.text_content().unwrap_or_default().starts_with(caption))
        .unwrap_or_else(|| panic!("no {caption:?} field"));
    let input: web_sys::HtmlInputElement = label
        .query_selector("input")
        .unwrap()
        .unwrap()
        .unchecked_into();
    input.set_value(value);
    input
        .dispatch_event(
            &web_sys::Event::new_with_event_init_dict("input", &{
                let init = web_sys::EventInit::new();
                init.set_bubbles(true);
                init
            })
            .unwrap(),
        )
        .unwrap();
}

/// Picks the option with `value` in the select of the label whose caption is `caption`.
fn pick(scope: &web_sys::Element, caption: &str, value: &str) {
    let label = all(scope, "label.climate-field")
        .into_iter()
        .find(|l| l.text_content().unwrap_or_default().starts_with(caption))
        .unwrap_or_else(|| panic!("no {caption:?} field"));
    let select: web_sys::HtmlSelectElement = label
        .query_selector("select")
        .unwrap()
        .unwrap()
        .unchecked_into();
    select.set_value(value);
    select
        .dispatch_event(
            &web_sys::Event::new_with_event_init_dict("change", &{
                let init = web_sys::EventInit::new();
                init.set_bubbles(true);
                init
            })
            .unwrap(),
        )
        .unwrap();
}

fn body_of(stub: &FetchStub, method: &str, url: &str) -> Value {
    let request = stub
        .requests()
        .into_iter()
        .find(|r| r.method == method && r.url == url)
        .unwrap_or_else(|| panic!("no {method} {url} in {:?}", stub.requests()));
    serde_json::from_str(&request.body).unwrap_or(Value::Null)
}

#[wasm_bindgen_test]
async fn lists_watches_with_state() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/network", 200, &two_watches());
    let m = mount("/settings/network-presence");
    wait_for(&m.host, ".alarm-row", 2).await;
    let rows = texts(&m.host, ".alarm-row");
    assert!(
        rows[0].contains("Roborock") && rows[0].contains("device"),
        "{rows:?}"
    );
    assert!(
        rows[0].contains("192.168.1.42 · every 1 min · connected"),
        "{rows:?}"
    );
    assert!(rows[1].contains("disconnected for 14 min"), "{rows:?}");
    assert!(!rows[1].contains("device"), "{rows:?}");
    assert_eq!(all(&m.host, ".dot--online").len(), 1);
    assert_eq!(all(&m.host, ".dot--offline").len(), 1);
}

#[wasm_bindgen_test]
async fn add_host_puts_a_standalone_watch() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/network", 200, &two_watches());
    stub.reply(
        "PUT",
        "/api/v1/network/nas_2",
        200,
        &watch("nas_2", "NAS", "nas.local", None, "waiting", 0).to_string(),
    );
    let m = mount("/settings/network-presence");
    wait_for(&m.host, ".alarm-row", 2).await;
    let form = all(&m.host, "section.alarm-section").pop().unwrap();
    type_into(&form, "Name", "NAS");
    type_into(&form, "IP or hostname", "nas.local");
    pick(&form, "Ping every", "60");
    pick(&form, "Room", "office");
    button(&form, "Add").click();
    settle().await;
    let body = body_of(&stub, "PUT", "/api/v1/network/nas_2");
    assert_eq!(
        body,
        json!({"name": "NAS", "host": "nas.local", "intervalS": 60, "enabled": true, "area": "office", "device": null})
    );
}

#[wasm_bindgen_test]
async fn pause_from_edit_puts_enabled_false() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/network", 200, &two_watches());
    stub.reply(
        "PUT",
        "/api/v1/network/nas",
        200,
        &watch("nas", "NAS", "nas.local", None, "paused", 0).to_string(),
    );
    let m = mount("/settings/network-presence");
    wait_for(&m.host, ".alarm-row", 2).await;
    let row = all(&m.host, ".alarm-row").remove(1);
    button(&row, "Edit").click();
    settle().await;
    let switch = row.query_selector("[role=switch]").unwrap().unwrap();
    assert_eq!(
        switch.get_attribute("aria-checked").as_deref(),
        Some("true")
    );
    switch.unchecked_into::<web_sys::HtmlElement>().click();
    settle().await;
    button(&row, "Save").click();
    settle().await;
    let body = body_of(&stub, "PUT", "/api/v1/network/nas");
    assert_eq!(body["enabled"], false);
    assert_eq!(body["host"], "nas.local");
}

#[wasm_bindgen_test]
async fn remove_deletes() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/network", 200, &two_watches());
    stub.reply("DELETE", "/api/v1/network/nas", 204, "");
    let m = mount("/settings/network-presence");
    wait_for(&m.host, ".alarm-row", 2).await;
    let row = all(&m.host, ".alarm-row").remove(1);
    button(&row, "Edit").click();
    settle().await;
    button(&row, "Remove").click();
    settle().await;
    assert!(
        stub.requests()
            .iter()
            .any(|r| r.method == "DELETE" && r.url == "/api/v1/network/nas")
    );
}

#[wasm_bindgen_test]
async fn settings_list_shows_network_presence_entry() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/network", 200, &two_watches());
    let m = mount("/settings");
    wait_for(&m.host, "a[href=\"/settings/network-presence\"]", 1).await;
    let entry = texts(&m.host, "a[href=\"/settings/network-presence\"]").remove(0);
    assert!(entry.contains("Network presence"), "{entry}");
    assert!(entry.contains("2 watched"), "{entry}");
}

#[wasm_bindgen_test]
async fn cannot_ping_shows_the_hint_once() {
    let stub = FetchStub::install();
    let both = json!([
        watch("nas", "NAS", "nas.local", None, "cannot_ping", 0),
        watch("tv", "TV", "192.168.1.43", None, "cannot_ping", 0),
    ])
    .to_string();
    stub.reply("GET", "/api/v1/network", 200, &both);
    let m = mount("/settings/network-presence");
    wait_for(&m.host, ".alarm-row", 2).await;
    let hints = texts(&m.host, ".network-hint");
    assert_eq!(hints.len(), 1, "{hints:?}");
    assert!(
        hints[0].contains("The hub cannot send pings. On the server, run: sysctl"),
        "{hints:?}"
    );
}

#[wasm_bindgen_test]
async fn no_hint_while_pings_work() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/network", 200, &two_watches());
    let m = mount("/settings/network-presence");
    wait_for(&m.host, ".alarm-row", 2).await;
    assert!(texts(&m.host, ".network-hint").is_empty());
}
