#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::settings::SettingsScreen;
use nidoo_web::screens::system_settings::SystemSettingsPage;
use nidoo_web::theme::Theme;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

fn now() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

/// A system view as the hub serves it: 12 d 4 h up, a temperature sensor, every service up.
fn view_json() -> Value {
    json!({
        "startedAt": now() - (86_400 * 12 + 4 * 3600),
        "uptimePct": 99.8,
        "restarts": [
            {"startedAt": now() - 86_400, "downSecs": 120, "stop": "unexpected"},
            {"startedAt": now() - 2 * 86_400, "downSecs": 30, "stop": "clean"},
            {"startedAt": now() - 3 * 86_400, "downSecs": null, "stop": "first"},
        ],
        "cpu": {"serverPct": 23.0, "hubPct": 4.0},
        "ram": {"usedBytes": 5_368_709_120u64, "totalBytes": 17_179_869_184u64, "hubBytes": 180_000_000u64},
        "disk": {"usedBytes": 225_485_783_040u64, "totalBytes": 536_870_912_000u64, "dbBytes": 3_200_000_000u64},
        "temperatureC": 51.0,
        "devices": 84,
        "entities": 612,
        "retained": 2431,
        "commandsToday": 318,
        "services": [
            {"service": "broker", "state": "up", "since": now() - 3600},
            {"service": "zigbee2mqtt", "state": "up", "since": now() - 3600},
            {"service": "database", "state": "up", "since": now() - 3600},
        ],
        "notify": false,
    })
}

struct Mounted {
    _app: Box<dyn std::any::Any>,
    host: web_sys::Element,
}

fn mount(path_: &str) -> Mounted {
    nidoo_web::api::series_cache::forget_day_series();
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
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(RwSignal::new(Lang::En));
        provide_context(RwSignal::new(Theme::Auto));
        nidoo_web::nav::install_nav();
        view! {
            <Router>
                <Routes fallback=|| "fallback">
                    <Route path=path!("/settings") view=SettingsScreen />
                    <Route path=path!("/settings/system") view=SystemSettingsPage />
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

/// The tile whose caption is `label`.
fn tile(host: &web_sys::Element, label: &str) -> web_sys::HtmlElement {
    all(host, ".system__tile")
        .into_iter()
        .find(|t| {
            t.query_selector(".system__label")
                .unwrap()
                .and_then(|l| l.text_content())
                .as_deref()
                == Some(label)
        })
        .unwrap_or_else(|| panic!("no {label:?} tile in {:?}", texts(host, ".system__tile")))
}

fn part(tile: &web_sys::HtmlElement, class: &str) -> String {
    tile.query_selector(class)
        .unwrap()
        .and_then(|e| e.text_content())
        .unwrap_or_default()
}

#[wasm_bindgen_test]
async fn settings_list_shows_system_entry() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/system", 200, &view_json().to_string());
    let m = mount("/settings");
    wait_for(&m.host, "a[href=\"/settings/system\"]", 1).await;
    for _ in 0..40 {
        if texts(&m.host, "a[href=\"/settings/system\"]")[0].contains("Up 12 d 4 h") {
            break;
        }
        gloo_timers::future::TimeoutFuture::new(25).await;
    }
    let entry = texts(&m.host, "a[href=\"/settings/system\"]").remove(0);
    assert!(entry.contains("System"), "{entry}");
    assert!(entry.contains("Up 12 d 4 h"), "{entry}");
}

#[wasm_bindgen_test]
async fn tiles_show_values_and_units() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/system", 200, &view_json().to_string());
    let m = mount("/settings/system");
    wait_for(&m.host, ".system__tile", 10).await;
    let hero = texts(&m.host, ".system__hero").remove(0);
    assert!(hero.contains("Up 12 d 4 h"), "{hero}");
    assert!(hero.contains("99.8 % up over 30 days"), "{hero}");
    assert!(hero.contains("2 restarts"), "{hero}");
    let hero_link = all(&m.host, ".system__hero").remove(0);
    assert_eq!(
        hero_link.get_attribute("href").as_deref(),
        Some("/settings/system?show=restarts")
    );
    let cpu = tile(&m.host, "CPU");
    assert_eq!(part(&cpu, ".system__value"), "23 %");
    assert_eq!(part(&cpu, ".system__sub"), "hub 4 %");
    assert_eq!(
        cpu.get_attribute("href").as_deref(),
        Some("/settings/system?show=cpu")
    );
    let ram = tile(&m.host, "RAM");
    assert_eq!(part(&ram, ".system__value"), "31 %");
    assert_eq!(part(&ram, ".system__sub"), "hub 180 MB");
    let disk = tile(&m.host, "Disk");
    assert_eq!(part(&disk, ".system__value"), "42 %");
    assert_eq!(part(&disk, ".system__sub"), "database 3.2 GB");
    assert_eq!(
        part(&tile(&m.host, "Temperature"), ".system__value"),
        "51 °C"
    );
    let devices = tile(&m.host, "Devices");
    assert_eq!(part(&devices, ".system__value"), "84");
    assert_eq!(part(&devices, ".system__sub"), "612 entities");
    assert_eq!(
        part(&tile(&m.host, "Commands today"), ".system__value"),
        "318"
    );
    assert!(part(&tile(&m.host, "Retained"), ".system__value").ends_with("431"));
    assert_eq!(all(&m.host, ".dot--online").len(), 3);
}

