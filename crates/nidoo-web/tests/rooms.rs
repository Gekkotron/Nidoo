#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_api::{AreaView, CameraInfo, CamerasView, EntityView};
use nidoo_core::model::{
    Capabilities, EntityCategory, EntityState, Kind, UNASSIGNED_AREA, UiOverride,
};
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::home::HomeScreen;
use nidoo_web::screens::room::RoomScreen;
use nidoo_web::shell::Shell;
use nidoo_web::theme::Theme;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

fn entity(id: &str, name: &str, kind: Kind, area: &str) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: name.into(),
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
        area: area.into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState::default(),
    }
}

fn area(id: &str, count: usize) -> AreaView {
    AreaView {
        id: id.into(),
        name: format!("Room {id}"),
        icon: None,
        order: None,
        entity_count: count,
        primary_count: count,
        group_names: Default::default(),
        group_favorites: Default::default(),
    }
}

fn add_entities(house: House) {
    house.entities.set(
        [
            entity("light.a", "Lamp a", Kind::Light, "a"),
            entity("light.b", "Lamp b", Kind::Light, "b"),
            entity("switch.p", "Plug", Kind::Switch, UNASSIGNED_AREA),
        ]
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect(),
    );
}

struct Mounted {
    _app: Box<dyn std::any::Any>,
    host: web_sys::Element,
}

/// A house with rooms a and b (one light each) and an unassigned switch, opened at `path`.
async fn mount(path: &str) -> Mounted {
    mount_with(path, true).await.0
}

/// As [`mount`]; without `with_entities` the areas are there but no entity has arrived yet.
async fn mount_with(path: &str, with_entities: bool) -> (Mounted, House) {
    mount_lang(path, with_entities, RwSignal::new(Lang::En)).await
}

/// As [`mount_with`], with the language signal the test can flip.
async fn mount_lang(path: &str, with_entities: bool, lang: RwSignal<Lang>) -> (Mounted, House) {
    mount_cameras(path, with_entities, lang, CamerasView::default()).await
}

/// As [`mount_lang`], with the hub's camera list.
async fn mount_cameras(
    path: &str,
    with_entities: bool,
    lang: RwSignal<Lang>,
    cameras: CamerasView,
) -> (Mounted, House) {
    let window = web_sys::window().unwrap();
    window
        .history()
        .unwrap()
        .push_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(path))
        .unwrap();
    let document = window.document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let house = House::new();
    house.areas.set(vec![
        area("a", 1),
        area("b", 1),
        area("c", 0),
        area(UNASSIGNED_AREA, 1),
    ]);
    if with_entities {
        add_entities(house);
    }
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(RwSignal::new(Theme::Auto));
        provide_context(lang);
        provide_context(nidoo_web::cameras::CameraList(RwSignal::new(
            cameras.clone(),
        )));
        provide_context(nidoo_web::components::toast::Toasts::new());
        provide_context(nidoo_web::commands::Pending::new());
        provide_context(nidoo_web::components::confirm::Confirming::new());
        nidoo_web::nav::install_nav();
        let badge: Signal<Option<String>> = Signal::stored(None);
        view! {
            <Router>
                <nidoo_web::nav::TrackBackStack />
                <Shell
                    climate_on=Signal::from(false)
                    alarm_armed=Signal::from(false)
                    solar_active=Signal::from(false)
                    alerts_badge=badge
                >
                    <Routes fallback=|| ()>
                        <Route path=path!("/dashboards/:id") view=HomeScreen />
                        <Route path=path!("/rooms/:id") view=RoomScreen />
                    </Routes>
                </Shell>
            </Router>
        }
    });
    tick().await;
    tick().await;
    (
        Mounted {
            _app: Box::new(app),
            host,
        },
        house,
    )
}

fn texts(host: &web_sys::Element, selector: &str) -> Vec<String> {
    let list = host.query_selector_all(selector).unwrap();
    (0..list.length())
        .map(|i| list.item(i).unwrap().text_content().unwrap_or_default())
        .collect()
}

