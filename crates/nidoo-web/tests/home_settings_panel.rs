#![cfg(target_arch = "wasm32")]

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::tick;
use nidoo_api::{EnergyView, EntityView, HomeSettings, WsEvent};
use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, StateValue, UiOverride};
use nidoo_web::components::now::Now;
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use nidoo_web::screens::home_settings_panel::HomeSettingsPanel;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;
wasm_bindgen_test_configure!(run_in_browser);

fn entity(id: &str, kind: Kind, device_class: Option<&str>) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: id.into(),
        device_name: None,
        label: None,
        kind,
        protocol_kind: kind,
        component: kind.component().to_string(),
        device_class: device_class.map(Into::into),
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "salon".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState {
            value: StateValue::Unknown,
            ..Default::default()
        },
    }
}

async fn mount() -> (web_sys::Element, House) {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let house = House::new();
    for e in [
        entity("alpha_select", Kind::Select, None),
        entity("virtual_mode", Kind::Select, None),
        entity("t_hall", Kind::Sensor, Some("temperature")),
    ] {
        house.apply(WsEvent::EntityUpserted(e));
    }
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(house);
            provide_context(Toasts::new());
            provide_context(Now(RwSignal::new(1_700_000_060)));
            view! { <HomeSettingsPanel /> }
        },
    ));
    tick().await;
    (host, house)
}

fn input(host: &web_sys::Element, id: &str) -> web_sys::HtmlInputElement {
    host.query_selector(&format!("#{id}"))
        .unwrap()
        .unwrap()
        .unchecked_into()
}

/// Opens the Mode picker (the panel's first picker field).
async fn open_mode(host: &web_sys::Element) {
    let button: web_sys::HtmlElement = host
        .query_selector("button.picker-field")
        .unwrap()
        .expect("mode field")
        .unchecked_into();
    button.click();
    tick().await;
    TimeoutFuture::new(30).await;
}

/// The entity ids listed by the open picker, in order.
fn mode_values(host: &web_sys::Element) -> Vec<String> {
    let options = host.query_selector_all("[role=option]").unwrap();
    (0..options.length())
        .filter_map(|i| {
            let id = options.item(i)?.unchecked_into::<web_sys::Element>().id();
            id.split_once("-opt-").map(|(_, e)| e.to_owned())
        })
        .collect()
}

#[wasm_bindgen_test]
async fn panel_lists_selects_and_saves_draft() {
    let (host, house) = mount().await;
    open_mode(&host).await;
    // Only selects (the sensor is no mode), by title, with a pinned None row.
    assert_eq!(mode_values(&host), ["alpha_select", "virtual_mode"]);
    assert!(host.query_selector("[id$=-pin]").unwrap().is_some());
    assert_eq!(input(&host, "home-Cold").placeholder(), "15");
    assert_eq!(input(&host, "home-Hot").placeholder(), "26");

    house.home_settings.set(HomeSettings {
        cold_below: Some(12.0),
        ..Default::default()
    });
    tick().await;
    tick().await;
    assert_eq!(input(&host, "home-Cold").value(), "12");
}

#[wasm_bindgen_test]
async fn picking_a_mode_sets_the_draft() {
    let (host, _house) = mount().await;
    open_mode(&host).await;
    let row: web_sys::HtmlElement = host
        .query_selector("[id$='-opt-virtual_mode']")
        .unwrap()
        .expect("virtual_mode row")
        .unchecked_into();
    row.click();
    tick().await;
    TimeoutFuture::new(30).await;
    let field = host.query_selector("button.picker-field").unwrap().unwrap();
    assert!(field.text_content().unwrap().contains("virtual_mode"));
}

#[wasm_bindgen_test]
async fn mode_list_hides_non_primary_selects_but_keeps_the_chosen_one() {
    let (host, house) = mount().await;
    let mut diag = entity("diag_select", Kind::Select, None);
    diag.category = EntityCategory::Diagnostic;
    house.apply(WsEvent::EntityUpserted(diag));
    tick().await;
    open_mode(&host).await;
    assert!(!mode_values(&host).contains(&"diag_select".to_owned()));
    house.home_settings.set(HomeSettings {
        mode_entity: Some("diag_select".into()),
        ..Default::default()
    });
    tick().await;
    tick().await;
    assert!(mode_values(&host).contains(&"diag_select".to_owned()));
}

#[wasm_bindgen_test]
async fn save_keeps_the_energy_view() {
    let stub = FetchStub::install();
    let stored = HomeSettings {
        energy_view: EnergyView::Flux,
        ..Default::default()
    };
    stub.reply(
        "PUT",
        "/api/v1/home/settings",
        200,
        &serde_json::to_string(&stored).unwrap(),
    );
    let (host, house) = mount().await;
    house.home_settings.set(stored);
    tick().await;
    host.query_selector(".home-settings__save")
        .unwrap()
        .unwrap()
        .unchecked_into::<web_sys::HtmlElement>()
        .click();
    tick().await;
    TimeoutFuture::new(50).await;
    let puts = stub.requests();
    assert_eq!(puts.len(), 1);
    assert!(
        puts[0].body.contains("\"energy_view\":\"flux\""),
        "{}",
        puts[0].body
    );
}
