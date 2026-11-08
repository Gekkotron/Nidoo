#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use nidoo_climate::doc::{
    Block, Day, Decision, HhMm, Mode, ModeRules, Rules, Setpoint, Source, ZoneSettings, ZoneState,
};
use nidoo_climate::view::{LearnStatus, ZoneView};
use nidoo_web::climate::eco_pill::EcoPill;
use nidoo_web::climate::proposal::RulesPanel;
use nidoo_web::climate::room_list::RoomList;
use nidoo_web::climate::room_panel::RoomPanel;
use nidoo_web::climate::target_stepper::TargetStepper;
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use std::sync::{Arc, Mutex};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{Element, HtmlElement, HtmlInputElement};
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

async fn settle() {
    tick().await;
    gloo_timers::future::TimeoutFuture::new(50).await;
    tick().await;
}

/// Tests load no stylesheet: the columns need a height for the vertical pointer mapping.
fn install_layout() {
    let document = web_sys::window().unwrap().document().unwrap();
    if document.get_element_by_id("rooms-test-layout").is_some() {
        return;
    }
    let style = document.create_element("style").unwrap();
    style.set_id("rooms-test-layout");
    style.set_text_content(Some(
        ".climate-timeline__col { position: relative; height: 480px; } \
         .climate-timeline__bar { position: absolute; left: 0; right: 0; }",
    ));
    document.head().unwrap().append_child(&style).unwrap();
}

fn find_button(host: &Element, label: &str) -> Option<HtmlElement> {
    let all = host.query_selector_all("button").unwrap();
    (0..all.length())
        .filter_map(|i| all.item(i))
        .map(|n| n.unchecked_into::<HtmlElement>())
        .find(|b| b.text_content().as_deref() == Some(label))
}

async fn mount_rules(zone: ZoneView) -> Element {
    install_layout();
    let current = RwSignal::new(zone);
    let host = host();
    let mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(Toasts::new());
        view! {
            <RulesPanel
                zone=current.into()
                on_changed=Callback::new(|_| {})
                now=Signal::from((Day::Mon, 0))
            />
        }
    });
    std::mem::forget(mounted);
    tick().await;
    host
}

#[wasm_bindgen_test]
async fn a_room_without_rules_can_create_a_schedule() {
    let stub = FetchStub::install();
    stub.reply("PUT", "/api/v1/climate/zones/bedroom-2", 200, "{}");
    let host = mount_rules(zone("bedroom-2", "Bedroom 2", ZoneState::Pending, None)).await;
    assert!(
        host.text_content()
            .unwrap()
            .contains("No approved schedule yet")
    );

    find_button(&host, "Create schedule")
        .expect("Create schedule")
        .click();
    settle().await;
    assert_eq!(
        host.query_selector_all(".climate-timeline__col")
            .unwrap()
            .length(),
        7
    );
    let base: HtmlInputElement = host
        .query_selector(".climate-timeline__base-input")
        .unwrap()
        .expect("base field")
        .unchecked_into();
    assert_eq!(base.value(), "16");
    base.set_value("15.5");
    base.dispatch_event(&web_sys::Event::new("change").unwrap())
        .unwrap();
    tick().await;

    // Tap Monday just after 18:00 (blocks snap down to the quarter): two hours at base + 2.
    let monday = host
        .query_selector(".climate-timeline__col")
        .unwrap()
        .unwrap();
    let rect = monday.get_bounding_client_rect();
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_client_x((rect.left() + rect.width() / 2.0) as i32);
    init.set_client_y((rect.top() + rect.height() * 0.76) as i32);
    monday
        .dispatch_event(&web_sys::PointerEvent::new_with_event_init_dict("click", &init).unwrap())
        .unwrap();
    tick().await;

    find_button(&host, "Save").expect("Save").click();
    settle().await;
    let puts: Vec<_> = stub
        .requests()
        .into_iter()
        .filter(|r| r.method == "PUT")
        .collect();
    assert_eq!(puts.len(), 1, "{puts:?}");
    assert_eq!(puts[0].url, "/api/v1/climate/zones/bedroom-2");
    let body = &puts[0].body;
    assert!(body.contains("\"base\":15.5"), "{body}");
    assert!(body.contains("\"setpoint\":17.5"), "{body}");
    assert!(body.contains("18:00") && body.contains("20:00"), "{body}");
}

