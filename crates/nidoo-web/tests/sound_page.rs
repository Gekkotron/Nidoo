#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_api::{AreaView, EntityView};
use nidoo_core::model::{
    Availability, Capabilities, EntityCategory, EntityState, Kind, StateValue, UiOverride,
};
use nidoo_web::commands::Pending;
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::settings::SettingsScreen;
use nidoo_web::screens::sound_settings::SoundSettingsPage;
use nidoo_web::theme::Theme;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

const KITCHEN: &str = "nidoo_sound_aaaaaaaaaaaa";
const BEDROOM: &str = "nidoo_sound_bbbbbbbbbbbb";

fn entity(
    id: &str,
    device: Option<(&str, &str)>,
    kind: Kind,
    value: StateValue,
    availability: Availability,
    area: &str,
) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: device.map(|d| d.0.into()),
        name: id.into(),
        device_name: device.map(|d| d.1.into()),
        label: None,
        kind,
        component: kind.component().to_string(),
        protocol_kind: kind,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Config,
        enabled_by_default: true,
        area: area.into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState {
            value,
            availability,
            ..EntityState::default()
        },
    }
}

/// The four entities of one satellite.
fn satellite(
    device: &str,
    name: &str,
    area: &str,
    allowed: bool,
    connected: bool,
    availability: Availability,
) -> Vec<EntityView> {
    let d = Some((device, name));
    vec![
        entity(
            &format!("{device}_volume"),
            d,
            Kind::Number,
            StateValue::Number(40.0),
            availability,
            area,
        ),
        entity(
            &format!("{device}_mute"),
            d,
            Kind::Switch,
            StateValue::OnOff(false),
            availability,
            area,
        ),
        entity(
            &format!("{device}_allowed"),
            d,
            Kind::Switch,
            StateValue::OnOff(allowed),
            availability,
            area,
        ),
        entity(
            &format!("{device}_connected"),
            d,
            Kind::BinarySensor,
            StateValue::OnOff(connected),
            availability,
            area,
        ),
    ]
}

fn other_entity() -> EntityView {
    entity(
        "lamp_1",
        Some(("zigbee_lamp", "Hall lamp")),
        Kind::Switch,
        StateValue::OnOff(true),
        Availability::Online,
        "hall",
    )
}

struct Mounted {
    _app: Box<dyn std::any::Any>,
    host: web_sys::Element,
}

fn mount(path_: &str, entities: Vec<EntityView>) -> Mounted {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    web_sys::window()
        .unwrap()
        .history()
        .unwrap()
        .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(path_))
        .unwrap();
    let house = House::new();
    house.areas.set(vec![AreaView {
        id: "kitchen".into(),
        name: "Kitchen".into(),
        icon: None,
        order: None,
        entity_count: 0,
        primary_count: 0,
        group_names: Default::default(),
        group_favorites: Default::default(),
    }]);
    house
        .entities
        .set(entities.into_iter().map(|e| (e.id.clone(), e)).collect());
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(Toasts::new());
        provide_context(Pending::new());
        provide_context(RwSignal::new(Lang::En));
        provide_context(RwSignal::new(Theme::Auto));
        nidoo_web::nav::install_nav();
        view! {
            <Router>
                <Routes fallback=|| "fallback">
                    <Route path=path!("/settings") view=SettingsScreen />
                    <Route path=path!("/settings/sound") view=SoundSettingsPage />
                </Routes>
            </Router>
        }
    });
    Mounted {
        _app: Box::new(app),
        host,
    }
}

fn all(host: &web_sys::Element, selector: &str) -> Vec<web_sys::HtmlElement> {
    let nodes = host.query_selector_all(selector).unwrap();
    (0..nodes.length())
        .map(|i| nodes.item(i).unwrap().unchecked_into())
        .collect()
}

fn texts(host: &web_sys::Element, selector: &str) -> Vec<String> {
    all(host, selector)
        .iter()
        .map(|e| e.text_content().unwrap_or_default())
        .collect()
}

async fn settle() {
    tick().await;
    gloo_timers::future::TimeoutFuture::new(50).await;
    tick().await;
}

