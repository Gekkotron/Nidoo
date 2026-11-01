#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::Router;
use nidoo_api::{
    ActionReport, Ambiance, AmbianceAction, AmbianceView, ApplyReport, Category, CategoryAction,
    Outcome, Problem, Target, WsEvent,
};
use nidoo_web::ambiances::row::HomeAmbiances;
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

fn lights_off() -> AmbianceAction {
    AmbianceAction::Category(CategoryAction {
        what: Category::Lights,
        to: Target::Off,
        brightness: None,
        area: None,
        except: Vec::new(),
    })
}

fn mood(id: &str, order: u32, pinned: bool, with_action: bool) -> Ambiance {
    Ambiance {
        id: id.into(),
        name: format!("Mood {id}"),
        subtitle: Some(format!("About {id}")),
        icon: None,
        pinned,
        order,
        actions: if with_action {
            vec![lights_off()]
        } else {
            Vec::new()
        },
        extra: Default::default(),
    }
}

fn view(a: Ambiance) -> AmbianceView {
    AmbianceView {
        id: a.id.clone(),
        ambiance: Some(a),
        invalid: None,
    }
}

struct Mounted {
    _app: Box<dyn std::any::Any>,
    host: web_sys::Element,
    house: House,
    toasts: Toasts,
}

/// The row alone, in a house the test seeds; `loaded` says whether the list has arrived.
fn mount(list: Vec<AmbianceView>, loaded: bool) -> Mounted {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let house = House::new();
    if loaded {
        house.apply(WsEvent::AmbiancesChanged { ambiances: list });
    }
    let toasts = Toasts::new();
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(toasts);
        provide_context(RwSignal::new(Lang::En));
        nidoo_web::nav::install_nav();
        view! { <Router><HomeAmbiances /></Router> }
    });
    Mounted {
        _app: Box::new(app),
        host,
        house,
        toasts,
    }
}

fn all(host: &web_sys::Element, selector: &str) -> Vec<web_sys::HtmlElement> {
    let nodes = host.query_selector_all(selector).unwrap();
    (0..nodes.length())
        .map(|i| nodes.item(i).unwrap().unchecked_into())
        .collect()
}

fn texts(host: &web_sys::Element, selector: &str) -> Vec<String> {
    all(host, selector)
        .iter()
        .map(|e| e.text_content().unwrap_or_default())
        .collect()
}

fn toast_texts(m: &Mounted) -> Vec<String> {
    m.toasts
        .0
        .get_untracked()
        .into_iter()
        .map(|t| t.text)
        .collect()
}

async fn settle() {
    tick().await;
    gloo_timers::future::TimeoutFuture::new(50).await;
    tick().await;
}

fn report(failed: Vec<Problem>) -> String {
    let outcome = if failed.is_empty() {
        Outcome::Ok
    } else {
        Outcome::Failed
    };
    serde_json::to_string(&ApplyReport {
        ambiance: "a".into(),
        started_at: 0,
        actions: vec![ActionReport {
            index: 0,
            outcome,
            total: 3,
            failed,
            skipped: Vec::new(),
            reason: None,
        }],
    })
    .unwrap()
}

#[wasm_bindgen_test]
async fn row_hidden_until_loaded() {
    let m = mount(vec![view(mood("a", 1, true, true))], false);
    tick().await;
    assert!(m.host.query_selector(".ambiances-row").unwrap().is_none());
    m.house.apply(WsEvent::AmbiancesChanged {
        ambiances: vec![view(mood("a", 1, true, true))],
    });
    tick().await;
    assert!(m.host.query_selector(".ambiances-row").unwrap().is_some());
    let text = m.host.text_content().unwrap();
    assert!(text.contains("One mood, one tap"), "{text}");
    let link = m.host.query_selector("a.ambiances__link").unwrap().unwrap();
    assert_eq!(
        link.get_attribute("href").as_deref(),
        Some("/ambiances/about")
    );
}

#[wasm_bindgen_test]
async fn row_shows_first_four_pinned() {
    let list: Vec<_> = (1..=6)
        .map(|n| view(mood(&format!("m{n}"), n, n != 2, true)))
        .collect();
    let m = mount(list, true);
    tick().await;
    assert_eq!(
        texts(&m.host, ".mood-tile__name"),
        ["Mood m1", "Mood m3", "Mood m4", "Mood m5"]
    );
    assert_eq!(texts(&m.host, ".mood-tile__sub")[0], "About m1");
}

