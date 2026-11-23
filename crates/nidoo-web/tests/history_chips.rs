#![cfg(target_arch = "wasm32")]
//! `/history/:id` for a vacuum: one chart, with a chip per value the hub reports.

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_api::EntityView;
use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::history_detail::HistoryDetailScreen;
use nidoo_web::theme::Theme;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

/// The request as the browser reports it: gloo-net leaves a trailing `&` on a query.
fn fields_url(range: &str) -> String {
    format!("/api/v1/history/fields?entity=vacuum.roborock&range={range}&")
}

fn series_url(range: &str) -> String {
    format!("/api/v1/history/series?entity=vacuum.roborock&range={range}&")
}

fn now() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

fn vacuum() -> EntityView {
    EntityView {
        id: "vacuum.roborock".into(),
        device_id: Some("roborock".into()),
        name: "Roborock".into(),
        device_name: Some("Roborock".into()),
        label: Some("Roborock".into()),
        kind: Kind::Vacuum,
        component: Kind::Vacuum.component().to_string(),
        protocol_kind: Kind::Vacuum,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "living".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState::default(),
    }
}

/// The hub's reply for a vacuum: its activity, battery and fan speed over the last 24 hours.
fn vacuum_fields(headline: bool) -> Value {
    let to = now();
    let from = to - 86_400;
    let mut tracks = Vec::new();
    if headline {
        tracks.push(json!({
            "data": {"type": "steps", "changes": [
                [from, "docked"], [from + 30_000, "cleaning"], [from + 50_000, "docked"]
            ]},
        }));
    }
    tracks.push(json!({
        "field": "battery", "unit": "%",
        "data": {"type": "number", "window": 60, "points": [
            [from + 60, 100.0, 100.0, 100.0], [from + 31_000, 52.0, 52.0, 52.0]
        ]},
    }));
    tracks.push(json!({
        "field": "fan_speed",
        "data": {"type": "steps", "changes": [[from, "turbo"]]},
    }));
    json!({"from": from, "to": to, "tracks": tracks})
}

struct Mounted {
    _app: Box<dyn std::any::Any>,
    host: web_sys::Element,
}

