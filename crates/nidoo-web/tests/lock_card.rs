#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::Router;
use nidoo_api::EntityView;
use nidoo_core::model::{
    Availability, Capabilities, EntityCategory, EntityState, GenericState, GenericValue, Kind,
    StateValue, UiOverride,
};
use nidoo_core::types::GenericCaps;
use nidoo_web::commands::Pending;
use nidoo_web::components::confirm::Confirming;
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use nidoo_web::screens::generic_panel::GenericPanel;
use nidoo_web::screens::home::card_for;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

const ID: &str = "lock.x";

fn state(raw: &str, availability: Availability) -> EntityState {
    let mut g = GenericState::default();
    g.fields
        .insert("state".into(), GenericValue::Text(raw.into()));
    EntityState {
        value: StateValue::Generic(g),
        availability,
        ..Default::default()
    }
}

fn lock(raw: &str, availability: Availability) -> EntityView {
    EntityView {
        id: ID.into(),
        device_id: None,
        name: "Thing".into(),
        device_name: None,
        label: None,
        kind: Kind::Generic,
        component: "lock".into(),
        protocol_kind: Kind::Generic,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "salon".into(),
        part_of: None,
        capabilities: Capabilities {
            generic: Some(GenericCaps {
                controls: vec!["locked".into()],
                ..Default::default()
            }),
            ..Default::default()
        },
        ui: UiOverride::default(),
        state: state(raw, availability),
    }
}

struct Mounted {
    host: web_sys::Element,
    house: House,
    pending: Pending,
}

fn mount(entity: EntityView) -> Mounted {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let house = House::new();
    house
        .entities
        .update(|m| drop(m.insert(entity.id.clone(), entity.clone())));
    let pending = Pending::new();
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(house);
            provide_context(Toasts::new());
            provide_context(pending);
            provide_context(Confirming::new());
            let entity = entity.clone();
            view! { <Router>{card_for(entity)}</Router> }
        },
    ));
    Mounted {
        host,
        house,
        pending,
    }
}

fn button(host: &web_sys::Element, selector: &str) -> web_sys::HtmlElement {
    host.query_selector(selector)
        .unwrap()
        .expect(selector)
        .unchecked_into()
}

fn sent(m: &Mounted) -> bool {
    m.pending.0.get_untracked().contains(ID)
}

#[wasm_bindgen_test]
async fn locked_lock_asks_before_unlocking() {
    let m = mount(lock("locked", Availability::Online));
    tick().await;
    button(&m.host, ".card__controls .btn").click();
    tick().await;
    let question = m
        .host
        .query_selector(".confirm__question")
        .unwrap()
        .expect("a question")
        .text_content()
        .unwrap();
    assert_eq!(question, "Unlock Thing?");
    assert!(!sent(&m));
    button(&m.host, ".confirm__buttons .btn").click();
    assert!(sent(&m));
}

#[wasm_bindgen_test]
async fn unlocked_lock_locks_at_once() {
    let m = mount(lock("unlocked", Availability::Online));
    tick().await;
    button(&m.host, ".card__controls .btn").click();
    assert!(sent(&m));
    assert!(m.host.query_selector(".confirm").unwrap().is_none());
}

#[wasm_bindgen_test]
async fn offline_lock_button_is_disabled() {
    let m = mount(lock("locked", Availability::Offline));
    tick().await;
    assert!(
        button(&m.host, ".card__controls .btn").has_attribute("disabled"),
        "the button should be disabled"
    );
}

#[wasm_bindgen_test]
async fn question_closes_when_the_lock_unlocks_by_hand() {
    let m = mount(lock("locked", Availability::Online));
    tick().await;
    button(&m.host, ".card__controls .btn").click();
    tick().await;
    assert!(m.host.query_selector(".confirm").unwrap().is_some());
    m.house.entities.update(|map| {
        if let Some(e) = map.get_mut(ID) {
            e.state = state("unlocked", Availability::Online);
        }
    });
    tick().await;
    assert!(m.host.query_selector(".confirm").unwrap().is_none());
    assert!(!sent(&m));
}

#[wasm_bindgen_test]
async fn mounted_card_follows_availability() {
    // A dashboard mounts the card once; the frame must follow the live state like the face.
    let m = mount(lock("locked", Availability::Offline));
    tick().await;
    assert!(m.host.query_selector(".card--offline").unwrap().is_some());
    m.house.entities.update(|map| {
        if let Some(e) = map.get_mut(ID) {
            e.state = state("locked", Availability::Online);
        }
    });
    tick().await;
    assert!(m.host.query_selector(".card--offline").unwrap().is_none());
    assert!(m.host.query_selector(".dot--offline").unwrap().is_none());
}

#[wasm_bindgen_test]
async fn unlocked_lock_uses_the_warning_colour() {
    let m = mount(lock("unlocked", Availability::Online));
    tick().await;
    assert!(m.host.query_selector(".card--warn").unwrap().is_some());
    assert!(m.host.query_selector(".card--alert").unwrap().is_none());
}

#[wasm_bindgen_test]
async fn the_lock_sheet_offers_no_one_tap_unlock() {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let entity = lock("locked", Availability::Online);
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(House::new());
            provide_context(Toasts::new());
            provide_context(Pending::new());
            let entity = entity.clone();
            let memo = Memo::new(move |_| Some(entity.clone()));
            view! { <GenericPanel entity=memo /> }
        },
    ));
    tick().await;
    let text = host.text_content().unwrap_or_default();
    assert!(!text.contains("Unlock"), "{text}");
}
