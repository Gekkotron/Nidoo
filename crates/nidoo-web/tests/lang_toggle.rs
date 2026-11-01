#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::Router;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::shell::Shell;
use nidoo_web::theme::Theme;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

/// Switching the language on a mounted screen rewrites its text, with no remount.
#[wasm_bindgen_test]
async fn the_sidebar_follows_the_language_without_a_remount() {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let lang = RwSignal::new(Lang::En);
    let _app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(House::new());
        provide_context(RwSignal::new(Theme::Auto));
        provide_context(lang);
        nidoo_web::nav::install_nav();
        view! {
            <Router>
                <Shell
                    climate_on=RwSignal::new(false)
                    alarm_armed=Signal::from(false)
                    solar_active=Signal::from(false)
                    alerts_badge=Signal::stored(None::<String>)
                >
                    <p>"child"</p>
                </Shell>
            </Router>
        }
    });
    tick().await;
    let sidebar = || {
        host.query_selector(".sidebar")
            .unwrap()
            .expect("a sidebar")
            .text_content()
            .unwrap_or_default()
    };
    assert!(sidebar().contains("Automations"), "{}", sidebar());

    lang.set(Lang::Fr);
    tick().await;
    let fr = sidebar();
    assert!(fr.contains("Automatisations"), "{fr}");
    assert!(!fr.contains("Automations"), "{fr}");
}

fn entity(
    id: &str,
    kind: nidoo_core::model::Kind,
    value: nidoo_core::model::StateValue,
) -> nidoo_api::EntityView {
    use nidoo_core::model::{Capabilities, EntityCategory, EntityState, UiOverride};
    nidoo_api::EntityView {
        id: id.into(),
        device_id: None,
        name: "Thing".into(),
        device_name: None,
        label: None,
        kind,
        component: kind.component().to_string(),
        protocol_kind: kind,
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
            value,
            ..Default::default()
        },
    }
}

/// A card's value follows the toggle without the entity changing.
#[wasm_bindgen_test]
async fn card_values_follow_the_language_without_a_remount() {
    use nidoo_core::model::{CoverState, Kind, StateValue};
    let mut sensor = entity("sensor.t", Kind::Sensor, StateValue::Number(21.46));
    sensor.unit = Some("W".into());
    let mut cover = entity(
        "cover.c",
        Kind::Cover,
        StateValue::Cover(CoverState {
            state: Some("open".into()),
            position: Some(40),
        }),
    );
    cover.device_name = Some("Thing".into());
    let mut heater = entity(
        "water_heater.w",
        Kind::WaterHeater,
        StateValue::WaterHeater(nidoo_core::model::WaterHeaterState {
            mode: Some("heat".into()),
            current: Some(48.0),
            target: Some(55.5),
        }),
    );
    heater.name = "Tank".into();
    heater.capabilities.hvac_modes = vec!["heat".into(), "off".into()];
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let lang = RwSignal::new(Lang::En);
    let _app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(House::new());
        provide_context(nidoo_web::components::toast::Toasts::new());
        provide_context(nidoo_web::commands::Pending::new());
        provide_context(lang);
        view! {
            <Router>
                {nidoo_web::screens::home::card_for(sensor.clone())}
                {nidoo_web::screens::home::card_for(cover.clone())}
                <nidoo_web::components::card_place::InSheet>
                    {nidoo_web::screens::home::card_for(heater.clone())}
                </nidoo_web::components::card_place::InSheet>
            </Router>
        }
    });
    tick().await;
    let text = || host.text_content().unwrap_or_default();
    assert!(text().contains("21.5\u{a0}W"), "{}", text());
    assert!(text().contains("Open · 40\u{a0}%"), "{}", text());
    // A card title, a stepper label and aria-labels, a select option.
    assert!(text().contains("Thing · Cover"), "{}", text());
    assert!(text().contains("55.5\u{a0}°C"), "{}", text());
    assert!(text().contains("Heat"), "{}", text());
    let labels = || host.inner_html();
    assert!(labels().contains("aria-label=\"Decrease\""), "{}", labels());

    lang.set(Lang::Fr);
    tick().await;
    assert!(text().contains("21,5\u{a0}W"), "{}", text());
    assert!(text().contains("Ouvert · 40\u{a0}%"), "{}", text());
    assert!(text().contains("Thing · Volet"), "{}", text());
    assert!(!text().contains("Thing · Cover"), "{}", text());
    assert!(text().contains("55,5\u{a0}°C"), "{}", text());
    assert!(text().contains("Chauffage"), "{}", text());
    assert!(!text().contains("Heat"), "{}", text());
    assert!(labels().contains("aria-label=\"Baisser\""), "{}", labels());
    assert!(
        !labels().contains("aria-label=\"Decrease\""),
        "{}",
        labels()
    );
}