fn two_cameras(live: bool) -> CamerasView {
    CamerasView {
        live,
        cameras: ["front_door", "garden"]
            .map(|id| CameraInfo {
                id: id.into(),
                name: id.into(),
            })
            .into(),
    }
}

/// With `[frigate] live` on, the House overview shows every camera, one tile each.
#[wasm_bindgen_test]
async fn the_house_shows_every_camera_while_live() {
    let lang = RwSignal::new(Lang::En);
    let (m, _) = mount_cameras("/dashboards/house", true, lang, two_cameras(true)).await;
    let tiles = m
        .host
        .query_selector_all(".house-cameras .camera-widget")
        .unwrap();
    assert_eq!(tiles.length(), 2);
    assert!(texts(&m.host, ".room__title").contains(&"Cameras".to_owned()));
}

/// With `[frigate] live` off, the House overview has no Cameras section.
#[wasm_bindgen_test]
async fn the_house_has_no_cameras_while_live_is_off() {
    let lang = RwSignal::new(Lang::En);
    let (m, _) = mount_cameras("/dashboards/house", true, lang, two_cameras(false)).await;
    assert!(m.host.query_selector(".house-cameras").unwrap().is_none());
}

/// The Favorites title follows the language with no remount.
#[wasm_bindgen_test]
async fn the_favorites_title_follows_the_language() {
    let lang = RwSignal::new(Lang::Fr);
    let (m, house) = mount_lang("/dashboards/house", true, lang).await;
    house.entities.update(|map| {
        map.get_mut("light.a").unwrap().ui.favorite = true;
    });
    tick().await;
    let titles = || texts(&m.host, ".room .room__title");
    assert!(titles().contains(&"Favoris".to_owned()), "{:?}", titles());
    assert!(!titles().contains(&"Favorites".to_owned()));

    lang.set(Lang::En);
    tick().await;
    assert!(titles().contains(&"Favorites".to_owned()), "{:?}", titles());
    assert!(!titles().contains(&"Favoris".to_owned()));
}

/// A room with nothing on Home says so in the active language, and follows a toggle.
#[wasm_bindgen_test]
async fn the_empty_room_note_follows_the_language() {
    let lang = RwSignal::new(Lang::Fr);
    let (m, _) = mount_lang("/rooms/c", true, lang).await;
    let fr = m.host.text_content().unwrap_or_default();
    assert!(fr.contains("Rien ici pour l'instant."), "{fr}");
    assert!(!fr.contains("Nothing here yet."), "{fr}");

    lang.set(Lang::En);
    tick().await;
    let en = m.host.text_content().unwrap_or_default();
    assert!(en.contains("Nothing here yet."), "{en}");
    assert!(!en.contains("Rien ici pour l'instant."), "{en}");
}

#[wasm_bindgen_test]
async fn overview_shows_every_room() {
    let m = mount("/dashboards/house").await;
    assert_eq!(
        texts(&m.host, ".room .room__title"),
        ["Room a", "Room b", "Room Unassigned"]
    );
    assert!(m.host.query_selector("nav.chips").unwrap().is_none());
}

#[wasm_bindgen_test]
async fn room_route_shows_one_room() {
    let m = mount("/rooms/a").await;
    assert_eq!(texts(&m.host, ".room .room__title"), ["Room a"]);
    assert_eq!(
        m.host.query_selector_all(".room .card").unwrap().length(),
        1
    );
    let link = m
        .host
        .query_selector(".sidebar__rooms a[href=\"/rooms/a\"]")
        .unwrap()
        .expect("sidebar link");
    assert_eq!(link.get_attribute("aria-current").as_deref(), Some("page"));
}

/// Moves the page to `path` the way Back/Forward does; the mounted router follows popstate.
async fn go(path: &str) {
    let window = web_sys::window().unwrap();
    window
        .history()
        .unwrap()
        .push_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(path))
        .unwrap();
    window
        .dispatch_event(&web_sys::Event::new("popstate").unwrap())
        .unwrap();
    tick().await;
}

