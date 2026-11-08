#![cfg(target_arch = "wasm32")]

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::Router;
use nidoo_alarm::doc::AlarmSettings;
use nidoo_alarm::view::AlarmView;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::settings::SettingsScreen;
use nidoo_web::setup::SetupState;
use nidoo_web::setup::done::DoneStep;
use nidoo_web::setup::services::ReadTimeoutMs;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

const SKIPPED_KEY: &str = "nidoo.setup.skipped";

const STATUS: &str = r#"{"mode":"running","step":"done","finished":false,"bundled":false,
"location":{"city":"Bordeaux","country":"FR","latitude":44.84,"longitude":-0.58,"timezone":"Europe/Paris"},
"mqtt":{"host":"192.0.2.10","port":1883,"user":null,"password":{"set":false},"client_id":null,"z2m_base":null},
"database":{"host":"192.0.2.11","port":5432,"database":"nidoo","user":"nidoo","password":{"set":false}},
"restarting":false}"#;

fn mount_done() -> web_sys::Element {
    mount_done_with(None)
}

fn mount_done_with(timeout_ms: Option<u32>) -> web_sys::Element {
    let status = serde_json::from_str(STATUS).unwrap();
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(RwSignal::new(Lang::En));
            provide_context(House::new());
            if let Some(ms) = timeout_ms {
                provide_context(ReadTimeoutMs(ms));
            }
            provide_context(SetupState(RwSignal::new(Some(status))));
            view! { <Router><DoneStep /></Router> }
        },
    ));
    host
}

async fn settle() {
    for _ in 0..8 {
        TimeoutFuture::new(15).await;
        tick().await;
    }
}

fn storage() -> web_sys::Storage {
    web_sys::window().unwrap().local_storage().unwrap().unwrap()
}

fn pathname() -> String {
    web_sys::window().unwrap().location().pathname().unwrap()
}

#[wasm_bindgen_test]
async fn open_my_house_finishes_and_leaves() {
    let stub = FetchStub::install();
    stub.reply(
        "POST",
        "/api/v1/setup/finish",
        200,
        &STATUS.replace(r#""finished":false"#, r#""finished":true"#),
    );
    let window = web_sys::window().unwrap();
    let history = window.history().unwrap();
    let start = window.location().href().unwrap();
    let host = mount_done();
    settle().await;
    let open: web_sys::HtmlElement = host
        .query_selector(".done__open")
        .unwrap()
        .expect("Open my house")
        .unchecked_into();
    // Two quick clicks must send one request.
    open.click();
    open.click();
    settle().await;
    let posts = stub
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST" && r.url == "/api/v1/setup/finish")
        .count();
    let path = pathname();
    history
        .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&start))
        .unwrap();
    assert_eq!(posts, 1);
    assert_eq!(path, "/dashboards/house");
}

#[wasm_bindgen_test]
async fn finish_error_is_shown() {
    let stub = FetchStub::install();
    stub.reply(
        "POST",
        "/api/v1/setup/finish",
        500,
        r#"{"error":"disk full"}"#,
    );
    let host = mount_done();
    settle().await;
    let open: web_sys::HtmlElement = host
        .query_selector(".done__open")
        .unwrap()
        .unwrap()
        .unchecked_into();
    open.click();
    settle().await;
    let problem = host.query_selector(".loc__problem").unwrap().unwrap();
    assert!(problem.text_content().unwrap().contains("disk full"));
}

/// Answers every service read as "not set up", except climate (500) and cameras (500).
fn stub_services(stub: &FetchStub) {
    let off = r#"{"enabled":false}"#;
    stub.reply("GET", "/api/v1/settings/weather", 200, off);
    stub.reply(
        "GET",
        "/api/v1/settings/solcast",
        200,
        r#"{"enabled":false,"api_key":{"set":false}}"#,
    );
    stub.reply(
        "GET",
        "/api/v1/settings/push",
        200,
        r#"{"service_account":{"set":false},"project_id":null}"#,
    );
    stub.reply(
        "GET",
        "/api/v1/settings/climate",
        500,
        r#"{"error":"down"}"#,
    );
    stub.reply(
        "GET",
        "/api/v1/settings/frigate",
        500,
        r#"{"error":"down"}"#,
    );
    let alarm = AlarmView {
        enabled: false,
        entity_id: "alarm_control_panel.nidoo_alarm_house".into(),
        state: "disarmed".into(),
        mode: None,
        source: None,
        since: 0,
        deadline: None,
        cause: vec![],
        open_at_arm: vec![],
        settings: AlarmSettings::default(),
        settings_invalid: None,
        warnings: vec![],
    };
    stub.reply(
        "GET",
        "/api/v1/alarm",
        200,
        &serde_json::to_string(&alarm).unwrap(),
    );
}

