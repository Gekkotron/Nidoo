#![cfg(target_arch = "wasm32")]

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::tick;
use nidoo_api::EntityView;
use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::setup::services::energy::EnergyForm;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

fn sensor(id: &str, device_class: &str, role: Option<&str>) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: id.into(),
        device_name: None,
        label: None,
        kind: Kind::Sensor,
        component: Kind::Sensor.component().to_string(),
        protocol_kind: Kind::Sensor,
        device_class: Some(device_class.into()),
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "salon".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride {
            energy_role: role.map(Into::into),
            ..Default::default()
        },
        state: EntityState::default(),
    }
}

fn mount(house: House) -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(RwSignal::new(Lang::En));
        provide_context(house);
        view! { <EnergyForm /> }
    })
    .forget();
    host
}

fn change(el: &web_sys::Element) {
    el.dispatch_event(
        &web_sys::Event::new_with_event_init_dict("change", &{
            let init = web_sys::EventInit::new();
            init.set_bubbles(true);
            init
        })
        .unwrap(),
    )
    .unwrap();
}

async fn settle() {
    for _ in 0..8 {
        TimeoutFuture::new(15).await;
        tick().await;
    }
}

fn house_with(entities: Vec<EntityView>) -> House {
    let house = House::new();
    house
        .entities
        .set(entities.into_iter().map(|e| (e.id.clone(), e)).collect());
    house
}

fn patches(stub: &FetchStub) -> Vec<(String, String)> {
    stub.requests()
        .into_iter()
        .filter(|r| r.method == "PATCH" && r.url.ends_with("/override"))
        .map(|r| (r.url, r.body))
        .collect()
}

#[wasm_bindgen_test]
async fn moving_a_role_clears_the_old_holder() {
    let stub = FetchStub::install();
    let a = sensor("meter_a", "power", Some("grid"));
    let b = sensor("meter_b", "energy", None);
    stub.reply("GET", "/api/v1/home/settings", 200, "{}");
    for e in [&a, &b] {
        stub.reply(
            "PATCH",
            &format!("/api/v1/entities/{}/override", e.id),
            200,
            &serde_json::to_string(e).unwrap(),
        );
    }
    let host = mount(house_with(vec![a, b]));
    tick().await;
    let select: web_sys::HtmlSelectElement = host
        .query_selector("select.energy__role")
        .unwrap()
        .expect("grid select")
        .unchecked_into();
    assert_eq!(select.value(), "meter_a");
    select.set_value("meter_b");
    change(&select);
    settle().await;
    let got = patches(&stub);
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(got[0].0, "/api/v1/entities/meter_a/override");
    assert!(got[0].1.contains("\"energyRole\":null"), "{}", got[0].1);
    assert_eq!(got[1].0, "/api/v1/entities/meter_b/override");
    assert!(got[1].1.contains("\"energyRole\":\"grid\""), "{}", got[1].1);
}

#[wasm_bindgen_test]
async fn a_failed_move_is_reported_and_the_select_keeps_the_real_holder() {
    let stub = FetchStub::install();
    let a = sensor("meter_a", "power", Some("grid"));
    let b = sensor("meter_b", "energy", None);
    stub.reply("GET", "/api/v1/home/settings", 200, "{}");
    stub.reply(
        "PATCH",
        "/api/v1/entities/meter_a/override",
        500,
        r#"{"error":"hub exploded"}"#,
    );
    let host = mount(house_with(vec![a, b]));
    tick().await;
    let select: web_sys::HtmlSelectElement = host
        .query_selector("select.energy__role")
        .unwrap()
        .unwrap()
        .unchecked_into();
    select.set_value("meter_b");
    change(&select);
    settle().await;
    let alert = host
        .query_selector("[role=alert]")
        .unwrap()
        .expect("alert line")
        .text_content()
        .unwrap();
    assert!(alert.contains("hub exploded"), "alert: {alert}");
    assert_eq!(select.value(), "meter_a");
    assert_eq!(patches(&stub).len(), 1);
}