#[wasm_bindgen_test]
async fn cancelling_a_new_schedule_goes_back_to_none() {
    let host = mount_rules(zone("bedroom-3", "Bedroom 3", ZoneState::Pending, None)).await;
    find_button(&host, "Create schedule")
        .expect("Create schedule")
        .click();
    tick().await;
    find_button(&host, "Cancel").expect("Cancel").click();
    tick().await;
    assert!(
        host.text_content()
            .unwrap()
            .contains("No approved schedule yet")
    );
    assert!(
        host.query_selector(".climate-timeline__col")
            .unwrap()
            .is_none()
    );
}

fn zone(id: &str, name: &str, state: ZoneState, setpoint: Option<f64>) -> ZoneView {
    ZoneView {
        zone: id.into(),
        name: name.into(),
        orphan: false,
        settings: ZoneSettings {
            state,
            ..ZoneSettings::default()
        },
        settings_invalid: None,
        entities: vec![],
        reading: Some(19.0),
        reading_age_s: Some(30),
        room_sensor: None,
        thermometer: None,
        correction_step_at: None,
        presence: None,
        decision: setpoint.map(|t| Decision {
            zone: id.into(),
            mode: Some(Mode::Heat),
            setpoint: Some(Setpoint::Temp(t)),
            source: Source::Schedule,
            cause: None,
            reason: String::new(),
            reading: None,
            reading_age_s: None,
            outdoor_mean: None,
            home: true,
            holiday: false,
            applied: true,
            at: 0,
            entities: Default::default(),
            eco_restore: Default::default(),
            adjust: None,
        }),
        overrides: vec![],
        eco: None,
        proposal: None,
        learn: LearnStatus::Off,
    }
}

fn host() -> Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    host.set_attribute("style", "width: 900px").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    host
}

fn button(host: &Element, name: &str) -> HtmlElement {
    let all = host.query_selector_all(".climate-room").unwrap();
    (0..all.length())
        .filter_map(|i| all.item(i))
        .map(|n| n.unchecked_into::<HtmlElement>())
        .find(|b| {
            b.query_selector(".climate-room__name")
                .unwrap()
                .and_then(|n| n.text_content())
                .as_deref()
                == Some(name)
        })
        .expect("room button")
}

#[wasm_bindgen_test]
async fn room_list_marks_the_selected_room_and_selects_on_click() {
    let zones = vec![
        zone("living-room", "Living room", ZoneState::Pending, None),
        zone("bedroom-1", "Bedroom 1", ZoneState::Active, Some(18.0)),
        zone("bedroom-2", "Bedroom 2", ZoneState::Active, Some(21.0)),
    ];
    let picked = Arc::new(Mutex::new(None::<String>));
    let sink = picked.clone();
    let host = host();
    let mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        let sink = sink.clone();
        provide_context(nidoo_web::components::now::install_now());
        view! {
            <RoomList
                zones=Signal::from(zones.clone())
                selected=Signal::from(Some("bedroom-1".to_owned()))
                on_select=Callback::new(move |id: String| *sink.lock().unwrap() = Some(id))
            />
        }
    });
    std::mem::forget(mounted);
    tick().await;

    let bedroom = button(&host, "Bedroom 1");
    assert_eq!(
        bedroom.get_attribute("aria-current").as_deref(),
        Some("true")
    );
    assert!(
        button(&host, "Living room")
            .get_attribute("aria-current")
            .is_none()
    );
    let sub = bedroom
        .query_selector(".climate-room__sub")
        .unwrap()
        .and_then(|n| n.text_content());
    assert_eq!(sub.as_deref(), Some("Heat 18° · Active"));
    let dot = |name: &str| {
        button(&host, name)
            .query_selector(".climate-room__dot")
            .unwrap()
            .expect("dot")
            .class_list()
    };
    assert!(dot("Bedroom 1").contains("climate-temp--mid"));
    assert!(dot("Living room").contains("climate-temp--none"));

    button(&host, "Bedroom 2").click();
    tick().await;
    assert_eq!(picked.lock().unwrap().as_deref(), Some("bedroom-2"));
}