#[wasm_bindgen_test]
async fn moving_from_one_room_to_another_shows_the_new_room() {
    // Both rooms are known: the page must follow the id, not only the known/gone flag.
    let m = mount("/rooms/a").await;
    go("/rooms/b").await;
    assert_eq!(
        texts(&m.host, ".breadcrumb [aria-current=\"page\"]"),
        ["Room b"]
    );
    assert_eq!(texts(&m.host, ".room .room__title"), ["Room b"]);
    let cards = texts(&m.host, ".room .card");
    assert_eq!(cards.len(), 1);
    assert!(cards[0].contains("Lamp b"), "{cards:?}");
}

#[wasm_bindgen_test]
async fn room_route_unknown_room() {
    let m = mount("/rooms/zz").await;
    assert_eq!(texts(&m.host, ".room__gone"), ["This room is gone."]);
    assert!(m.host.query_selector(".room").unwrap().is_none());
}

#[wasm_bindgen_test]
async fn sidebar_lists_rooms_with_counts() {
    let m = mount("/dashboards/house").await;
    let rooms = m.host.query_selector(".sidebar__rooms").unwrap().unwrap();
    for id in ["a", "b"] {
        let link = rooms
            .query_selector(&format!("a[href=\"/rooms/{id}\"]"))
            .unwrap()
            .unwrap_or_else(|| panic!("no link for {id}"));
        assert!(link.text_content().unwrap().contains(&format!("Room {id}")));
        let count = link.query_selector(".sidebar__count").unwrap().unwrap();
        assert_eq!(count.text_content().as_deref(), Some("1"));
    }
    assert_eq!(rooms.query_selector_all("a").unwrap().length(), 2);
}

#[wasm_bindgen_test]
async fn room_page_waits_for_its_cards_without_calling_the_room_gone() {
    // The areas are in, the entities are not: the room exists and is empty for now.
    let (m, house) = mount_with("/rooms/a", false).await;
    assert!(m.host.query_selector(".room__gone").unwrap().is_none());
    assert_eq!(texts(&m.host, ".room .room__empty"), ["Nothing here yet."]);
    add_entities(house);
    tick().await;
    assert!(m.host.query_selector(".room__gone").unwrap().is_none());
    assert_eq!(
        m.host.query_selector_all(".room .card").unwrap().length(),
        1
    );
}

#[wasm_bindgen_test]
async fn room_with_nothing_on_home_is_empty_not_gone() {
    // The hub knows the room c, where nothing shows on Home.
    let (m, _house) = mount_with("/rooms/c", true).await;
    assert!(m.host.query_selector(".room__gone").unwrap().is_none());
    assert_eq!(texts(&m.host, ".room .room__empty"), ["Nothing here yet."]);
}

#[wasm_bindgen_test]
async fn room_with_an_accented_or_spaced_id_is_highlighted() {
    // Area ids are room names; the address bar and the links carry them percent-encoded.
    for (id, encoded) in [
        ("Séjour", "S%C3%A9jour"),
        ("Salle de bain", "Salle%20de%20bain"),
    ] {
        let (m, house) = mount_with(&format!("/rooms/{encoded}"), true).await;
        house.areas.update(|list| list.push(area(id, 1)));
        house.entities.update(|map| {
            let lamp = entity("light.named", "Named lamp", Kind::Light, id);
            map.insert(lamp.id.clone(), lamp);
        });
        tick().await;
        assert_eq!(texts(&m.host, ".room .room__title"), [format!("Room {id}")]);
        let link = m
            .host
            .query_selector(&format!(".sidebar__rooms a[href=\"/rooms/{encoded}\"]"))
            .unwrap()
            .unwrap_or_else(|| panic!("no sidebar link to /rooms/{encoded}"));
        assert_eq!(link.get_attribute("aria-current").as_deref(), Some("page"));
        let crumb = texts(&m.host, ".breadcrumb [aria-current=\"page\"]");
        assert_eq!(crumb, [format!("Room {id}")]);
    }
}

/// The ids of the cards in `section`, in the order on screen.
fn card_ids(section: &web_sys::Element) -> Vec<String> {
    let list = section.query_selector_all("[data-card-id]").unwrap();
    (0..list.length())
        .filter_map(|i| {
            list.item(i)?
                .unchecked_into::<web_sys::Element>()
                .get_attribute("data-card-id")
        })
        .collect()
}