#[wasm_bindgen_test]
async fn off_peak_from_the_card_keeps_the_rest_of_home_settings() {
    let stub = FetchStub::install();
    let doc = r#"{"eco_setpoint":17.5,"mode_entity":"virtual_mode","indoor_entity":null,"cold_below":12.0,"hot_above":28.0,"energy_view":"flux","off_peak":[],"off_peak_history":[]}"#;
    stub.reply("GET", "/api/v1/home/settings", 200, doc);
    stub.reply("PUT", "/api/v1/home/settings", 200, doc);
    let host = mount(house_with(vec![]));
    settle().await;
    let add: web_sys::HtmlElement = host
        .query_selector(".home-settings__add")
        .unwrap()
        .expect("add button")
        .unchecked_into();
    add.click();
    tick().await;
    let times = host.query_selector_all("input[type=time]").unwrap();
    assert_eq!(times.length(), 2);
    for (i, v) in ["22:00", "06:00"].into_iter().enumerate() {
        let input: web_sys::HtmlInputElement = times.item(i as u32).unwrap().unchecked_into();
        input.set_value(v);
        change(&input);
    }
    let save: web_sys::HtmlElement = host
        .query_selector(".home-settings__save")
        .unwrap()
        .expect("save button")
        .unchecked_into();
    save.click();
    settle().await;
    let put = stub
        .requests()
        .into_iter()
        .find(|r| r.method == "PUT" && r.url == "/api/v1/home/settings")
        .expect("a PUT");
    let sent: serde_json::Value = serde_json::from_str(&put.body).unwrap();
    let mut expected: serde_json::Value = serde_json::from_str(doc).unwrap();
    expected["off_peak"] = serde_json::json!([{"start":"22:00","end":"06:00"}]);
    assert_eq!(sent, expected);
}

#[wasm_bindgen_test]
async fn off_peak_card_saves_the_device_and_keeps_the_rest() {
    let stub = FetchStub::install();
    let doc = r#"{"eco_setpoint":17.5,"mode_entity":null,"indoor_entity":null,"cold_below":null,"hot_above":null,"energy_view":"orbite","off_peak":[],"off_peak_history":[],"off_peak_device":{"entity":"sensor.tariff","values":["OFF"]}}"#;
    stub.reply("GET", "/api/v1/home/settings", 200, doc);
    stub.reply("PUT", "/api/v1/home/settings", 200, doc);
    let host = mount(house_with(vec![sensor("sensor.tariff", "enum", None)]));
    settle().await;
    // With a device set, the windows read as its backup.
    let text = host.text_content().unwrap_or_default();
    assert!(text.contains("Backup windows"), "{text}");
    let save: web_sys::HtmlElement = host
        .query_selector(".home-settings__save")
        .unwrap()
        .expect("save button")
        .unchecked_into();
    save.click();
    settle().await;
    let puts: Vec<serde_json::Value> = stub
        .requests()
        .into_iter()
        .filter(|r| r.method == "PUT" && r.url == "/api/v1/home/settings")
        .map(|r| serde_json::from_str(&r.body).unwrap())
        .collect();
    let expected: serde_json::Value = serde_json::from_str(doc).unwrap();
    assert_eq!(puts.last(), Some(&expected));

    let remove: web_sys::HtmlElement = host
        .query_selector(".home-settings__device-clear")
        .unwrap()
        .expect("remove device button")
        .unchecked_into();
    remove.click();
    tick().await;
    save.click();
    settle().await;
    let sent: serde_json::Value = stub
        .requests()
        .into_iter()
        .rfind(|r| r.method == "PUT")
        .map(|r| serde_json::from_str(&r.body).unwrap())
        .unwrap();
    assert!(sent.get("off_peak_device").is_none(), "{sent}");
    assert_eq!(sent["off_peak"], serde_json::json!([]));
}

