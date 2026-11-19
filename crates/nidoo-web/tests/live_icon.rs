#![cfg(target_arch = "wasm32")]
//! The live icons on the climate and vacuum card tiles: what each state draws, and that they move.

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::Router;
use nidoo_api::EntityView;
use nidoo_core::model::{
    Availability, Capabilities, ClimateState, EntityCategory, EntityState, Kind, StateValue,
    UiOverride, VacuumState,
};
use nidoo_web::commands::Pending;
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use nidoo_web::screens::home::card_for;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

fn entity(id: &str, kind: Kind, value: StateValue) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: "Thing".into(),
        device_name: None,
        label: None,
        kind,
        component: kind.component().to_string(),
        protocol_kind: kind,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "salon".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState {
            value,
            ..Default::default()
        },
    }
}

fn mount(entity: EntityView) -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(House::new());
            provide_context(Toasts::new());
            provide_context(Pending::new());
            view! { <Router>{card_for(entity)}</Router> }
        },
    ));
    host
}

fn has(host: &web_sys::Element, selector: &str) -> bool {
    host.query_selector(selector).unwrap().is_some()
}

fn count(host: &web_sys::Element, selector: &str) -> u32 {
    host.query_selector_all(selector).unwrap().length()
}

async fn sleep_ms(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

fn climate(mode: &str) -> EntityView {
    let mut e = entity(
        "climate.bedroom",
        Kind::Climate,
        StateValue::Climate(ClimateState {
            current: Some(22.0),
            target: Some(21.0),
            mode: Some(mode.into()),
            fan_mode: Some("low".into()),
            action: None,
        }),
    );
    e.state.availability = Availability::Online;
    e.capabilities.min_temp = Some(7.0);
    e.capabilities.max_temp = Some(30.0);
    e.capabilities.temp_step = Some(0.5);
    e
}

#[wasm_bindgen_test]
async fn cooling_tile_blows_cool_air() {
    let host = mount(climate("cool"));
    tick().await;
    assert!(has(&host, ".card__tile svg.live-icon.live-icon--cool"));
    assert_eq!(count(&host, ".card__tile .live-icon__flow"), 3);
    assert_eq!(count(&host, ".card__tile .live-icon__drop"), 0);
}

#[wasm_bindgen_test]
async fn heating_tile_is_heat_toned() {
    let host = mount(climate("heat"));
    tick().await;
    assert!(has(&host, ".card__tile svg.live-icon--heat"));
}

#[wasm_bindgen_test]
async fn drying_tile_drips() {
    let host = mount(climate("dry"));
    tick().await;
    assert_eq!(count(&host, ".card__tile .live-icon__drop"), 3);
    assert_eq!(count(&host, ".card__tile .live-icon__flow"), 0);
}

#[wasm_bindgen_test]
async fn off_tile_hangs_the_vane_closed() {
    let host = mount(climate("off"));
    tick().await;
    assert!(has(
        &host,
        ".card__tile svg.live-icon--off .live-icon__vane"
    ));
    assert_eq!(count(&host, ".card__tile .live-icon__flow"), 0);
}

#[wasm_bindgen_test]
async fn offline_climate_tile_is_off() {
    let mut e = climate("cool");
    e.state.availability = Availability::Offline;
    let host = mount(e);
    tick().await;
    assert!(has(&host, ".card__tile svg.live-icon--off"));
}

#[wasm_bindgen_test]
async fn airflow_moves_between_frames() {
    let host = mount(climate("cool"));
    tick().await;
    let flow = host.query_selector(".live-icon__flow").unwrap().unwrap();
    let before = flow.get_attribute("stroke-dashoffset");
    sleep_ms(250).await;
    assert_ne!(before, flow.get_attribute("stroke-dashoffset"));
}
