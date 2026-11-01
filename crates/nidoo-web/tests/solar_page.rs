#![cfg(target_arch = "wasm32")]

use chrono::{Duration, NaiveDate, TimeZone};
use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::Router;
use nidoo_api::SolarView;
use nidoo_web::house::House;
use nidoo_web::screens::solar::SolarScreen;
use nidoo_web::solar::measured::SolarMeasured;
use nidoo_web::solar::model::{day_bounds, local_date};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

fn now_s() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

/// The instant of a house-local wall-clock time on `date` (DST-safe: no `from + n * 3600`).
fn at(date: NaiveDate, h: u32, m: u32) -> u64 {
    chrono_tz::Europe::Paris
        .from_local_datetime(&date.and_hms_opt(h, m, 0).unwrap())
        .earliest()
        .expect("local time exists")
        .timestamp() as u64
}

/// Local noon today, inside the producing periods.
fn noon() -> u64 {
    at(local_date(chrono_tz::Europe::Paris, now_s()), 12, 0)
}

/// Today's and tomorrow's 48 half-hour periods (a bell from 08:00 to 18:00 local), 7 days, and
/// `sites` roofs.
fn solar(sites: usize) -> SolarView {
    let zone = chrono_tz::Europe::Paris;
    let today = local_date(zone, now_s());
    let bell = |date: NaiveDate| {
        let end_of_day = day_bounds(zone, date).1;
        (1..=48u64)
            .map(|i| {
                let end = if i == 48 {
                    end_of_day
                } else {
                    at(date, (i / 2) as u32, (i % 2 * 30) as u32)
                };
                let hour = (i as f64 - 0.5) / 2.0;
                let kw = if (8.0..18.0).contains(&hour) {
                    (3.0 - (hour - 13.0).abs() * 0.5).max(0.2)
                } else {
                    0.0
                };
                serde_json::json!({"end": end, "minutes": 30, "estimate": kw,
                                   "low": kw * 0.6, "high": kw * 1.3})
            })
            .collect::<Vec<_>>()
    };
    let mut periods = bell(today);
    periods.extend(bell(today + Duration::days(1)));
    let days: Vec<_> = (1..=7)
        .map(|n| {
            let d = (today + Duration::days(n)).format("%Y-%m-%d").to_string();
            serde_json::json!({"date": d, "estimate": 4.0 + n as f64, "low": 3.0, "high": 12.0})
        })
        .collect();
    let sites: Vec<_> = (0..sites)
        .map(|i| serde_json::json!({"index": i, "today": 5.1 + i as f64 * 2.2}))
        .collect();
    serde_json::from_value(serde_json::json!({
        "fetched_at": now_s(), "stale": false, "next_refresh_at": now_s() + 3600,
        "calls_today": 3, "daily_limit": 10,
        "now": {"power": 0.7, "remaining_today": 5.2},
        "today": {"date": today.format("%Y-%m-%d").to_string(), "estimate": 12.4,
                  "low": 9.8, "high": 14.1,
                  "peak": {"power": 3.0, "at": at(today, 13, 0)},
                  "best_hours": {"from": at(today, 10, 0), "to": at(today, 14, 0)}},
        "days": days, "sites": sites, "periods": periods,
        "history": {"forecasts": []}
    }))
    .expect("solar fixture")
}

async fn mount(solar: Option<SolarView>, loaded: bool) -> web_sys::Element {
    mount_with(solar, loaded, RwSignal::new(now_s())).await
}

async fn mount_with(
    solar: Option<SolarView>,
    loaded: bool,
    clock: RwSignal<u64>,
) -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let house = House::new();
    house.solar.set(solar);
    house.solar_loaded.set(loaded);
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            nidoo_web::nav::install_nav();
            provide_context(house);
            provide_context(nidoo_web::components::now::Now(clock));
            provide_context(SolarMeasured {
                today_kwh: RwSignal::new(Some(6.4)),
                periods: RwSignal::new(Vec::new()),
            });
            view! { <Router><SolarScreen /></Router> }
        },
    ));
    tick().await;
    host
}

fn count(host: &web_sys::Element, selector: &str) -> u32 {
    host.query_selector_all(selector).unwrap().length()
}

