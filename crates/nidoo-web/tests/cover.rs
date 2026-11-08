#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::Router;
use nidoo_api::{EntityView, WsEvent};
use nidoo_core::model::{
    Capabilities, CoverState, EntityCategory, EntityState, Kind, StateValue, UiOverride,
};
use nidoo_web::cards::cover::CoverCard;
use nidoo_web::commands::Pending;
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

fn entity(
    id: &str,
    name: &str,
    kind: Kind,
    part_of: Option<&str>,
    value: StateValue,
) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: name.into(),
        device_name: None,
        label: None,
        kind,
        protocol_kind: kind,
        component: kind.component().to_string(),
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "salon".into(),
        part_of: part_of.map(Into::into),
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState {
            value,
            ..Default::default()
        },
    }
}

#[wasm_bindgen_test]
async fn moving_badge_follows_the_child_without_a_rebuild() {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();

    let cover = entity(
        "cover.hall",
        "Hall",
        Kind::Cover,
        None,
        StateValue::Cover(CoverState {
            state: Some("open".into()),
            position: Some(100),
        }),
    );
    let house = House::new();
    house.apply(WsEvent::EntityUpserted(cover.clone()));
    house.apply(WsEvent::EntityUpserted(entity(
        "binary_sensor.hall_moving",
        "Hall Moving",
        Kind::BinarySensor,
        Some("cover.hall"),
        StateValue::OnOff(false),
    )));
    let _mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(Toasts::new());
        provide_context(Pending::new());
        view! {
            <Router>
                <CoverCard entity=cover.clone() />
            </Router>
        }
    });
    tick().await;
    let name = host.query_selector(".card__name").unwrap().expect("card");
    assert!(host.query_selector(".badge").unwrap().is_none());

    house.apply(WsEvent::StateChanged {
        id: "binary_sensor.hall_moving".into(),
        state: EntityState {
            value: StateValue::OnOff(true),
            ..Default::default()
        },
    });
    tick().await;
    let badge = host.query_selector(".badge").unwrap().expect("badge shows");
    assert_eq!(badge.text_content().as_deref(), Some("Moving"));
    assert!(name.is_connected(), "the card was not rebuilt");

    house.apply(WsEvent::StateChanged {
        id: "binary_sensor.hall_moving".into(),
        state: EntityState {
            value: StateValue::OnOff(false),
            ..Default::default()
        },
    });
    tick().await;
    assert!(host.query_selector(".badge").unwrap().is_none());
}
