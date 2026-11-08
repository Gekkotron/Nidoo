#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::Router;
use nidoo_api::WeatherView;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::weather::WeatherScreen;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

fn full_weather() -> WeatherView {
    let day = |d: u32| {
        serde_json::json!({
            "date": format!("2099-01-0{d}"), "condition": "rain",
            "min": 9.0, "max": 13.0, "precipitation_probability": 80.0,
            "apparent_max": 10.0, "precipitation": 4.5, "wind_speed_max": 33.0,
            "wind_gusts_max": 61.0, "wind_direction": 90.0, "humidity": 77.0,
            "pressure": 1002.0, "cloud_cover": 88.0, "uv_index_max": 2.0
        })
    };
    serde_json::from_value(serde_json::json!({
        "place": {"name": "X", "admin": "Y"},
        "fetched_at": (js_sys::Date::now() / 1000.0) as u64, "stale": false,
        "current": {
            "temperature": 9.4, "apparent_temperature": 5.3, "humidity": 70.0,
            "cloud_cover": 81.0, "pressure": 1015.0, "wind_speed": 19.0,
            "wind_gusts": 48.0, "wind_direction": 246.0, "uv_index": 0.0,
            "condition": "showers", "daylight": true
        },
        "today": {
            "min": 9.0, "max": 14.0, "precipitation": 6.0,
            "precipitation_probability": 93.0, "wind_gusts_max": 73.0
        },
        "rain_next_hour": 0.0, "tonight_min": 9.0,
        "days": [day(2), day(3), day(4), day(5), day(6)]
    }))
    .expect("weather fixture")
}

async fn mount(weather: Option<WeatherView>, loaded: bool) -> web_sys::Element {
    set_url("/weather");
    mount_in(RwSignal::new(Lang::En), weather, loaded).await
}

/// Puts the page address back to `url`: the tests share one browser page and the router reads it.
fn set_url(url: &str) {
    let history = web_sys::window().unwrap().history().unwrap();
    history
        .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(url))
        .unwrap();
}

async fn mount_without_reset(weather: Option<WeatherView>) -> web_sys::Element {
    mount_in(RwSignal::new(Lang::En), weather, true).await
}

async fn mount_in(
    lang: RwSignal<Lang>,
    weather: Option<WeatherView>,
    loaded: bool,
) -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let house = House::new();
    house.weather.set(weather);
    house.weather_loaded.set(loaded);
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            nidoo_web::nav::install_nav();
            provide_context(house);
            provide_context(lang);
            view! { <Router><WeatherScreen /></Router> }
        },
    ));
    tick().await;
    host
}

#[wasm_bindgen_test]
async fn weather_page_shows_tiles_and_days() {
    let host = mount(Some(full_weather()), true).await;
    assert_eq!(
        host.query_selector_all(".weather-tile").unwrap().length(),
        8
    );
    assert_eq!(host.query_selector_all(".weather-day").unwrap().length(), 6);
    assert!(host.query_selector(".sheet__back").unwrap().is_some());
    let text = host.text_content().unwrap_or_default();
    assert!(text.contains("Open-Meteo"), "{text}");
    assert!(text.contains("No rain within the hour"), "{text}");
}

#[wasm_bindgen_test]
async fn weather_page_unavailable() {
    let host = mount(None, true).await;
    assert_eq!(
        host.query_selector_all(".weather-tile").unwrap().length(),
        0
    );
    let text = host.text_content().unwrap_or_default();
    assert!(text.contains("The weather is unavailable."), "{text}");
}

#[wasm_bindgen_test]
async fn weather_page_is_empty_before_the_first_fetch() {
    let host = mount(None, false).await;
    let text = host.text_content().unwrap_or_default();
    assert!(!text.contains("unavailable"), "{text}");
    assert!(host.query_selector(".sheet__back").unwrap().is_some());
}

#[wasm_bindgen_test]
async fn weather_page_back_link_is_compact() {
    let host = mount(Some(full_weather()), true).await;
    let back = host.query_selector(".sheet__back").unwrap().unwrap();
    let page = host.query_selector(".weather-page").unwrap().unwrap();
    assert!(back.get_bounding_client_rect().width() < page.get_bounding_client_rect().width());
}

