#![cfg(target_arch = "wasm32")]
//! The rich cards: what each shows on a tile and what its sheet adds.

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::Router;
use nidoo_api::EntityView;
use nidoo_core::model::{
    Capabilities, ClimateState, EntityCategory, EntityState, Kind, StateValue, UiOverride,
    VacuumState, WaterHeaterState,
};
use nidoo_web::commands::Pending;
use nidoo_web::components::card_place::{CardPlace, provide_card_place};
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use nidoo_web::screens::home::card_for;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

fn entity(id: &str, kind: Kind, value: StateValue) -> EntityView {
    EntityView {
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

/// Mounts the card for `entity` in `place` (the default, Overview, when `None`).
fn mount(entity: EntityView, place: Option<CardPlace>) -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(House::new());
            provide_context(Toasts::new());
            provide_context(Pending::new());
            if let Some(place) = place {
                provide_card_place(place);
            }
            view! { <Router>{card_for(entity)}</Router> }
        },
    ));
    host
}

fn has(host: &web_sys::Element, selector: &str) -> bool {
    host.query_selector(selector).unwrap().is_some()
}

fn water_heater() -> EntityView {
    let mut e = entity(
        "water_heater.tank",
        Kind::WaterHeater,
        StateValue::WaterHeater(WaterHeaterState {
            mode: Some("heat".into()),
            current: Some(15.0),
            target: Some(55.0),
        }),
    );
    e.capabilities.hvac_modes = vec!["heat".into(), "off".into()];
    e
}

#[wasm_bindgen_test]
async fn water_heater_tile_has_menu_not_stepper() {
    let host = mount(water_heater(), None);
    tick().await;
    assert!(has(
        &host,
        ".card__trailing button[aria-label=\"More options\"]"
    ));
    assert!(!has(&host, ".stepper"));
    assert!(!has(&host, "select"));
}

#[wasm_bindgen_test]
async fn water_heater_sheet_has_full_controls() {
    let host = mount(water_heater(), Some(CardPlace::Sheet));
    tick().await;
    assert!(has(&host, ".stepper"));
    // The sheet is where the menu leads: no menu there.
    assert!(!has(&host, ".card__more"));
    assert!(has(&host, "select"));
}

fn temperature(last_update: Option<u64>) -> EntityView {
    let mut e = entity("sensor.salon_t", Kind::Sensor, StateValue::Number(20.66));
    e.device_class = Some("temperature".into());
    e.unit = Some("°C".into());
    e.state.last_update = last_update;
    e
}

fn now_s() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

#[wasm_bindgen_test]
async fn temperature_card_without_history_has_no_curve() {
    let host = mount(temperature(None), None);
    tick().await;
    gloo_timers::future::TimeoutFuture::new(50).await;
    tick().await;
    assert!(!has(&host, "svg.spark"));
    let value = host.query_selector(".temp__value").unwrap().unwrap();
    assert_eq!(value.text_content().unwrap(), "20.7 °C");
    assert!(has(&host, ".card--cool"));
}

#[wasm_bindgen_test]
async fn temperature_badge_from_last_update() {
    let host = mount(temperature(Some(now_s() - 60)), None);
    tick().await;
    let badge = host
        .query_selector(".card__trailing .card__badge")
        .unwrap()
        .unwrap();
    assert_eq!(badge.text_content().unwrap(), "Live");

    let host = mount(temperature(None), None);
    tick().await;
    assert!(!has(&host, ".card__badge"));
}

fn climate(mode: &str) -> EntityView {
    let mut e = entity(
        "climate.bedroom",
        Kind::Climate,
        StateValue::Climate(ClimateState {
            current: Some(22.0),
            target: Some(15.5),
            mode: Some(mode.into()),
            fan_mode: Some("low".into()),
            action: None,
        }),
    );
    e.capabilities.min_temp = Some(7.0);
    e.capabilities.max_temp = Some(30.0);
    e.capabilities.temp_step = Some(0.5);
    e.capabilities.hvac_modes = vec!["heat".into(), "cool".into(), "off".into()];
    e.capabilities.fan_modes = vec!["low".into(), "high".into()];
    e
}

