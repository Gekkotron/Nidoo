#![cfg(target_arch = "wasm32")]

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::tick;
use nidoo_api::{AreaView, DeviceView, EntityView, SetupStep};
use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::setup::devices::DevicesStep;
use nidoo_web::setup::wizard::WizardNav;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

fn device(id: &str, name: &str, entity_ids: &[&str]) -> DeviceView {
    serde_json::from_value(serde_json::json!({
        "id": id, "ieee": id, "friendly_name": name, "model": "E1743", "vendor": "Acme",
        "description": null, "supported": true, "interview_state": "SUCCESSFUL",
        "power_source": null, "entity_ids": entity_ids, "availability": "online",
        "battery": null, "linkquality": null
    }))
    .unwrap()
}

fn entity(id: &str, device_id: &str) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: Some(device_id.into()),
        name: id.into(),
        device_name: None,
        label: None,
        kind: Kind::Sensor,
        component: Kind::Sensor.component().to_string(),
        protocol_kind: Kind::Sensor,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: nidoo_core::model::UNASSIGNED_AREA.into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState::default(),
    }
}

fn area(id: &str, name: &str) -> AreaView {
    AreaView {
        id: id.into(),
        name: name.into(),
        icon: None,
        order: None,
        entity_count: 0,
        primary_count: 0,
        group_names: Default::default(),
        group_favorites: Default::default(),
    }
}

fn mount(house: House, empty_after_ms: u32) -> (web_sys::Element, RwSignal<SetupStep>) {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let step = RwSignal::new(SetupStep::Devices);
    leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(RwSignal::new(Lang::En));
        provide_context(WizardNav { step });
        provide_context(house);
        view! { <DevicesStep z2m_base=String::new() empty_after_ms=empty_after_ms /> }
    })
    .forget();
    (host, step)
}

#[wasm_bindgen_test]
async fn no_devices_points_back_to_the_broker() {
    let _stub = FetchStub::install();
    let (host, step) = mount(House::new(), 10);
    tick().await;
    TimeoutFuture::new(50).await;
    tick().await;
    assert!(host.text_content().unwrap().contains("under zigbee2mqtt"));
    let back: web_sys::HtmlButtonElement = host
        .query_selector(".devices__back")
        .unwrap()
        .expect("back button")
        .unchecked_into();
    back.click();
    tick().await;
    assert_eq!(step.get_untracked(), SetupStep::Mqtt);
}

#[wasm_bindgen_test]
async fn choosing_a_room_moves_every_entity_of_the_device() {
    let stub = FetchStub::install();
    let house = House::new();
    house.devices.set(vec![device("d1", "Lamp", &["e1", "e2"])]);
    house.entities.set(
        [("e1", "d1"), ("e2", "d1"), ("e3", "other")]
            .into_iter()
            .map(|(id, dev)| (id.to_owned(), entity(id, dev)))
            .collect(),
    );
    house.areas.set(vec![area("kitchen", "Kitchen")]);
    let (host, _step) = mount(house, 5000);
    tick().await;
    let select: web_sys::HtmlSelectElement = host
        .query_selector("select.devices__room")
        .unwrap()
        .expect("room select")
        .unchecked_into();
    select.set_value("kitchen");
    select
        .dispatch_event(
            &web_sys::Event::new_with_event_init_dict("change", &{
                let init = web_sys::EventInit::new();
                init.set_bubbles(true);
                init
            })
            .unwrap(),
        )
        .unwrap();
    for _ in 0..6 {
        TimeoutFuture::new(15).await;
        tick().await;
    }
    let puts: Vec<_> = stub
        .requests()
        .into_iter()
        .filter(|r| r.method == "PATCH" && r.url.ends_with("/override"))
        .collect();
    let mut urls: Vec<&str> = puts.iter().map(|r| r.url.as_str()).collect();
    urls.sort();
    assert_eq!(
        urls,
        [
            "/api/v1/entities/e1/override",
            "/api/v1/entities/e2/override"
        ]
    );
    assert!(puts.iter().all(|r| r.body.contains("\"area\":\"kitchen\"")));
}

#[wasm_bindgen_test]
async fn a_failed_entity_move_is_reported_and_the_select_reverts() {
    let stub = FetchStub::install();
    let house = House::new();
    house.devices.set(vec![device("d1", "Lamp", &["e1", "e2"])]);
    house.entities.set(
        [("e1", "d1"), ("e2", "d1")]
            .into_iter()
            .map(|(id, dev)| (id.to_owned(), entity(id, dev)))
            .collect(),
    );
    house.areas.set(vec![area("kitchen", "Kitchen")]);
    stub.reply(
        "PATCH",
        "/api/v1/entities/e1/override",
        200,
        &serde_json::to_string(&entity("e1", "d1")).unwrap(),
    );
    stub.reply_after(
        "PATCH",
        "/api/v1/entities/e2/override",
        500,
        r#"{"error":"hub exploded"}"#,
        120,
    );
    let store = house.entities;
    let (host, _step) = mount(house, 5000);
    tick().await;
    let select: web_sys::HtmlSelectElement = host
        .query_selector("select.devices__room")
        .unwrap()
        .expect("room select")
        .unchecked_into();
    select.set_value("kitchen");
    select
        .dispatch_event(
            &web_sys::Event::new_with_event_init_dict("change", &{
                let init = web_sys::EventInit::new();
                init.set_bubbles(true);
                init
            })
            .unwrap(),
        )
        .unwrap();
    // The hub's upsert for entity 1 lands while entity 2's request is still in flight.
    TimeoutFuture::new(60).await;
    store.update(|m| m.get_mut("e1").unwrap().area = "kitchen".into());
    tick().await;
    for _ in 0..16 {
        TimeoutFuture::new(15).await;
        tick().await;
    }
    let alert = host
        .query_selector("[role=alert]")
        .unwrap()
        .unwrap()
        .text_content()
        .unwrap();
    assert!(alert.contains("hub exploded"), "alert: {alert}");
    assert!(
        alert.contains("not moved") || alert.contains("incomplete"),
        "alert: {alert}"
    );
    // The upsert rebuilt the row, so look the select up again.
    let select: web_sys::HtmlSelectElement = host
        .query_selector("select.devices__room")
        .unwrap()
        .expect("room select")
        .unchecked_into();
    assert_eq!(select.value(), "");
}