#[wasm_bindgen_test]
async fn unknown_retained_reads_unknown() {
    let stub = FetchStub::install();
    let mut view = view_json();
    view["retained"] = Value::Null;
    stub.reply("GET", "/api/v1/system", 200, &view.to_string());
    let m = mount("/settings/system");
    wait_for(&m.host, ".system__tile", 10).await;
    assert_eq!(
        part(&tile(&m.host, "Retained"), ".system__value"),
        "unknown"
    );
}

#[wasm_bindgen_test]
async fn missing_temperature_hides_tile() {
    let stub = FetchStub::install();
    let mut view = view_json();
    view["temperatureC"] = Value::Null;
    stub.reply("GET", "/api/v1/system", 200, &view.to_string());
    let m = mount("/settings/system");
    wait_for(&m.host, ".system__tile", 9).await;
    let labels = texts(&m.host, ".system__tile .system__label");
    assert!(!labels.iter().any(|l| l == "Temperature"), "{labels:?}");
    assert_eq!(labels.len(), 9, "{labels:?}");
}

#[wasm_bindgen_test]
async fn down_service_is_red() {
    let stub = FetchStub::install();
    let mut view = view_json();
    view["services"][1]["state"] = json!("down");
    stub.reply("GET", "/api/v1/system", 200, &view.to_string());
    let m = mount("/settings/system");
    wait_for(&m.host, ".system__tile", 10).await;
    let z2m = tile(&m.host, "Zigbee2MQTT");
    assert!(z2m.query_selector(".dot--offline").unwrap().is_some());
    assert_eq!(part(&z2m, ".system__value"), "Down");
    assert_eq!(all(&m.host, ".dot--online").len(), 2);
}

fn requests_to(stub: &FetchStub, method: &str, url: &str) -> Vec<fetch_stub::Recorded> {
    stub.requests()
        .into_iter()
        .filter(|r| r.method == method && r.url == url)
        .collect()
}

#[wasm_bindgen_test]
async fn notify_switch_puts_and_reverts_on_503() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/system", 200, &view_json().to_string());
    stub.reply("PUT", "/api/v1/system/notify", 200, r#"{"notify":true}"#);
    let m = mount("/settings/system");
    wait_for(&m.host, "[role=switch]", 1).await;
    let switch = all(&m.host, "[role=switch]").remove(0);
    assert_eq!(
        switch.get_attribute("aria-checked").as_deref(),
        Some("false")
    );
    switch.click();
    settle().await;
    let puts = requests_to(&stub, "PUT", "/api/v1/system/notify");
    assert_eq!(puts.len(), 1);
    assert_eq!(
        serde_json::from_str::<Value>(&puts[0].body).unwrap(),
        json!({"notify": true})
    );
    assert_eq!(
        switch.get_attribute("aria-checked").as_deref(),
        Some("true")
    );

    // The database is down: the switch goes back and the page says why.
    stub.reply(
        "PUT",
        "/api/v1/system/notify",
        503,
        r#"{"error":"database unavailable"}"#,
    );
    switch.click();
    settle().await;
    assert_eq!(
        switch.get_attribute("aria-checked").as_deref(),
        Some("true")
    );
    let errors = texts(&m.host, ".home-settings__error");
    assert!(
        errors.iter().any(|e| e.contains("database unavailable")),
        "{errors:?}"
    );
}

#[wasm_bindgen_test]
async fn double_click_sends_one_put() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/system", 200, &view_json().to_string());
    stub.reply("PUT", "/api/v1/system/notify", 200, r#"{"notify":true}"#);
    let m = mount("/settings/system");
    wait_for(&m.host, "[role=switch]", 1).await;
    let switch = all(&m.host, "[role=switch]").remove(0);
    switch.click();
    switch.click();
    settle().await;
    assert_eq!(requests_to(&stub, "PUT", "/api/v1/system/notify").len(), 1);
    assert_eq!(
        switch.get_attribute("aria-checked").as_deref(),
        Some("true")
    );
}