/// Mounts `entity` the way a room section does: from the house, rebuilt on every change of it.
/// Returns the host, the house and the toasts (a refused command leaves one).
fn mount_live(entity: EntityView) -> (web_sys::Element, House, Toasts) {
    mount_copies(entity, 1)
}

/// Like [`mount_live`], with the card shown `copies` times (Favorites and its room on Home).
fn mount_copies(entity: EntityView, copies: usize) -> (web_sys::Element, House, Toasts) {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let house = House::new();
    let toasts = Toasts::new();
    let id = entity.id.clone();
    house.entities.update(|m| {
        m.insert(id.clone(), entity);
    });
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(house);
            provide_context(toasts);
            provide_context(Pending::new());
            let card = Memo::new(move |_| house.entities.with(|m| m.get(&id).cloned()));
            let cards = (0..copies)
                .map(|_| view! { {move || card.get().map(card_for)} })
                .collect_view();
            view! { <Router>{cards}</Router> }
        },
    ));
    (host, house, toasts)
}

fn ring(host: &web_sys::Element) -> web_sys::Element {
    host.query_selector("[role=slider]").unwrap().unwrap()
}

fn attr(el: &web_sys::Element, name: &str) -> String {
    el.get_attribute(name).unwrap_or_default()
}

fn key(el: &web_sys::Element, key: &str) {
    let init = web_sys::KeyboardEventInit::new();
    init.set_key(key);
    init.set_bubbles(true);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
}

/// A pointer event at `deg` on the ring's circle (clockwise from the right).
fn pointer(target: &web_sys::EventTarget, kind: &str, el: &web_sys::Element, deg: f64) {
    let buttons = if kind == "pointerup" { 0 } else { 1 };
    pointer_at(target, kind, el, deg, 1.0 / 3.0, buttons);
}

/// A pointer event at `deg`, `share` of the ring element's width from its centre.
fn pointer_at(
    target: &web_sys::EventTarget,
    kind: &str,
    el: &web_sys::Element,
    deg: f64,
    share: f64,
    buttons: u16,
) {
    let rect = el.get_bounding_client_rect();
    let (cx, cy) = (
        rect.left() + rect.width() / 2.0,
        rect.top() + rect.height() / 2.0,
    );
    let r = rect.width() * share;
    let init = web_sys::PointerEventInit::new();
    init.set_client_x((cx + r * deg.to_radians().cos()) as i32);
    init.set_client_y((cy + r * deg.to_radians().sin()) as i32);
    init.set_buttons(buttons);
    init.set_pointer_id(1);
    init.set_bubbles(true);
    let ev = web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap();
    target.dispatch_event(&ev).unwrap();
}

async fn wait(ms: u32) {
    gloo_timers::future::TimeoutFuture::new(ms).await;
    tick().await;
}

#[wasm_bindgen_test]
async fn ring_is_an_accessible_slider() {
    let host = mount(climate("heat"), None);
    tick().await;
    let r = ring(&host);
    assert_eq!(attr(&r, "aria-valuenow"), "15.5");
    assert_eq!(attr(&r, "aria-valuemin"), "7");
    assert_eq!(attr(&r, "aria-valuemax"), "30");
    assert_eq!(attr(&r, "tabindex"), "0");
    assert!(host.text_content().unwrap().contains("22"));
}

#[wasm_bindgen_test]
async fn keys_send_one_command() {
    let (host, _house, toasts) = mount_live(climate("heat"));
    tick().await;
    for _ in 0..3 {
        key(&ring(&host), "ArrowUp");
        tick().await;
    }
    assert_eq!(attr(&ring(&host), "aria-valuenow"), "17");
    wait(300).await;
    assert_eq!(
        toasts.0.with_untracked(Vec::len),
        0,
        "sent before the pause"
    );
    wait(900).await;
    assert_eq!(
        toasts.0.with_untracked(Vec::len),
        1,
        "one command for three keys"
    );
}