#[wasm_bindgen_test]
async fn tile_tap_toasts_done() {
    let stub = FetchStub::install();
    stub.reply(
        "POST",
        "/api/v1/ambiances/a/apply",
        200,
        &report(Vec::new()),
    );
    let m = mount(vec![view(mood("a", 1, true, true))], true);
    tick().await;
    all(&m.host, "button.mood-tile")[0].click();
    settle().await;
    assert_eq!(toast_texts(&m), ["Mood a: done"]);
    assert_eq!(stub.requests().len(), 1);
}

#[wasm_bindgen_test]
async fn tile_tap_toasts_partial_count() {
    let stub = FetchStub::install();
    let failed = vec![
        Problem {
            id: "x".into(),
            reason: "no answer".into(),
        },
        Problem {
            id: "y".into(),
            reason: "no answer".into(),
        },
    ];
    stub.reply("POST", "/api/v1/ambiances/a/apply", 200, &report(failed));
    let m = mount(vec![view(mood("a", 1, true, true))], true);
    tick().await;
    all(&m.host, "button.mood-tile")[0].click();
    settle().await;
    assert_eq!(toast_texts(&m), ["Mood a: 2 devices did not respond"]);
}

#[wasm_bindgen_test]
async fn tile_is_busy_while_applying() {
    let stub = FetchStub::install();
    stub.hang("POST", "/api/v1/ambiances/a/apply");
    let m = mount(vec![view(mood("a", 1, true, true))], true);
    tick().await;
    let tile = &all(&m.host, "button.mood-tile")[0];
    assert_eq!(tile.get_attribute("aria-busy").as_deref(), Some("false"));
    tile.click();
    settle().await;
    let tile = &all(&m.host, "button.mood-tile")[0];
    assert_eq!(tile.get_attribute("aria-busy").as_deref(), Some("true"));
    assert!(tile.class_list().contains("mood-tile--pressed"));
}

#[wasm_bindgen_test]
async fn tile_busy_clears_on_error() {
    let stub = FetchStub::install();
    stub.reply(
        "POST",
        "/api/v1/ambiances/a/apply",
        503,
        r#"{"error":"publish failed"}"#,
    );
    let m = mount(vec![view(mood("a", 1, true, true))], true);
    tick().await;
    all(&m.host, "button.mood-tile")[0].click();
    settle().await;
    assert_eq!(stub.requests().len(), 1);
    assert_eq!(toast_texts(&m), ["publish failed"]);
    let tile = &all(&m.host, "button.mood-tile")[0];
    assert_ne!(tile.get_attribute("aria-busy").as_deref(), Some("true"));
    assert!(!tile.class_list().contains("mood-tile--pressed"));
    assert!(!tile.has_attribute("disabled"));
}

#[wasm_bindgen_test]
async fn invitation_with_no_ambiances_offers_examples() {
    let stub = FetchStub::install();
    let m = mount(Vec::new(), true);
    tick().await;
    let invite = m
        .host
        .query_selector("a.mood-tile--invite")
        .unwrap()
        .unwrap();
    assert_eq!(
        invite.get_attribute("href").as_deref(),
        Some("/ambiances/new")
    );
    assert!(invite.text_content().unwrap().contains("Create a mood"));
    let button = &all(&m.host, "button.mood-invite__examples")[0];
    assert!(
        button
            .text_content()
            .unwrap()
            .contains("Add the four examples")
    );
    button.click();
    settle().await;
    // The first write is tried (the stub answers 404, which stops the rest).
    let puts = stub.requests();
    assert_eq!(puts.len(), 1, "{puts:?}");
    assert_eq!(puts[0].method, "PUT");
    assert!(puts[0].url.starts_with("/api/v1/ambiances/"));
}

#[wasm_bindgen_test]
async fn invitation_with_unpinned_links_to_page() {
    let m = mount(vec![view(mood("a", 1, false, true))], true);
    tick().await;
    let invite = m
        .host
        .query_selector("a.mood-tile--invite")
        .unwrap()
        .unwrap();
    assert_eq!(invite.get_attribute("href").as_deref(), Some("/ambiances"));
    assert!(invite.text_content().unwrap().contains("Choose your moods"));
    assert!(
        m.host
            .query_selector(".mood-invite__examples")
            .unwrap()
            .is_none()
    );
}

#[wasm_bindgen_test]
async fn empty_reading_tile_links_to_editor() {
    let m = mount(vec![view(mood("read", 1, true, false))], true);
    tick().await;
    assert!(m.host.query_selector("button.mood-tile").unwrap().is_none());
    let tile = m.host.query_selector("a.mood-tile").unwrap().unwrap();
    assert_eq!(
        tile.get_attribute("href").as_deref(),
        Some("/ambiances/read")
    );
    assert_eq!(texts(&m.host, ".mood-tile__sub"), ["Add a lamp"]);
}
