#![cfg(target_arch = "wasm32")]

use std::collections::BTreeMap;

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_api::{EntityView, FlowView, FlowWriteOutcome, HomeSettings};
use nidoo_core::flow::wake_up::{WakeSpeaker, WakeUp, to_flow};
use nidoo_core::flow::{Flow, NodeKind};
use nidoo_core::model::{
    Capabilities, Command, EntityCategory, EntityState, Kind, MediaChoice, MediaPlayerState,
    SOUND_DEVICE_PREFIX, StateValue, UiOverride,
};
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::wake_up::{WakeUpEditorScreen, WakeUpScreen};
use nidoo_web::theme::Theme;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

const LABELS_URL: &str = "/api/v1/flow-labels";
const EMPTY_LABELS: &str = r#"{"catalog":{"labels":[],"choices":[]},"counts":{}}"#;

struct Mounted {
    /// Unmounts on drop, so the next test's router is the only one handling link clicks.
    _app: Box<dyn std::any::Any>,
    host: web_sys::Element,
    house: House,
}

fn entity(id: &str, device: Option<(&str, &str)>, kind: Kind, value: StateValue) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: device.map(|d| d.0.into()),
        name: id.into(),
        device_name: device.map(|d| d.1.into()),
        label: None,
        kind,
        component: "x".into(),
        protocol_kind: kind,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "room".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState {
            value,
            ..Default::default()
        },
    }
}

fn choice(id: &str, name: &str) -> MediaChoice {
    MediaChoice {
        id: id.into(),
        name: name.into(),
    }
}

fn player() -> EntityView {
    let mut e = entity(
        "music_player",
        None,
        Kind::MediaPlayer,
        StateValue::MediaPlayer(MediaPlayerState {
            playlists: vec![choice("favorites", "Favorites"), choice("calm", "Calm")],
            source_list: vec![choice("cast", "Cast"), choice("nidoo", "Nidoo speakers")],
            ..Default::default()
        }),
    );
    e.capabilities.media_features = vec!["play_playlist".into()];
    e
}

/// A speaker's volume and mute entities (`<slug>_volume`, `<slug>_mute`).
fn speaker(slug: &str, name: &str) -> [EntityView; 2] {
    let device = format!("{SOUND_DEVICE_PREFIX}{slug}");
    [
        entity(
            &format!("{slug}_volume"),
            Some((&device, name)),
            Kind::Number,
            StateValue::Number(30.0),
        ),
        entity(
            &format!("{slug}_mute"),
            Some((&device, name)),
            Kind::Switch,
            StateValue::OnOff(false),
        ),
    ]
}

fn entities(with_pinson: bool) -> BTreeMap<String, EntityView> {
    let mut all = Vec::new();
    if with_pinson {
        all.push(player());
    }
    all.extend(speaker("bedroom", "Bedroom"));
    all.extend(speaker("living", "Living"));
    all.into_iter().map(|e| (e.id.clone(), e)).collect()
}

fn alarm() -> WakeUp {
    WakeUp {
        hour: 7,
        minute: 0,
        player: "music_player".into(),
        source: Some("nidoo".into()),
        speakers: vec![WakeSpeaker {
            volume_entity: "bedroom_volume".into(),
            mute_entity: Some("bedroom_mute".into()),
            volume: 25.0,
        }],
        muted: vec!["living_mute".into()],
        ..WakeUp::default()
    }
}

fn view_of(flow: Flow) -> FlowView {
    FlowView {
        definition: flow,
        invalid: None,
        active_run: None,
        runs: Vec::new(),
    }
}

fn host_element() -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    host
}

fn mount(path: &str, wake_up: bool, with_pinson: bool, flows: Vec<FlowView>) -> Mounted {
    let host = host_element();
    web_sys::window()
        .unwrap()
        .history()
        .unwrap()
        .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(path))
        .unwrap();
    let house = House::new();
    house.home_settings.set(HomeSettings {
        wake_up,
        ..Default::default()
    });
    house.entities.set(entities(with_pinson));
    house.flows.set(flows);
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(Toasts::new());
        provide_context(RwSignal::new(Lang::En));
        provide_context(RwSignal::new(Theme::Auto));
        nidoo_web::nav::install_nav();
        view! {
            <Router>
                <Routes fallback=|| "fallback">
                    <Route path=path!("/wake-up") view=WakeUpScreen />
                    <Route path=path!("/wake-up/:id") view=WakeUpEditorScreen />
                </Routes>
            </Router>
        }
    });
    Mounted {
        _app: Box::new(app),
        host,
        house,
    }
}

