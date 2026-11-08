#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_api::{
    ActionReport, Ambiance, AmbianceAction, AmbianceView, ApplyReport, Category, CategoryAction,
    EntityView, Outcome, Problem, Target, WsEvent,
};
use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
use nidoo_web::ambiances::page::AmbiancesScreen;
use nidoo_web::ambiances::report::AmbianceReportScreen;
use nidoo_web::components::toast::Toasts;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::shell::Shell;
use nidoo_web::theme::Theme;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

fn mood(id: &str, order: u32, pinned: bool) -> Ambiance {
    Ambiance {
        id: id.into(),
        name: format!("Mood {id}"),
        subtitle: Some(format!("About {id}")),
        icon: None,
        pinned,
        order,
        actions: Vec::new(),
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

fn seed(house: House, list: Vec<AmbianceView>) {
    house.apply(WsEvent::AmbiancesChanged { ambiances: list });
}

fn lamp() -> EntityView {
    EntityView {
        id: "lamp".into(),
        device_id: None,
        name: "Desk lamp".into(),
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
        area: "salon".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState::default(),
    }
}

/// The page at `path`, with a house the test seeds; the app is dropped with the result.
struct Mounted {
    _app: Box<dyn std::any::Any>,
    host: web_sys::Element,
    house: House,
}

fn mount(path_: &str, seed_house: impl FnOnce(House)) -> Mounted {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    web_sys::window()
        .unwrap()
        .history()
        .unwrap()
        .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(path_))
        .unwrap();
    let house = House::new();
    seed_house(house);
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(Toasts::new());
        provide_context(RwSignal::new(Lang::En));
        nidoo_web::nav::install_nav();
        view! {
            <Router>
                <Routes fallback=|| "fallback">
                    <Route path=path!("/ambiances") view=AmbiancesScreen />
                    <Route path=path!("/ambiances/:id/report") view=AmbianceReportScreen />
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

async fn settle() {
    tick().await;
    gloo_timers::future::TimeoutFuture::new(50).await;
    tick().await;
}

/// Waits until `selector` matches at least `n` elements, then lets the handlers attach.
async fn wait_for(host: &web_sys::Element, selector: &str, n: usize) {
    for _ in 0..80 {
        if all(host, selector).len() >= n {
            break;
        }
        gloo_timers::future::TimeoutFuture::new(25).await;
    }
    let found = all(host, selector).len();
    assert!(found >= n, "waited for {n} x {selector:?}, found {found}");
    settle().await;
}

#[wasm_bindgen_test]
async fn sidebar_has_my_moods_between_overview_and_automations() {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let house = House::new();
    let badge: Signal<Option<String>> = Signal::stored(None);
    let _app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(RwSignal::new(Theme::Auto));
        provide_context(RwSignal::new(Lang::En));
        nidoo_web::nav::install_nav();
        view! {
            <Router>
                <Shell
                    climate_on=RwSignal::new(false)
                    alarm_armed=Signal::from(false)
                    solar_active=Signal::from(false)
                    alerts_badge=badge
                >
                    <p></p>
                </Shell>
            </Router>
        }
    });
    tick().await;
    let sidebar = host
        .query_selector("aside.sidebar")
        .unwrap()
        .unwrap()
        .text_content()
        .unwrap();
    let at = |label: &str| {
        sidebar
            .find(label)
            .unwrap_or_else(|| panic!("{label} missing from {sidebar}"))
    };
    assert!(at("Overview") < at("My moods") && at("My moods") < at("Automations"));
    let link = host
        .query_selector(".sidebar a[href=\"/ambiances\"]")
        .unwrap();
    assert!(link.is_some());
}

#[wasm_bindgen_test]
async fn page_lists_in_order_with_actions() {
    let m = mount("/ambiances", |h| {
        seed(
            h,
            vec![
                view(mood("b", 2, false)),
                view(mood("c", 3, true)),
                view(mood("a", 1, true)),
            ],
        )
    });
    tick().await;
    assert_eq!(
        texts(&m.host, ".mood-card__name"),
        ["Mood a", "Mood b", "Mood c"]
    );
    assert_eq!(texts(&m.host, ".mood-card__sub")[0], "About a");
    let cards = all(&m.host, ".mood-card");
    let first = &cards[0];
    let text = first.text_content().unwrap();
    for label in ["Apply", "Edit", "Delete"] {
        assert!(text.contains(label), "{label} missing from {text}");
    }
    let edit = first.query_selector("a.mood-card__edit").unwrap().unwrap();
    assert_eq!(edit.get_attribute("href").as_deref(), Some("/ambiances/a"));
    assert!(
        first
            .query_selector("[role=switch][aria-checked=true]")
            .unwrap()
            .is_some()
    );
    assert!(
        cards[1]
            .query_selector("[role=switch][aria-checked=false]")
            .unwrap()
            .is_some()
    );
    assert!(first.query_selector(".mood-card__up").unwrap().is_some());
    assert!(first.query_selector(".mood-card__down").unwrap().is_some());
    // No "Last run" link until a report for that mood exists.
    assert!(
        m.host
            .query_selector(".mood-card__report")
            .unwrap()
            .is_none()
    );
    m.house.apply(WsEvent::AmbianceApplied {
        report: ApplyReport {
            ambiance: "b".into(),
            started_at: 0,
            actions: Vec::new(),
        },
    });
    tick().await;
    let links = all(&m.host, ".mood-card__report");
    assert_eq!(links.len(), 1);
    assert_eq!(
        links[0].get_attribute("href").as_deref(),
        Some("/ambiances/b/report")
    );
    // The link sits on the second card, the one that ran.
    assert!(
        all(&m.host, ".mood-card")[1]
            .query_selector(".mood-card__report")
            .unwrap()
            .is_some()
    );
}

#[wasm_bindgen_test]
async fn pin_and_move_write_through_put() {
    let stub = FetchStub::install();
    let echo = |a: Ambiance| serde_json::to_string(&view(a)).unwrap();
    stub.reply("PUT", "/api/v1/ambiances/a", 200, &echo(mood("a", 1, true)));
    stub.reply(
        "PUT",
        "/api/v1/ambiances/b",
        200,
        &echo(mood("b", 1, false)),
    );
    let m = mount("/ambiances", |h| {
        seed(
            h,
            vec![view(mood("a", 1, false)), view(mood("b", 2, false))],
        )
    });
    wait_for(&m.host, ".mood-card [role=switch]", 2).await;
    all(&m.host, ".mood-card")[0]
        .query_selector("[role=switch]")
        .unwrap()
        .unwrap()
        .unchecked_into::<web_sys::HtmlElement>()
        .click();
    settle().await;
    let puts = stub.requests();
    assert_eq!(puts.len(), 1);
    assert_eq!(puts[0].method, "PUT");
    assert_eq!(puts[0].url, "/api/v1/ambiances/a");
    assert!(puts[0].body.contains("\"pinned\":true"), "{}", puts[0].body);

    all(&m.host, ".mood-card")[0]
        .query_selector(".mood-card__down")
        .unwrap()
        .unwrap()
        .unchecked_into::<web_sys::HtmlElement>()
        .click();
    settle().await;
    let puts = stub.requests();
    assert_eq!(puts.len(), 3, "{puts:?}");
    let body = |id: &str| {
        puts.iter()
            .skip(1)
            .find(|r| r.url.ends_with(id))
            .unwrap()
            .body
            .clone()
    };
    assert!(body("/a").contains("\"order\":2"), "{}", body("/a"));
    assert!(body("/b").contains("\"order\":1"), "{}", body("/b"));
}

#[wasm_bindgen_test]
async fn page_shows_only_four_hint() {
    let five: Vec<_> = (1..=5)
        .map(|n| view(mood(&format!("m{n}"), n, true)))
        .collect();
    let m = mount("/ambiances", |h| seed(h, five));
    tick().await;
    assert!(
        m.host
            .text_content()
            .unwrap()
            .contains("Only the first 4 pinned moods show on Home.")
    );
    let four: Vec<_> = (1..=4)
        .map(|n| view(mood(&format!("m{n}"), n, true)))
        .collect();
    seed(m.house, four);
    tick().await;
    assert!(!m.host.text_content().unwrap().contains("Only the first 4"));
}

#[wasm_bindgen_test]
async fn invalid_ambiance_card_offers_delete_only() {
    let m = mount("/ambiances", |h| {
        seed(
            h,
            vec![AmbianceView {
                id: "broken".into(),
                ambiance: None,
                invalid: Some("unknown icon".into()),
            }],
        )
    });
    tick().await;
    let card = all(&m.host, ".mood-card").remove(0);
    let text = card.text_content().unwrap();
    assert!(
        text.contains("broken") && text.contains("unknown icon"),
        "{text}"
    );
    assert!(text.contains("Delete"));
    for absent in [
        ".mood-card__apply",
        ".mood-card__edit",
        "[role=switch]",
        ".mood-card__up",
    ] {
        assert!(
            card.query_selector(absent).unwrap().is_none(),
            "{absent} shown"
        );
    }
}

#[wasm_bindgen_test]
async fn empty_state_offers_examples() {
    let m = mount("/ambiances", |h| seed(h, Vec::new()));
    tick().await;
    let text = m.host.text_content().unwrap();
    for label in ["No mood yet.", "New mood", "Add the four examples"] {
        assert!(text.contains(label), "{label} missing from {text}");
    }
    assert!(text.contains("How it works"));
}

#[wasm_bindgen_test]
async fn examples_button_disables_while_writing() {
    let stub = FetchStub::install();
    stub.hang("PUT", "/api/v1/ambiances/good_night");
    let m = mount("/ambiances", |h| seed(h, Vec::new()));
    tick().await;
    let button: web_sys::HtmlButtonElement = m
        .host
        .query_selector(".ambiances__examples")
        .unwrap()
        .unwrap()
        .unchecked_into();
    assert!(!button.disabled());
    button.click();
    settle().await;
    assert!(
        button.disabled(),
        "still enabled while the write is pending"
    );
    let puts = stub.requests();
    assert_eq!(puts.len(), 1, "{puts:?}");
    assert_eq!(puts[0].url, "/api/v1/ambiances/good_night");
    assert!(
        puts[0].body.contains("\"name\":\"Good night\""),
        "{}",
        puts[0].body
    );
}

fn category(what: Category, to: Target) -> AmbianceAction {
    AmbianceAction::Category(CategoryAction {
        what,
        to,
        brightness: None,
        area: None,
        except: Vec::new(),
    })
}

fn problem(id: &str, reason: &str) -> Problem {
    Problem {
        id: id.into(),
        reason: reason.into(),
    }
}

#[wasm_bindgen_test]
async fn report_page_lists_failed_and_skipped() {
    let mut night = mood("night", 1, true);
    night.actions = vec![
        category(Category::Lights, Target::Off),
        category(Category::Plugs, Target::Off),
    ];
    let m = mount("/ambiances/night/report", |h| {
        h.apply(WsEvent::EntityUpserted(lamp()));
        seed(h, vec![view(night)]);
        h.apply(WsEvent::AmbianceApplied {
            report: ApplyReport {
                ambiance: "night".into(),
                started_at: 1_792_402_860,
                actions: vec![
                    ActionReport {
                        index: 0,
                        outcome: Outcome::Failed,
                        total: 3,
                        failed: vec![problem("lamp", "no answer"), problem("ghost", "timed out")],
                        skipped: vec![problem("lamp", "already off")],
                        reason: None,
                    },
                    ActionReport {
                        index: 1,
                        outcome: Outcome::Ok,
                        total: 2,
                        failed: Vec::new(),
                        skipped: Vec::new(),
                        reason: None,
                    },
                ],
            },
        });
    });
    tick().await;
    let text = m.host.text_content().unwrap();
    for expected in [
        "Mood night",
        "Every light in the house: off",
        "Every plug in the house: off",
        "Failed",
        "Skipped",
        "Done",
        "Desk lamp: no answer",
        "ghost: timed out",
        "Desk lamp: already off",
    ] {
        assert!(text.contains(expected), "{expected} missing from {text}");
    }
    // The page shows the browser-local time of the run, computed the same way.
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(1_792_402_860_f64 * 1000.0));
    let time = chrono::NaiveTime::from_hms_opt(date.get_hours(), date.get_minutes(), 0).unwrap();
    let expected = nidoo_web::i18n::home_time(Lang::En, time);
    assert!(text.contains(&expected), "{expected} missing from {text}");
}

#[wasm_bindgen_test]
async fn report_page_without_a_run_says_so() {
    let m = mount("/ambiances/night/report", |h| {
        seed(h, vec![view(mood("night", 1, true))])
    });
    tick().await;
    assert!(
        m.host
            .text_content()
            .unwrap()
            .contains("Not run since this page was opened.")
    );
}
