#![cfg(target_arch = "wasm32")]
//! `/history/:id` over a calendar period: the unit chips, the `‹ name ›` row, and the year compare.

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
use serde_json::json;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

fn sensor() -> EntityView {
    EntityView {
        id: "sensor.t".into(),
        device_id: Some("probe".into()),
        name: "Probe".into(),
        device_name: Some("Probe".into()),
        label: Some("Probe".into()),
        kind: Kind::Sensor,
        component: Kind::Sensor.component().to_string(),
        protocol_kind: Kind::Sensor,
        device_class: Some("temperature".into()),
        unit: Some("°C".into()),
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
        .set([sensor()].into_iter().map(|e| (e.id.clone(), e)).collect());
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

async fn wait_until(what: &str, host: &web_sys::Element, stub: &FetchStub, ok: impl Fn() -> bool) {
    for _ in 0..120 {
        if ok() {
            settle().await;
            return;
        }
        gloo_timers::future::TimeoutFuture::new(25).await;
    }
    let urls: Vec<_> = stub.requests().into_iter().map(|r| r.url).collect();
    panic!("waited for {what}: {urls:?} {}", host.inner_html());
}

fn search() -> String {
    web_sys::window().unwrap().location().search().unwrap()
}

/// Answers a window request with or without the trailing `&` gloo-net may leave on a query.
fn stub_window(stub: &FetchStub, path: &str, query: &str, body: &str) {
    stub.reply(
        "GET",
        &format!("/api/v1/history/{path}?entity=sensor.t{query}"),
        200,
        body,
    );
    stub.reply(
        "GET",
        &format!("/api/v1/history/{path}?entity=sensor.t{query}&"),
        200,
        body,
    );
}

fn stub_page(stub: &FetchStub) {
    let extent = r#"{"first_date":"2022-01-01","today":"2026-11-26"}"#;
    for url in [
        "/api/v1/history/extent?entity=sensor.t",
        "/api/v1/history/extent?entity=sensor.t&",
    ] {
        stub.reply("GET", url, 200, extent);
    }
    let series = json!({"window": 600, "points": [
        [1_700_000_000, 19.0, 18.0, 20.0], [1_700_086_400, 21.0, 20.0, 22.0]
    ]})
    .to_string();
    let fields = json!({"from": 1_700_000_000, "to": 1_700_172_800, "tracks": []}).to_string();
    for (from, to, upto) in [
        ("2026-11-01", "2026-11-30", ""),
        ("2026-10-01", "2026-10-31", ""),
        ("2025-10-01", "2025-10-31", ""),
    ] {
        let q = format!("&from={from}&to={to}{upto}");
        stub_window(stub, "series", &q, &series);
        stub_window(stub, "fields", &q, &fields);
    }
}

fn name(host: &web_sys::Element) -> String {
    texts(host, ".period-bar__name")
        .into_iter()
        .next()
        .unwrap_or_default()
}

#[wasm_bindgen_test]
async fn a_month_steps_back_and_compares_with_a_year() {
    let stub = FetchStub::install();
    stub_page(&stub);
    let m = mount(Lang::En, "/history/sensor.t?period=month");
    wait_until("November 2026", &m.host, &stub, || {
        name(&m.host) == "November 2026" && !all(&m.host, ".period-bar__compare select").is_empty()
    })
    .await;
    assert_eq!(texts(&m.host, ".period-bar__now"), ["This month"]);
    assert_eq!(
        texts(&m.host, ".history-detail__chips .chip--selected"),
        ["Month"]
    );

    all(&m.host, "button[aria-label='Previous period']")[0].click();
    wait_until("October 2026", &m.host, &stub, || {
        name(&m.host) == "October 2026"
    })
    .await;
    assert!(search().contains("at=2026-10-01"), "{}", search());

    let select: web_sys::HtmlSelectElement = all(&m.host, ".period-bar__compare select")
        .remove(0)
        .unchecked_into();
    select.set_value("1");
    select
        .dispatch_event(&web_sys::Event::new("change").unwrap())
        .unwrap();
    wait_until("the 2025 request", &m.host, &stub, || {
        stub.requests().iter().any(|r| {
            r.url
                .starts_with("/api/v1/history/series?entity=sensor.t&from=2025-10-01&to=2025-10-31")
        })
    })
    .await;
    assert!(search().contains("vs=1"), "{}", search());
}

#[wasm_bindgen_test]
async fn a_legacy_range_link_selects_its_unit() {
    let stub = FetchStub::install();
    stub_page(&stub);
    let m = mount(Lang::En, "/history/sensor.t?range=7d");
    wait_until("the Week chip", &m.host, &stub, || {
        texts(&m.host, ".history-detail__chips .chip--selected") == ["Week"]
    })
    .await;
}

#[wasm_bindgen_test]
async fn the_first_month_of_data_cannot_step_back() {
    let stub = FetchStub::install();
    stub_page(&stub);
    let m = mount(Lang::En, "/history/sensor.t?period=month&at=2022-01-01");
    let next = "button[aria-label='Next period']";
    let prev = "button[aria-label='Previous period']";
    wait_until("the extent", &m.host, &stub, || {
        all(&m.host, next)
            .first()
            .is_some_and(|b| !b.has_attribute("disabled"))
    })
    .await;
    assert_eq!(name(&m.host), "January 2022");
    assert!(all(&m.host, prev)[0].has_attribute("disabled"));
}

#[wasm_bindgen_test]
async fn custom_opens_the_picker_from_the_hour_views() {
    let stub = FetchStub::install();
    stub_page(&stub);
    let m = mount(Lang::En, "/history/sensor.t?period=1h");
    wait_until("the chips", &m.host, &stub, || {
        !all(&m.host, ".history-detail__chips .chip").is_empty()
    })
    .await;
    all(&m.host, ".history-detail__chips .chip")
        .into_iter()
        .find(|c| c.text_content().unwrap_or_default().starts_with("Custom"))
        .unwrap()
        .click();
    wait_until("the picker", &m.host, &stub, || {
        !all(&m.host, ".period-pop").is_empty()
    })
    .await;
}
