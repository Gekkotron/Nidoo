#![cfg(target_arch = "wasm32")]

use std::collections::BTreeSet;

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::Router;
use oikos_api::{AlertChannel, AlertSeverity, AlertView, AreaView, EntityView, WsEvent};
use oikos_core::model::{
    AlarmMode, Capabilities, EntityCategory, EntityState, Kind, StateValue, UiOverride,
};
use oikos_web::commands::Pending;
use oikos_web::components::toast::Toasts;
use oikos_web::house::House;
use oikos_web::i18n::Lang;
use oikos_web::screens::alerts::AlertsScreen;
use oikos_web::screens::security::SecurityScreen;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

fn entity(id: &str, name: &str, kind: Kind, class: Option<&str>, value: StateValue) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: name.into(),
        device_name: None,
        label: None,
        kind,
        protocol_kind: kind,
        component: kind.component().to_string(),
        device_class: class.map(Into::into),
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "garage".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState {
            value,
            ..Default::default()
        },
    }
}

fn panel(state: &str) -> EntityView {
    let mut e = entity(
        "panel",
        "House alarm",
        Kind::AlarmPanel,
        None,
        StateValue::Alarm(state.into()),
    );
    e.capabilities.alarm_modes =
        BTreeSet::from([AlarmMode::Home, AlarmMode::Away, AlarmMode::Night]);
    e
}

fn siren() -> EntityView {
    let mut e = entity(
        "siren",
        "Siren",
        Kind::Switch,
        None,
        StateValue::OnOff(false),
    );
    e.ui.security_action = true;
    e
}

fn house(entities: Vec<EntityView>) -> House {
    let house = House::new();
    house.apply(WsEvent::AreasChanged {
        areas: vec![AreaView {
            id: "garage".into(),
            name: "Garage".into(),
            icon: None,
            order: None,
            entity_count: 0,
            primary_count: 0,
            group_names: Default::default(),
            group_favorites: Default::default(),
        }],
    });
    for e in entities {
        house.apply(WsEvent::EntityUpserted(e));
    }
    house
}

fn mount(house: House) -> (web_sys::Element, Box<dyn std::any::Any>) {
    mount_with(house, None, false)
}

/// Mounts the Security screen, or the Alerts screen, with `lang` in context when given.
fn mount_with(
    house: House,
    lang: Option<RwSignal<Lang>>,
    alerts: bool,
) -> (web_sys::Element, Box<dyn std::any::Any>) {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        if let Some(lang) = lang {
            provide_context(lang);
        }
        oikos_web::nav::install_nav();
        provide_context(Toasts::new());
        provide_context(Pending::new());
        provide_context(oikos_web::components::now::install_now());
        view! {
            <Router>
                {if alerts {
                    view! { <AlertsScreen /> }.into_any()
                } else {
                    view! { <SecurityScreen /> }.into_any()
                }}
            </Router>
        }
    });
    (host, Box::new(mounted))
}

fn button_with_text(host: &web_sys::Element, text: &str) -> Option<web_sys::HtmlElement> {
    let buttons = host.query_selector_all("button").unwrap();
    (0..buttons.length())
        .filter_map(|i| buttons.item(i))
        .map(|n| n.unchecked_into::<web_sys::HtmlElement>())
        .find(|b| b.text_content().unwrap_or_default().trim() == text)
}

