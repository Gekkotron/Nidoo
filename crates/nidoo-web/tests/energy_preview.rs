#![cfg(target_arch = "wasm32")]

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_api::{EnergyView, HomeSettings};
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::settings::SettingsPageScreen;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;
wasm_bindgen_test_configure!(run_in_browser);

async fn mount(stored: EnergyView, lang: Lang) -> web_sys::Element {
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
            Some("/settings/appearance"),
        )
        .unwrap();
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            let house = House::new();
            house.home_settings.set(HomeSettings {
                energy_view: stored,
                ..Default::default()
            });
            provide_context(house);
            provide_context(Toasts::new());
            provide_context(RwSignal::new(lang));
            provide_context(RwSignal::new(nidoo_web::theme::Theme::Auto));
            nidoo_web::nav::install_nav();
            view! {
                <Router>
                    <Routes fallback=|| "fallback">
                        <Route path=path!("/settings/:page") view=SettingsPageScreen />
                    </Routes>
                </Router>
            }
        },
    ));
    tick().await;
    host
}

fn options(host: &web_sys::Element) -> Vec<web_sys::HtmlElement> {
    let group = host
        .query_selector(".segmented[aria-label='Energy view']")
        .unwrap()
        .expect("the Energy view group");
    let nodes = group.query_selector_all(".segmented__option").unwrap();
    (0..nodes.length())
        .map(|i| nodes.item(i).unwrap().unchecked_into())
        .collect()
}

#[wasm_bindgen_test]
async fn appearance_offers_three_energy_views() {
    let host = mount(EnergyView::Classic, Lang::En).await;
    let texts: Vec<String> = options(&host)
        .iter()
        .map(|o| o.text_content().unwrap_or_default())
        .collect();
    assert_eq!(texts, ["Classic", "Flux", "Orbite"]);
}

#[wasm_bindgen_test]
async fn choosing_orbite_saves_and_switches_the_preview() {
    let stub = FetchStub::install();
    let stored = HomeSettings {
        energy_view: EnergyView::Orbite,
        ..Default::default()
    };
    stub.reply(
        "PUT",
        "/api/v1/home/settings",
        200,
        &serde_json::to_string(&stored).unwrap(),
    );
    let host = mount(EnergyView::Classic, Lang::En).await;
    assert!(
        host.query_selector(".energy-preview[data-view='classic']")
            .unwrap()
            .is_some()
    );
    options(&host)[2].click();
    // The stub logs a request only once it has read the body: wait for it, not a fixed delay.
    let put = stub.wait_for("PUT").await.expect("the view is saved");
    assert!(
        put.body.contains("\"energy_view\":\"orbite\""),
        "{}",
        put.body
    );
    tick().await;
    TimeoutFuture::new(50).await;
    assert_eq!(stub.requests().len(), 1);
    assert!(
        host.query_selector(".energy-preview[data-view='orbite']")
            .unwrap()
            .is_some()
    );
}

#[wasm_bindgen_test]
async fn preview_follows_the_stored_view() {
    let host = mount(EnergyView::Flux, Lang::En).await;
    assert!(
        host.query_selector(".energy-preview[data-view='flux']")
            .unwrap()
            .is_some()
    );
}

#[wasm_bindgen_test]
async fn preview_is_hidden_from_screen_readers() {
    let host = mount(EnergyView::Orbite, Lang::Fr).await;
    let svg = host
        .query_selector(".energy-preview svg")
        .unwrap()
        .expect("the preview drawing");
    assert_eq!(svg.get_attribute("aria-hidden").as_deref(), Some("true"));
    assert!(svg.query_selector("circle").unwrap().is_some());
    let caption = host
        .query_selector(".energy-preview__caption")
        .unwrap()
        .unwrap();
    assert_eq!(
        caption.text_content().unwrap(),
        "Aperçu avec des valeurs d'exemple"
    );
}