fn mount(lang: Lang, url: &str) -> Mounted {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    web_sys::window()
        .unwrap()
        .history()
        .unwrap()
        .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(url))
        .unwrap();
    let house = House::new();
    house
        .entities
        .set([vacuum()].into_iter().map(|e| (e.id.clone(), e)).collect());
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(RwSignal::new(lang));
        provide_context(RwSignal::new(Theme::Auto));
        nidoo_web::nav::install_nav();
        view! {
            <Router>
                <Routes fallback=|| "fallback">
                    <Route path=path!("/history/:id") view=HistoryDetailScreen />
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

async fn wait_for(host: &web_sys::Element, selector: &str, n: usize) {
    for _ in 0..80 {
        if all(host, selector).len() >= n {
            break;
        }
        gloo_timers::future::TimeoutFuture::new(25).await;
    }
    let found = all(host, selector).len();
    assert!(
        found >= n,
        "waited for {n} x {selector:?}, found {found}: {}",
        host.inner_html()
    );
    settle().await;
}

fn stub_range(stub: &FetchStub, range: &str, fields: &Value, series: &Value) {
    stub.reply("GET", &fields_url(range), 200, &fields.to_string());
    stub.reply("GET", &series_url(range), 200, &series.to_string());
}

fn empty_series() -> Value {
    json!({"window": 60, "points": []})
}

fn search() -> String {
    web_sys::window().unwrap().location().search().unwrap()
}

#[wasm_bindgen_test]
async fn a_vacuum_opens_on_battery_and_activity() {
    let stub = FetchStub::install();
    stub_range(&stub, "24h", &vacuum_fields(true), &empty_series());
    let m = mount(Lang::Fr, "/history/vacuum.roborock?range=24h");
    wait_for(&m.host, ".history-chart__chips .chip", 3).await;
    assert_eq!(
        texts(&m.host, ".history-chart__chips .chip"),
        ["Activité", "Batterie", "Vitesse d'aspiration"]
    );
    assert_eq!(
        texts(&m.host, ".history-chart__chips .chip[aria-pressed=true]"),
        ["Activité", "Batterie"]
    );
    assert_eq!(
        all(&m.host, ".history-chart__line").len(),
        1,
        "battery line drawn"
    );
    assert!(all(&m.host, ".field-tracks").is_empty());
}

#[wasm_bindgen_test]
async fn tapping_a_word_chip_swaps_the_word_and_writes_the_url() {
    let stub = FetchStub::install();
    stub_range(&stub, "24h", &vacuum_fields(true), &empty_series());
    let m = mount(Lang::Fr, "/history/vacuum.roborock?range=24h");
    wait_for(&m.host, ".history-chart__chips .chip", 3).await;
    all(&m.host, ".history-chart__chips .chip")[2].click();
    wait_for(&m.host, ".history-chart__chips .chip[aria-pressed=true]", 2).await;
    assert_eq!(
        texts(&m.host, ".history-chart__chips .chip[aria-pressed=true]"),
        ["Batterie", "Vitesse d'aspiration"]
    );
    let q = search();
    assert!(
        q.contains("show=battery%2Cfan_speed") || q.contains("show=battery,fan_speed"),
        "{q}"
    );
}

#[wasm_bindgen_test]
async fn range_change_keeps_the_selection() {
    let stub = FetchStub::install();
    stub_range(&stub, "24h", &vacuum_fields(true), &empty_series());
    // The week's reply has no activity track, so its first chip is the series' "Valeur" rather than "Activité": seeing them
    // proves the 7 d reply has landed, not a stale day.
    stub_range(&stub, "7d", &vacuum_fields(false), &empty_series());
    let m = mount(
        Lang::Fr,
        "/history/vacuum.roborock?range=24h&show=battery,fan_speed",
    );
    wait_for(&m.host, ".history-chart__chips .chip", 3).await;
    let week = all(&m.host, ".history-detail__chips .chip")
        .into_iter()
        .find(|c| c.text_content().unwrap_or_default() == "7\u{a0}j")
        .expect("the 7 j chip");
    week.click();
    for _ in 0..80 {
        if texts(&m.host, ".history-chart__chips .chip")
            .first()
            .map(String::as_str)
            == Some("Valeur")
        {
            break;
        }
        gloo_timers::future::TimeoutFuture::new(25).await;
    }
    settle().await;
    assert_eq!(
        texts(&m.host, ".history-chart__chips .chip"),
        ["Valeur", "Batterie", "Vitesse d'aspiration"],
        "the 7 d reply's chips"
    );
    let q = search();
    assert!(q.contains("range=7d") && q.contains("show=battery"), "{q}");
    assert_eq!(
        texts(&m.host, ".history-chart__chips .chip[aria-pressed=true]"),
        ["Batterie", "Vitesse d'aspiration"]
    );
}

#[wasm_bindgen_test]
async fn activity_shades_the_cleaning_behind_the_battery_line() {
    let stub = FetchStub::install();
    stub_range(&stub, "24h", &vacuum_fields(true), &empty_series());
    let m = mount(Lang::Fr, "/history/vacuum.roborock?range=24h");
    wait_for(&m.host, ".history-chart__line", 1).await;
    assert_eq!(
        all(&m.host, ".history-chart__period").len(),
        1,
        "cleaning only"
    );
    assert_eq!(all(&m.host, ".history-chart__periods--behind").len(), 1);
    assert_eq!(
        texts(&m.host, ".history-chart__legend .history-chart__key-item"),
        ["À la base", "Nettoyage"]
    );
}

#[wasm_bindgen_test]
async fn a_word_alone_has_periods_and_no_line() {
    let stub = FetchStub::install();
    stub_range(&stub, "24h", &vacuum_fields(true), &empty_series());
    let m = mount(
        Lang::Fr,
        "/history/vacuum.roborock?range=24h&show=fan_speed",
    );
    wait_for(&m.host, ".history-chart__period", 1).await;
    assert!(all(&m.host, ".history-chart__line").is_empty());
    assert!(all(&m.host, ".history-chart__periods--behind").is_empty());
    assert_eq!(all(&m.host, ".history-chart__period").len(), 1);
    assert_eq!(
        texts(&m.host, ".history-chart__legend .history-chart__key-item"),
        ["Turbo"]
    );
}

#[wasm_bindgen_test]
async fn a_failed_fields_reply_with_no_series_offers_retry() {
    let stub = FetchStub::install();
    stub.reply("GET", &fields_url("24h"), 500, "{\"error\":\"boom\"}");
    stub.reply("GET", &series_url("24h"), 200, &empty_series().to_string());
    let m = mount(Lang::Fr, "/history/vacuum.roborock?range=24h");
    wait_for(&m.host, ".history-chart__err", 1).await;
    let retry = all(&m.host, ".history-chart__err button");
    assert_eq!(retry.len(), 1, "a Retry button");
    let count =
        |stub: &FetchStub, url: &str| stub.requests().iter().filter(|r| r.url == url).count();
    let (f0, s0) = (
        count(&stub, &fields_url("24h")),
        count(&stub, &series_url("24h")),
    );
    retry[0].click();
    for _ in 0..80 {
        if count(&stub, &fields_url("24h")) > f0 && count(&stub, &series_url("24h")) > s0 {
            break;
        }
        gloo_timers::future::TimeoutFuture::new(25).await;
    }
    assert!(count(&stub, &fields_url("24h")) > f0, "fields asked again");
    assert!(count(&stub, &series_url("24h")) > s0, "series asked again");
}

#[wasm_bindgen_test]
async fn a_plain_sensor_has_no_chip_row() {
    let stub = FetchStub::install();
    let to = now();
    let fields = json!({"from": to - 86_400, "to": to, "tracks": []});
    let series = json!({"window": 60, "points": [
        [to - 3_600, 20.0, 20.0, 20.0], [to - 60, 22.0, 22.0, 22.0]
    ]});
    stub_range(&stub, "24h", &fields, &series);
    let m = mount(Lang::En, "/history/vacuum.roborock?range=24h");
    wait_for(&m.host, ".history-chart__line", 1).await;
    assert!(all(&m.host, ".history-chart__chips").is_empty());
    assert_eq!(all(&m.host, ".history-chart__line").len(), 1);
}