/// Units follow a no-break space in both languages, and the page flips language in place.
#[wasm_bindgen_test]
async fn the_tiles_use_no_break_units_and_follow_the_language() {
    let lang = RwSignal::new(Lang::Fr);
    let host = mount_in(lang, Some(full_weather()), true).await;
    let fr = host.text_content().unwrap_or_default();
    for unit in ["19\u{A0}km/h", "70\u{A0}%", "1015\u{A0}hPa", "6,0\u{A0}mm"] {
        assert!(fr.contains(unit), "{unit} in {fr}");
    }
    for ascii in ["19 km/h", "70 %", "6,0 mm"] {
        assert!(!fr.contains(ascii), "{ascii} in {fr}");
    }
    assert!(fr.contains("Vent"), "{fr}");
    assert!(fr.contains("risque 93\u{A0}%"), "{fr}");

    lang.set(Lang::En);
    tick().await;
    let en = host.text_content().unwrap_or_default();
    assert!(en.contains("Wind"), "{en}");
    assert!(en.contains("6.0\u{A0}mm"), "{en}");
    assert!(en.contains("93\u{A0}% chance"), "{en}");
    assert!(!en.contains("Vent"), "{en}");
}

fn chosen_cards(host: &web_sys::Element) -> Vec<String> {
    let nodes = host.query_selector_all(".weather-day--on").unwrap();
    (0..nodes.length())
        .map(|i| nodes.item(i).unwrap().text_content().unwrap_or_default())
        .collect()
}

#[wasm_bindgen_test]
async fn the_today_card_comes_first_and_is_chosen_by_default() {
    let host = mount(Some(full_weather()), true).await;
    let cards = host.query_selector_all(".weather-day").unwrap();
    let first = cards.item(0).unwrap();
    assert!(first.text_content().unwrap().contains("Today"));
    let first: web_sys::Element = first.unchecked_into();
    assert_eq!(first.get_attribute("aria-pressed").as_deref(), Some("true"));
    assert_eq!(chosen_cards(&host).len(), 1);
    let text = host.text_content().unwrap_or_default();
    assert!(text.contains("Now"), "{text}");
    assert!(text.contains("No rain within the hour"), "{text}");
}

#[wasm_bindgen_test]
async fn a_day_in_the_query_shows_that_days_figures() {
    set_url("/weather?day=2099-01-03");
    let host = mount_without_reset(Some(full_weather())).await;
    let text = host.text_content().unwrap_or_default();
    // 2099-01-03 is a Saturday.
    assert!(text.contains("Saturday 3"), "{text}");
    assert!(text.contains("33\u{A0}km/h"), "{text}");
    assert!(text.contains("4.5\u{A0}mm"), "{text}");
    assert!(text.contains("61\u{A0}km/h"), "{text}");
    assert!(!text.contains("No rain within the hour"), "{text}");
    assert!(!text.contains("Tonight"), "{text}");
    let chosen = chosen_cards(&host);
    assert_eq!(chosen.len(), 1);
    assert!(chosen[0].contains("Sat 3"), "{chosen:?}");
    assert!(host.query_selector(".sheet__back").unwrap().is_some());
}

#[wasm_bindgen_test]
async fn an_unknown_day_in_the_query_shows_today() {
    set_url("/weather?day=2000-01-01");
    let host = mount_without_reset(Some(full_weather())).await;
    let text = host.text_content().unwrap_or_default();
    assert!(text.contains("No rain within the hour"), "{text}");
    assert!(chosen_cards(&host)[0].contains("Today"));
}

#[wasm_bindgen_test]
async fn tapping_a_card_switches_the_day_and_tapping_today_returns() {
    let host = mount(Some(full_weather()), true).await;
    let cards = host.query_selector_all(".weather-day").unwrap();
    let card = |i| -> web_sys::HtmlElement { cards.item(i).unwrap().unchecked_into() };
    card(3).click();
    tick().await;
    tick().await;
    let text = host.text_content().unwrap_or_default();
    assert!(text.contains("Sunday 4"), "{text}");
    assert!(!text.contains("No rain within the hour"), "{text}");
    let search = web_sys::window().unwrap().location().search().unwrap();
    assert_eq!(search, "?day=2099-01-04");

    let cards = host.query_selector_all(".weather-day").unwrap();
    let today: web_sys::HtmlElement = cards.item(0).unwrap().unchecked_into();
    today.click();
    tick().await;
    tick().await;
    let text = host.text_content().unwrap_or_default();
    assert!(text.contains("No rain within the hour"), "{text}");
    assert!(chosen_cards(&host)[0].contains("Today"));
    set_url("/");
}