fn click(host: &web_sys::Element, selector: &str) {
    host.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no {selector}"))
        .unchecked_into::<web_sys::HtmlElement>()
        .click();
}

fn key(el: &web_sys::Element, key: &str) {
    let init = web_sys::KeyboardEventInit::new();
    init.set_key(key);
    init.set_bubbles(true);
    let event =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    el.dispatch_event(&event).unwrap();
}

/// The pencil opens the room's edit mode: a grip on every card, which the arrow keys move one
/// place; Cancel puts the Home order back and closes it.
#[wasm_bindgen_test]
async fn a_room_in_edit_mode_reorders_its_cards_until_cancelled() {
    let (m, house) = mount_with("/rooms/a", true).await;
    house.entities.update(|map| {
        for (id, name) in [("light.a2", "Lamp a2"), ("light.a3", "Lamp a3")] {
            map.insert(id.into(), entity(id, name, Kind::Light, "a"));
        }
    });
    tick().await;
    let room = m.host.query_selector(".room").unwrap().unwrap();
    assert_eq!(card_ids(&room), ["light.a", "light.a2", "light.a3"]);
    assert!(m.host.query_selector(".room-card__grip").unwrap().is_none());

    click(&m.host, ".room__rename");
    tick().await;
    assert!(room.class_list().contains("room--editing"));
    assert!(m.host.query_selector(".room__field").unwrap().is_some());
    assert_eq!(
        m.host
            .query_selector_all(".room-card__grip")
            .unwrap()
            .length(),
        3
    );

    let grip = m.host.query_selector(".room-card__grip").unwrap().unwrap();
    key(&grip, "ArrowRight");
    tick().await;
    assert_eq!(card_ids(&room), ["light.a2", "light.a", "light.a3"]);

    click(&m.host, ".room__form button[type=\"button\"]");
    tick().await;
    assert!(!room.class_list().contains("room--editing"));
    assert_eq!(card_ids(&room), ["light.a", "light.a2", "light.a3"]);
    assert!(m.host.query_selector(".room-card__grip").unwrap().is_none());
}

/// Cards follow the stored `order` the edit mode saves; a card with none comes after them.
#[wasm_bindgen_test]
async fn a_room_lists_its_cards_by_their_saved_place() {
    let (m, house) = mount_with("/rooms/a", true).await;
    house.entities.update(|map| {
        for (id, name, order) in [
            ("light.a2", "Lamp a2", Some(2)),
            ("light.a3", "Lamp a3", Some(1)),
        ] {
            let mut lamp = entity(id, name, Kind::Light, "a");
            lamp.ui.order = order;
            map.insert(id.into(), lamp);
        }
    });
    tick().await;
    let room = m.host.query_selector(".room").unwrap().unwrap();
    assert_eq!(card_ids(&room), ["light.a3", "light.a2", "light.a"]);
}

/// Unassigned can be reordered but not renamed; Favorites mixes rooms and has no pencil.
#[wasm_bindgen_test]
async fn unassigned_reorders_without_a_name_and_favorites_cannot_be_edited() {
    let m = mount(&format!("/rooms/{UNASSIGNED_AREA}")).await;
    click(&m.host, ".room__rename");
    tick().await;
    assert!(m.host.query_selector(".room--editing").unwrap().is_some());
    assert!(m.host.query_selector(".room__field").unwrap().is_none());
    assert_eq!(
        m.host
            .query_selector_all(".room-card__grip")
            .unwrap()
            .length(),
        1
    );

    let (m, house) = mount_with("/dashboards/house", true).await;
    house.entities.update(|map| {
        map.get_mut("light.a").unwrap().ui.favorite = true;
    });
    tick().await;
    let favorites = m.host.query_selector(".room").unwrap().unwrap();
    assert_eq!(texts(&favorites, ".room__title"), ["Favorites"]);
    assert!(favorites.query_selector(".room__rename").unwrap().is_none());
}
