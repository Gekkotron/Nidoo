#![cfg(target_arch = "wasm32")]
//! The shared card frame: the room line follows where the card is shown, and the trailing
//! control never sits inside the header button.

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::Router;
use nidoo_api::{AreaView, EntityView};
use nidoo_core::model::{
    Capabilities, EntityCategory, EntityState, Kind, LightState, StateValue, UNASSIGNED_AREA,
    UiOverride,
};
use nidoo_web::commands::Pending;
use nidoo_web::components::card_frame::CardFrame;
use nidoo_web::components::card_place::{CardPlace, provide_card_place};
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use nidoo_web::screens::home::card_for;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

fn switch(area: &str) -> EntityView {
    EntityView {
        id: "switch.pump".into(),
        device_id: None,
        name: "Pump".into(),
        device_name: None,
        label: None,
        kind: Kind::Switch,
        component: Kind::Switch.component().to_string(),
        protocol_kind: Kind::Switch,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: area.into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState {
            value: StateValue::OnOff(false),
            ..Default::default()
        },
    }
}

fn salon() -> AreaView {
    AreaView {
        id: "salon".into(),
        name: "Salon".into(),
        icon: None,
        order: None,
        entity_count: 1,
        primary_count: 1,
        group_names: Default::default(),
        group_favorites: Default::default(),
    }
}

/// Mounts `view` with the contexts a card needs, the house knowing the room "Salon", and `place`
/// provided when given.
fn mount<V: IntoView + 'static>(
    place: Option<CardPlace>,
    view: impl FnOnce() -> V + Send + 'static,
) -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            let house = House::new();
            house.areas.set(vec![salon()]);
            provide_context(house);
            provide_context(Toasts::new());
            provide_context(Pending::new());
            if let Some(place) = place {
                provide_card_place(place);
            }
            view! { <Router>{view()}</Router> }
        },
    ));
    host
}

fn room_line(host: &web_sys::Element) -> Option<String> {
    host.query_selector(".card__room")
        .unwrap()
        .map(|e| e.text_content().unwrap_or_default())
}

#[wasm_bindgen_test]
async fn room_line_shows_in_overview() {
    let host = mount(None, || card_for(switch("salon")));
    tick().await;
    assert_eq!(room_line(&host).as_deref(), Some("Salon"));
    assert!(
        host.query_selector(".card__header .card__name")
            .unwrap()
            .is_some()
    );
}

#[wasm_bindgen_test]
async fn room_line_hidden_in_room_and_sheet() {
    for place in [CardPlace::Room, CardPlace::Sheet] {
        let host = mount(Some(place), || card_for(switch("salon")));
        tick().await;
        assert_eq!(room_line(&host), None, "{place:?}");
    }
}

#[wasm_bindgen_test]
async fn unassigned_has_no_room_line() {
    let host = mount(None, || card_for(switch(UNASSIGNED_AREA)));
    tick().await;
    assert_eq!(room_line(&host), None);
}

#[wasm_bindgen_test]
async fn trailing_sits_outside_the_header() {
    let host = mount(None, || {
        let trailing = view! { <button type="button" role="switch" class="probe"></button> };
        view! {
            <CardFrame
                entity=switch("salon")
                on_click=Callback::new(|()| {})
                trailing=trailing.into_any()
                state_line="Off".to_owned()
            >
                <span></span>
            </CardFrame>
        }
    });
    tick().await;
    assert!(
        host.query_selector(".card__header .card__trailing")
            .unwrap()
            .is_none()
    );
    assert!(
        host.query_selector(".card__header [role=switch]")
            .unwrap()
            .is_none()
    );
    assert!(
        host.query_selector(".card__trailing [role=switch]")
            .unwrap()
            .is_some()
    );
    let state = host.query_selector(".card__state").unwrap().unwrap();
    assert_eq!(state.text_content().unwrap(), "Off");
}

fn light(level: u8) -> EntityView {
    let mut e = switch("salon");
    e.id = "light.bed".into();
    e.name = "Bed".into();
    e.kind = Kind::Light;
    e.component = Kind::Light.component().to_string();
    e.protocol_kind = Kind::Light;
    e.capabilities.brightness = true;
    e.state.value = StateValue::Light(LightState {
        on: true,
        brightness: Some(level),
        ..Default::default()
    });
    e
}

#[wasm_bindgen_test]
async fn light_card_shows_state_line_and_percent() {
    let host = mount(None, || card_for(light(36)));
    tick().await;
    let state = host.query_selector(".card__state").unwrap().unwrap();
    assert_eq!(state.text_content().unwrap(), "On · 14 %");
    let level = host.query_selector(".level__value").unwrap().unwrap();
    assert_eq!(level.text_content().unwrap(), "14 %");
    assert!(
        host.query_selector(".card__trailing [role=switch][aria-checked=true]")
            .unwrap()
            .is_some()
    );
}

#[wasm_bindgen_test]
async fn switch_toggle_is_trailing() {
    let host = mount(None, || card_for(switch("salon")));
    tick().await;
    assert!(
        host.query_selector(".card__trailing [role=switch][aria-checked=false]")
            .unwrap()
            .is_some()
    );
    let state = host.query_selector(".card__state").unwrap().unwrap();
    assert_eq!(state.text_content().unwrap(), "Off");
}
