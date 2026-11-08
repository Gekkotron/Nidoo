#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::Router;
use nidoo_api::{DeviceView, UpdateView};
use nidoo_core::model::Availability;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::settings::{SettingsScreen, ZigbeePage};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

fn device(name: &str, update: Option<UpdateView>) -> DeviceView {
    DeviceView {
        id: format!("id-{name}"),
        ieee: "0x1".into(),
        friendly_name: name.into(),
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
        update,
    }
}

async fn mount<V: IntoView + 'static>(
    devices: Vec<DeviceView>,
    screen: fn() -> V,
) -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            let house = House::new();
            house.devices.set(devices.clone());
            provide_context(house);
            provide_context(RwSignal::new(nidoo_web::theme::Theme::Auto));
            view! { <Router>{screen()}</Router> }
        },
    ));
    tick().await;
    host
}

#[wasm_bindgen_test]
async fn settings_lists_devices_with_updates() {
    let up = UpdateView {
        installed: Some("1.0".into()),
        latest: Some("1.1".into()),
        available: true,
    };
    let host = mount(
        vec![device("Hall lamp", Some(up)), device("Quiet plug", None)],
        ZigbeePage,
    )
    .await;
    let text = host.text_content().unwrap();
    assert!(text.contains("Updates available"));
    assert!(text.contains("Hall lamp"));
    assert!(text.contains("1.0 → 1.1"));
    assert!(!text.contains("All devices are up to date"));
}

#[wasm_bindgen_test]
async fn settings_shows_up_to_date_when_none() {
    let host = mount(vec![device("Quiet plug", None)], ZigbeePage).await;
    assert!(
        host.text_content()
            .unwrap()
            .contains("All devices are up to date")
    );
}

#[wasm_bindgen_test]
async fn settings_list_shows_the_waiting_updates_on_the_zigbee_row() {
    let up = UpdateView {
        installed: Some("1.0".into()),
        latest: Some("1.1".into()),
        available: true,
    };
    let host = mount(vec![device("Hall lamp", Some(up))], SettingsScreen).await;
    let text = host.text_content().unwrap();
    assert!(text.contains("Zigbee2MQTT"));
    assert!(text.contains("1 update"));
    assert!(!text.contains("Hall lamp"));
    assert!(
        host.query_selector("a[href='/settings/zigbee2mqtt']")
            .unwrap()
            .is_some()
    );
}

/// Like [`mount`], with the language in context and handed back so a test can flip it.
async fn mount_in<V: IntoView + 'static>(
    lang: RwSignal<Lang>,
    devices: Vec<DeviceView>,
    screen: fn() -> V,
) -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            let house = House::new();
            house.devices.set(devices.clone());
            provide_context(house);
            provide_context(RwSignal::new(nidoo_web::theme::Theme::Auto));
            provide_context(lang);
            view! { <Router>{screen()}</Router> }
        },
    ));
    tick().await;
    host
}

/// The Settings list names its pages in the active language and follows a toggle with no remount.
#[wasm_bindgen_test]
async fn settings_list_follows_the_language_without_a_remount() {
    let lang = RwSignal::new(Lang::Fr);
    let host = mount_in(lang, vec![], SettingsScreen).await;
    let fr = host.text_content().unwrap();
    assert!(fr.contains("Apparence"), "{fr}");
    assert!(fr.contains("À propos"), "{fr}");
    assert!(fr.contains("Réglages"), "{fr}");
    assert!(!fr.contains("Appearance"), "{fr}");
    assert!(!fr.contains("About"), "{fr}");

    lang.set(Lang::En);
    tick().await;
    let en = host.text_content().unwrap();
    assert!(en.contains("Appearance"), "{en}");
    assert!(en.contains("About"), "{en}");
    assert!(!en.contains("Apparence"), "{en}");
    assert!(!en.contains("À propos"), "{en}");
}

/// A sub-page's panels, rows and update count switch language on the spot.
#[wasm_bindgen_test]
async fn the_zigbee_page_follows_the_language_without_a_remount() {
    let up = UpdateView {
        installed: Some("1.0".into()),
        latest: Some("1.1".into()),
        available: true,
    };
    let lang = RwSignal::new(Lang::Fr);
    let host = mount_in(lang, vec![device("Hall lamp", Some(up))], ZigbeePage).await;
    let fr = host.text_content().unwrap();
    assert!(fr.contains("Mises à jour disponibles"), "{fr}");
    assert!(fr.contains("Passerelle"), "{fr}");
    assert!(fr.contains("Ouvrir les appareils Z2M"), "{fr}");
    assert!(!fr.contains("Updates available"), "{fr}");
    assert!(!fr.contains("Permit join"), "{fr}");

    lang.set(Lang::En);
    tick().await;
    let en = host.text_content().unwrap();
    assert!(en.contains("Updates available"), "{en}");
    assert!(en.contains("Permit join"), "{en}");
    assert!(!en.contains("Mises à jour disponibles"), "{en}");
    assert!(!en.contains("Passerelle"), "{en}");
}

/// The Zigbee row counts waiting updates with the right plural in French (0 and 1 are singular).
#[wasm_bindgen_test]
async fn the_zigbee_row_pluralises_in_french() {
    let up = || UpdateView {
        installed: Some("1.0".into()),
        latest: Some("1.1".into()),
        available: true,
    };
    let lang = RwSignal::new(Lang::Fr);
    let host = mount_in(
        lang,
        vec![device("A", Some(up())), device("B", Some(up()))],
        SettingsScreen,
    )
    .await;
    let two = host.text_content().unwrap();
    assert!(two.contains("2 mises à jour"), "{two}");
    lang.set(Lang::En);
    tick().await;
    assert!(host.text_content().unwrap().contains("2 updates"));
}
