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
use nidoo_web::live_icon::view::LivePicture;
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

fn mount_picture(entity: EntityView) -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(House::new());
            provide_context(Toasts::new());
            provide_context(Pending::new());
            view! { <LivePicture entity=entity /> }
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

fn vacuum(activity: &str) -> EntityView {
    let mut e = entity(
        "vacuum.robot",
        Kind::Vacuum,
        StateValue::Vacuum(VacuumState {
            state: Some(activity.into()),
            battery: Some(100),
            fan_speed: Some("turbo".into()),
        }),
    );
    e.state.availability = Availability::Online;
    e.capabilities.vacuum_actions = ["start", "pause", "return_to_base", "clean_spot"]
        .map(String::from)
        .to_vec();
    e.capabilities.fan_speeds = vec!["quiet".into(), "turbo".into()];
    e
}

#[wasm_bindgen_test]
async fn cleaning_tile_drives_rows() {
    let host = mount(vacuum("cleaning"));
    tick().await;
    assert!(has(
        &host,
        ".card__tile svg.live-icon--moving .live-icon__trail"
    ));
    let robot = host
        .query_selector(".card__tile .live-icon__robot")
        .unwrap()
        .unwrap();
    let before = robot.get_attribute("transform");
    sleep_ms(250).await;
    assert_ne!(before, robot.get_attribute("transform"));
}

#[wasm_bindgen_test]
async fn returning_tile_heads_for_the_dock() {
    let host = mount(vacuum("returning"));
    tick().await;
    assert!(has(
        &host,
        ".card__tile svg.live-icon--moving .live-icon__dock"
    ));
    assert!(!has(&host, ".card__tile .live-icon__trail"));
}

#[wasm_bindgen_test]
async fn docked_tile_charges() {
    let host = mount(vacuum("docked"));
    tick().await;
    assert!(has(
        &host,
        ".card__tile svg.live-icon--docked .live-icon__bolt"
    ));
}

#[wasm_bindgen_test]
async fn error_tile_is_still() {
    let host = mount(vacuum("error"));
    tick().await;
    assert!(has(
        &host,
        ".card__tile svg.live-icon--plain .live-icon__robot"
    ));
    assert!(!has(&host, ".card__tile .live-icon__trail"));
    assert!(!has(&host, ".card__tile .live-icon__bolt"));
}

#[wasm_bindgen_test]
async fn the_big_picture_is_the_room() {
    let host = mount(vacuum("cleaning"));
    tick().await;
    assert!(has(
        &host,
        ".vacuum__picture svg.live-icon--picture.live-icon--moving"
    ));
    assert!(!has(&host, "svg.robot"));
}

/// Loads the real stylesheets once, so computed colours are the ones a user sees.
fn with_styles() {
    let document = web_sys::window().unwrap().document().unwrap();
    if document
        .get_element_by_id("live-icon-test-styles")
        .is_some()
    {
        return;
    }
    let style = document.create_element("style").unwrap();
    style.set_id("live-icon-test-styles");
    style.set_text_content(Some(concat!(
        include_str!("../styles/tokens.css"),
        include_str!("../styles/main.css"),
        include_str!("../styles/cards.css"),
    )));
    document.head().unwrap().append_child(&style).unwrap();
}

fn computed(el: &web_sys::Element, property: &str) -> String {
    web_sys::window()
        .unwrap()
        .get_computed_style(el)
        .unwrap()
        .unwrap()
        .get_property_value(property)
        .unwrap()
}

#[wasm_bindgen_test]
async fn heating_air_stands_out_from_the_unit() {
    with_styles();
    let host = mount(climate("heat"));
    tick().await;
    let svg = host
        .query_selector(".card__tile svg.live-icon")
        .unwrap()
        .unwrap();
    let flow = host
        .query_selector(".card__tile .live-icon__flow")
        .unwrap()
        .unwrap();
    assert_ne!(computed(&svg, "color"), computed(&flow, "stroke"));
}

#[wasm_bindgen_test]
async fn a_cleaning_robot_stands_out_from_the_room() {
    with_styles();
    let host = mount(vacuum("cleaning"));
    tick().await;
    let svg = host
        .query_selector(".card__tile svg.live-icon")
        .unwrap()
        .unwrap();
    let robot = host
        .query_selector(".card__tile .live-icon__robot")
        .unwrap()
        .unwrap();
    assert_ne!(computed(&svg, "color"), computed(&robot, "color"));
}

#[wasm_bindgen_test]
async fn picture_draws_the_cool_unit() {
    let host = mount_picture(climate("cool"));
    tick().await;
    assert!(has(
        &host,
        "svg.live-icon.live-icon--picture.live-icon--cool"
    ));
    assert_eq!(count(&host, ".live-icon__flow"), 3);
    let svg = host.query_selector("svg").unwrap().unwrap();
    assert_eq!(svg.get_attribute("viewBox").as_deref(), Some("1 2.5 22 22"));
}

#[wasm_bindgen_test]
async fn picture_draws_the_cleaning_room() {
    let host = mount_picture(vacuum("cleaning"));
    tick().await;
    assert!(has(
        &host,
        "svg.live-icon--picture.live-icon--moving .live-icon__trail"
    ));
}

#[wasm_bindgen_test]
async fn picture_ids_differ_from_the_tile() {
    let host = mount(climate("cool"));
    let document = web_sys::window().unwrap().document().unwrap();
    let extra = document.create_element("div").unwrap();
    host.append_child(&extra).unwrap();
    let entity = climate("cool");
    std::mem::forget(leptos::mount::mount_to(
        extra.clone().unchecked_into(),
        move || view! { <LivePicture entity=entity /> },
    ));
    tick().await;
    let nodes = host
        .query_selector_all("mask[id], linearGradient[id]")
        .unwrap();
    let mut ids: Vec<String> = (0..nodes.length())
        .map(|i| {
            nodes
                .item(i)
                .unwrap()
                .unchecked_into::<web_sys::Element>()
                .id()
        })
        .collect();
    let total = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), total, "duplicate ids");
    assert!(count(&host, "mask") >= 2);
}