/// The switch inside `row` whose caption is `label`.
fn switch(row: &web_sys::Element, label: &str) -> web_sys::HtmlElement {
    all(row, "label.override__flag")
        .into_iter()
        .find(|l| l.text_content().unwrap_or_default().trim() == label)
        .unwrap_or_else(|| panic!("no {label:?} switch in {:?}", row.text_content()))
        .query_selector("[role=switch]")
        .unwrap()
        .unwrap()
        .unchecked_into()
}

fn two_satellites() -> Vec<EntityView> {
    let mut all = satellite(
        KITCHEN,
        "Kitchen speaker",
        "kitchen",
        true,
        true,
        Availability::Online,
    );
    // Listed after "Bedroom" by name, though its id sorts first.
    all.extend(satellite(
        BEDROOM,
        "Bedroom",
        "unassigned",
        false,
        false,
        Availability::Online,
    ));
    all.push(other_entity());
    all
}

fn accepted() -> String {
    json!({"accepted": true, "reason": null, "state": null}).to_string()
}

#[wasm_bindgen_test]
async fn lists_satellites_by_name_and_ignores_other_devices() {
    let _stub = FetchStub::install();
    let m = mount("/settings/sound", two_satellites());
    settle().await;
    let rows = texts(&m.host, ".sound-row");
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(rows[0].contains("Bedroom"), "{rows:?}");
    assert!(rows[1].contains("Kitchen speaker"), "{rows:?}");
    assert!(rows[1].contains("Kitchen"), "{rows:?}");
    assert!(!m.host.text_content().unwrap().contains("Hall lamp"));
    assert_eq!(all(&m.host, ".dot--online").len(), 1);
    assert_eq!(all(&m.host, ".dot--offline").len(), 1);
    let link = all(&m.host, ".sound-row a")
        .into_iter()
        .find(|a| a.text_content().unwrap_or_default() == "Kitchen speaker")
        .expect("name links to the entity page");
    assert_eq!(
        link.get_attribute("href").as_deref(),
        Some("/entity/nidoo_sound_aaaaaaaaaaaa_allowed")
    );
}

#[wasm_bindgen_test]
async fn a_satellite_not_allowed_shows_the_waiting_badge() {
    let _stub = FetchStub::install();
    let m = mount("/settings/sound", two_satellites());
    settle().await;
    let rows = all(&m.host, ".sound-row");
    assert!(
        rows[0]
            .text_content()
            .unwrap()
            .contains("Waiting for approval")
    );
    assert!(
        !rows[1]
            .text_content()
            .unwrap()
            .contains("Waiting for approval")
    );
    assert_eq!(all(&m.host, ".sound-waiting").len(), 1);
}

#[wasm_bindgen_test]
async fn no_satellite_shows_the_empty_state() {
    let _stub = FetchStub::install();
    let m = mount("/settings/sound", vec![other_entity()]);
    settle().await;
    assert!(all(&m.host, ".sound-row").is_empty());
    let text = m.host.text_content().unwrap();
    assert!(text.contains("No sound satellite found yet"), "{text}");
    assert!(text.contains("Sound"), "{text}");
}

#[wasm_bindgen_test]
async fn allowed_switch_sends_turn_on() {
    let stub = FetchStub::install();
    stub.reply(
        "POST",
        &format!("/api/v1/entities/{BEDROOM}_allowed/command"),
        200,
        &accepted(),
    );
    let m = mount("/settings/sound", two_satellites());
    settle().await;
    let row = all(&m.host, ".sound-row").remove(0);
    let allowed = switch(&row, "Allowed");
    assert_eq!(
        allowed.get_attribute("aria-checked").as_deref(),
        Some("false")
    );
    allowed.click();
    settle().await;
    let sent = stub
        .requests()
        .into_iter()
        .find(|r| r.method == "POST" && r.url.ends_with("_allowed/command"))
        .unwrap_or_else(|| panic!("no command in {:?}", stub.requests()));
    assert_eq!(
        sent.url,
        format!("/api/v1/entities/{BEDROOM}_allowed/command")
    );
    let body: Value = serde_json::from_str(&sent.body).unwrap();
    assert_eq!(body["type"], "turn_on");
}