#[wasm_bindgen_test]
async fn drag_survives_a_remount() {
    let (host, house, toasts) = mount_live(climate("heat"));
    tick().await;
    let r = ring(&host);
    pointer(&r, "pointerdown", &r, 270.0);
    tick().await;
    assert_eq!(attr(&ring(&host), "aria-valuenow"), "18.5");
    // The room temperature changes mid-drag: the card is rebuilt.
    house.entities.update(|m| {
        if let Some(StateValue::Climate(c)) =
            m.get_mut("climate.bedroom").map(|e| &mut e.state.value)
        {
            c.current = Some(23.0);
        }
    });
    tick().await;
    assert_eq!(
        attr(&ring(&host), "aria-valuenow"),
        "18.5",
        "the draft survives"
    );
    let r = ring(&host);
    pointer(&r, "pointermove", &r, 0.0);
    tick().await;
    let window: web_sys::EventTarget = web_sys::window().unwrap().into();
    pointer(&window, "pointerup", &r, 0.0);
    tick().await;
    assert_eq!(attr(&ring(&host), "aria-valuenow"), "26");
    wait(500).await;
    assert_eq!(
        toasts.0.with_untracked(Vec::len),
        1,
        "one command for the whole drag"
    );
}

#[wasm_bindgen_test]
async fn off_ring_is_disabled() {
    let (host, _house, toasts) = mount_live(climate("off"));
    tick().await;
    let r = ring(&host);
    assert_eq!(attr(&r, "aria-disabled"), "true");
    key(&r, "ArrowUp");
    pointer(&r, "pointerdown", &r, 270.0);
    tick().await;
    assert_eq!(attr(&ring(&host), "aria-valuenow"), "15.5");
    assert!(r.text_content().unwrap().contains("Off"));
    wait(900).await;
    assert_eq!(toasts.0.with_untracked(Vec::len), 0);
}

#[wasm_bindgen_test]
async fn climate_sheet_has_full_controls() {
    let host = mount(climate("heat"), Some(CardPlace::Sheet));
    tick().await;
    assert!(has(&host, ".pick__select"));
    let host = mount(climate("heat"), None);
    tick().await;
    assert!(!has(&host, ".pick__select"));
    assert!(has(&host, ".card__trailing select.card__badge"));
}

fn vacuum(activity: &str) -> EntityView {
    let mut e = entity(
        "vacuum.robot",
        Kind::Vacuum,
        StateValue::Vacuum(VacuumState {
            state: Some(activity.into()),
            battery: Some(100),
            fan_speed: Some("turbo".into()),
        }),
    );
    e.capabilities.vacuum_actions = ["start", "pause", "return_to_base", "clean_spot"]
        .map(String::from)
        .to_vec();
    e.capabilities.fan_speeds = vec!["quiet".into(), "turbo".into()];
    e
}

#[wasm_bindgen_test]
async fn vacuum_tile_is_wide_with_sentence() {
    let host = mount(vacuum("docked"), None);
    tick().await;
    assert!(has(&host, ".card--wide"));
    assert!(host.text_content().unwrap().contains("At home."));
    assert!(host.text_content().unwrap().contains("100"));
    assert!(has(&host, "svg.robot"));
    assert!(!has(&host, "select"));
    // The header, Start and Send home.
    assert_eq!(host.query_selector_all("button").unwrap().length(), 3);
    assert!(has(&host, "button[aria-label=\"Send home\"]"));
}

#[wasm_bindgen_test]
async fn vacuum_drawing_sweeps_dust_only_while_cleaning() {
    let count = |host: &web_sys::Element, selector: &str| {
        host.query_selector_all(selector).unwrap().length()
    };
    let host = mount(vacuum("cleaning"), None);
    tick().await;
    assert_eq!(count(&host, "svg.robot--cleaning .robot__brush"), 1);
    assert_eq!(count(&host, "svg.robot .robot__dust"), 22);
    // Docked: the brush is still drawn, the dust is gone.
    let host = mount(vacuum("docked"), None);
    tick().await;
    assert_eq!(count(&host, "svg.robot .robot__brush"), 1);
    assert_eq!(count(&host, "svg.robot .robot__dust"), 0);
}

#[wasm_bindgen_test]
async fn vacuum_sheet_has_full_controls() {
    let host = mount(vacuum("cleaning"), Some(CardPlace::Sheet));
    tick().await;
    assert!(!has(&host, ".card--wide"));
    assert!(has(&host, "select"));
    assert!(has(&host, "button[aria-label=\"Spot\"]"));
    assert!(host.text_content().unwrap().contains("Pause"));
}

