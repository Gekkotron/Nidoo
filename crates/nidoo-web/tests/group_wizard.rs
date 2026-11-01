#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_api::{AreaView, EntityView, WsEvent};
use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::group_wizard::GroupWizardScreen;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

fn entity(id: &str, kind: Kind, protocol_kind: Kind, area: &str) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: id.into(),
        device_name: None,
        label: None,
        kind,
        component: kind.component().to_string(),
        protocol_kind,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: area.into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState::default(),
    }
}

fn area(id: &str, name: &str) -> AreaView {
    AreaView {
        id: id.into(),
        name: name.into(),
        icon: None,
        order: None,
        entity_count: 1,
        primary_count: 1,
        group_names: Default::default(),
        group_favorites: Default::default(),
    }
}

fn mount() -> web_sys::Element {
    mount_lang(RwSignal::new(Lang::En))
}

fn mount_lang(lang: RwSignal<Lang>) -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    web_sys::window()
        .unwrap()
        .history()
        .unwrap()
        .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/groups/new"))
        .unwrap();
    let house = House::new();
    for e in [
        entity("lamp", Kind::Light, Kind::Light, "salon"),
        entity("plug", Kind::Light, Kind::Switch, "cuisine"),
        entity("fan", Kind::Switch, Kind::Switch, "salon"),
    ] {
        house.apply(WsEvent::EntityUpserted(e));
    }
    house.apply(WsEvent::AreasChanged {
        areas: vec![
            area("salon", "Salon"),
            area("cuisine", "Cuisine"),
            area("Unassigned", "Unassigned"),
        ],
    });
    let mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(Toasts::new());
        provide_context(lang);
        nidoo_web::nav::install_nav();
        view! {
            <Router>
                <Routes fallback=|| "fallback">
                    <Route path=path!("/groups/new") view=GroupWizardScreen />
                </Routes>
            </Router>
        }
    });
    mounted.forget();
    host
}

async fn click(host: &web_sys::Element, selector: &str, text: &str) {
    let buttons = host.query_selector_all(selector).unwrap();
    for i in 0..buttons.length() {
        let el: web_sys::HtmlElement = buttons.item(i).unwrap().unchecked_into();
        if el.text_content().unwrap().contains(text) {
            el.click();
            tick().await;
            return;
        }
    }
    panic!("no {selector} reading {text:?}");
}

#[wasm_bindgen_test]
async fn step_one_lists_controllable_kinds_only() {
    let host = mount();
    tick().await;
    let text = host.text_content().unwrap();
    for kind in [
        "light",
        "switch",
        "cover",
        "climate",
        "number",
        "select",
        "button",
        "vacuum",
        "water heater",
    ] {
        assert!(
            text.to_lowercase().contains(kind),
            "{kind} missing from {text}"
        );
    }
    for kind in [
        "sensor",
        "binary sensor",
        "alarm control panel",
        "alarm panel",
    ] {
        assert!(!text.contains(kind), "{kind} shown in {text}");
    }
}

#[wasm_bindgen_test]
async fn step_two_filters_by_effective_kind_and_groups_by_room() {
    let host = mount();
    tick().await;
    click(&host, "button.card", "Light").await;
    click(&host, "button.btn", "Next").await;
    let text = host.text_content().unwrap();
    assert!(text.contains("Salon") && text.contains("Cuisine"), "{text}");
    let options = host
        .query_selector_all(".picker-body--inline [role=option]")
        .unwrap();
    assert_eq!(options.length(), 2);
    assert!(!text.contains("fan"), "{text}");
}

#[wasm_bindgen_test]
async fn step_three_hides_members_by_default() {
    let host = mount();
    tick().await;
    click(&host, "button.card", "Light").await;
    click(&host, "button.btn", "Next").await;
    click(&host, ".picker-body--inline [role=option]", "lamp").await;
    click(&host, "button.btn", "Next").await;
    let switches = host.query_selector_all("[role=switch]").unwrap();
    let toggle = (0..switches.length())
        .map(|i| {
            switches
                .item(i)
                .unwrap()
                .unchecked_into::<web_sys::Element>()
        })
        .find(|el| el.text_content().unwrap().contains("Hide members on Home"))
        .unwrap_or_else(|| panic!("no Hide members switch in {}", host.inner_html()));
    assert_eq!(
        toggle.get_attribute("aria-checked").as_deref(),
        Some("true")
    );
}

#[wasm_bindgen_test]
async fn the_wizard_follows_the_language_without_a_remount() {
    let lang = RwSignal::new(Lang::Fr);
    let host = mount_lang(lang);
    tick().await;
    let text = host.text_content().unwrap();
    assert!(text.contains("Nouveau groupe"), "{text}");
    assert!(text.contains("Étape 1 sur 3"), "{text}");
    assert!(
        text.contains("Lumière") && text.contains("Chauffe-eau"),
        "{text}"
    );
    assert!(
        !text.contains("New group") && !text.contains("Step 1"),
        "{text}"
    );

    click(&host, "button.card", "Lumière").await;
    click(&host, "button.btn", "Suivant").await;
    let text = host.text_content().unwrap();
    assert!(
        text.contains("Membres") && text.contains("Étape 2 sur 3"),
        "{text}"
    );
    assert!(
        host.query_selector(".picker-body--inline input[type=search]")
            .unwrap()
            .is_some(),
        "a filter field"
    );

    click(&host, ".picker-body--inline [role=option]", "lamp").await;
    click(&host, "button.btn", "Suivant").await;
    let text = host.text_content().unwrap();
    assert!(text.contains("Nom et pièce"), "{text}");
    assert!(text.contains("Sans pièce"), "{text}");
    assert!(text.contains("Masquer les membres sur l'accueil"), "{text}");
    assert!(text.contains("Créer le groupe"), "{text}");
    assert!(!text.contains("Unassigned"), "{text}");
    let options = host.query_selector_all("option").unwrap();
    let no_room = (0..options.length())
        .filter(|i| {
            options
                .item(*i)
                .and_then(|o| o.text_content())
                .is_some_and(|t| t == "Sans pièce")
        })
        .count();
    assert_eq!(no_room, 1, "one empty-room option only: {text}");

    lang.set(Lang::En);
    tick().await;
    let text = host.text_content().unwrap();
    assert!(
        text.contains("New group") && text.contains("Step 3 of 3"),
        "{text}"
    );
    assert!(text.contains("Hide members on Home"), "{text}");
    assert!(text.contains("Create group"), "{text}");
    assert!(
        !text.contains("Nouveau groupe") && !text.contains("Créer le groupe"),
        "{text}"
    );
}
