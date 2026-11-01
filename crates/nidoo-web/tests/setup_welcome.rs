#![cfg(target_arch = "wasm32")]

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::tick;
use nidoo_api::SetupStep;
use nidoo_web::i18n::Lang;
use nidoo_web::setup::welcome::WelcomeStep;
use nidoo_web::setup::wizard::WizardNav;
use nidoo_web::setup::{Applying, SetupState, reload_to};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

// gloo-net leaves a trailing `&` on a query URL, so stub keys end with one.
#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

const PLACE: &str = r#"[{"name":"Bordeaux","admin":"Nouvelle-Aquitaine","country":"FR","latitude":44.84,"longitude":-0.58,"timezone":"Europe/Paris"}]"#;
const STATUS: &str = r#"{"mode":"setup","step":"mqtt","finished":false,"bundled":false,"location":null,"mqtt":null,"database":null,"restarting":false}"#;

struct Mounted {
    host: web_sys::Element,
    step: RwSignal<SetupStep>,
    _handle: Box<dyn std::any::Any>,
}

fn mount() -> Mounted {
    mount_with(None)
}

fn mount_with(status: Option<&str>) -> Mounted {
    let status = status.map(|s| serde_json::from_str(s).unwrap());
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let step = RwSignal::new(SetupStep::Welcome);
    let handle = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(RwSignal::new(Lang::En));
        provide_context(WizardNav { step });
        provide_context(Applying::new(reload_to));
        provide_context(SetupState(RwSignal::new(status)));
        view! { <WelcomeStep /> }
    });
    Mounted {
        host,
        step,
        _handle: Box::new(handle),
    }
}

fn input(host: &web_sys::Element, selector: &str) -> web_sys::HtmlInputElement {
    host.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no {selector}"))
        .unchecked_into()
}

fn button(host: &web_sys::Element, selector: &str) -> web_sys::HtmlButtonElement {
    host.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no {selector}"))
        .unchecked_into()
}

async fn type_into(host: &web_sys::Element, selector: &str, value: &str) {
    let el = input(host, selector);
    el.set_value(value);
    let event = web_sys::Event::new_with_event_init_dict("input", &{
        let init = web_sys::EventInit::new();
        init.set_bubbles(true);
        init
    })
    .unwrap();
    el.dispatch_event(&event).unwrap();
    tick().await;
}

async fn settle() {
    for _ in 0..8 {
        TimeoutFuture::new(15).await;
        tick().await;
    }
}

#[wasm_bindgen_test]
async fn picking_a_place_saves_its_coordinates() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        "/api/v1/setup/geocode?city=Bordeaux&country=FR&",
        200,
        PLACE,
    );
    stub.reply("PUT", "/api/v1/setup/location", 200, STATUS);
    let m = mount();
    tick().await;
    type_into(&m.host, "#loc-city", "Bordeaux").await;
    type_into(&m.host, "#loc-country", "FR").await;
    button(&m.host, ".loc__search").click();
    settle().await;
    button(&m.host, ".loc__result").click();
    tick().await;
    let next = button(&m.host, ".loc__save");
    assert!(!next.disabled());
    next.click();
    settle().await;
    let put = stub
        .requests()
        .into_iter()
        .find(|r| r.method == "PUT")
        .expect("a PUT");
    assert_eq!(
        put.body,
        r#"{"city":"Bordeaux","country":"FR","latitude":44.84,"longitude":-0.58,"timezone":"Europe/Paris"}"#
    );
    assert_eq!(m.step.get_untracked(), SetupStep::Mqtt);
}

#[wasm_bindgen_test]
async fn geocoder_down_still_lets_coordinates_through() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        "/api/v1/setup/geocode?city=Bordeaux&",
        502,
        r#"{"error":"the city lookup failed"}"#,
    );
    let m = mount();
    tick().await;
    type_into(&m.host, "#loc-city", "Bordeaux").await;
    button(&m.host, ".loc__search").click();
    settle().await;
    let details: web_sys::HtmlDetailsElement = m
        .host
        .query_selector("details")
        .unwrap()
        .unwrap()
        .unchecked_into();
    assert!(details.open(), "the disclosure opens");
    assert!(m.host.text_content().unwrap().contains("No place found"));
    assert!(button(&m.host, ".loc__save").disabled());
    type_into(&m.host, "#loc-lat", "44.84").await;
    type_into(&m.host, "#loc-lon", "-0.58").await;
    type_into(&m.host, "#loc-tz", "Europe/Paris").await;
    assert!(!button(&m.host, ".loc__save").disabled());
    type_into(&m.host, "#loc-tz", "Europe/Nowhere").await;
    assert!(button(&m.host, ".loc__save").disabled());
    assert!(m.host.text_content().unwrap().contains("Unknown time zone"));
}

