#![cfg(target_arch = "wasm32")]
//! Back from a card's sheet returns to the room it was opened from on the overview. Its own test
//! binary: a router left over from an earlier test in the same page would swallow the link click.

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_api::{AreaView, EntityView};
use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::nav::{BackLink, TrackBackStack};
use nidoo_web::screens::home::HomeScreen;
use nidoo_web::theme::Theme;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

fn light(id: &str, area: &str) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: format!("Lamp {area}"),
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

fn area(id: &str) -> AreaView {
    AreaView {
        id: id.into(),
        name: id.into(),
        icon: None,
        order: None,
        entity_count: 1,
        primary_count: 1,
        group_names: Default::default(),
        group_favorites: Default::default(),
    }
}

async fn settle() {
    for _ in 0..3 {
        tick().await;
    }
    gloo_timers::future::TimeoutFuture::new(50).await;
}

#[wasm_bindgen_test]
async fn back_from_a_card_returns_to_its_room() {
    let window = web_sys::window().unwrap();
    window
        .history()
        .unwrap()
        .push_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/dashboards/house"))
        .unwrap();
    let document = window.document().unwrap();
    let body = document.body().unwrap();
    // Tall enough that the second room sits well below the fold.
    let spacer = document.create_element("div").unwrap();
    spacer.set_attribute("style", "height: 3000px").unwrap();
    body.append_child(&spacer).unwrap();
    let host = document.create_element("div").unwrap();
    body.append_child(&host).unwrap();

    let house = House::new();
    // The second room's id needs encoding: area ids are room names.
    house
        .areas
        .set(vec![area("Cuisine"), area("Salle de bain")]);
    house.entities.set(
        [
            light("light.k", "Cuisine"),
            light("light.b", "Salle de bain"),
        ]
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect(),
    );
    let _app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
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
                    <Route path=path!("/dashboards/:id") view=HomeScreen />
                    <Route path=path!("/entity/:id") view=|| view! { <BackLink /> } />
                </Routes>
            </Router>
        }
    });
    settle().await;

    let anchor = "room-Salle%20de%20bain";
    assert!(
        host.query_selector(&format!("section.room[id=\"{anchor}\"]"))
            .unwrap()
            .is_some(),
        "the second room section carries its anchor"
    );
    let header: web_sys::HtmlElement = host
        .query_selector(&format!("section.room[id=\"{anchor}\"] .card__header"))
        .unwrap()
        .expect("a card in the second room")
        .unchecked_into();
    header.click();
    settle().await;
    assert_eq!(window.location().pathname().unwrap(), "/entity/light.b");

    let back: web_sys::HtmlElement = host
        .query_selector("a.sheet__back")
        .unwrap()
        .expect("a Back link")
        .unchecked_into();
    // The fragment is the anchor, escaped once more: the router decodes it before the lookup.
    assert_eq!(
        back.get_attribute("href").as_deref(),
        Some("/dashboards/house#room-Salle%2520de%2520bain")
    );
    back.click();
    settle().await;
    assert_eq!(window.location().pathname().unwrap(), "/dashboards/house");
    let section = document
        .get_element_by_id(anchor)
        .expect("the second room is back on the page");
    // Scrolled to the room, not to the top of the overview.
    let y = window.scroll_y().unwrap();
    assert!(
        y > 1000.0,
        "scrollY {y}, hash {:?}, room top {}, body height {}",
        window.location().hash(),
        section.get_bounding_client_rect().top(),
        body.scroll_height()
    );
    // In view: at the top, or as near as the end of the page lets it come.
    let top = section.get_bounding_client_rect().top();
    let height = window.inner_height().unwrap().as_f64().unwrap();
    assert!(
        (-1.0..height / 2.0).contains(&top),
        "the room is in view: {top}"
    );
}