#[wasm_bindgen_test]
async fn refetch_keeps_the_draft_and_the_open_editor() {
    let mut start = zone("living-room", "Living room", ZoneState::Active, Some(16.0));
    start.settings.rules = Rules {
        heat: Some(ModeRules {
            base: Setpoint::Temp(15.0),
            blocks: vec![Block {
                days: vec![Day::Mon],
                from: HhMm::parse("07:00").unwrap(),
                to: HhMm::parse("09:00").unwrap(),
                setpoint: Setpoint::Temp(19.0),
                evidence: None,
            }],
        }),
        cool: None,
    };
    let current = RwSignal::new(start.clone());
    let host = host();
    let mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(Toasts::new());
        view! {
            <RulesPanel
                zone=current.into()
                on_changed=Callback::new(|_| {})
                now=Signal::from((Day::Mon, 0))
            />
        }
    });
    std::mem::forget(mounted);
    tick().await;

    let find = |label: &str| {
        let all = host.query_selector_all("button").unwrap();
        (0..all.length())
            .filter_map(|i| all.item(i))
            .map(|n| n.unchecked_into::<HtmlElement>())
            .find(|b| b.text_content().as_deref() == Some(label))
    };
    find("Edit schedule").expect("Edit schedule").click();
    tick().await;
    let bar: HtmlElement = host
        .query_selector(".climate-timeline__bar")
        .unwrap()
        .expect("a bar")
        .unchecked_into();
    bar.click();
    tick().await;
    assert!(
        host.query_selector(".climate-timeline__editor")
            .unwrap()
            .is_some()
    );

    // Edit the draft: the block goes to 21 °C.
    let temp: HtmlInputElement = host
        .query_selector(".climate-timeline__editor input[inputmode=decimal]")
        .unwrap()
        .expect("temperature field")
        .unchecked_into();
    temp.set_value("21");
    temp.dispatch_event(&web_sys::Event::new("change").unwrap())
        .unwrap();
    tick().await;

    // A refetch: a fresh reading, and approved rules changed elsewhere (the block at 17 °C).
    let mut fresh = start;
    fresh.reading = Some(19.4);
    if let Some(heat) = fresh.settings.rules.heat.as_mut() {
        heat.blocks[0].setpoint = Setpoint::Temp(17.0);
    }
    current.set(fresh);
    tick().await;

    let shown = host
        .query_selector(".climate-timeline__bar .climate-timeline__temp")
        .unwrap()
        .and_then(|n| n.text_content());
    assert_eq!(
        shown.as_deref(),
        Some("21°"),
        "the draft wins over the refetch"
    );

    assert!(
        host.query_selector(".climate-timeline__editor")
            .unwrap()
            .is_some(),
        "the block editor stays open"
    );
    assert!(find("Save").is_some(), "still editing");
}

/// The room list and panel mounted with a language the test flips.
fn mount_room(zone: ZoneView, lang: RwSignal<Lang>) -> Element {
    let host = host();
    let zones = vec![zone.clone()];
    let current = RwSignal::new(zone);
    let mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(Toasts::new());
        provide_context(House::new());
        provide_context(nidoo_web::components::now::install_now());
        provide_context(lang);
        view! {
            <RoomList
                zones=Signal::from(zones.clone())
                selected=Signal::from(Some("bedroom-1".to_owned()))
                on_select=Callback::new(|_: String| {})
            />
            <RoomPanel
                zone=current.into()
                tz=Signal::from(chrono_tz::Tz::UTC)
                on_changed=Callback::new(|_| {})
            />
        }
    });
    std::mem::forget(mounted);
    host
}

