//! Shared by the search page's browser tests: a house of two rooms, the app mounted at an address
//! over it, and helpers to type in the field and read the page. Each test file pulls it in with
//! `#[path = "support/search_harness.rs"] mod harness;`.
//!
//! A router left over from an earlier test in the same page swallows the next link click, so a test
//! that clicks a link is the only test of its file.
#![allow(dead_code)]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{A, Route, Router, Routes};
use leptos_router::path;
use nidoo_api::{AreaView, EntityView};
use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::nav::{BackLink, TrackBackStack};
use nidoo_web::screens::search::SearchScreen;
use nidoo_web::theme::Theme;
use wasm_bindgen::JsCast;

pub fn light(id: &str, name: &str, area: &str) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: name.into(),
        device_name: None,
        label: None,
        kind: Kind::Light,
        component: Kind::Light.component().to_string(),
        protocol_kind: Kind::Light,
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

pub fn area(id: &str, name: &str) -> AreaView {
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

pub async fn settle() {
    for _ in 0..3 {
        tick().await;
    }
    gloo_timers::future::TimeoutFuture::new(50).await;
}

/// Longer than the page's debounce.
pub async fn debounce() {
    gloo_timers::future::TimeoutFuture::new(300).await;
}

/// The mounted page; unmounts and leaves the document on drop, so the next test's router is the
/// only one handling link clicks.
pub struct Mounted {
    _app: Box<dyn std::any::Any>,
    pub host: web_sys::Element,
}

impl Drop for Mounted {
    fn drop(&mut self) {
        self.host.remove();
    }
}

pub fn window() -> web_sys::Window {
    web_sys::window().unwrap()
}

/// Opens `url` (path and query), then mounts the app over `entities`.
pub async fn mount(url: &str, areas: Vec<AreaView>, entities: Vec<EntityView>) -> Mounted {
    window()
        .history()
        .unwrap()
        .push_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(url))
        .unwrap();
    let document = window().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let house = House::new();
    house.areas.set(areas);
    house
        .entities
        .set(entities.into_iter().map(|e| (e.id.clone(), e)).collect());
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(RwSignal::new(Theme::Auto));
        provide_context(RwSignal::new(Lang::En));
        provide_context(nidoo_web::components::toast::Toasts::new());
        provide_context(nidoo_web::commands::Pending::new());
        provide_context(nidoo_web::components::confirm::Confirming::new());
        nidoo_web::nav::install_nav();
        view! {
            <Router>
                <TrackBackStack />
                <Routes fallback=|| ()>
                    <Route path=path!("/search") view=SearchScreen />
                    <Route path=path!("/entity/:id") view=|| view! { <BackLink /> } />
                    <Route
                        path=path!("/previous")
                        view=|| view! { <A href="/search" attr:id="open-search">"go"</A> }
                    />
                </Routes>
            </Router>
        }
    });
    settle().await;
    Mounted {
        _app: Box::new(app),
        host,
    }
}

pub fn input(m: &Mounted) -> web_sys::HtmlInputElement {
    m.host
        .query_selector("input.search__input")
        .unwrap()
        .expect("the search field")
        .unchecked_into()
}

pub fn type_in(field: &web_sys::HtmlInputElement, text: &str) {
    field.set_value(text);
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    field
        .dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &init).unwrap())
        .unwrap();
}

pub fn press_escape(field: &web_sys::HtmlInputElement) {
    let init = web_sys::KeyboardEventInit::new();
    init.set_key("Escape");
    init.set_bubbles(true);
    field
        .dispatch_event(
            &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap(),
        )
        .unwrap();
}

pub fn text_of(m: &Mounted) -> String {
    m.host.text_content().unwrap_or_default()
}

pub fn card_count(m: &Mounted) -> u32 {
    m.host.query_selector_all(".card").unwrap().length()
}

pub fn kitchen_house() -> (Vec<AreaView>, Vec<EntityView>) {
    (
        vec![area("kitchen", "Kitchen"), area("garage", "Garage")],
        vec![
            light("light.k", "Kitchen lamp", "kitchen"),
            light("light.g", "Garage lamp", "garage"),
        ],
    )
}
