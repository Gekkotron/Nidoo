#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_api::{EntityView, WsEvent};
use nidoo_core::model::{
    Capabilities, EntityCategory, EntityState, GenericState, GenericValue, Kind, StateValue,
    UiOverride,
};
use nidoo_core::types::GenericCaps;
use nidoo_web::commands::Pending;
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::entity_sheet::{EntitySheetScreen, SheetBody};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{Event, EventInit, HtmlInputElement};
wasm_bindgen_test_configure!(run_in_browser);

fn sensor(value: f64) -> EntityView {
    EntityView {
        id: "sensor.hall".into(),
        device_id: None,
        name: "Hall".into(),
        device_name: None,
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
            value: StateValue::Number(value),
            ..Default::default()
        },
    }
}

fn type_into(input: &HtmlInputElement, text: &str) {
    input.set_value(text);
    let init = EventInit::new();
    init.set_bubbles(true);
    let event = Event::new_with_event_init_dict("input", &init).unwrap();
    input.dispatch_event(&event).unwrap();
}

#[wasm_bindgen_test]
async fn moving_from_one_sheet_to_another_shows_the_new_entity() {
    let window = web_sys::window().unwrap();
    window
        .history()
        .unwrap()
        .push_state_with_url(
            &wasm_bindgen::JsValue::NULL,
            "",
            Some("/entity/sensor.hall"),
        )
        .unwrap();
    let document = window.document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();

    let house = House::new();
    house.apply(WsEvent::EntityUpserted(sensor(1.0)));
    let mut kitchen = sensor(2.0);
    kitchen.id = "sensor.kitchen".into();
    kitchen.name = "Kitchen".into();
    house.apply(WsEvent::EntityUpserted(kitchen));
    let _mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        nidoo_web::nav::install_nav();
        provide_context(Toasts::new());
        provide_context(Pending::new());
        view! {
            <Router>
                <Routes fallback=|| ()>
                    <Route path=path!("/entity/:id") view=EntitySheetScreen />
                </Routes>
            </Router>
        }
    });
    tick().await;
    let title = |host: &web_sys::Element| {
        host.query_selector(".sheet__title")
            .unwrap()
            .and_then(|t| t.text_content())
    };
    assert_eq!(title(&host).as_deref(), Some("Hall"));

    // Both entities are known: the sheet must follow the id, not only the known/unknown flag.
    window
        .history()
        .unwrap()
        .push_state_with_url(
            &wasm_bindgen::JsValue::NULL,
            "",
            Some("/entity/sensor.kitchen"),
        )
        .unwrap();
    window
        .dispatch_event(&Event::new("popstate").unwrap())
        .unwrap();
    tick().await;
    assert_eq!(title(&host).as_deref(), Some("Kitchen"));
}

#[wasm_bindgen_test]
async fn state_echo_keeps_the_override_draft() {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();

    let house = House::new();
    house.apply(WsEvent::EntityUpserted(sensor(1.0)));
    let _mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        nidoo_web::nav::install_nav();
        provide_context(Toasts::new());
        provide_context(Pending::new());
        view! {
            <Router>
                <SheetBody id="sensor.hall".to_owned() />
            </Router>
        }
    });
    tick().await;

    let input: HtmlInputElement = host
        .query_selector(".override input[type=text]")
        .unwrap()
        .expect("name field")
        .unchecked_into();
    type_into(&input, "Hallway");
    tick().await;
    let save = host
        .query_selector(".override__save")
        .unwrap()
        .expect("save button");
    assert!(
        !save.has_attribute("disabled"),
        "the edit makes the form dirty"
    );

    // A save the test server cannot take fails and leaves an error on the form.
    save.clone()
        .unchecked_into::<web_sys::HtmlElement>()
        .click();
    gloo_timers::future::TimeoutFuture::new(500).await;
    let error = host
        .query_selector(".override__error")
        .unwrap()
        .expect("error line");
    let message = error.text_content().unwrap_or_default();
    assert!(!message.is_empty(), "the failed save shows an error");

    let echo = EntityState {
        value: StateValue::Number(2.0),
        ..Default::default()
    };
    house.apply(WsEvent::StateChanged {
        id: "sensor.hall".into(),
        state: echo,
    });
    tick().await;

    assert!(input.is_connected(), "the editor was not rebuilt");
    assert_eq!(input.value(), "Hallway");
    assert!(!save.has_attribute("disabled"), "the draft is still dirty");
    let rebuilt: HtmlInputElement = host
        .query_selector(".override input[type=text]")
        .unwrap()
        .unwrap()
        .unchecked_into();
    assert_eq!(error.text_content().unwrap_or_default(), message);
    assert!(input.is_same_node(Some(&rebuilt)));
}

fn generic(
    component: &str,
    controls: &[&str],
    secret: bool,
    fields: &[(&str, GenericValue)],
) -> EntityView {
    let mut e = sensor(0.0);
    e.id = format!("{component}.x");
    e.kind = Kind::Generic;
    e.protocol_kind = Kind::Generic;
    e.component = component.into();
    e.capabilities = Capabilities {
        generic: Some(GenericCaps {
            controls: controls.iter().map(|s| s.to_string()).collect(),
            secret,
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut g = GenericState::default();
    for (k, v) in fields {
        g.fields.insert((*k).into(), v.clone());
    }
    e.state.value = StateValue::Generic(g);
    e
}

async fn sheet_text(entity: EntityView) -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let id = entity.id.clone();
    let house = House::new();
    house.apply(WsEvent::EntityUpserted(entity));
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(house);
            nidoo_web::nav::install_nav();
            provide_context(Toasts::new());
            provide_context(Pending::new());
            view! {
                <Router>
                    <SheetBody id=id.clone() />
                </Router>
            }
        },
    ));
    tick().await;
    host
}