#[wasm_bindgen_test]
async fn the_room_follows_the_language_without_a_remount() {
    let lang = RwSignal::new(Lang::Fr);
    let host = mount_room(
        zone("bedroom-1", "Bedroom 1", ZoneState::Active, Some(18.5)),
        lang,
    );
    tick().await;
    let text = || host.text_content().unwrap_or_default();
    for french in [
        "Consigne ·",
        "Chauffage",
        "18,5°",
        "Maintenant",
        "Programmation",
        "Aucune programmation approuvée pour l'instant",
        "Cette semaine",
        "Active",
        "Arrêt",
        "En attente",
    ] {
        assert!(text().contains(french), "{french} in {}", text());
    }
    for hidden in [
        "Target",
        "Now",
        "Heat",
        "This week",
        "Pending",
        // In the room settings, behind the ⚙.
        "Apprendre maintenant",
        "Décisions récentes",
    ] {
        assert!(!text().contains(hidden), "{hidden} in {}", text());
    }
    let raise: HtmlElement = host
        .query_selector(".climate-stepper__step:last-of-type")
        .unwrap()
        .expect("+ button")
        .unchecked_into();
    assert_eq!(
        raise.get_attribute("aria-label").as_deref(),
        Some("Monter la consigne")
    );

    lang.set(Lang::En);
    tick().await;
    for english in ["Target ·", "Now", "Heat", "18.5°", "This week", "Pending"] {
        assert!(text().contains(english), "{english} in {}", text());
    }
    for hidden in [
        "Consigne",
        "Maintenant",
        "Chauffage",
        "Cette semaine",
        "Learn now",
        "Recent decisions",
    ] {
        assert!(!text().contains(hidden), "{hidden} in {}", text());
    }
    assert_eq!(
        raise.get_attribute("aria-label").as_deref(),
        Some("Raise target")
    );
}

const OVERRIDE_URL: &str = "/api/v1/climate/zones/living-room/override";

/// The − / + stepper alone, in English, over a zone the test can change.
async fn mount_stepper(current: RwSignal<ZoneView>) -> Element {
    let host = host();
    let mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(Toasts::new());
        provide_context(RwSignal::new(Lang::En));
        view! {
            <TargetStepper
                zone=current.into()
                now=Signal::from((Day::Mon, 0))
                on_changed=Callback::new(|_| {})
            />
        }
    });
    std::mem::forget(mounted);
    settle().await;
    host
}

fn stepper_value(host: &Element) -> String {
    host.query_selector(".climate-stepper__value")
        .unwrap()
        .expect("target value")
        .text_content()
        .unwrap_or_default()
}

fn step_button(host: &Element, label: &str) -> HtmlElement {
    let all = host.query_selector_all(".climate-stepper__step").unwrap();
    (0..all.length())
        .filter_map(|i| all.item(i))
        .map(|n| n.unchecked_into::<HtmlElement>())
        .find(|b| b.get_attribute("aria-label").as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no {label} button"))
}

fn posts(stub: &FetchStub) -> Vec<String> {
    stub.requests()
        .into_iter()
        .filter(|r| r.method == "POST" && r.url == OVERRIDE_URL)
        .map(|r| r.body)
        .collect()
}

async fn past_the_pause() {
    gloo_timers::future::TimeoutFuture::new(1_600).await;
    settle().await;
}

#[wasm_bindgen_test]
async fn taps_send_one_hold_after_the_pause() {
    let stub = FetchStub::install();
    stub.reply("POST", OVERRIDE_URL, 200, "{}");
    let current = RwSignal::new(zone(
        "living-room",
        "Living room",
        ZoneState::Active,
        Some(19.0),
    ));
    let host = mount_stepper(current).await;
    assert_eq!(stepper_value(&host), "19°");

    let raise = step_button(&host, "Raise target");
    for _ in 0..3 {
        raise.click();
        tick().await;
    }
    assert_eq!(stepper_value(&host), "20.5°");
    assert!(posts(&stub).is_empty(), "nothing sent while tapping");

    past_the_pause().await;
    let sent = posts(&stub);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].contains("\"setpoint\":20.5"), "{}", sent[0]);
    assert!(sent[0].contains("\"minutes\":120"), "{}", sent[0]);
    let chips = host
        .query_selector(".climate-stepper__chips")
        .unwrap()
        .expect("duration chips");
    let text = chips.text_content().unwrap_or_default();
    assert!(text.contains("1 h") && text.contains("2 h"), "{text}");
    assert!(!text.contains("Until next change"), "{text}");
}

