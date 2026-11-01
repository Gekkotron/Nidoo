#![cfg(target_arch = "wasm32")]

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::tick;
use nidoo_api::SetupStep;
use nidoo_web::i18n::Lang;
use nidoo_web::setup::services::ServicesStep;
use nidoo_web::setup::wizard::WizardNav;
use nidoo_web::setup::{Applying, SetupState};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

struct Mounted {
    host: web_sys::Element,
    _handle: Box<dyn std::any::Any>,
}

fn mount() -> Mounted {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let step = RwSignal::new(SetupStep::Services);
    let applying = Applying::new(|_| {});
    let handle = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(RwSignal::new(Lang::En));
        provide_context(WizardNav { step });
        provide_context(applying);
        provide_context(SetupState(RwSignal::new(None)));
        view! { <ServicesStep /> }
    });
    Mounted {
        host,
        _handle: Box::new(handle),
    }
}

fn click(host: &web_sys::Element, selector: &str) {
    let el: web_sys::HtmlElement = host
        .query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no {selector}"))
        .unchecked_into();
    el.click();
}

fn type_into(host: &web_sys::Element, selector: &str, text: &str) {
    let input: web_sys::HtmlInputElement = host
        .query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no {selector}"))
        .unchecked_into();
    input.set_value(text);
    let event = web_sys::Event::new("input").unwrap();
    input.dispatch_event(&event).unwrap();
}

async fn settle() {
    for _ in 0..8 {
        TimeoutFuture::new(15).await;
        tick().await;
    }
}

fn put_body(stub: &FetchStub) -> serde_json::Value {
    let put = stub
        .requests()
        .into_iter()
        .find(|r| r.method == "PUT")
        .expect("a PUT");
    serde_json::from_str(&put.body).unwrap()
}

#[wasm_bindgen_test]
async fn frigate_save_keeps_the_fields_it_does_not_show() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        "/api/v1/settings/frigate",
        200,
        r#"{"url":"http://192.0.2.4:5000","user":"viewer","password":{"set":true},"media_url":null,"bridge":true,"labels":["person"],"cameras":[],"zones":["porch"],"cooldown":90,"severity":"alert","ttl":7,"topic_prefix":"frigate","live":false}"#,
    );
    stub.reply(
        "PUT",
        "/api/v1/settings/frigate",
        200,
        r#"{"restarting":true}"#,
    );
    let m = mount();
    settle().await;
    click(&m.host, "#svc-cameras .svc__head");
    settle().await;
    type_into(&m.host, "#frigate-url", "http://192.0.2.5:5000");
    type_into(&m.host, "#frigate-password", "hunter2");
    tick().await;
    click(&m.host, "#svc-cameras .svc__save");
    settle().await;
    let body = put_body(&stub);
    assert_eq!(body["url"], "http://192.0.2.5:5000");
    assert_eq!(body["password"], "hunter2");
    assert_eq!(body["zones"], serde_json::json!(["porch"]));
    assert_eq!(body["cooldown"], 90);
    assert_eq!(body["severity"], "alert");
    assert_eq!(body["ttl"], 7);
    assert_eq!(body["topic_prefix"], "frigate");
}

#[wasm_bindgen_test]
async fn an_empty_secret_field_keeps_the_stored_key() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        "/api/v1/settings/solcast",
        200,
        r#"{"enabled":false,"sites":["abcd-1234"],"daily_limit":10,"api_key":{"set":true}}"#,
    );
    stub.reply(
        "PUT",
        "/api/v1/settings/solcast",
        200,
        r#"{"restarting":true}"#,
    );
    let m = mount();
    settle().await;
    click(&m.host, "#svc-solar .svc__head");
    settle().await;
    click(&m.host, "#svc-solar button.switch");
    tick().await;
    click(&m.host, "#svc-solar .svc__save");
    settle().await;
    let body = put_body(&stub);
    assert_eq!(body["enabled"], true);
    assert_eq!(body["sites"], serde_json::json!(["abcd-1234"]));
    let key = &body["api_key"];
    assert_eq!(*key, serde_json::json!(""));
}

#[wasm_bindgen_test]
async fn push_upload_sends_the_file_text() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        "/api/v1/settings/push",
        200,
        r#"{"service_account":{"set":false},"project_id":null,"prefix":"oikos/push","legacy_prefix":"legacy","retry_interval":30,"max_retries":5,"history_size":100,"history_days":7}"#,
    );
    stub.reply(
        "PUT",
        "/api/v1/settings/push",
        200,
        r#"{"restarting":true}"#,
    );
    let m = mount();
    settle().await;
    click(&m.host, "#svc-push .svc__head");
    settle().await;
    let text = r#"{"project_id":"demo-123"}"#;
    let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(text));
    let file = web_sys::File::new_with_str_sequence(&parts, "key.json").unwrap();
    let transfer = web_sys::DataTransfer::new().unwrap();
    transfer.items().add_with_file(&file).unwrap();
    let input: web_sys::HtmlInputElement = m
        .host
        .query_selector("#push-file")
        .unwrap()
        .unwrap()
        .unchecked_into();
    input.set_files(transfer.files().as_ref());
    input
        .dispatch_event(&web_sys::Event::new("change").unwrap())
        .unwrap();
    settle().await;
    click(&m.host, "#svc-push .svc__save");
    settle().await;
    let body = put_body(&stub);
    assert_eq!(body["service_account"], text);
    assert_eq!(body["prefix"], "oikos/push");
    assert_eq!(body["history_days"], 7);
}