fn rings(host: &web_sys::Element) -> Vec<web_sys::Element> {
    let list = host.query_selector_all("[role=slider]").unwrap();
    (0..list.length())
        .map(|i| list.item(i).unwrap().dyn_into().unwrap())
        .collect()
}

fn window_target() -> web_sys::EventTarget {
    web_sys::window().unwrap().into()
}

#[wasm_bindgen_test]
async fn the_ring_you_drag_shows_the_value_when_shown_twice() {
    let (host, _house, toasts) = mount_copies(climate("heat"), 2);
    tick().await;
    let first = rings(&host)[0].clone();
    pointer(&first, "pointerdown", &first, 270.0);
    tick().await;
    for r in rings(&host) {
        assert_eq!(
            attr(&r, "aria-valuenow"),
            "18.5",
            "every copy follows the drag"
        );
    }
    pointer(&window_target(), "pointerup", &first, 270.0);
    wait(500).await;
    assert_eq!(
        toasts.0.with_untracked(Vec::len),
        1,
        "one command for two copies"
    );
}

#[wasm_bindgen_test]
async fn keys_move_the_ring_when_shown_twice() {
    let (host, _house, _toasts) = mount_copies(climate("heat"), 2);
    tick().await;
    for _ in 0..3 {
        key(&rings(&host)[0], "ArrowUp");
        tick().await;
    }
    assert_eq!(attr(&rings(&host)[0], "aria-valuenow"), "17");
}

#[wasm_bindgen_test]
async fn a_tap_on_the_centre_sends_nothing() {
    let (host, _house, toasts) = mount_live(climate("heat"));
    tick().await;
    let r = ring(&host);
    pointer_at(&r, "pointerdown", &r, 0.0, 0.02, 1);
    tick().await;
    pointer_at(&window_target(), "pointerup", &r, 0.0, 0.02, 0);
    wait(500).await;
    assert_eq!(attr(&ring(&host), "aria-valuenow"), "15.5");
    assert_eq!(toasts.0.with_untracked(Vec::len), 0);
}

#[wasm_bindgen_test]
async fn a_lost_release_is_sent_at_once_not_on_a_later_tap() {
    let (host, _house, toasts) = mount_live(climate("heat"));
    tick().await;
    let r = ring(&host);
    pointer(&r, "pointerdown", &r, 270.0);
    tick().await;
    // The button came up outside the window: the next move has no button down.
    pointer_at(&window_target(), "pointermove", &r, 300.0, 1.0 / 3.0, 0);
    wait(500).await;
    assert_eq!(
        toasts.0.with_untracked(Vec::len),
        1,
        "sent when the drag was seen to end"
    );
    pointer(&window_target(), "pointerup", &r, 300.0);
    wait(500).await;
    assert_eq!(
        toasts.0.with_untracked(Vec::len),
        1,
        "a later release sends nothing more"
    );
}

#[wasm_bindgen_test]
async fn focus_stays_on_the_ring_across_a_remount() {
    let (host, house, _toasts) = mount_live(climate("heat"));
    tick().await;
    ring(&host)
        .dyn_into::<web_sys::SvgElement>()
        .unwrap()
        .focus()
        .unwrap();
    house.entities.update(|m| {
        if let Some(StateValue::Climate(c)) =
            m.get_mut("climate.bedroom").map(|e| &mut e.state.value)
        {
            c.current = Some(23.5);
        }
    });
    wait(50).await;
    let active = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .active_element()
        .unwrap();
    assert!(
        active.is_same_node(Some(&ring(&host))),
        "focus is on the new ring"
    );
}