/// A series as the hub serves it: `n` hourly points ending now, all at `level`.
fn series_json(level: f64, n: i64) -> String {
    let end = now() as i64;
    let points: Vec<Value> = (0..n)
        .map(|i| json!([end - (n - 1 - i) * 3600, level, level, level]))
        .collect();
    json!({"window": 86_400, "points": points}).to_string()
}

/// The address the page asks for (gloo leaves a trailing `&` on a query-less request builder).
fn series_url(id: &str, range: &str) -> String {
    format!("/api/v1/history/series?entity={id}&range={range}&")
}

#[wasm_bindgen_test]
async fn cpu_tile_opens_two_line_chart() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/system", 200, &view_json().to_string());
    stub.reply(
        "GET",
        &series_url("system_cpu", "24h"),
        200,
        &series_json(40.0, 6),
    );
    stub.reply(
        "GET",
        &series_url("system_cpu_hub", "24h"),
        200,
        &series_json(5.0, 6),
    );
    let m = mount("/settings/system?show=cpu");
    wait_for(&m.host, "path.history-chart__line", 2).await;
    assert_eq!(all(&m.host, "path.history-chart__line").len(), 2);
    let key = texts(&m.host, ".history-chart__key-item");
    assert_eq!(key.len(), 2, "{key:?}");
    assert_eq!(key[0].trim(), "Server");
    assert_eq!(key[1].trim(), "Hub");
    // The overview is gone; the range chips are the three the page offers.
    assert!(all(&m.host, ".system__tile").is_empty());
    let chips = texts(&m.host, ".system__chips .chip");
    assert_eq!(chips, ["24h", "7d", "30d"]);
    // A longer range asks the hub for it.
    stub.reply(
        "GET",
        &series_url("system_cpu", "7d"),
        200,
        &series_json(40.0, 6),
    );
    stub.reply(
        "GET",
        &series_url("system_cpu_hub", "7d"),
        200,
        &series_json(5.0, 6),
    );
    all(&m.host, ".system__chips .chip")[1].click();
    wait_for(&m.host, "path.history-chart__line", 2).await;
    assert_eq!(
        requests_to(&stub, "GET", &series_url("system_cpu_hub", "7d")).len(),
        1
    );
}

#[wasm_bindgen_test]
async fn temperature_chart_has_one_line() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/system", 200, &view_json().to_string());
    stub.reply(
        "GET",
        &series_url("system_temperature", "24h"),
        200,
        &series_json(51.0, 6),
    );
    let m = mount("/settings/system?show=temperature");
    wait_for(&m.host, "path.history-chart__line", 1).await;
    assert_eq!(all(&m.host, "path.history-chart__line").len(), 1);
    assert_eq!(texts(&m.host, ".history-chart__key-item").len(), 1);
}

#[wasm_bindgen_test]
async fn restarts_list_newest_first_with_unknown_downtime() {
    let stub = FetchStub::install();
    let mut view = view_json();
    view["restarts"][0]["downSecs"] = Value::Null;
    stub.reply("GET", "/api/v1/system", 200, &view.to_string());
    let m = mount("/settings/system?show=restarts");
    wait_for(&m.host, ".system__restart", 3).await;
    assert_eq!(
        texts(&m.host, ".system__restart-stop"),
        ["Unexpected stop", "Clean stop", "First start"]
    );
    assert_eq!(
        texts(&m.host, ".system__restart-down"),
        ["down ?", "down 30 s", ""]
    );
}

#[wasm_bindgen_test]
async fn tiles_draw_a_two_line_trend() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/system", 200, &view_json().to_string());
    stub.reply(
        "GET",
        &series_url("system_cpu", "24h"),
        200,
        &series_json(40.0, 6),
    );
    stub.reply(
        "GET",
        &series_url("system_cpu_hub", "24h"),
        200,
        &series_json(5.0, 6),
    );
    let m = mount("/settings/system");
    wait_for(&m.host, ".system__tile", 10).await;
    wait_for(&m.host, ".system__tile .spark__hub", 1).await;
    let cpu = tile(&m.host, "CPU");
    assert!(cpu.query_selector(".spark__server").unwrap().is_some());
    assert!(cpu.query_selector(".spark__hub").unwrap().is_some());
    // No history for the disk: its trend takes no room.
    let disk = tile(&m.host, "Disk");
    assert!(disk.query_selector(".spark__server").unwrap().is_none());
}