#[wasm_bindgen_test]
async fn fan_sheet_lists_power_speed_and_preset() {
    let host = sheet_text(generic(
        "fan",
        &["power", "percentage", "preset"],
        false,
        &[],
    ))
    .await;
    let text = host.text_content().unwrap();
    for label in ["Power", "Percentage", "Preset"] {
        assert!(text.contains(label), "{label} missing in {text}");
    }
}

#[wasm_bindgen_test]
async fn secret_text_uses_a_password_field() {
    let host = sheet_text(generic("text", &["value"], true, &[])).await;
    assert!(
        host.query_selector("input[type=password]")
            .unwrap()
            .is_some(),
        "password input"
    );
}

#[wasm_bindgen_test]
async fn remote_sheet_shows_last_press() {
    let host = sheet_text(generic(
        "device_automation",
        &[],
        false,
        &[
            ("last", GenericValue::Text("on".into())),
            ("at", GenericValue::Number(1_000_000_000.0)),
        ],
    ))
    .await;
    let text = host.text_content().unwrap();
    assert!(text.contains("Last: On"), "{text}");
    assert!(text.contains("ago"), "{text}");
}

#[wasm_bindgen_test]
async fn update_sheet_shows_available_version() {
    let host = sheet_text(generic(
        "update",
        &[],
        false,
        &[
            ("installed", GenericValue::Text("1.0".into())),
            ("latest", GenericValue::Text("1.1".into())),
        ],
    ))
    .await;
    let text = host.text_content().unwrap();
    assert!(text.contains("Firmware 1.0 → 1.1"), "{text}");
    assert!(text.contains("Update available"), "{text}");
}

fn plug() -> EntityView {
    let mut e = sensor(1.0);
    e.id = "switch.pump".into();
    e.name = "Pump".into();
    e.kind = Kind::Switch;
    e.protocol_kind = Kind::Switch;
    e.component = Kind::Switch.component().to_string();
    e.ui.show_as = Some("plug".into());
    e.state.value = StateValue::OnOff(true);
    e
}

#[wasm_bindgen_test]
async fn the_sheet_follows_the_language_without_a_remount() {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();

    let house = House::new();
    house.apply(WsEvent::EntityUpserted(plug()));
    let lang = RwSignal::new(Lang::Fr);
    let _mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(lang);
        nidoo_web::nav::install_nav();
        provide_context(Toasts::new());
        provide_context(Pending::new());
        view! {
            <Router>
                <SheetBody id="switch.pump".to_owned() />
            </Router>
        }
    });
    tick().await;
    let text = || host.text_content().unwrap_or_default();
    for fr in [
        "Afficher comme",
        "Prise",
        "Commandes",
        "Réglages",
        "Général",
        "Enregistrer",
    ] {
        assert!(text().contains(fr), "{fr} missing: {}", text());
    }
    for en in ["Show as", "Plug", "Controls", "Settings", "General", "Save"] {
        assert!(!text().contains(en), "{en} present: {}", text());
    }
    let html = || host.inner_html();
    assert!(html().contains("Voir l'historique"), "{}", html());

    lang.set(Lang::En);
    tick().await;
    for en in ["Show as", "Plug", "Controls", "Settings", "General", "Save"] {
        assert!(text().contains(en), "{en} missing: {}", text());
    }
    for fr in [
        "Afficher comme",
        "Prise",
        "Commandes",
        "Réglages",
        "Général",
        "Enregistrer",
    ] {
        assert!(!text().contains(fr), "{fr} present: {}", text());
    }

    lang.set(Lang::Fr);
    tick().await;
    assert!(text().contains("Afficher comme"), "{}", text());
}

#[wasm_bindgen_test]
async fn an_unknown_entity_is_named_in_the_language() {
    let window = web_sys::window().unwrap();
    window
        .history()
        .unwrap()
        .push_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/entity/ghost"))
        .unwrap();
    let document = window.document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();

    let house = House::new();
    house.apply(WsEvent::EntityUpserted(sensor(1.0)));
    let lang = RwSignal::new(Lang::Fr);
    let _mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(lang);
        nidoo_web::nav::install_nav();
        provide_context(Toasts::new());
        provide_context(Pending::new());
        view! {
            <Router>
                <Routes fallback=|| ()>
                    <Route path=path!("/entity/:id") view=EntitySheetScreen />
                </Routes>
            </Router>
        }
    });
    tick().await;
    let text = || host.text_content().unwrap_or_default();
    assert!(text().contains("Entité inconnue"), "{}", text());
    assert!(!text().contains("Unknown entity"), "{}", text());
    lang.set(Lang::En);
    tick().await;
    assert!(text().contains("Unknown entity"), "{}", text());
    assert!(!text().contains("Entité inconnue"), "{}", text());
}
