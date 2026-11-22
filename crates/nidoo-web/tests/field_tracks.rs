#![cfg(target_arch = "wasm32")]
//! `/history/:id` for a vacuum: one row per value the hub reports, and the main chart steps
//! aside when the headline is a word it cannot draw.

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
const URL: &str = "/api/v1/history/fields?entity=vacuum.roborock&range=24h&";

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

fn mount(lang: Lang) -> Mounted {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    web_sys::window()
        .unwrap()
        .history()
        .unwrap()
        .replace_state_with_url(
            &wasm_bindgen::JsValue::NULL,
            "",
            Some("/history/vacuum.roborock?range=24h"),
        )
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

#[wasm_bindgen_test]
async fn a_vacuum_shows_a_row_per_value_and_hides_the_main_chart() {
    let stub = FetchStub::install();
    stub.reply("GET", URL, 200, &vacuum_fields(true).to_string());
    let m = mount(Lang::Fr);
    wait_for(&m.host, ".field-tracks__row", 3).await;
    assert_eq!(
        texts(&m.host, ".field-tracks__label"),
        ["Activité", "Batterie", "Vitesse d'aspiration"]
    );
    let words = texts(&m.host, ".field-tracks__word");
    assert!(words.contains(&"Nettoyage".to_owned()), "{words:?}");
    assert!(words.contains(&"À la base".to_owned()), "{words:?}");
    assert!(words.contains(&"Turbo".to_owned()), "{words:?}");
    assert_eq!(
        texts(&m.host, ".field-tracks__value")[1],
        "52\u{A0}%",
        "the latest battery reading"
    );
    assert!(all(&m.host, ".history-chart").is_empty());
}

#[wasm_bindgen_test]
async fn without_a_headline_track_the_main_chart_stays() {
    let stub = FetchStub::install();
    stub.reply("GET", URL, 200, &vacuum_fields(false).to_string());
    let m = mount(Lang::En);
    wait_for(&m.host, ".field-tracks__row", 2).await;
    assert_eq!(
        texts(&m.host, ".field-tracks__label"),
        ["Battery", "Fan speed"]
    );
    assert_eq!(all(&m.host, ".history-chart").len(), 1);
}

#[wasm_bindgen_test]
async fn no_tracks_leave_only_the_main_chart() {
    let stub = FetchStub::install();
    let to = now();
    stub.reply(
        "GET",
        URL,
        200,
        &json!({"from": to - 86_400, "to": to, "tracks": []}).to_string(),
    );
    let m = mount(Lang::En);
    settle().await;
    gloo_timers::future::TimeoutFuture::new(100).await;
    assert!(all(&m.host, ".field-tracks__row").is_empty());
    assert!(all(&m.host, ".field-tracks__shimmer").is_empty());
    assert_eq!(all(&m.host, ".history-chart").len(), 1);
}