#[wasm_bindgen_test]
async fn mute_switch_and_volume_slider_send_commands() {
    let stub = FetchStub::install();
    stub.reply(
        "POST",
        &format!("/api/v1/entities/{KITCHEN}_mute/command"),
        200,
        &accepted(),
    );
    stub.reply(
        "POST",
        &format!("/api/v1/entities/{KITCHEN}_volume/command"),
        200,
        &accepted(),
    );
    let m = mount("/settings/sound", two_satellites());
    settle().await;
    let row = all(&m.host, ".sound-row").remove(1);
    switch(&row, "Mute").click();
    let slider: web_sys::HtmlInputElement = row
        .query_selector("input[type=range]")
        .unwrap()
        .unwrap()
        .unchecked_into();
    assert_eq!(slider.value(), "40");
    slider.set_value("65");
    slider
        .dispatch_event(
            &web_sys::Event::new_with_event_init_dict("change", &{
                let init = web_sys::EventInit::new();
                init.set_bubbles(true);
                init
            })
            .unwrap(),
        )
        .unwrap();
    settle().await;
    let requests = stub.requests();
    let find = |suffix: &str| {
        requests
            .iter()
            .find(|r| r.method == "POST" && r.url.ends_with(suffix))
            .map(|r| serde_json::from_str::<Value>(&r.body).unwrap())
            .unwrap_or_else(|| panic!("no {suffix} in {requests:?}"))
    };
    assert_eq!(find("_mute/command")["type"], "turn_on");
    let volume = find("_volume/command");
    assert_eq!(volume["type"], "set_number");
    assert_eq!(volume["value"], 65.0);
}

#[wasm_bindgen_test]
async fn offline_service_shows_the_not_running_line() {
    let _stub = FetchStub::install();
    let entities = satellite(
        KITCHEN,
        "Kitchen speaker",
        "kitchen",
        true,
        true,
        Availability::Offline,
    );
    let m = mount("/settings/sound", entities);
    settle().await;
    assert!(
        m.host
            .text_content()
            .unwrap()
            .contains("The sound service is not running"),
    );
    assert_eq!(all(&m.host, ".dot--online").len(), 0);
    assert_eq!(all(&m.host, ".dot--offline").len(), 1);
}

#[wasm_bindgen_test]
async fn online_service_shows_no_warning() {
    let _stub = FetchStub::install();
    let m = mount("/settings/sound", two_satellites());
    settle().await;
    assert!(
        !m.host
            .text_content()
            .unwrap()
            .contains("The sound service is not running"),
    );
}

#[wasm_bindgen_test]
async fn settings_list_has_a_sound_entry() {
    let _stub = FetchStub::install();
    let m = mount("/settings", two_satellites());
    settle().await;
    let entry = texts(&m.host, "a[href=\"/settings/sound\"]");
    assert_eq!(entry.len(), 1);
    assert!(entry[0].contains("Sound"), "{entry:?}");
}

const SPEAK: &str = "/api/v1/sound/speak";

fn type_into(input: &web_sys::HtmlInputElement, text: &str) {
    input.set_value(text);
    input
        .dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();
}

fn spoken(stub: &FetchStub) -> Vec<Value> {
    stub.requests()
        .into_iter()
        .filter(|r| r.method == "POST" && r.url == SPEAK)
        .map(|r| serde_json::from_str(&r.body).unwrap())
        .collect()
}