#[wasm_bindgen_test]
async fn a_refetch_during_the_wait_keeps_the_new_target() {
    let stub = FetchStub::install();
    stub.reply("POST", OVERRIDE_URL, 200, "{}");
    let base = zone("living-room", "Living room", ZoneState::Active, Some(19.0));
    let current = RwSignal::new(base.clone());
    let host = mount_stepper(current).await;

    step_button(&host, "Raise target").click();
    tick().await;
    assert_eq!(stepper_value(&host), "19.5°");
    current.set(ZoneView {
        reading: Some(18.0),
        ..base.clone()
    });
    settle().await;
    assert_eq!(stepper_value(&host), "19.5°", "a refetch before the send");

    past_the_pause().await;
    assert_eq!(posts(&stub).len(), 1);
    assert_eq!(stepper_value(&host), "19.5°", "sent, not yet refetched");
    current.set(zone(
        "living-room",
        "Living room",
        ZoneState::Active,
        Some(19.5),
    ));
    settle().await;
    assert_eq!(stepper_value(&host), "19.5°");
    current.set(zone(
        "living-room",
        "Living room",
        ZoneState::Active,
        Some(18.0),
    ));
    settle().await;
    assert_eq!(stepper_value(&host), "18°", "the hub's value again");
}

#[wasm_bindgen_test]
async fn a_chip_resends_with_its_duration() {
    let stub = FetchStub::install();
    stub.reply("POST", OVERRIDE_URL, 200, "{}");
    let current = RwSignal::new(zone(
        "living-room",
        "Living room",
        ZoneState::Active,
        Some(19.0),
    ));
    let host = mount_stepper(current).await;
    step_button(&host, "Raise target").click();
    past_the_pause().await;
    assert_eq!(posts(&stub).len(), 1);

    find_button(&host, "1 h").expect("1 h chip").click();
    settle().await;
    let sent = posts(&stub);
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(sent[1].contains("\"setpoint\":19.5"), "{}", sent[1]);
    assert!(sent[1].contains("\"minutes\":60"), "{}", sent[1]);
    assert!(
        host.query_selector(".climate-stepper__chips")
            .unwrap()
            .is_none()
    );
}

#[wasm_bindgen_test]
async fn a_pending_room_cannot_step() {
    let current = RwSignal::new(zone(
        "living-room",
        "Living room",
        ZoneState::Pending,
        Some(19.0),
    ));
    let host = mount_stepper(current).await;
    assert_eq!(stepper_value(&host), "19°");
    for label in ["Lower target", "Raise target"] {
        assert!(
            step_button(&host, label).has_attribute("disabled"),
            "{label}"
        );
    }
}

const ECO_URL: &str = "/api/v1/climate/zones/living-room/eco";

/// The eco pill alone, in English, in UTC.
async fn mount_eco(zone: ZoneView) -> Element {
    let host = host();
    let current = RwSignal::new(zone);
    let mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(Toasts::new());
        provide_context(RwSignal::new(Lang::En));
        provide_context(nidoo_web::components::now::install_now());
        view! {
            <EcoPill
                zone=current.into()
                tz=Signal::from(chrono_tz::Tz::UTC)
                on_changed=Callback::new(|_| {})
            />
        }
    });
    std::mem::forget(mounted);
    settle().await;
    host
}

fn requests_to(stub: &FetchStub, method: &str, url: &str) -> usize {
    stub.requests()
        .into_iter()
        .filter(|r| r.method == method && r.url == url)
        .count()
}

