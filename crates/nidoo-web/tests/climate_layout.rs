#![cfg(target_arch = "wasm32")]
//! Layout checks that need the real stylesheet: the climate rules and the screen width.

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::tick;
use nidoo_api::{EntityView, WsEvent};
use nidoo_climate::doc::HouseSettings;
use nidoo_climate::doc::{Block, Day, HhMm, ModeRules, Setpoint};
use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
use nidoo_web::climate::settings_sheet::SettingsSheet;
use nidoo_web::climate::timeline::WeekTimeline;
use nidoo_web::components::now::Now;
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::Element;
wasm_bindgen_test_configure!(run_in_browser);

fn install_styles() {
    let document = web_sys::window().unwrap().document().unwrap();
    if document
        .get_element_by_id("climate-layout-styles")
        .is_some()
    {
        return;
    }
    let style = document.create_element("style").unwrap();
    style.set_id("climate-layout-styles");
    style.set_text_content(Some(concat!(
        include_str!("../styles/tokens.css"),
        include_str!("../styles/main.css")
    )));
    document.head().unwrap().append_child(&style).unwrap();
}

fn host(width: u32) -> Element {
    install_styles();
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    host.set_attribute("style", &format!("width: {width}px"))
        .unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    host
}

fn block(from: &str, to: &str, t: f64) -> Block {
    Block {
        days: vec![Day::Mon],
        from: HhMm::parse(from).unwrap(),
        to: HhMm::parse(to).unwrap(),
        setpoint: Setpoint::Temp(t),
        evidence: None,
    }
}

#[wasm_bindgen_test]
async fn a_short_block_is_drawn_to_scale() {
    let rules = RwSignal::new(ModeRules {
        base: Setpoint::Temp(15.0),
        blocks: vec![block("07:00", "07:15", 19.0), block("07:15", "09:00", 18.0)],
    });
    let host = host(900);
    let mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        view! { <WeekTimeline rules=rules.into() editable=false on_change=Callback::new(|_| {}) /> }
    });
    std::mem::forget(mounted);
    tick().await;

    let col = host
        .query_selector(".climate-timeline__col")
        .unwrap()
        .unwrap();
    let short = col
        .query_selector(".climate-timeline__bar")
        .unwrap()
        .unwrap();
    let scale = col.get_bounding_client_rect().height() * 15.0 / 1440.0;
    let drawn = short.get_bounding_client_rect().height();
    // The 4 px floor keeps a tiny block visible; nothing beyond that.
    assert!(
        drawn <= scale.max(4.0) + 0.5,
        "15 min should be {scale:.1} px tall, drawn {drawn:.1} px"
    );
}

#[wasm_bindgen_test]
fn the_header_spans_the_climate_screen() {
    let host = host(1800);
    host.set_inner_html(
        "<div class=\"shell__content\">\
           <header class=\"content-header\"></header>\
           <main class=\"screen\"><div class=\"climate-screen\"></div></main>\
         </div>",
    );
    let width = |selector: &str| {
        host.query_selector(selector)
            .unwrap()
            .unwrap()
            .get_bounding_client_rect()
            .width()
    };
    assert!(width(".screen") > 1300.0, "the screen is not capped");
    assert_eq!(width(".content-header"), width(".screen"));
}

fn sensor(id: &str, kind: Kind, class: &str) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: id.into(),
        device_name: None,
        label: None,
        kind,
        component: kind.component().to_string(),
        protocol_kind: kind,
        device_class: Some(class.into()),
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "hall".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState::default(),
    }
}

