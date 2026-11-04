#![cfg(target_arch = "wasm32")]
//! The Home card of a speaker satellite: volume, mute and a speaker test once allowed, an Allow
//! button while it waits for approval.

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::Router;
use nidoo_api::EntityView;
use nidoo_core::model::{
    Availability, Capabilities, EntityCategory, EntityState, Kind, StateValue, UiOverride,
};
use nidoo_web::commands::Pending;
use nidoo_web::components::confirm::Confirming;
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use nidoo_web::screens::home::card_for;
use serde_json::Value;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

const DEVICE: &str = "nidoo_sound_x";
const VOLUME: &str = "nidoo_sound_x_volume";
const ALLOWED: &str = "nidoo_sound_x_allowed";
const SPEAK: &str = "/api/v1/sound/speak";

fn entity(object: &str, kind: Kind, value: StateValue) -> EntityView {
    let component = match kind {
        Kind::Number => "number",
        Kind::Switch => "switch",
        _ => "binary_sensor",
    };
    let id = format!("{DEVICE}_{object}");
    EntityView {
        part_of: (object != "volume").then(|| VOLUME.to_owned()),
        id,
        device_id: Some(DEVICE.into()),
        name: object.into(),
        device_name: Some("Kitchen speaker".into()),
        label: None,
        kind,
        component: component.into(),
        protocol_kind: kind,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "kitchen".into(),
        capabilities: Capabilities {
            min: Some(0.0),
            max: Some(100.0),
            step: Some(1.0),
            ..Default::default()
        },
        ui: UiOverride::default(),
        state: EntityState {
            value,
            availability: Availability::Online,
            ..Default::default()
        },
    }
}

/// A satellite with volume 53, unmuted, `allowed` and `connected` as given.
fn satellite(allowed: bool, connected: bool) -> Vec<EntityView> {
    vec![
        entity("volume", Kind::Number, StateValue::Number(53.0)),
        entity("mute", Kind::Switch, StateValue::OnOff(false)),
        entity("allowed", Kind::Switch, StateValue::OnOff(allowed)),
        entity(
            "connected",
            Kind::BinarySensor,
            StateValue::OnOff(connected),
        ),
    ]
}

struct Mounted {
    host: web_sys::Element,
    pending: Pending,
}

fn mount(entities: Vec<EntityView>) -> Mounted {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let house = House::new();
    house.entities.update(|m| {
        for e in &entities {
            m.insert(e.id.clone(), e.clone());
        }
    });
    let volume = entities
        .iter()
        .find(|e| e.id == VOLUME)
        .cloned()
        .expect("volume");
    let pending = Pending::new();
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(house);
            provide_context(Toasts::new());
            provide_context(pending);
            provide_context(Confirming::new());
            let volume = volume.clone();
            view! { <Router>{card_for(volume)}</Router> }
        },
    ));
    Mounted { host, pending }
}

fn find(host: &web_sys::Element, selector: &str) -> Option<web_sys::HtmlElement> {
    host.query_selector(selector)
        .unwrap()
        .map(|e| e.unchecked_into())
}

#[wasm_bindgen_test]
async fn an_allowed_satellite_shows_volume_mute_and_test() {
    let m = mount(satellite(true, true));
    tick().await;
    let slider: web_sys::HtmlInputElement = find(&m.host, ".satellite__volume input")
        .expect("a volume slider")
        .unchecked_into();
    assert_eq!(slider.value(), "53");
    let shown = find(&m.host, ".satellite__value")
        .expect("the value")
        .text_content();
    assert_eq!(shown.as_deref(), Some("53"));
    assert!(find(&m.host, ".satellite__mute").is_some(), "a Mute button");
    assert!(
        find(&m.host, ".satellite__speak").is_some(),
        "a test button"
    );
    assert!(find(&m.host, ".satellite__allow").is_none());
    assert!(find(&m.host, ".dot--online").is_some());
}

#[wasm_bindgen_test]
async fn a_waiting_satellite_offers_allow_only() {
    let m = mount(satellite(false, true));
    tick().await;
    let text = m.host.text_content().unwrap_or_default();
    assert!(text.contains("Waiting for approval"), "{text}");
    assert!(find(&m.host, ".satellite__volume").is_none(), "no slider");
    assert!(
        find(&m.host, ".satellite__speak").is_none(),
        "no test button"
    );
    find(&m.host, ".satellite__allow").expect("Allow").click();
    assert!(m.pending.0.get_untracked().contains(ALLOWED), "Allow sends");
}

#[wasm_bindgen_test]
async fn the_test_button_says_a_sentence_on_that_satellite() {
    let stub = FetchStub::install();
    stub.reply("POST", SPEAK, 202, r#"{"players":1}"#);
    let m = mount(satellite(true, true));
    tick().await;
    find(&m.host, ".satellite__speak")
        .expect("test button")
        .click();
    for _ in 0..5 {
        tick().await;
    }
    let sent: Vec<Value> = stub
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST" && r.url == SPEAK)
        .map(|r| serde_json::from_str(&r.body).unwrap())
        .collect();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0]["satellite"], "x");
    assert_eq!(sent[0]["text"], "Speaker test: Kitchen speaker");
}

#[wasm_bindgen_test]
async fn a_failed_test_shows_the_reason() {
    let stub = FetchStub::install();
    stub.reply(
        "POST",
        SPEAK,
        409,
        r#"{"error":"speech is not set up on this hub"}"#,
    );
    let m = mount(satellite(true, true));
    tick().await;
    find(&m.host, ".satellite__speak")
        .expect("test button")
        .click();
    for _ in 0..5 {
        tick().await;
    }
    let error = find(&m.host, ".satellite__error").expect("an error line");
    assert!(
        error
            .text_content()
            .unwrap_or_default()
            .contains("speech is not set up"),
        "{:?}",
        error.text_content()
    );
}

#[wasm_bindgen_test]
async fn a_disconnected_satellite_shows_offline() {
    let m = mount(satellite(true, false));
    tick().await;
    assert!(find(&m.host, ".dot--offline").is_some());
}