#[wasm_bindgen_test]
async fn eco_starts_from_the_pill() {
    let stub = FetchStub::install();
    stub.reply("PUT", ECO_URL, 200, "{}");
    let host = mount_eco(zone(
        "living-room",
        "Living room",
        ZoneState::Active,
        Some(19.0),
    ))
    .await;

    find_button(&host, "Eco…").expect("eco pill").click();
    settle().await;
    let dialog = host
        .query_selector("dialog.popup--small")
        .unwrap()
        .expect("the eco popup");
    let date: HtmlInputElement = dialog
        .query_selector("input[type=date]")
        .unwrap()
        .expect("date")
        .unchecked_into();
    assert!(dialog.query_selector("input[type=time]").unwrap().is_some());
    date.set_value("2099-01-01");
    date.dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();
    tick().await;

    find_button(&host, "Start eco").expect("Start eco").click();
    settle().await;
    assert_eq!(requests_to(&stub, "PUT", ECO_URL), 1);
    assert!(host.query_selector("dialog").unwrap().is_none());
}

#[wasm_bindgen_test]
async fn a_running_eco_cancels_in_one_tap() {
    let stub = FetchStub::install();
    stub.reply("DELETE", ECO_URL, 200, "{}");
    let mut z = zone("living-room", "Living room", ZoneState::Active, Some(19.0));
    z.eco = Some(nidoo_climate::doc::RoomEco {
        until: 4_000_000_000,
        set_at: 0,
    });
    let host = mount_eco(z).await;
    let pill = host
        .query_selector(".climate-eco-pill")
        .unwrap()
        .expect("eco pill");
    let text = pill.text_content().unwrap_or_default();
    assert!(text.contains("Eco until"), "{text}");

    pill.query_selector("button[aria-label='Cancel eco']")
        .unwrap()
        .expect("cancel")
        .unchecked_into::<HtmlElement>()
        .click();
    settle().await;
    assert_eq!(requests_to(&stub, "DELETE", ECO_URL), 1);
}

#[wasm_bindgen_test]
async fn an_off_room_has_no_eco_pill() {
    let host = mount_eco(zone("living-room", "Living room", ZoneState::Off, None)).await;
    assert!(host.query_selector(".climate-eco-pill").unwrap().is_none());
}

/// The client ends a query with "&".
const DECISIONS_URL: &str = "/api/v1/climate/zones/living-room/decisions?limit=50&";

/// One room's panel, in English, over a zone the test can change.
async fn mount_panel(current: RwSignal<ZoneView>) -> Element {
    let host = host();
    let mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(Toasts::new());
        provide_context(House::new());
        provide_context(nidoo_web::components::now::install_now());
        provide_context(RwSignal::new(Lang::En));
        view! {
            <RoomPanel
                zone=current.into()
                tz=Signal::from(chrono_tz::Tz::UTC)
                on_changed=Callback::new(|_| {})
            />
        }
    });
    std::mem::forget(mounted);
    settle().await;
    host
}

fn reply_decisions(stub: &FetchStub) {
    let decision = zone("living-room", "Living room", ZoneState::Active, Some(19.0))
        .decision
        .unwrap();
    let log = nidoo_climate::view::DecisionLog {
        recent: vec![decision],
        older: vec![],
    };
    stub.reply(
        "GET",
        DECISIONS_URL,
        200,
        &serde_json::to_string(&log).unwrap(),
    );
}

async fn open_room_settings(host: &Element) -> Element {
    host.query_selector(".climate-card__gear")
        .unwrap()
        .expect("the room's ⚙")
        .unchecked_into::<HtmlElement>()
        .click();
    settle().await;
    host.query_selector("dialog.popup")
        .unwrap()
        .expect("the room settings")
}

#[wasm_bindgen_test]
async fn the_gear_opens_the_room_settings() {
    let stub = FetchStub::install();
    reply_decisions(&stub);
    let current = RwSignal::new(zone(
        "living-room",
        "Living room",
        ZoneState::Active,
        Some(19.0),
    ));
    let host = mount_panel(current).await;
    assert!(host.query_selector("dialog").unwrap().is_none());
    assert_eq!(
        requests_to(&stub, "GET", DECISIONS_URL),
        0,
        "fetched on open"
    );

    let dialog = open_room_settings(&host).await;
    let text = dialog.text_content().unwrap_or_default();
    for word in [
        "Room settings · Living room",
        "Room thermometer",
        "Learn now",
        "Recent decisions",
    ] {
        assert!(text.contains(word), "{word} in {text}");
    }
    assert!(
        dialog
            .query_selector(".climate-correction-card [role=switch]")
            .unwrap()
            .is_some()
    );
    assert_eq!(requests_to(&stub, "GET", DECISIONS_URL), 1);
}