fn player(state: &str, color: Option<&str>) -> EntityView {
    let idle = state == "idle";
    let mut e = entity(
        "music_player",
        Kind::MediaPlayer,
        StateValue::MediaPlayer(nidoo_core::model::MediaPlayerState {
            state: Some(state.into()),
            title: (!idle).then(|| "Get Lucky".into()),
            artist: (!idle).then(|| "Daft Punk".into()),
            album: (!idle).then(|| "Random Access Memories".into()),
            image_url: (!idle).then(|| "https://img.example/cover.jpg".into()),
            color: color.map(Into::into),
            duration_s: (!idle).then_some(248.0),
            position_s: (!idle).then_some(30.0),
            position_at_ms: None,
        }),
    );
    e.capabilities.media_features = ["play", "pause", "next", "previous", "play_media"]
        .iter()
        .map(|f| (*f).to_string())
        .collect();
    e
}

fn tinted(host: &web_sys::Element) -> bool {
    host.query_selector(".card")
        .unwrap()
        .and_then(|card| card.get_attribute("style"))
        .is_some_and(|style| style.contains("--media-tint"))
}

#[wasm_bindgen_test]
async fn media_card_playing_shows_cover_title_and_pause() {
    let host = mount(player("playing", Some("#3d2466")), None);
    tick().await;
    assert!(has(&host, ".card--wide"));
    assert!(has(&host, "img[src=\"https://img.example/cover.jpg\"]"));
    let text = host.text_content().unwrap();
    assert!(text.contains("Get Lucky"), "{text}");
    assert!(
        text.contains("Daft Punk · Random Access Memories"),
        "{text}"
    );
    assert!(has(&host, "button[aria-label=\"Pause\"]"));
    assert!(has(&host, "button[aria-label=\"Next\"]"));
    assert!(has(&host, "button[aria-label=\"Previous\"]"));
    assert!(has(&host, ".media__progress"));
    assert!(tinted(&host));
}

#[wasm_bindgen_test]
async fn media_card_paused_shows_play() {
    let host = mount(player("paused", Some("#3d2466")), None);
    tick().await;
    assert!(has(&host, "button[aria-label=\"Play\"]"));
    assert!(!has(&host, "button[aria-label=\"Pause\"]"));
}

#[wasm_bindgen_test]
async fn media_card_idle_shows_the_placeholder_and_play() {
    let host = mount(player("idle", None), None);
    tick().await;
    assert!(has(&host, ".media__placeholder"));
    assert!(!has(&host, "img"));
    let text = host.text_content().unwrap();
    assert!(text.contains("Nothing playing"), "{text}");
    assert!(text.contains("Play some music"), "{text}");
    assert!(has(&host, "button[aria-label=\"Play\"]"));
    assert!(!has(&host, ".media__progress"));
}

#[wasm_bindgen_test]
async fn media_card_without_color_is_plain() {
    let host = mount(player("playing", None), None);
    tick().await;
    assert!(!tinted(&host));
}

#[wasm_bindgen_test]
async fn media_card_hides_buttons_the_player_lacks() {
    let mut e = player("playing", None);
    e.capabilities.media_features = vec!["play".into(), "pause".into()];
    let host = mount(e, None);
    tick().await;
    assert!(has(&host, "button[aria-label=\"Pause\"]"));
    assert!(!has(&host, "button[aria-label=\"Next\"]"));
    assert!(!has(&host, "button[aria-label=\"Previous\"]"));
}

#[wasm_bindgen_test]
async fn media_card_offline() {
    let mut e = player("playing", Some("#3d2466"));
    e.state.availability = nidoo_core::model::Availability::Offline;
    let host = mount(e, None);
    tick().await;
    assert!(has(&host, ".card--offline"));
}

#[wasm_bindgen_test]
async fn media_card_follows_the_language() {
    let lang = RwSignal::new(nidoo_web::i18n::Lang::Fr);
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    std::mem::forget(leptos::mount::mount_to(
        host.clone().unchecked_into(),
        move || {
            provide_context(House::new());
            provide_context(Toasts::new());
            provide_context(Pending::new());
            provide_context(lang);
            view! { <Router>{card_for(player("idle", None))}</Router> }
        },
    ));
    tick().await;
    assert!(host.text_content().unwrap().contains("Rien ne joue"));
    assert!(has(&host, "button[aria-label=\"Lecture\"]"));
    lang.set(nidoo_web::i18n::Lang::En);
    tick().await;
    assert!(host.text_content().unwrap().contains("Nothing playing"));
}