#[wasm_bindgen_test]
async fn shows_panel_summary_and_tiles() {
    let door = entity(
        "door",
        "Garage door",
        Kind::BinarySensor,
        Some("door"),
        StateValue::OnOff(true),
    );
    let smoke = entity(
        "smoke",
        "Hall smoke",
        Kind::BinarySensor,
        Some("smoke"),
        StateValue::OnOff(false),
    );
    let (host, _mounted) = mount(house(vec![panel("armed_away"), door, smoke, siren()]));
    tick().await;
    let text = host.text_content().unwrap_or_default();
    for expected in [
        "Armed away",
        "1 open",
        "Garage",
        "Open",
        "Clear",
        "Off",
        "Actions",
        "Openings",
        "Fire",
        "Recent alerts",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
    let door_tile = host
        .query_selector("a.tile[aria-label='Garage door, garage, open']")
        .unwrap();
    assert!(door_tile.is_some(), "{}", host.inner_html());
    assert!(
        host.query_selector(".hero.hero--armed").unwrap().is_some(),
        "the armed hero is violet"
    );
    assert!(button_with_text(&host, "Disarm").is_some());
}

#[wasm_bindgen_test]
async fn triggered_hero_shows_disarm_only() {
    let (host, _mounted) = mount(house(vec![panel("triggered")]));
    tick().await;
    let buttons = host.query_selector_all(".hero button").unwrap();
    assert_eq!(buttons.length(), 1, "{}", host.inner_html());
    let only = buttons.item(0).unwrap();
    assert_eq!(only.text_content().unwrap_or_default().trim(), "Disarm");
    assert!(host.query_selector(".hero--triggered").unwrap().is_some());
    let live = host
        .query_selector(".hero [aria-live]")
        .unwrap()
        .expect("live region");
    assert_eq!(
        live.get_attribute("aria-live").as_deref(),
        Some("assertive")
    );
}

#[wasm_bindgen_test]
async fn ask_action_confirms_before_sending() {
    let (host, _mounted) = mount(house(vec![siren()]));
    tick().await;
    let tap: web_sys::HtmlElement = host
        .query_selector(".tile__tap")
        .unwrap()
        .expect("action tile")
        .unchecked_into();
    tap.click();
    tick().await;
    let text = host.text_content().unwrap_or_default();
    assert!(text.contains("Turn on Siren?"), "{text}");
    assert!(button_with_text(&host, "Turn on").is_some());
    let cancel = button_with_text(&host, "Cancel").expect("cancel button");
    assert!(
        host.query_selector(".card--pending").unwrap().is_none(),
        "nothing was sent"
    );
    assert!(host.query_selector(".tile--confirm").unwrap().is_some());
    cancel.click();
    tick().await;
    assert!(host.query_selector(".tile--confirm").unwrap().is_none());
    assert!(host.query_selector(".tile__tap").unwrap().is_some());
}

#[wasm_bindgen_test]
async fn confirm_closes_when_the_entity_goes_offline() {
    let house = house(vec![siren()]);
    let (host, _mounted) = mount(house);
    tick().await;
    host.query_selector(".tile__tap")
        .unwrap()
        .unwrap()
        .unchecked_into::<web_sys::HtmlElement>()
        .click();
    tick().await;
    assert!(host.query_selector(".tile--confirm").unwrap().is_some());
    let mut offline = siren();
    offline.state.availability = oikos_core::model::Availability::Offline;
    house.apply(WsEvent::EntityUpserted(offline));
    tick().await;
    assert!(host.query_selector(".tile--confirm").unwrap().is_none());
}

#[wasm_bindgen_test]
async fn empty_states() {
    let (host, _mounted) = mount(house(vec![]));
    tick().await;
    let text = host.text_content().unwrap_or_default();
    assert!(text.contains("Nothing on Security yet."), "{text}");

    let door = entity(
        "door",
        "Door",
        Kind::BinarySensor,
        Some("door"),
        StateValue::OnOff(false),
    );
    let (host, _mounted) = mount(house(vec![door]));
    tick().await;
    let text = host.text_content().unwrap_or_default();
    assert!(text.contains("No alarm panel."), "{text}");
    assert!(text.contains("All clear"), "{text}");
}

fn alert(id: &str, sent_at: u64) -> AlertView {
    AlertView {
        id: id.into(),
        channel: AlertChannel::Camera,
        severity: AlertSeverity::Info,
        title: "Front door".into(),
        body: "Someone is at the gate".into(),
        image_url: None,
        video_url: None,
        entity_id: None,
        camera: None,
        actions: vec![],
        source: None,
        silent: false,
        resolved: false,
        resolved_at: None,
        sent_at,
    }
}

fn present(host: &web_sys::Element, expected: &[&str], absent: &[&str]) {
    let text = host.text_content().unwrap_or_default();
    for word in expected {
        assert!(text.contains(word), "missing {word:?} in {text}");
    }
    for word in absent {
        assert!(!text.contains(word), "unexpected {word:?} in {text}");
    }
}

/// The Security screen reads French, then English again, without a remount.
#[wasm_bindgen_test]
async fn security_follows_the_language_toggle() {
    let door = entity(
        "door",
        "Garage door",
        Kind::BinarySensor,
        Some("door"),
        StateValue::OnOff(true),
    );
    let lang = RwSignal::new(Lang::Fr);
    let (host, _mounted) = mount_with(
        house(vec![panel("disarmed"), door, siren()]),
        Some(lang),
        false,
    );
    tick().await;
    present(
        &host,
        &[
            "Désactivée",
            "Présent",
            "1 ouvert",
            "Ouvertures",
            "Alertes récentes",
        ],
        &[
            "Disarmed",
            "Disarm",
            "1 open",
            "Openings",
            "Recent alerts",
            "Home",
        ],
    );
    let label = host
        .query_selector("a.tile[aria-label='Garage door, garage, ouverte']")
        .unwrap();
    assert!(label.is_some(), "{}", host.inner_html());
    // The confirm question and its buttons.
    let tap: web_sys::HtmlElement = host
        .query_selector(".tile__tap")
        .unwrap()
        .expect("action tile")
        .unchecked_into();
    tap.click();
    tick().await;
    present(
        &host,
        &["Activer Siren\u{A0}?", "Annuler"],
        &["Cancel", "Turn on"],
    );

    lang.set(Lang::En);
    tick().await;
    present(
        &host,
        &["Turn on Siren?", "Cancel"],
        &["Annuler", "Activer", "Désactivée"],
    );
    host.query_selector(".tile--confirm button:last-child")
        .unwrap()
        .unwrap()
        .unchecked_into::<web_sys::HtmlElement>()
        .click();
    tick().await;
    present(
        &host,
        &["Disarmed", "1 open", "Openings", "Recent alerts"],
        &["Désactivée", "1 ouvert", "Ouvertures"],
    );
    assert!(
        host.query_selector("a.tile[aria-label='Garage door, garage, open']")
            .unwrap()
            .is_some()
    );
}

/// The Alerts page: the day heading and the age follow the toggle.
#[wasm_bindgen_test]
async fn alerts_follow_the_language_toggle() {
    let now = (js_sys::Date::now() / 1000.0) as u64;
    let house = house(vec![]);
    house.apply(WsEvent::AlertsChanged {
        alerts: vec![alert("a", now - 5)],
    });
    let lang = RwSignal::new(Lang::Fr);
    let (host, _mounted) = mount_with(house, Some(lang), true);
    tick().await;
    present(
        &host,
        &[
            "Aujourd'hui",
            "Alertes",
            "Caméra",
            "À l'instant",
            "Tous",
            "Front door",
            "Someone is at the gate",
        ],
        &["Today", "Just now", "Camera"],
    );
    lang.set(Lang::En);
    tick().await;
    present(
        &host,
        &["Today", "Alerts", "Camera", "Just now", "All"],
        &["Aujourd'hui", "À l'instant", "Caméra", "Tous"],
    );
}

/// An armed panel offers Disarm, worded in the active language.
#[wasm_bindgen_test]
async fn armed_panel_controls_follow_the_language_toggle() {
    let lang = RwSignal::new(Lang::Fr);
    let (host, _mounted) = mount_with(house(vec![panel("armed_away")]), Some(lang), false);
    tick().await;
    present(
        &host,
        &["Activée (absent)", "Désactiver"],
        &["Armed", "Disarm"],
    );
    lang.set(Lang::En);
    tick().await;
    present(&host, &["Armed away", "Disarm"], &["Activée", "Désactiver"]);
}