const FRIGATE_TWO_CAMERAS: &str = r#"{"url":"http://192.0.2.4:5000","user":null,"password":{"set":true},"media_url":null,"bridge":false,"labels":[],"cameras":["front","back"],"zones":[],"cooldown":60,"severity":"alert","ttl":7,"topic_prefix":"frigate","live":false}"#;

fn chip_pressed(host: &web_sys::Element, name: &str) -> Option<String> {
    let chips = host
        .query_selector_all("#svc-cameras .svc__chips .chip")
        .unwrap();
    for i in 0..chips.length() {
        let el: web_sys::Element = chips.get(i).unwrap().unchecked_into();
        if el.text_content().unwrap().trim() == name {
            return el.get_attribute("aria-pressed");
        }
    }
    None
}

fn click_chip(host: &web_sys::Element, name: &str) {
    let chips = host
        .query_selector_all("#svc-cameras .svc__chips .chip")
        .unwrap();
    for i in 0..chips.length() {
        let el: web_sys::HtmlElement = chips.get(i).unwrap().unchecked_into();
        if el.text_content().unwrap().trim() == name {
            el.click();
            return;
        }
    }
    panic!("no chip {name}");
}

#[wasm_bindgen_test]
async fn a_deselected_stored_camera_stays_and_can_come_back() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/settings/frigate", 200, FRIGATE_TWO_CAMERAS);
    stub.reply(
        "PUT",
        "/api/v1/settings/frigate",
        200,
        r#"{"restarting":true}"#,
    );
    let m = mount();
    settle().await;
    click(&m.host, "#svc-cameras .svc__head");
    settle().await;
    click_chip(&m.host, "back");
    settle().await;
    assert_eq!(chip_pressed(&m.host, "back").as_deref(), Some("false"));
    click_chip(&m.host, "front");
    settle().await;
    assert_eq!(chip_pressed(&m.host, "front").as_deref(), Some("false"));
    click_chip(&m.host, "back");
    click_chip(&m.host, "front");
    settle().await;
    click(&m.host, "#svc-cameras .svc__save");
    settle().await;
    let mut cams: Vec<String> = serde_json::from_value(put_body(&stub)["cameras"].clone()).unwrap();
    cams.sort();
    assert_eq!(cams, ["back", "front"]);
}

#[wasm_bindgen_test]
async fn frigate_422_shows_as_received() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/settings/frigate", 200, FRIGATE_TWO_CAMERAS);
    stub.reply(
        "PUT",
        "/api/v1/settings/frigate",
        422,
        r#"{"error":"Enter the password for the new address"}"#,
    );
    let m = mount();
    settle().await;
    click(&m.host, "#svc-cameras .svc__head");
    settle().await;
    type_into(&m.host, "#frigate-url", "http://192.0.2.9:5000");
    tick().await;
    click(&m.host, "#svc-cameras .svc__save");
    settle().await;
    let text = m.host.text_content().unwrap();
    assert!(
        text.contains("Enter the password for the new address"),
        "{text}"
    );
}

#[wasm_bindgen_test]
async fn a_file_without_project_id_is_not_sent() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        "/api/v1/settings/push",
        200,
        r#"{"service_account":{"set":false},"project_id":null,"prefix":"oikos/push","legacy_prefix":"legacy","retry_interval":30,"max_retries":5,"history_size":100,"history_days":7}"#,
    );
    let m = mount();
    settle().await;
    click(&m.host, "#svc-push .svc__head");
    settle().await;
    let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(r#"{"type":"x"}"#));
    let file = web_sys::File::new_with_str_sequence(&parts, "key.json").unwrap();
    let transfer = web_sys::DataTransfer::new().unwrap();
    transfer.items().add_with_file(&file).unwrap();
    let input: web_sys::HtmlInputElement = m
        .host
        .query_selector("#push-file")
        .unwrap()
        .unwrap()
        .unchecked_into();
    input.set_files(transfer.files().as_ref());
    input
        .dispatch_event(&web_sys::Event::new("change").unwrap())
        .unwrap();
    settle().await;
    click(&m.host, "#svc-push .svc__save");
    settle().await;
    assert!(stub.requests().iter().all(|r| r.method != "PUT"));
    let text = m.host.text_content().unwrap();
    assert!(text.contains("not a service-account key"), "{text}");
}

#[wasm_bindgen_test]
async fn a_double_click_on_save_sends_one_put() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        "/api/v1/settings/solcast",
        200,
        r#"{"enabled":false,"sites":["abcd-1234"],"daily_limit":10,"api_key":{"set":true}}"#,
    );
    stub.reply_after(
        "PUT",
        "/api/v1/settings/solcast",
        200,
        r#"{"restarting":true}"#,
        60,
    );
    let m = mount();
    settle().await;
    click(&m.host, "#svc-solar .svc__head");
    settle().await;
    click(&m.host, "#svc-solar .svc__save");
    click(&m.host, "#svc-solar .svc__save");
    settle().await;
    let puts = stub
        .requests()
        .into_iter()
        .filter(|r| r.method == "PUT")
        .count();
    assert_eq!(puts, 1);
}
