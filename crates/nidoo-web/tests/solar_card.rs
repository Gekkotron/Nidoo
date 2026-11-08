#![cfg(target_arch = "wasm32")]

use chrono::{Duration, NaiveDate, TimeZone};
use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::Router;
use nidoo_api::{SolarView, WeatherView, WsEvent};
use nidoo_web::commands::Pending;
use nidoo_web::components::toast::Toasts;
use nidoo_web::home_header::view::HomeHeader;
use nidoo_web::house::House;
use nidoo_web::solar::measured::SolarMeasured;
use nidoo_web::solar::model::{day_bounds, local_date};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

fn now_s() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

fn weather(fetched_at: u64) -> WeatherView {
    serde_json::from_value(serde_json::json!({
        "place": {"name": "X"},
        "fetched_at": fetched_at, "stale": false,
        "current": {"temperature": 9.0, "condition": "showers", "daylight": true},
        "today": {"min": 5.0, "max": 14.0},
        "rain_next_hour": null, "tonight_min": null, "days": []
    }))
    .expect("weather fixture")
}

/// The instant of a house-local wall-clock time on `date` (DST-safe).
fn at(date: NaiveDate, h: u32, m: u32) -> u64 {
    chrono_tz::Europe::Paris
        .from_local_datetime(&date.and_hms_opt(h, m, 0).unwrap())
        .earliest()
        .expect("local time exists")
        .timestamp() as u64
}

/// Today's 48 half-hour periods (a bell from 08:00 to 18:00 local) and a tomorrow.
fn solar(fetched_at: u64, stale: bool) -> SolarView {
    let zone = chrono_tz::Europe::Paris;
    let today = local_date(zone, now_s());
    let (_, end_of_day) = day_bounds(zone, today);
    let periods: Vec<_> = (1..=48u64)
        .map(|i| {
            let end = if i == 48 {
                end_of_day
            } else {
                at(today, (i / 2) as u32, (i % 2 * 30) as u32)
            };
            let hour = (i as f64 - 0.5) / 2.0;
            let kw = if (8.0..18.0).contains(&hour) {
                (3.0 - (hour - 13.0).abs() * 0.5).max(0.2)
            } else {
                0.0
            };
            serde_json::json!({"end": end, "minutes": 30, "estimate": kw, "low": kw * 0.6, "high": kw * 1.3})
        })
        .collect();
    let next = (today + Duration::days(1)).format("%Y-%m-%d").to_string();
    serde_json::from_value(serde_json::json!({
        "fetched_at": fetched_at, "stale": stale, "next_refresh_at": null,
        "calls_today": 1, "daily_limit": 10,
        "now": {"power": null, "remaining_today": 5.2},
        "today": {"date": today.format("%Y-%m-%d").to_string(), "estimate": 12.4, "low": 8.0, "high": 15.0,
                  "peak": null, "best_hours": null},
        "days": [{"date": next, "estimate": 8.1, "low": 5.0, "high": 11.0}],
        "sites": [], "periods": periods,
        "history": {"forecasts": []}
    }))
    .expect("solar fixture")
}

fn measured_points() -> Vec<(u64, f64)> {
    let zone = chrono_tz::Europe::Paris;
    let today = local_date(zone, now_s());
    (20..=24u64)
        .map(|i| (at(today, (i / 2) as u32, (i % 2 * 30) as u32), 1.5))
        .collect()
}

async fn mount(
    weather: Option<WeatherView>,
    solar: Option<SolarView>,
    measured: Option<(f64, Vec<(u64, f64)>)>,
) -> (web_sys::Element, House) {
    mount_at(weather, solar, measured, None).await
}

/// Like `mount`, with the clock set to `clock` instead of the real time.
async fn mount_at(
    weather: Option<WeatherView>,
    solar: Option<SolarView>,
    measured: Option<(f64, Vec<(u64, f64)>)>,
    clock: Option<u64>,
) -> (web_sys::Element, House) {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let house = House::new();
    house.weather.set(weather);
    house.solar.set(solar);
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(house);
            provide_context(Toasts::new());
            provide_context(Pending::new());
            if let Some(t) = clock {
                provide_context(nidoo_web::components::now::Now(RwSignal::new(t)));
            }
            if let Some((kwh, periods)) = measured {
                provide_context(SolarMeasured {
                    today_kwh: RwSignal::new(Some(kwh)),
                    periods: RwSignal::new(periods),
                });
            }
            view! { <Router><HomeHeader /></Router> }
        },
    ));
    tick().await;
    (host, house)
}

/// The text of the first match, with no-break spaces turned into plain ones.
fn text(host: &web_sys::Element, selector: &str) -> String {
    host.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("{selector} missing"))
        .text_content()
        .unwrap_or_default()
        .replace('\u{a0}', " ")
}