#[wasm_bindgen_test]
async fn off_peak_value_box_survives_house_updates() {
    let stub = FetchStub::install();
    let doc = r#"{"off_peak":[],"off_peak_history":[],"off_peak_device":{"entity":"sensor.tariff","values":["OFF"]}}"#;
    stub.reply("GET", "/api/v1/home/settings", 200, doc);
    let house = house_with(vec![
        sensor("sensor.tariff", "enum", None),
        sensor("sensor.other", "power", None),
    ]);
    let host = mount(house);
    settle().await;
    let input: web_sys::HtmlInputElement = host
        .query_selector("input[aria-label='Add a value']")
        .unwrap()
        .expect("value box")
        .unchecked_into();
    input.set_value("PEA");
    input
        .dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();
    tick().await;
    // Any entity in the house changing state must not rebuild the box under the cursor.
    house.entities.update(|m| {
        if let Some(e) = m.get_mut("sensor.other") {
            e.state.value = nidoo_core::model::StateValue::Number(42.0);
        }
        // The device's own reading changes too, so the chips are drawn again.
        if let Some(e) = m.get_mut("sensor.tariff") {
            e.state.value = nidoo_core::model::StateValue::Text("NEWCODE".into());
        }
    });
    settle().await;
    let text = host.text_content().unwrap_or_default();
    assert!(text.contains("NEWCODE"), "the chips did not redraw: {text}");
    let again = host
        .query_selector("input[aria-label='Add a value']")
        .unwrap()
        .expect("value box");
    assert!(again.is_same_node(Some(&input)), "the box was rebuilt");
    assert_eq!(input.value(), "PEA");
}

fn with_role(e: &EntityView, role: Option<&str>) -> String {
    let mut e = e.clone();
    e.ui.energy_role = role.map(Into::into);
    serde_json::to_string(&e).unwrap()
}

async fn choose(host: &web_sys::Element, value: &str) -> web_sys::HtmlSelectElement {
    let select: web_sys::HtmlSelectElement = host
        .query_selector("select.energy__role")
        .unwrap()
        .expect("grid select")
        .unchecked_into();
    select.set_value(value);
    change(&select);
    settle().await;
    select
}

#[wasm_bindgen_test]
async fn the_reply_updates_the_house_without_a_socket_echo() {
    let stub = FetchStub::install();
    let a = sensor("meter_a", "power", Some("grid"));
    let b = sensor("meter_b", "energy", None);
    stub.reply("GET", "/api/v1/home/settings", 200, "{}");
    stub.reply(
        "PATCH",
        "/api/v1/entities/meter_a/override",
        200,
        &with_role(&a, None),
    );
    stub.reply(
        "PATCH",
        "/api/v1/entities/meter_b/override",
        200,
        &with_role(&b, Some("grid")),
    );
    let house = house_with(vec![a, b]);
    let store = house.entities;
    let host = mount(house);
    tick().await;
    let select = choose(&host, "meter_b").await;
    assert_eq!(select.value(), "meter_b");
    store.with_untracked(|m| {
        assert_eq!(m["meter_a"].ui.energy_role, None);
        assert_eq!(m["meter_b"].ui.energy_role.as_deref(), Some("grid"));
    });
    // A second move right away clears B, not A again.
    choose(&host, "").await;
    let got = patches(&stub);
    assert_eq!(got.len(), 3, "{got:?}");
    assert_eq!(got[2].0, "/api/v1/entities/meter_b/override");
    assert!(got[2].1.contains("\"energyRole\":null"), "{}", got[2].1);
}

#[wasm_bindgen_test]
async fn a_failed_second_step_says_the_role_is_now_empty() {
    let stub = FetchStub::install();
    let a = sensor("meter_a", "power", Some("grid"));
    let b = sensor("meter_b", "energy", None);
    stub.reply("GET", "/api/v1/home/settings", 200, "{}");
    stub.reply(
        "PATCH",
        "/api/v1/entities/meter_a/override",
        200,
        &with_role(&a, None),
    );
    stub.reply(
        "PATCH",
        "/api/v1/entities/meter_b/override",
        500,
        r#"{"error":"hub exploded"}"#,
    );
    let host = mount(house_with(vec![a, b]));
    tick().await;
    let select = choose(&host, "meter_b").await;
    let alert = host
        .query_selector("[role=alert]")
        .unwrap()
        .unwrap()
        .text_content()
        .unwrap();
    assert!(alert.contains("hub exploded"), "alert: {alert}");
    assert!(alert.contains("is now empty"), "alert: {alert}");
    assert_eq!(select.value(), "");
}