#[wasm_bindgen_test]
async fn the_say_box_sends_to_every_room_then_to_one() {
    let stub = FetchStub::install();
    stub.reply("POST", SPEAK, 202, r#"{"players":2}"#);
    let m = mount("/settings/sound", two_satellites());
    settle().await;
    let input: web_sys::HtmlInputElement =
        all(&m.host, ".say-box__text").remove(0).unchecked_into();
    let send = all(&m.host, ".say-box__send").remove(0);
    assert!(send.has_attribute("disabled"), "Send waits for a text");
    // Only the rooms holding a satellite: Kitchen (the other one is unassigned).
    assert_eq!(
        texts(&m.host, ".say-box__room option"),
        ["All rooms", "Kitchen"]
    );

    type_into(&input, "Dinner is ready");
    settle().await;
    assert!(!send.has_attribute("disabled"));
    send.click();
    settle().await;
    assert_eq!(texts(&m.host, ".say-box__done"), ["Said on 2 satellites"]);
    assert_eq!(input.value(), "", "the box clears once sent");

    let select: web_sys::HtmlSelectElement =
        all(&m.host, ".say-box__room").remove(0).unchecked_into();
    select.set_value("kitchen");
    select
        .dispatch_event(&web_sys::Event::new("change").unwrap())
        .unwrap();
    type_into(&input, "Hello");
    settle().await;
    send.click();
    settle().await;
    let sent = spoken(&stub);
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(sent[0]["text"], "Dinner is ready");
    assert_eq!(sent[0]["room"], Value::Null);
    assert_eq!(sent[1]["room"], "kitchen");
}

#[wasm_bindgen_test]
async fn the_say_box_shows_why_it_failed() {
    let stub = FetchStub::install();
    stub.reply(
        "POST",
        SPEAK,
        409,
        r#"{"error":"no satellite can speak in this room"}"#,
    );
    let m = mount("/settings/sound", two_satellites());
    settle().await;
    let input: web_sys::HtmlInputElement =
        all(&m.host, ".say-box__text").remove(0).unchecked_into();
    type_into(&input, "Hello");
    settle().await;
    all(&m.host, ".say-box__send").remove(0).click();
    settle().await;
    assert_eq!(
        texts(&m.host, ".say-box__error"),
        ["no satellite can speak in this room"]
    );
    assert_eq!(input.value(), "Hello", "the text stays to try again");
}

#[wasm_bindgen_test]
async fn each_allowed_row_has_a_test_button_that_speaks_on_that_satellite() {
    let stub = FetchStub::install();
    stub.reply("POST", SPEAK, 202, r#"{"players":1}"#);
    let m = mount("/settings/sound", two_satellites());
    settle().await;
    // Rows by name: Bedroom (waiting, so no button), then Kitchen speaker.
    let rows = all(&m.host, ".sound-row");
    assert!(
        rows[0]
            .query_selector(".satellite__speak")
            .unwrap()
            .is_none()
    );
    let speak: web_sys::HtmlElement = rows[1]
        .query_selector(".satellite__speak")
        .unwrap()
        .expect("test button")
        .unchecked_into();
    speak.click();
    settle().await;
    let sent = spoken(&stub);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0]["satellite"], "aaaaaaaaaaaa");
    assert_eq!(sent[0]["text"], "Speaker test: Kitchen speaker");
    assert_eq!(sent[0]["room"], Value::Null);
}

#[wasm_bindgen_test]
async fn a_failed_row_test_shows_the_reason_under_that_row() {
    let stub = FetchStub::install();
    stub.reply(
        "POST",
        SPEAK,
        409,
        r#"{"error":"this satellite cannot speak now"}"#,
    );
    let mut entities = two_satellites();
    entities.extend(satellite(
        "nidoo_sound_cccccccccccc",
        "Office",
        "kitchen",
        true,
        true,
        Availability::Online,
    ));
    let m = mount("/settings/sound", entities);
    settle().await;
    // Rows by name: Bedroom, Kitchen speaker, Office.
    let rows = all(&m.host, ".sound-row");
    let speak: web_sys::HtmlElement = rows[2]
        .query_selector(".satellite__speak")
        .unwrap()
        .expect("test button")
        .unchecked_into();
    speak.click();
    settle().await;
    let rows = all(&m.host, ".sound-row");
    let reason = |row: &web_sys::HtmlElement| {
        row.query_selector(".satellite__error")
            .unwrap()
            .and_then(|e| e.text_content())
    };
    assert_eq!(reason(&rows[1]), None);
    assert_eq!(
        reason(&rows[2]).as_deref(),
        Some("this satellite cannot speak now")
    );
}