fn find(host: &web_sys::Element, selector: &str) -> Option<web_sys::HtmlElement> {
    host.query_selector(selector)
        .unwrap()
        .map(|e| e.unchecked_into())
}

fn one(host: &web_sys::Element, selector: &str) -> web_sys::HtmlElement {
    find(host, selector).unwrap_or_else(|| panic!("no {selector:?}"))
}

fn all(host: &web_sys::Element, selector: &str) -> Vec<web_sys::HtmlElement> {
    let list = host.query_selector_all(selector).unwrap();
    (0..list.length())
        .map(|i| list.item(i).unwrap().unchecked_into())
        .collect()
}

fn text(host: &web_sys::Element, selector: &str) -> String {
    one(host, selector).text_content().unwrap_or_default()
}

async fn settle() {
    tick().await;
    gloo_timers::future::TimeoutFuture::new(50).await;
    tick().await;
}

fn schedule_url(cron: &str) -> String {
    format!(
        "/api/v1/schedule?cron={}&count=3",
        nidoo_web::api::encode_id(cron)
    )
}

#[wasm_bindgen_test]
async fn lists_a_saved_alarm() {
    let stub = FetchStub::install();
    stub.reply(
        "GET",
        &schedule_url("0 7 * * 1-5"),
        200,
        r#"{"timezone":"Europe/Paris","next":["2026-11-23T07:00:00+01:00"]}"#,
    );
    let flow = to_flow("wake-up-1", "Weekday alarm", true, &alarm());
    let m = mount("/wake-up", true, true, vec![view_of(flow)]);
    settle().await;
    gloo_timers::future::TimeoutFuture::new(100).await;
    assert_eq!(all(&m.host, ".wake-alarm").len(), 1);
    assert_eq!(text(&m.host, ".wake-alarm__time"), "07:00");
    assert_eq!(text(&m.host, ".wake-alarm__name"), "Weekday alarm");
    let meta = text(&m.host, ".wake-alarm__meta");
    assert!(meta.contains("Favorites"), "{meta}");
    assert!(meta.contains("Bedroom 25 %"), "{meta}");
    assert!(meta.contains("1 min"), "{meta}");
    let next = text(&m.host, ".wake-alarm__next");
    assert!(next.starts_with("Next alarm:"), "{next}");
    assert!(next.contains("07:00"), "{next}");
    assert!(find(&m.host, ".wake-add").is_some());
}

#[wasm_bindgen_test]
async fn new_alarm_saves_a_managed_flow() {
    let stub = FetchStub::install();
    stub.reply("GET", LABELS_URL, 200, EMPTY_LABELS);
    stub.reply("POST", LABELS_URL, 200, EMPTY_LABELS);
    let saved = view_of(to_flow("wake-up-1", "Wake-up 07:00", true, &alarm()));
    let outcome = FlowWriteOutcome {
        flow: saved,
        warning: None,
    };
    stub.reply(
        "PUT",
        "/api/v1/flows/wake-up-1",
        200,
        &serde_json::to_string(&outcome).unwrap(),
    );
    let m = mount("/wake-up/new", true, true, Vec::new());
    settle().await;
    // The output starts on Nidoo speakers, so both speakers are listed, checked.
    let rows = all(&m.host, ".wake-speaker");
    assert_eq!(rows.len(), 2);
    // Untick the second one (Living).
    let checks = all(&m.host, ".wake-speaker__check");
    checks[1].click();
    settle().await;
    one(&m.host, ".wake-save").click();
    let put = stub.wait_for("PUT").await.expect("a PUT");
    assert_eq!(put.url, "/api/v1/flows/wake-up-1");
    let flow: Flow = serde_json::from_str(&put.body).unwrap();
    assert_eq!(flow.labels, vec!["wake-up".to_owned()]);
    assert_eq!(flow.name, "Wake-up 07:00");
    let body: serde_json::Value = serde_json::from_str(&put.body).unwrap();
    assert_eq!(body["wake_up"]["days"], serde_json::json!([1, 2, 3, 4, 5]));
    let commands: Vec<(String, Command)> = flow
        .nodes
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Command(c) => Some((c.entity.clone(), c.command.clone())),
            _ => None,
        })
        .collect();
    assert!(commands.iter().any(|(entity, command)| {
        entity == "music_player"
            && matches!(
                command,
                Command::PlayPlaylist { playlist, fade_in_s: 60, .. } if playlist == "favorites"
            )
    }));
    assert!(
        commands
            .iter()
            .any(|(entity, command)| entity == "living_mute"
                && matches!(command, Command::TurnOn { .. }))
    );
    assert!(
        commands
            .iter()
            .any(|(entity, command)| entity == "bedroom_volume"
                && matches!(command, Command::SetNumber { .. }))
    );
    // The label did not exist: it is created, then named.
    let posts: Vec<serde_json::Value> = stub
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST")
        .map(|r| serde_json::from_str(&r.body).unwrap())
        .collect();
    assert_eq!(posts.len(), 2, "{posts:?}");
    assert_eq!(posts[0]["op"], "create_label");
    assert_eq!(posts[0]["name"], "wake-up");
    assert_eq!(posts[1]["op"], "rename_label");
    assert_eq!(posts[1]["id"], "wake-up");
    assert_eq!(posts[1]["name"], "Wake-up");
    drop(m);
}

