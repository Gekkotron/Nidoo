#![cfg(target_arch = "wasm32")]

#[path = "support/fetch_stub.rs"]
mod fetch_stub;

use fetch_stub::FetchStub;
use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_api::{EntityView, GroupMemberView, GroupView, WsEvent};
use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::group_sheet_v2::GroupSheetV2Screen;
use nidoo_web::screens::groups::GroupsScreen;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

fn entity(id: &str, name: &str) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: name.into(),
        device_name: None,
        label: None,
        kind: Kind::Light,
        component: "light".into(),
        protocol_kind: Kind::Light,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "salon".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState::default(),
    }
}

fn group() -> GroupView {
    GroupView {
        id: "lights".into(),
        name: "Salon lights".into(),
        kind: Kind::Light,
        members: vec![
            GroupMemberView {
                id: "lamp".into(),
                name: "Lamp".into(),
                present: true,
                mismatch: false,
            },
            GroupMemberView {
                id: "ghost".into(),
                name: "ghost".into(),
                present: false,
                mismatch: false,
            },
        ],
        hide_members: true,
        entity: entity("group_lights", "Salon lights"),
        used_by_flows: vec![],
    }
}

fn mount(url: &str, lang: RwSignal<Lang>) -> (web_sys::Element, Toasts) {
    let toasts = Toasts::new();
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    web_sys::window()
        .unwrap()
        .history()
        .unwrap()
        .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(url))
        .unwrap();
    let house = House::new();
    house.apply(WsEvent::EntityUpserted(entity("lamp", "Lamp")));
    house.apply(WsEvent::EntityUpserted(entity(
        "group_lights",
        "Salon lights",
    )));
    let mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(toasts);
        provide_context(nidoo_web::commands::Pending::new());
        provide_context(lang);
        nidoo_web::nav::install_nav();
        view! {
            <Router>
                <Routes fallback=|| "fallback">
                    <Route path=path!("/groups") view=GroupsScreen />
                    <Route path=path!("/group/:id") view=GroupSheetV2Screen />
                </Routes>
            </Router>
        }
    });
    mounted.forget();
    (host, toasts)
}

async fn settle() {
    gloo_timers::future::TimeoutFuture::new(100).await;
    tick().await;
}

#[wasm_bindgen_test]
async fn the_groups_list_follows_the_language_without_a_remount() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        "/api/v1/groups",
        200,
        &serde_json::to_string(&vec![group()]).unwrap(),
    );
    let lang = RwSignal::new(Lang::Fr);
    let (host, _) = mount("/groups", lang);
    settle().await;
    let text = host.text_content().unwrap();
    assert!(
        text.contains("Groupes") && text.contains("1 groupe"),
        "{text}"
    );
    assert!(text.contains("2 appareils"), "{text}");
    assert!(text.contains("Nouveau groupe"), "{text}");
    assert!(
        !text.contains("New group") && !text.contains("2 devices"),
        "{text}"
    );

    lang.set(Lang::En);
    settle().await;
    let text = host.text_content().unwrap();
    assert!(
        text.contains("1 group") && !text.contains("1 groupe"),
        "{text}"
    );
    assert!(
        text.contains("2 devices") && text.contains("New group"),
        "{text}"
    );
    assert!(
        !text.contains("appareils") && !text.contains("Nouveau groupe"),
        "{text}"
    );
}

#[wasm_bindgen_test]
async fn the_group_sheet_follows_the_language_without_a_remount() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        "/api/v1/groups",
        200,
        &serde_json::to_string(&vec![group()]).unwrap(),
    );
    let lang = RwSignal::new(Lang::Fr);
    let (host, _) = mount("/group/lights", lang);
    settle().await;
    let text = host.text_content().unwrap();
    assert!(
        text.contains("Membres") && text.contains("Réglages"),
        "{text}"
    );
    assert!(text.contains("Commandes"), "{text}");
    assert!(text.contains("Masquer les membres sur l'accueil"), "{text}");
    assert!(text.contains("Afficher dans les favoris"), "{text}");
    assert!(
        text.contains("Modifier le groupe") && text.contains("Supprimer le groupe"),
        "{text}"
    );
    assert!(text.contains("2 lumières"), "{text}");
    assert!(text.contains("Introuvable"), "{text}");
    assert!(
        !text.contains("Members") && !text.contains("Not found"),
        "{text}"
    );

    lang.set(Lang::En);
    settle().await;
    let text = host.text_content().unwrap();
    assert!(
        text.contains("Members") && text.contains("Settings"),
        "{text}"
    );
    assert!(
        text.contains("Hide members on Home") && text.contains("Delete group"),
        "{text}"
    );
    assert!(
        text.contains("2 lights") && text.contains("Not found"),
        "{text}"
    );
    assert!(
        !text.contains("Membres") && !text.contains("Introuvable"),
        "{text}"
    );
}

#[wasm_bindgen_test]
async fn a_failed_group_load_keeps_the_hub_text_after_the_lead() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/groups", 500, r#"{"error":"hub down"}"#);
    let lang = RwSignal::new(Lang::Fr);
    let (host, toasts) = mount("/group/lights", lang);
    settle().await;
    let text = host.text_content().unwrap();
    assert!(text.contains("Ce groupe n'existe plus."), "{text}");
    let pushed: Vec<String> = toasts
        .0
        .with(|v| v.iter().map(|t| t.text.clone()).collect());
    assert_eq!(
        pushed,
        ["Impossible de charger le groupe\u{a0}: hub down".to_owned()]
    );
    drop(stub);
}