fn change(select: &web_sys::HtmlSelectElement, value: &str) {
    select.set_value(value);
    select
        .dispatch_event(
            &web_sys::Event::new_with_event_init_dict("change", &{
                let init = web_sys::EventInit::new();
                init.set_bubbles(true);
                init
            })
            .unwrap(),
        )
        .unwrap();
}

#[wasm_bindgen_test]
async fn a_room_change_on_the_hub_wins_over_a_forced_room() {
    let stub = FetchStub::install();
    let house = House::new();
    house.devices.set(vec![device("d1", "Lamp", &["e1", "e2"])]);
    house.entities.set(
        [("e1", "d1"), ("e2", "d1")]
            .into_iter()
            .map(|(id, dev)| (id.to_owned(), entity(id, dev)))
            .collect(),
    );
    house
        .areas
        .set(vec![area("kitchen", "Kitchen"), area("office", "Office")]);
    stub.reply(
        "PATCH",
        "/api/v1/entities/e1/override",
        200,
        &serde_json::to_string(&entity("e1", "d1")).unwrap(),
    );
    stub.reply(
        "PATCH",
        "/api/v1/entities/e2/override",
        500,
        r#"{"error":"hub exploded"}"#,
    );
    let store = house.entities;
    let (host, _step) = mount(house, 5000);
    tick().await;
    let select = |host: &web_sys::Element| -> web_sys::HtmlSelectElement {
        host.query_selector("select.devices__room")
            .unwrap()
            .expect("room select")
            .unchecked_into()
    };
    change(&select(&host), "kitchen");
    for _ in 0..16 {
        TimeoutFuture::new(15).await;
        tick().await;
    }
    assert_eq!(select(&host).value(), "", "the failed move reverts");
    // The device is moved elsewhere on the hub's side.
    store.update(|m| m.get_mut("e1").unwrap().area = "office".into());
    for _ in 0..4 {
        TimeoutFuture::new(15).await;
        tick().await;
    }
    assert_eq!(select(&host).value(), "office");
}

#[wasm_bindgen_test]
async fn a_move_reply_updates_the_store_and_the_select_is_off_meanwhile() {
    let stub = FetchStub::install();
    let house = House::new();
    house.devices.set(vec![device("d1", "Lamp", &["e1"])]);
    house.entities.set(
        [("e1".to_owned(), entity("e1", "d1"))]
            .into_iter()
            .collect(),
    );
    house.areas.set(vec![area("kitchen", "Kitchen")]);
    let mut moved = entity("e1", "d1");
    moved.area = "kitchen".into();
    stub.reply_after(
        "PATCH",
        "/api/v1/entities/e1/override",
        200,
        &serde_json::to_string(&moved).unwrap(),
        80,
    );
    let store = house.entities;
    let (host, _step) = mount(house, 5000);
    tick().await;
    let select: web_sys::HtmlSelectElement = host
        .query_selector("select.devices__room")
        .unwrap()
        .unwrap()
        .unchecked_into();
    change(&select, "kitchen");
    tick().await;
    let during: web_sys::HtmlSelectElement = host
        .query_selector("select.devices__room")
        .unwrap()
        .unwrap()
        .unchecked_into();
    assert!(during.disabled(), "no second move while one is in flight");
    for _ in 0..16 {
        TimeoutFuture::new(15).await;
        tick().await;
    }
    assert_eq!(store.with_untracked(|m| m["e1"].area.clone()), "kitchen");
    let after: web_sys::HtmlSelectElement = host
        .query_selector("select.devices__room")
        .unwrap()
        .unwrap()
        .unchecked_into();
    assert!(!after.disabled());
}

#[wasm_bindgen_test]
async fn devices_has_a_back_button_to_the_database() {
    let _stub = FetchStub::install();
    let (host, step) = mount(House::new(), 5000);
    tick().await;
    let back: web_sys::HtmlButtonElement = host
        .query_selector(".devices__actions .devices__prev")
        .unwrap()
        .expect("back button")
        .unchecked_into();
    back.click();
    tick().await;
    assert_eq!(step.get_untracked(), SetupStep::Database);
}