#[wasm_bindgen_test]
async fn summary_shows_skipped_services() {
    let stub = FetchStub::install();
    stub_services(&stub);
    storage().set_item(SKIPPED_KEY, r#"["cameras"]"#).unwrap();
    let host = mount_done();
    settle().await;
    storage().remove_item(SKIPPED_KEY).unwrap();
    let text = |id: &str| {
        host.query_selector(&format!("#done-{id}"))
            .unwrap()
            .unwrap_or_else(|| panic!("no row {id}"))
            .text_content()
            .unwrap()
    };
    assert!(text("cameras").contains("Skipped"), "{}", text("cameras"));
    assert!(text("weather").contains("Not set up"));
    // A read that fails is not "Not set up".
    assert!(
        text("climate").contains("Couldn't check"),
        "{}",
        text("climate")
    );
    assert!(
        host.query_selector("#done-climate.done__row--off")
            .unwrap()
            .is_some()
    );
    assert!(text("location").contains("Bordeaux, FR"));
    assert!(text("mqtt").contains("192.0.2.10:1883"));
    assert!(text("database").contains("192.0.2.11:5432 / nidoo"));
}

#[wasm_bindgen_test]
async fn run_setup_again_is_listed() {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(RwSignal::new(Lang::En));
            provide_context(House::new());
            provide_context(RwSignal::new(nidoo_web::theme::Theme::Auto));
            view! { <Router><SettingsScreen /></Router> }
        },
    ));
    tick().await;
    assert!(host.query_selector("a[href=\"/setup\"]").unwrap().is_some());
}

fn row_text(host: &web_sys::Element, id: &str) -> String {
    host.query_selector(&format!("#done-{id}"))
        .unwrap()
        .unwrap_or_else(|| panic!("no row {id}"))
        .text_content()
        .unwrap()
}

#[wasm_bindgen_test]
async fn a_pending_read_shows_checking_then_unknown_after_the_timeout() {
    let stub = FetchStub::install();
    stub_services(&stub);
    stub.hang("GET", "/api/v1/settings/solcast");
    let host = mount_done_with(Some(200));
    TimeoutFuture::new(60).await;
    tick().await;
    assert!(
        row_text(&host, "solar").contains("Checking"),
        "{}",
        row_text(&host, "solar")
    );
    assert!(row_text(&host, "weather").contains("Not set up"));
    TimeoutFuture::new(300).await;
    tick().await;
    assert!(row_text(&host, "solar").contains("Couldn't check"));
}

#[wasm_bindgen_test]
async fn a_late_answer_after_the_timeout_corrects_the_row() {
    let stub = FetchStub::install();
    stub_services(&stub);
    stub.reply_after(
        "GET",
        "/api/v1/settings/solcast",
        200,
        r#"{"enabled":false,"api_key":{"set":false}}"#,
        500,
    );
    let host = mount_done_with(Some(150));
    TimeoutFuture::new(350).await;
    tick().await;
    assert!(row_text(&host, "solar").contains("Couldn't check"));
    TimeoutFuture::new(500).await;
    tick().await;
    assert!(
        row_text(&host, "solar").contains("Not set up"),
        "{}",
        row_text(&host, "solar")
    );
}

const MOBILE_HOME: &str =
    r#"{"gateway_url":null,"gateway_key":{"set":false},"gateway_ready":false}"#;
const MOBILE_REMOTE: &str = r#"{"gateway_url":"https://house.example.ts.net","gateway_key":{"set":true},"gateway_ready":true}"#;
const MOBILE_CODE: &str = r#"{"payload":"{\"v\":1,\"hub\":\"http://192.0.2.10:8081\"}"}"#;

fn stub_mobile(stub: &FetchStub, view: &str) {
    stub.reply("GET", "/api/v1/settings/mobile", 200, view);
    stub.reply("GET", "/api/v1/setup/mobile-code", 200, MOBILE_CODE);
}