#[wasm_bindgen_test]
async fn solar_page_shows_top_curve_tiles_days_footer() {
    let host = mount_with(Some(solar(2)), true, RwSignal::new(noon())).await;
    assert!(host.query_selector(".solar-page__top").unwrap().is_some());
    assert!(
        host.query_selector(".solar-page__curve svg")
            .unwrap()
            .is_some()
    );
    assert!(host.query_selector(".sheet__back").unwrap().is_some());
    assert_eq!(count(&host, ".solar-day"), 7);
    let text = host
        .text_content()
        .unwrap_or_default()
        .replace('\u{a0}', " ");
    assert!(text.contains("Forecasts by Solcast"), "{text}");
    assert!(text.contains("Roof 2"), "{text}");
    assert!(
        text.contains("6.4 kWh produced · 12.4 kWh forecast"),
        "{text}"
    );
    let d = |sel: &str| {
        host.query_selector(sel)
            .unwrap()
            .and_then(|e| e.get_attribute("d"))
            .unwrap_or_default()
    };
    // No measured periods: the hours already past are a faint fill, with no production line.
    assert!(d(".solar-curve__measured").starts_with("M "), "no fill");
    assert_eq!(d(".solar-curve__measured-line"), "");
    assert!(count(&host, ".solar-curve__hour") >= 3);
    assert!(text.contains("Range 9.8–14.1 kWh"), "{text}");
    assert!(text.contains("Forecast peak"), "{text}");
    assert!(text.contains("Best hours"), "{text}");
    assert!(text.contains("10:00–14:00"), "{text}");
    assert!(text.contains("Solcast calls 3 / 10 today"), "{text}");
}

#[wasm_bindgen_test]
async fn tomorrow_tab_hides_measured_area() {
    let host = mount(Some(solar(2)), true).await;
    assert!(
        host.query_selector(".solar-curve__measured")
            .unwrap()
            .is_some()
    );
    let buttons = host.query_selector_all(".solar-tabs button").unwrap();
    assert_eq!(buttons.length(), 2);
    let tomorrow: web_sys::HtmlElement = buttons.item(1).unwrap().unchecked_into();
    tomorrow.click();
    tick().await;
    assert!(
        host.query_selector(".solar-curve__measured")
            .unwrap()
            .is_none()
    );
    assert!(host.query_selector(".solar-curve__band").unwrap().is_some());
    assert_eq!(
        tomorrow.get_attribute("aria-pressed").as_deref(),
        Some("true")
    );
}

/// The legend's entries, in order.
fn legend(host: &web_sys::Element) -> Vec<String> {
    let items = host.query_selector_all(".solar-legend li").unwrap();
    (0..items.length())
        .map(|i| items.item(i).unwrap().text_content().unwrap_or_default())
        .collect()
}

#[wasm_bindgen_test]
async fn legend_names_each_series_of_the_tab() {
    let host = mount_with(Some(solar(2)), true, RwSignal::new(noon())).await;
    // Nothing measured: the past hours are the forecast's estimate, not a production line.
    assert_eq!(
        legend(&host),
        ["Likely range", "Forecast", "Estimated so far", "Now"]
    );
    assert_eq!(count(&host, ".solar-legend__swatch"), 4);
    let buttons = host.query_selector_all(".solar-tabs button").unwrap();
    let tomorrow: web_sys::HtmlElement = buttons.item(1).unwrap().unchecked_into();
    tomorrow.click();
    tick().await;
    assert_eq!(legend(&host), ["Likely range", "Forecast"]);
}

#[wasm_bindgen_test]
async fn one_site_has_no_roofs_tile() {
    let host = mount(Some(solar(1)), true).await;
    let text = host.text_content().unwrap_or_default();
    assert!(!text.contains("Roof 1"), "{text}");
}

#[wasm_bindgen_test]
async fn solar_page_unavailable() {
    let host = mount(None, true).await;
    assert_eq!(count(&host, ".solar-tile"), 0);
    let text = host.text_content().unwrap_or_default();
    assert!(
        text.contains("The solar forecast is unavailable."),
        "{text}"
    );
    assert!(text.contains("secrets/solcast.key"), "{text}");
}

#[wasm_bindgen_test]
async fn nothing_before_first_fetch() {
    let host = mount(None, false).await;
    let text = host.text_content().unwrap_or_default();
    assert!(!text.contains("secrets/solcast.key"), "{text}");
    assert!(host.query_selector(".sheet__back").unwrap().is_some());
}

#[wasm_bindgen_test]
async fn clock_tick_keeps_the_page_nodes() {
    let clock = RwSignal::new(noon());
    let host = mount_with(Some(solar(2)), true, clock).await;
    let button = host
        .query_selector(".solar-tabs button")
        .unwrap()
        .expect("today button");
    let curve = host
        .query_selector(".solar-page__curve")
        .unwrap()
        .expect("curve card");
    let before = host.inner_html();
    clock.update(|t| *t += 60);
    tick().await;
    assert_ne!(before, host.inner_html(), "the tick changed nothing");
    assert!(button.is_connected() && curve.is_connected());
    let again = host.query_selector(".solar-tabs button").unwrap().unwrap();
    assert!(button.is_same_node(Some(&again)));
}
