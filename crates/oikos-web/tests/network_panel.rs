#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use oikos_api::{DeviceView, EntityView, WsEvent};
use oikos_core::model::{
    Availability, Capabilities, EntityCategory, EntityState, Kind, StateValue, UiOverride,
};
use oikos_web::commands::Pending;
use oikos_web::components::toast::Toasts;
use oikos_web::house::House;
use oikos_web::i18n::Lang;
use oikos_web::screens::entity_sheet::SheetBody;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

fn entity(id: &str, device_id: &str) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: Some(device_id.into()),
        name: "Vacuum".into(),
        device_name: Some("Roborock".into()),
        label: None,
        kind: Kind::Sensor,
        protocol_kind: Kind::Sensor,
        component: Kind::Sensor.component().to_string(),
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
            value: StateValue::Number(1.0),
            ..Default::default()
        },
    }
}

fn z2m_device(id: &str) -> DeviceView {
    DeviceView {
        id: id.into(),
        ieee: "0x1".into(),
        friendly_name: "Lamp".into(),
        model: None,
        vendor: None,
        description: None,
        supported: true,
        interview_state: "SUCCESSFUL".into(),
        power_source: None,
        entity_ids: vec![],
        availability: Availability::Online,
        battery: None,
        linkquality: None,
        update: None,
    }
}

fn now() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

fn watch_json(enabled: bool) -> Value {
    json!({
        "id": "vac_1",
        "spec": {
            "name": "Roborock network",
            "host": "192.168.1.42",
            "intervalS": 30,
            "enabled": enabled,
            "area": null,
            "device": "vac_1",
        },
        "entity": "network_vac_1",
        "status": {
            "state": if enabled { "connected" } else { "paused" },
            "since": now() - 100,
            "lastReply": now() - 12,
            "rttMs": 4.0,
        },
    })
}

struct Mounted {
    _app: Box<dyn std::any::Any>,
    host: web_sys::Element,
}

fn mount(entity: EntityView, devices: Vec<DeviceView>) -> Mounted {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let house = House::new();
    let id = entity.id.clone();
    house.apply(WsEvent::EntityUpserted(entity));
    house.apply(WsEvent::DevicesChanged { devices });
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(RwSignal::new(Lang::En));
        oikos_web::nav::install_nav();
        provide_context(Toasts::new());
        provide_context(Pending::new());
        view! {
            <leptos_router::components::Router>
                <SheetBody id=id.clone() />
            </leptos_router::components::Router>
        }
    });
    Mounted {
        _app: Box::new(app),
        host,
    }
}

async fn settle() {
    tick().await;
    gloo_timers::future::TimeoutFuture::new(100).await;
    tick().await;
}

fn panel_text(host: &web_sys::Element) -> Option<String> {
    let nodes = host.query_selector_all("section.sheet__panel").unwrap();
    (0..nodes.length())
        .filter_map(|i| nodes.item(i))
        .filter_map(|n| n.text_content())
        .find(|t| t.starts_with("Network"))
}

fn fire(el: &web_sys::Element, name: &str) {
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    el.dispatch_event(&web_sys::Event::new_with_event_init_dict(name, &init).unwrap())
        .unwrap();
}

fn switch(host: &web_sys::Element) -> web_sys::HtmlElement {
    let nodes = host.query_selector_all("label.override__flag").unwrap();
    let label: web_sys::Element = (0..nodes.length())
        .filter_map(|i| nodes.item(i))
        .map(|n| n.unchecked_into::<web_sys::Element>())
        .find(|l| {
            l.text_content()
                .unwrap_or_default()
                .contains("Watch on network")
        })
        .expect("watch switch");
    label
        .query_selector("button")
        .unwrap()
        .unwrap()
        .unchecked_into()
}

fn host_input(host: &web_sys::Element) -> web_sys::HtmlInputElement {
    host.query_selector(".network-form input")
        .unwrap()
        .expect("host field")
        .unchecked_into()
}

#[wasm_bindgen_test]
async fn panel_hidden_on_zigbee_device() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/network", 200, "[]");
    let m = mount(entity("light.lamp", "lamp_1"), vec![z2m_device("lamp_1")]);
    settle().await;
    assert_eq!(panel_text(&m.host), None);
}

#[wasm_bindgen_test]
async fn panel_hidden_on_group_entity() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/network", 200, "[]");
    let m = mount(entity("group_all", "group_all"), vec![]);
    settle().await;
    assert_eq!(panel_text(&m.host), None);
}

#[wasm_bindgen_test]
async fn panel_shows_existing_watch_state() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        "/api/v1/network",
        200,
        &json!([watch_json(true)]).to_string(),
    );
    let m = mount(entity("vacuum.roborock", "vac_1"), vec![]);
    settle().await;
    let text = panel_text(&m.host).expect("network panel");
    assert!(text.contains("connected"), "{text}");
    assert!(text.contains("12 s ago · 4 ms"), "{text}");
    assert!(text.contains("(90 s)"), "{text}");
    assert_eq!(host_input(&m.host).value(), "192.168.1.42");
}

#[wasm_bindgen_test]
async fn switch_on_puts_device_watch() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/network", 200, "[]");
    stub.reply(
        "PUT",
        "/api/v1/network/vac_1",
        200,
        &watch_json(true).to_string(),
    );
    let m = mount(entity("vacuum.roborock", "vac_1"), vec![]);
    settle().await;
    switch(&m.host).click();
    settle().await;
    let input = host_input(&m.host);
    assert_eq!(input.value(), "");
    input.set_value("192.168.1.42");
    fire(&input, "change");
    settle().await;
    let put = stub
        .requests()
        .into_iter()
        .find(|r| r.method == "PUT")
        .expect("a PUT");
    assert_eq!(put.url, "/api/v1/network/vac_1");
    let body: Value = serde_json::from_str(&put.body).unwrap();
    assert_eq!(body["device"], "vac_1");
    assert_eq!(body["host"], "192.168.1.42");
    assert_eq!(body["name"], "Roborock network");
    assert_eq!(body["enabled"], true);
}

#[wasm_bindgen_test]
async fn switch_off_pauses() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        "/api/v1/network",
        200,
        &json!([watch_json(true)]).to_string(),
    );
    stub.reply(
        "PUT",
        "/api/v1/network/vac_1",
        200,
        &watch_json(false).to_string(),
    );
    let m = mount(entity("vacuum.roborock", "vac_1"), vec![]);
    settle().await;
    switch(&m.host).click();
    settle().await;
    let put = stub
        .requests()
        .into_iter()
        .find(|r| r.method == "PUT")
        .expect("a PUT");
    let body: Value = serde_json::from_str(&put.body).unwrap();
    assert_eq!(body["enabled"], false);
    assert_eq!(body["host"], "192.168.1.42");
    assert_eq!(host_input(&m.host).value(), "192.168.1.42");
}