#[wasm_bindgen_test]
async fn a_refetch_keeps_the_room_settings_open() {
    let stub = FetchStub::install();
    reply_decisions(&stub);
    let base = zone("living-room", "Living room", ZoneState::Active, Some(19.0));
    let current = RwSignal::new(base.clone());
    let host = mount_panel(current).await;
    open_room_settings(&host).await;
    assert!(
        host.query_selector(".climate-decisions__list li")
            .unwrap()
            .is_some()
    );

    current.set(ZoneView {
        reading: Some(18.0),
        ..base
    });
    settle().await;
    assert!(
        host.query_selector("dialog.popup[open]").unwrap().is_some(),
        "still open"
    );
    assert!(
        host.query_selector(".climate-decisions__list li")
            .unwrap()
            .is_some(),
        "the decisions stay"
    );
}

#[wasm_bindgen_test]
async fn an_override_shows_as_a_chip_and_clears() {
    let stub = FetchStub::install();
    stub.reply("DELETE", OVERRIDE_URL, 200, "{}");
    let mut z = zone("living-room", "Living room", ZoneState::Active, Some(19.0));
    z.overrides = vec![nidoo_climate::doc::Override {
        entity: None,
        setpoint: Some(Setpoint::Temp(21.0)),
        mode: None,
        until: 4_000_000_000,
        origin: nidoo_climate::doc::OverrideOrigin::User,
    }];
    let host = mount_panel(RwSignal::new(z)).await;
    let holds = host.query_selector_all(".climate-card__hold").unwrap();
    assert_eq!(holds.length(), 1);
    let text = holds.item(0).unwrap().text_content().unwrap_or_default();
    assert!(text.contains("21°") && text.contains("set here"), "{text}");

    host.query_selector(".climate-card__holds button[aria-label='Clear']")
        .unwrap()
        .expect("the ✕")
        .unchecked_into::<HtmlElement>()
        .click();
    settle().await;
    assert_eq!(requests_to(&stub, "DELETE", OVERRIDE_URL), 1);
}

#[wasm_bindgen_test]
async fn correction_fields_fold_under_advanced() {
    let stub = FetchStub::install();
    reply_decisions(&stub);
    let mut base = zone("living-room", "Living room", ZoneState::Active, Some(19.0));
    base.settings.feedback.enabled = true;
    base.room_sensor = Some("sensor.living_room_temperature".into());
    let current = RwSignal::new(base.clone());
    let host = mount_panel(current).await;
    let dialog = open_room_settings(&host).await;
    let text = dialog.text_content().unwrap_or_default();
    assert!(
        text.contains("Step 0.5 °C · tolerance 0.5 °C · every 15 min"),
        "{text}"
    );
    let fold = dialog
        .query_selector("details.climate-correction__more")
        .unwrap()
        .expect("the Advanced settings fold");
    assert!(!fold.has_attribute("open"), "folded by default");
    assert!(
        fold.text_content()
            .unwrap_or_default()
            .contains("Advanced settings (7)")
    );
    assert_eq!(fold.query_selector_all("input.field").unwrap().length(), 7);

    // Opened, it stays open through a refetch (each saved field refetches).
    fold.set_attribute("open", "").unwrap();
    current.set(ZoneView {
        reading: Some(18.0),
        ..base
    });
    settle().await;
    let fold = host
        .query_selector("details.climate-correction__more")
        .unwrap()
        .expect("still there");
    assert!(fold.has_attribute("open"), "still open");
}