#[wasm_bindgen_test]
async fn card_beside_weather_links_to_solar_page() {
    let (host, _) = mount(Some(weather(now_s())), Some(solar(now_s(), false)), None).await;
    let cards = host
        .query_selector(".home-header__cards")
        .unwrap()
        .expect("cards wrapper");
    assert!(
        cards
            .query_selector(".home-header__weather")
            .unwrap()
            .is_some()
    );
    let card = cards
        .query_selector(".solar-card")
        .unwrap()
        .expect("solar card");
    assert_eq!(card.tag_name().to_lowercase(), "a");
    assert_eq!(card.get_attribute("href").as_deref(), Some("/solar"));
    assert_eq!(
        card.get_attribute("aria-label").as_deref(),
        Some("Solar forecast")
    );
    let t = card.text_content().unwrap().replace('\u{a0}', " ");
    assert!(t.contains("12.4 kWh forecast"), "{t}");
    assert!(t.contains("≈ 5.2 kWh more expected"), "{t}");
    assert!(t.contains("Tomorrow 8.1 kWh (5–11)"), "{t}");
}

#[wasm_bindgen_test]
async fn card_hidden_without_solar_or_when_a_day_old() {
    let (host, _) = mount(Some(weather(now_s())), None, None).await;
    assert!(host.query_selector(".solar-card").unwrap().is_none());
    assert!(
        host.query_selector(".home-header__cards")
            .unwrap()
            .is_none()
    );
    assert!(
        host.query_selector(".home-header__weather")
            .unwrap()
            .is_some()
    );
    let (host, _) = mount(
        Some(weather(now_s())),
        Some(solar(now_s() - 86_401, false)),
        None,
    )
    .await;
    assert!(host.query_selector(".solar-card").unwrap().is_none());
    assert!(
        host.query_selector(".home-header__cards")
            .unwrap()
            .is_none()
    );
}

#[wasm_bindgen_test]
async fn card_with_measured_shows_measured_title() {
    let zone = chrono_tz::Europe::Paris;
    let noon = at(local_date(zone, now_s()), 12, 0);
    let (host, _) = mount_at(
        None,
        Some(solar(now_s(), false)),
        Some((6.4, measured_points())),
        Some(noon),
    )
    .await;
    let title = text(&host, ".solar-card__title");
    assert_eq!(title.trim(), "6.4 kWh produced · 12.4 kWh forecast");
    let line = text(&host, ".solar-card__line");
    assert!(line.contains("≈ 11.6 kWh by tonight"), "{line}");
    let line_path = host
        .query_selector(".solar-curve__measured-line")
        .unwrap()
        .expect("measured line");
    assert!(
        line_path
            .get_attribute("d")
            .unwrap_or_default()
            .starts_with("M ")
    );
    // Production is drawn as a line only: no fill stacked on the forecast.
    let path = host
        .query_selector(".solar-curve__measured")
        .unwrap()
        .expect("measured path");
    assert_eq!(path.get_attribute("d").unwrap_or_default(), "");
}

#[wasm_bindgen_test]
async fn card_without_measured_shows_forecast_fill() {
    // At noon: just after midnight no period is over yet, so there is no fill to see.
    let noon = at(local_date(chrono_tz::Europe::Paris, now_s()), 12, 0);
    let (host, _) = mount_at(None, Some(solar(now_s(), false)), None, Some(noon)).await;
    let title = text(&host, ".solar-card__title");
    assert!(title.trim().starts_with("≈ "), "{title}");
    assert!(title.contains("so far · 12.4 kWh forecast"), "{title}");
    let d = |sel: &str| {
        host.query_selector(sel)
            .unwrap()
            .and_then(|e| e.get_attribute("d"))
            .unwrap_or_default()
    };
    // The hours already past are a faint fill; there is no production line.
    assert!(!d(".solar-curve__measured").is_empty());
    assert_eq!(d(".solar-curve__measured-line"), "");
}

#[wasm_bindgen_test]
async fn stale_card_says_data_from() {
    let (host, _) = mount(None, Some(solar(now_s(), true)), None).await;
    let t = text(&host, ".solar-card");
    assert!(t.contains("Data from"), "{t}");
    assert!(!t.contains("Tomorrow"), "{t}");
}

#[wasm_bindgen_test]
async fn solar_changed_refetch_does_not_panic() {
    let (host, house) = mount(Some(weather(now_s())), Some(solar(now_s(), false)), None).await;
    house.apply(WsEvent::SolarChanged);
    tick().await;
    assert!(host.query_selector(".home-header").unwrap().is_some());
}

#[wasm_bindgen_test]
async fn clock_tick_keeps_the_card_node() {
    use nidoo_web::components::now::Now;
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let house = House::new();
    house.solar.set(Some(solar(now_s(), false)));
    let zone = chrono_tz::Europe::Paris;
    let clock = RwSignal::new(at(local_date(zone, now_s()), 12, 0));
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(house);
            provide_context(Toasts::new());
            provide_context(Pending::new());
            provide_context(Now(clock));
            view! { <Router><HomeHeader /></Router> }
        },
    ));
    tick().await;
    let before = host.query_selector(".solar-card").unwrap().expect("card");
    let html = host.inner_html();
    clock.update(|t| *t += 60);
    tick().await;
    assert_ne!(html, host.inner_html(), "the tick changed nothing");
    let after = host.query_selector(".solar-card").unwrap().expect("card");
    assert!(before.is_same_node(Some(&after)));
    assert!(before.is_connected());
}