fn html(host: &web_sys::Element, selector: &str) -> web_sys::HtmlElement {
    host.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("{selector} is missing"))
        .unchecked_into()
}

fn mobile_gets(stub: &FetchStub) -> usize {
    stub.requests()
        .iter()
        .filter(|r| r.method == "GET" && r.url == "/api/v1/setup/mobile-code")
        .count()
}

#[wasm_bindgen_test]
async fn done_shows_the_phone_block_blurred() {
    let stub = FetchStub::install();
    stub_mobile(&stub, MOBILE_HOME);
    let host = mount_done();
    settle().await;
    let code = html(&host, ".phone__code");
    assert!(code.class_list().contains("phone__code--hidden"));
    assert_eq!(
        html(&host, ".phone__toggle").text_content().unwrap().trim(),
        "Show code"
    );
    assert!(host.text_content().unwrap().contains("Connect your phone"));
}

#[wasm_bindgen_test]
async fn show_code_reveals_and_hide_blurs() {
    let stub = FetchStub::install();
    stub_mobile(&stub, MOBILE_HOME);
    let host = mount_done();
    settle().await;
    let toggle = html(&host, ".phone__toggle");
    toggle.click();
    settle().await;
    let code = html(&host, ".phone__code");
    assert!(!code.class_list().contains("phone__code--hidden"));
    assert!(code.query_selector("svg").unwrap().is_some());
    toggle.click();
    settle().await;
    assert!(
        html(&host, ".phone__code")
            .class_list()
            .contains("phone__code--hidden")
    );
}

#[wasm_bindgen_test]
async fn home_only_status_without_gateway() {
    let stub = FetchStub::install();
    stub_mobile(&stub, MOBILE_HOME);
    let host = mount_done();
    settle().await;
    assert_eq!(
        html(&host, ".phone__status").text_content().unwrap().trim(),
        "Home network only"
    );
}

#[wasm_bindgen_test]
async fn saving_the_gateway_refetches_the_code() {
    let stub = FetchStub::install();
    stub_mobile(&stub, MOBILE_HOME);
    stub.reply("PUT", "/api/v1/settings/mobile", 200, MOBILE_REMOTE);
    let host = mount_done();
    settle().await;
    let before = mobile_gets(&stub);
    let url: web_sys::HtmlInputElement = html(&host, "#phone-gateway-url").unchecked_into();
    url.set_value("https://house.example.ts.net");
    url.dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();
    let key: web_sys::HtmlInputElement = html(&host, "#phone-gateway-key").unchecked_into();
    key.set_value(&"ab".repeat(32));
    key.dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();
    settle().await;
    html(&host, ".phone__save").click();
    let put = stub.wait_for("PUT").await.expect("a PUT");
    assert_eq!(put.url, "/api/v1/settings/mobile");
    let body: serde_json::Value = serde_json::from_str(&put.body).unwrap();
    assert_eq!(body["gateway_url"], "https://house.example.ts.net");
    assert_eq!(body["gateway_key"], "ab".repeat(32));
    settle().await;
    assert!(mobile_gets(&stub) > before);
    assert_eq!(
        html(&host, ".phone__status").text_content().unwrap().trim(),
        "At home and away"
    );
}

#[wasm_bindgen_test]
async fn a_refusal_shows_inline() {
    let stub = FetchStub::install();
    stub_mobile(&stub, MOBILE_HOME);
    stub.reply(
        "PUT",
        "/api/v1/settings/mobile",
        422,
        r#"{"error":"The gateway key must be 64 hex characters"}"#,
    );
    let host = mount_done();
    settle().await;
    html(&host, ".phone__save").click();
    stub.wait_for("PUT").await.expect("a PUT");
    settle().await;
    let alert = html(&host, ".phone [role=alert]");
    assert!(
        alert
            .text_content()
            .unwrap()
            .contains("The gateway key must be 64 hex characters")
    );
}

#[wasm_bindgen_test]
async fn open_my_house_still_works_without_the_code() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/settings/mobile", 200, MOBILE_HOME);
    stub.reply(
        "GET",
        "/api/v1/setup/mobile-code",
        503,
        r#"{"error":"database unreachable"}"#,
    );
    let host = mount_done();
    settle().await;
    let alert = html(&host, ".phone [role=alert]");
    assert!(
        alert
            .text_content()
            .unwrap()
            .contains("database unreachable")
    );
    assert!(!html(&host, ".done__open").has_attribute("disabled"));
}