#[wasm_bindgen_test]
async fn the_house_gear_opens_the_settings_popup() {
    let stub = FetchStub::install();
    let climate = nidoo_climate::view::ClimateView {
        enabled: true,
        house: Default::default(),
        house_invalid: None,
        history: true,
        timezone: "UTC".into(),
        zones: vec![zone(
            "living-room",
            "Living room",
            ZoneState::Active,
            Some(19.0),
        )],
    };
    stub.reply(
        "GET",
        "/api/v1/climate",
        200,
        &serde_json::to_string(&climate).unwrap(),
    );
    let host = host();
    let mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(Toasts::new());
        provide_context(House::new());
        provide_context(nidoo_web::components::now::install_now());
        provide_context(RwSignal::new(Lang::En));
        view! { <nidoo_web::screens::climate::ClimateScreen /> }
    });
    std::mem::forget(mounted);
    settle().await;
    settle().await;
    assert!(host.query_selector("dialog").unwrap().is_none());

    let gear = host
        .query_selector(".climate-screen__gear")
        .unwrap()
        .expect("the house ⚙");
    assert_eq!(
        gear.get_attribute("aria-label").as_deref(),
        Some("House settings")
    );
    gear.unchecked_into::<HtmlElement>().click();
    settle().await;
    let dialog = host
        .query_selector("dialog.popup")
        .unwrap()
        .expect("the house settings popup");
    let text = dialog.text_content().unwrap_or_default();
    assert!(text.contains("Season"), "{text}");

    let cancel = web_sys::Event::new("cancel").unwrap();
    dialog.dispatch_event(&cancel).unwrap();
    settle().await;
    assert!(host.query_selector("dialog").unwrap().is_none());
}

#[wasm_bindgen_test]
async fn a_late_reply_does_not_undo_a_newer_press() {
    let stub = FetchStub::install();
    stub.reply_after("POST", OVERRIDE_URL, 200, "{}", 500);
    let current = RwSignal::new(zone(
        "living-room",
        "Living room",
        ZoneState::Active,
        Some(19.0),
    ));
    let host = mount_stepper(current).await;
    let raise = step_button(&host, "Raise target");

    raise.click();
    // 19.5 is sent at 1.5 s and answered at 2 s; a press in between makes 20.
    gloo_timers::future::TimeoutFuture::new(1_600).await;
    assert_eq!(posts(&stub).len(), 1);
    raise.click();
    tick().await;
    assert_eq!(stepper_value(&host), "20°");
    gloo_timers::future::TimeoutFuture::new(600).await;
    current.set(zone(
        "living-room",
        "Living room",
        ZoneState::Active,
        Some(19.5),
    ));
    settle().await;
    assert_eq!(stepper_value(&host), "20°", "the newer press stays");
    assert!(
        host.query_selector(".climate-stepper__chips")
            .unwrap()
            .is_none(),
        "no chips for the older value"
    );

    past_the_pause().await;
    let sent = posts(&stub);
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(
        sent[1].contains("\"setpoint\":20.0") || sent[1].contains("\"setpoint\":20,"),
        "{}",
        sent[1]
    );
}

#[wasm_bindgen_test]
async fn an_off_decision_shows_off() {
    let mut z = zone("living-room", "Living room", ZoneState::Active, Some(19.0));
    z.decision.as_mut().unwrap().setpoint = Some(Setpoint::Off);
    let host = mount_stepper(RwSignal::new(z)).await;
    assert_eq!(stepper_value(&host), "Off");
}

#[wasm_bindgen_test]
async fn a_press_is_sent_when_the_card_goes_away() {
    let stub = FetchStub::install();
    stub.reply("POST", OVERRIDE_URL, 200, "{}");
    let current = RwSignal::new(zone(
        "living-room",
        "Living room",
        ZoneState::Active,
        Some(19.0),
    ));
    let shown = RwSignal::new(true);
    let host = host();
    let mounted = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(Toasts::new());
        provide_context(RwSignal::new(Lang::En));
        view! {
            <Show when=move || shown.get()>
                <TargetStepper
                    zone=current.into()
                    now=Signal::from((Day::Mon, 0))
                    on_changed=Callback::new(|_| {})
                />
            </Show>
        }
    });
    std::mem::forget(mounted);
    settle().await;

    step_button(&host, "Raise target").click();
    tick().await;
    shown.set(false);
    settle().await;
    let sent = posts(&stub);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].contains("\"setpoint\":19.5"), "{}", sent[0]);
}