#[wasm_bindgen_test]
async fn outdoor_picker_opens_on_thermometers() {
    let host = host(600);
    let house = House::new();
    for e in [
        sensor("t_out", Kind::Sensor, "temperature"),
        sensor("h_out", Kind::Sensor, "humidity"),
        sensor("door", Kind::BinarySensor, "door"),
    ] {
        house.apply(WsEvent::EntityUpserted(e));
    }
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(house);
            provide_context(Toasts::new());
            provide_context(Now(RwSignal::new(1_700_000_060)));
            view! {
                <SettingsSheet
                    house=HouseSettings::default()
                    on_saved=Callback::new(|_| {})
                    on_close=Callback::new(|_| {})
                />
            }
        },
    ));
    tick().await;
    // The Outdoor temperature field is the first picker field of the sheet.
    let button: web_sys::HtmlElement = host
        .query_selector("button.picker-field")
        .unwrap()
        .expect("outdoor field")
        .unchecked_into();
    button.click();
    tick().await;
    TimeoutFuture::new(30).await;
    let options = host.query_selector_all("[role=option]").unwrap();
    let ids: Vec<String> = (0..options.length())
        .map(|i| options.item(i).unwrap().unchecked_into::<Element>().id())
        .collect();
    assert!(ids.iter().any(|id| id.ends_with("-pin")), "{ids:?}");
    assert!(ids.iter().any(|id| id.ends_with("-opt-t_out")), "{ids:?}");
    assert!(!ids.iter().any(|id| id.ends_with("-opt-h_out")), "{ids:?}");
    assert!(!ids.iter().any(|id| id.ends_with("-opt-door")), "{ids:?}");
}

fn alarm_panel() -> EntityView {
    let mut e = sensor("alarm", Kind::AlarmPanel, "alarm");
    e.device_class = None;
    e
}

#[wasm_bindgen_test]
async fn presence_picker_suggests_alarm_panels() {
    let host = host(600);
    let house = House::new();
    for e in [alarm_panel(), sensor("humid", Kind::Sensor, "humidity")] {
        house.apply(WsEvent::EntityUpserted(e));
    }
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(house);
            provide_context(Toasts::new());
            provide_context(Now(RwSignal::new(1_700_000_060)));
            view! {
                <SettingsSheet
                    house=HouseSettings::default()
                    on_saved=Callback::new(|_| {})
                    on_close=Callback::new(|_| {})
                />
            }
        },
    ));
    tick().await;
    // The Presence field is the sheet's list field; its add button opens the picker.
    let add: web_sys::HtmlElement = host
        .query_selector(".chip--add")
        .unwrap()
        .expect("presence add button")
        .unchecked_into();
    add.click();
    tick().await;
    TimeoutFuture::new(30).await;
    let options = host.query_selector_all("[role=option]").unwrap();
    let ids: Vec<String> = (0..options.length())
        .map(|i| options.item(i).unwrap().unchecked_into::<Element>().id())
        .collect();
    assert!(ids.iter().any(|id| id.ends_with("-opt-alarm")), "{ids:?}");
    assert!(!ids.iter().any(|id| id.ends_with("-opt-humid")), "{ids:?}");
}

#[wasm_bindgen_test]
async fn room_thermometer_never_offers_other_sensors() {
    use nidoo_web::climate::entity_field::ThermometerField;
    let host = host(600);
    let house = House::new();
    for e in [
        sensor("t_room", Kind::Sensor, "temperature"),
        sensor("h_room", Kind::Sensor, "humidity"),
    ] {
        house.apply(WsEvent::EntityUpserted(e));
    }
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(house);
            provide_context(Now(RwSignal::new(1_700_000_060)));
            view! {
                <ThermometerField
                    value=Signal::stored(None)
                    auto=Signal::stored(None)
                    on_change=Callback::new(|_| {})
                />
            }
        },
    ));
    tick().await;
    let button: web_sys::HtmlElement = host
        .query_selector("button.picker-field")
        .unwrap()
        .expect("field")
        .unchecked_into();
    button.click();
    tick().await;
    TimeoutFuture::new(30).await;
    // Every chip shows the same pool: look at All explicitly.
    let chips = host.query_selector_all(".picker__chips .chip").unwrap();
    for i in 0..chips.length() {
        let chip: web_sys::HtmlElement = chips.item(i).unwrap().unchecked_into();
        if chip.text_content().unwrap() == "All" {
            chip.click();
        }
    }
    tick().await;
    let options = host.query_selector_all("[role=option]").unwrap();
    let ids: Vec<String> = (0..options.length())
        .map(|i| options.item(i).unwrap().unchecked_into::<Element>().id())
        .collect();
    assert!(ids.iter().any(|id| id.ends_with("-opt-t_room")), "{ids:?}");
    assert!(!ids.iter().any(|id| id.ends_with("-opt-h_room")), "{ids:?}");
}