#[wasm_bindgen_test]
async fn a_restarting_save_shows_the_overlay_target_once() {
    let stub = FetchStub::install();
    let restarting = STATUS.replace(r#""restarting":false"#, r#""restarting":true"#);
    stub.reply("PUT", "/api/v1/setup/location", 200, &restarting);
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let applying = Applying::new(|_| {});
    let step = RwSignal::new(SetupStep::Welcome);
    let _h = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(RwSignal::new(Lang::En));
        provide_context(WizardNav { step });
        provide_context(applying);
        view! { <WelcomeStep /> }
    });
    tick().await;
    type_into(&host, "#loc-city", "Bordeaux").await;
    type_into(&host, "#loc-lat", "44.84").await;
    type_into(&host, "#loc-lon", "-0.58").await;
    type_into(&host, "#loc-tz", "Europe/Paris").await;
    button(&host, ".loc__save").click();
    settle().await;
    assert_eq!(
        applying.target.get_untracked().as_deref(),
        Some("/setup?step=mqtt")
    );
    assert_eq!(step.get_untracked(), SetupStep::Welcome);
}

const LOCATED: &str = r#"{"mode":"setup","step":"mqtt","finished":false,"bundled":false,"location":{"city":"Bordeaux","country":"FR","latitude":44.84,"longitude":-0.58,"timezone":"Europe/Paris"},"mqtt":null,"database":null,"restarting":false}"#;

#[wasm_bindgen_test]
async fn welcome_starts_from_the_stored_location() {
    let _stub = FetchStub::install();
    let m = mount_with(Some(LOCATED));
    tick().await;
    assert_eq!(input(&m.host, "#loc-city").value(), "Bordeaux");
    assert!(!button(&m.host, ".loc__save").disabled());
}

#[wasm_bindgen_test]
async fn a_pick_without_a_time_zone_opens_the_coordinates() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        "/api/v1/setup/geocode?city=Bordeaux&",
        200,
        r#"[{"name":"Bordeaux","admin":null,"country":"FR","latitude":44.84,"longitude":-0.58,"timezone":null}]"#,
    );
    let m = mount();
    tick().await;
    type_into(&m.host, "#loc-city", "Bordeaux").await;
    button(&m.host, ".loc__search").click();
    settle().await;
    button(&m.host, ".loc__result").click();
    tick().await;
    let details: web_sys::HtmlDetailsElement = m
        .host
        .query_selector("details.loc__details")
        .unwrap()
        .unwrap()
        .unchecked_into();
    assert!(details.open(), "the time zone field is visible");
    assert!(m.host.text_content().unwrap().contains("Unknown time zone"));
}

#[wasm_bindgen_test]
async fn a_restart_already_underway_shows_the_overlay_not_an_error() {
    let stub = FetchStub::install();
    stub.reply(
        "PUT",
        "/api/v1/setup/location",
        503,
        r#"{"error":"restarting"}"#,
    );
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let applying = Applying::new(|_| {});
    let step = RwSignal::new(SetupStep::Welcome);
    let status = serde_json::from_str(LOCATED).unwrap();
    let _h = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(RwSignal::new(Lang::En));
        provide_context(WizardNav { step });
        provide_context(applying);
        provide_context(SetupState(RwSignal::new(Some(status))));
        view! { <WelcomeStep /> }
    });
    tick().await;
    button(&host, ".loc__save").click();
    settle().await;
    assert_eq!(
        applying.target.get_untracked().as_deref(),
        Some("/setup?step=mqtt")
    );
    assert!(!host.text_content().unwrap().contains("Applying"));
    assert!(button(&host, ".loc__save").disabled(), "no second save");
}