#[wasm_bindgen_test]
async fn no_day_blocks_save() {
    let stub = FetchStub::install();
    let m = mount("/wake-up/new", true, true, Vec::new());
    settle().await;
    // Weekdays are picked: turn each off.
    for chip in all(&m.host, ".wake-day") {
        if chip.class_list().contains("chip--selected") {
            chip.click();
        }
    }
    settle().await;
    assert_eq!(text(&m.host, ".wake-days__error"), "Pick at least one day.");
    let save: web_sys::HtmlButtonElement = one(&m.host, ".wake-save").unchecked_into();
    assert!(save.disabled());
    save.click();
    settle().await;
    assert!(stub.requests().iter().all(|r| r.method != "PUT"));
}

#[wasm_bindgen_test]
async fn without_pinson_shows_empty_state() {
    let _stub = FetchStub::install();
    let m = mount("/wake-up", true, false, Vec::new());
    settle().await;
    assert_eq!(
        text(&m.host, ".wake-empty__title"),
        "Music needs a media player"
    );
    assert!(find(&m.host, ".wake-add").is_none());
    assert!(find(&m.host, ".wake-alarm").is_none());
    // The editor shows the same, not a form.
    let m = mount("/wake-up/new", true, false, Vec::new());
    settle().await;
    assert_eq!(
        text(&m.host, ".wake-empty__title"),
        "Music needs a media player"
    );
    assert!(find(&m.host, ".wake-save").is_none());
}

#[wasm_bindgen_test]
async fn unreadable_alarm_has_only_delete() {
    let stub = FetchStub::install();
    stub.reply("DELETE", "/api/v1/flows/wake-up-3", 204, "");
    let mut flow = to_flow("wake-up-3", "Broken", true, &alarm());
    flow.extra
        .insert("wake_up".into(), serde_json::json!({"version": "nope"}));
    let m = mount("/wake-up", true, true, vec![view_of(flow)]);
    settle().await;
    assert_eq!(text(&m.host, ".wake-alarm__name"), "Unreadable alarm");
    assert!(find(&m.host, ".wake-alarm--bad").is_some());
    assert!(find(&m.host, ".wake-alarm__switch").is_none());
    assert!(find(&m.host, ".wake-alarm__body").is_none());
    one(&m.host, ".wake-alarm__delete").click();
    settle().await;
    // Delete asks first: the question names the alarm, and nothing is sent until "Delete".
    assert_eq!(text(&m.host, ".confirm__question"), "Delete wake-up-3?");
    assert!(find(&m.host, ".wake-alarm__delete").is_none());
    one(&m.host, ".confirm__buttons .btn:first-child").click();
    let del = stub.wait_for("DELETE").await.expect("a DELETE");
    assert_eq!(del.url, "/api/v1/flows/wake-up-3");
    settle().await;
    assert!(find(&m.host, ".wake-alarm").is_none());
    assert!(m.house.flows.get_untracked().is_empty());
}

#[wasm_bindgen_test]
async fn feature_off_shows_disabled_state() {
    let _stub = FetchStub::install();
    let m = mount("/wake-up", false, true, Vec::new());
    settle().await;
    assert_eq!(text(&m.host, ".wake-empty__title"), "Wake-up is off");
    let link = one(&m.host, ".wake-empty__link");
    assert_eq!(
        link.get_attribute("href").as_deref(),
        Some("/settings/wake-up")
    );
    assert!(find(&m.host, ".wake-add").is_none());
}
