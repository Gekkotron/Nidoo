#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::Router;
use nidoo_api::EntityView;
use nidoo_core::model::{
    Capabilities, EntityCategory, EntityState, GenericState, GenericValue, Kind, StateValue,
    UiOverride,
};
use nidoo_core::types::{self, GenericCaps};
use nidoo_web::cards::generic::GenericCard;
use nidoo_web::commands::Pending;
use nidoo_web::components::confirm::Confirming;
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::generic_panel::GenericPanel;
use nidoo_web::screens::home::card_for;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

fn generic(component: &str, controls: &[&str], text: &str) -> EntityView {
    let mut g = GenericState::default();
    g.fields
        .insert("state".into(), GenericValue::Text(text.into()));
    EntityView {
        id: format!("{component}.x"),
        device_id: None,
        name: "Thing".into(),
        device_name: None,
        label: None,
        kind: Kind::Generic,
        component: component.into(),
        protocol_kind: Kind::Generic,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "salon".into(),
        part_of: None,
        capabilities: Capabilities {
            generic: Some(GenericCaps {
                controls: controls.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            }),
            ..Default::default()
        },
        ui: UiOverride::default(),
        state: EntityState {
            value: StateValue::Generic(g),
            ..Default::default()
        },
    }
}

fn mount(entity: EntityView, use_card_for: bool) -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(House::new());
            provide_context(Toasts::new());
            provide_context(Pending::new());
            let entity = entity.clone();
            view! {
                <Router>
                    {if use_card_for {
                        card_for(entity)
                    } else {
                        view! { <GenericCard entity=entity /> }.into_any()
                    }}
                </Router>
            }
        },
    ));
    host
}

#[wasm_bindgen_test]
async fn lock_card_shows_locked_and_unlock_button() {
    assert!(types::type_def("lock").is_some());
    let host = mount(generic("lock", &["locked"], "locked"), false);
    tick().await;
    let text = host.text_content().unwrap();
    assert!(text.contains("Locked"), "{text}");
    assert!(text.contains("Unlock"), "{text}");
}

#[wasm_bindgen_test]
async fn unlocked_lock_card_has_alert_class() {
    let host = mount(generic("lock", &["locked"], "unlocked"), false);
    tick().await;
    assert!(host.query_selector(".card--alert").unwrap().is_some());
}

#[wasm_bindgen_test]
async fn generic_card_without_main_control_shows_state_only() {
    let host = mount(generic("device_tracker", &[], "home"), false);
    tick().await;
    assert!(host.text_content().unwrap().contains("Home"));
    // The header is the only button.
    assert_eq!(
        host.query_selector_all("button:not(.card__header)")
            .unwrap()
            .length(),
        0
    );
}

#[wasm_bindgen_test]
async fn main_control_override_none_hides_the_control() {
    let mut e = generic("lock", &["locked"], "locked");
    e.ui.main_control = Some("none".into());
    let host = mount(e, false);
    tick().await;
    assert_eq!(
        host.query_selector_all("button:not(.card__header)")
            .unwrap()
            .length(),
        0
    );
}

#[wasm_bindgen_test]
async fn remote_has_no_home_card() {
    let host = mount(generic("device_automation", &[], "press"), true);
    tick().await;
    assert!(host.query_selector(".card").unwrap().is_none());
}

#[wasm_bindgen_test]
async fn the_generic_panel_follows_the_language_without_a_remount() {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    // A lock's sheet has no controls of its own (its card asks before unlocking): use a fan.
    let entity = generic("fan", &["power"], "off");
    let lang = RwSignal::new(Lang::Fr);
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(House::new());
            provide_context(Toasts::new());
            provide_context(Pending::new());
            provide_context(lang);
            let entity = entity.clone();
            let memo = Memo::new(move |_| Some(entity.clone()));
            view! { <GenericPanel entity=memo /> }
        },
    ));
    tick().await;
    let text = || host.text_content().unwrap_or_default();
    assert!(text().contains("Détails"), "{}", text());
    assert!(!text().contains("Details"), "{}", text());
    lang.set(Lang::En);
    tick().await;
    assert!(text().contains("Details"), "{}", text());
    assert!(!text().contains("Détails"), "{}", text());
}

fn mount_in(entity: EntityView, lang: RwSignal<Lang>) -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(House::new());
            provide_context(Toasts::new());
            provide_context(Pending::new());
            provide_context(Confirming::new());
            provide_context(lang);
            let entity = entity.clone();
            view! { <Router>{card_for(entity)}</Router> }
        },
    ));
    host
}

#[wasm_bindgen_test]
async fn the_generic_card_follows_the_language_without_a_remount() {
    let lang = RwSignal::new(Lang::En);
    // Its own value (a lock).
    let host = mount_in(generic("lock", &["locked"], "locked"), lang);
    // An on/off toggle without action labels (a fan).
    let fan = mount_in(generic("fan", &["power"], "off"), lang);
    // A text control opens the sheet through an Edit button.
    let text = mount_in(generic("text", &["value"], "hello"), lang);
    tick().await;
    assert!(host.text_content().unwrap().contains("Locked"));
    assert!(fan.text_content().unwrap().contains("Off"));
    assert!(text.text_content().unwrap().contains("Edit"));
    lang.set(Lang::Fr);
    tick().await;
    let now = host.text_content().unwrap();
    assert!(
        now.contains("Verrouillé") && !now.contains("Locked"),
        "{now}"
    );
    let now = fan.text_content().unwrap();
    assert!(now.contains("Désactivé"), "{now}");
    let now = text.text_content().unwrap();
    assert!(now.contains("Modifier") && !now.contains("Edit"), "{now}");
    lang.set(Lang::En);
    tick().await;
    assert!(text.text_content().unwrap().contains("Edit"));
}

#[wasm_bindgen_test]
async fn a_device_select_keeps_its_option_words() {
    let mut e = generic("lock", &[], "x");
    e.kind = Kind::Select;
    e.protocol_kind = Kind::Select;
    e.component = Kind::Select.component().to_string();
    e.capabilities = Capabilities {
        options: vec!["off".into(), "Toggle".into()],
        ..Default::default()
    };
    e.state.value = StateValue::Text("off".into());
    let lang = RwSignal::new(Lang::Fr);
    let host = mount_in(e, lang);
    tick().await;
    let html = host.inner_html();
    assert!(html.contains(">Off<") && !html.contains("Arrêt"), "{html}");
    assert!(html.contains(">Toggle<"), "{html}");
}
