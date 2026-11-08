#![cfg(target_arch = "wasm32")]
//! Opening an authored dashboard's address directly: the screen paints before the dashboards
//! fetch lands, shows a loading state, and swaps in the dashboard once it does. Its own test
//! binary, so no router from another test reads the address.

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_api::{DashboardView, EntityView};
use nidoo_core::dashboard::{Dashboard, Span, Widget, WidgetConfig, WidgetKind};
use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::dashboard::DashboardScreen;
use nidoo_web::theme::Theme;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

fn light(id: &str) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: "Lamp".into(),
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
        area: "Kitchen".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState::default(),
    }
}

async fn settle() {
    for _ in 0..3 {
        tick().await;
    }
    gloo_timers::future::TimeoutFuture::new(50).await;
}

#[wasm_bindgen_test]
async fn a_dashboard_fetched_after_first_paint_replaces_unknown() {
    let window = web_sys::window().unwrap();
    window
        .history()
        .unwrap()
        .push_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/dashboards/d_late"))
        .unwrap();
    let document = window.document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();

    let house = House::new();
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
                <Routes fallback=|| ()>
                    <Route path=path!("/dashboards/:id") view=DashboardScreen />
                </Routes>
            </Router>
        }
    });
    settle().await;
    assert!(host.query_selector(".dashboard-loading").unwrap().is_some());
    assert!(host.query_selector(".dashboard-unknown").unwrap().is_none());

    house.dashboards_loaded.set(true);
    house.dashboards.set(vec![DashboardView {
        dashboard: Dashboard {
            id: "d_late".into(),
            name: "Late".into(),
            order: 1,
            widgets: vec![Widget {
                id: "w1".into(),
                kind: WidgetKind::Light,
                span: Span::X1x1,
                config: WidgetConfig::Entity {
                    entity_id: "light.k".into(),
                    title: None,
                },
                pos: None,
            }],
            show_header: false,
        },
        synthesized: false,
    }]);
    settle().await;
    assert!(host.query_selector(".dashboard-loading").unwrap().is_none());
    assert!(host.query_selector(".dashboard-unknown").unwrap().is_none());
    assert!(
        host.query_selector(".dashboard-edit-toolbar")
            .unwrap()
            .is_some()
    );
    // The state snapshot lands after the list: the tile swaps its placeholder for the card.
    assert!(
        host.query_selector(".widget-unavailable")
            .unwrap()
            .is_some()
    );
    house.entities.set(
        [light("light.k")]
            .into_iter()
            .map(|e| (e.id.clone(), e))
            .collect(),
    );
    settle().await;
    assert!(
        host.query_selector(".widget-unavailable")
            .unwrap()
            .is_none()
    );
}

/// Mounts the dashboard screen at `url` over `house`, in English.
fn mount_at(url: &str, house: House) -> (web_sys::Element, impl Drop) {
    let window = web_sys::window().unwrap();
    window
        .history()
        .unwrap()
        .push_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(url))
        .unwrap();
    let document = window.document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
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
                <Routes fallback=|| ()>
                    <Route path=path!("/dashboards/:id") view=DashboardScreen />
                </Routes>
            </Router>
        }
    });
    (host, app)
}

/// A dashboard `id` holding one solar power stat, titled `title`.
fn solar_stat(id: &str, title: Option<&str>) -> DashboardView {
    DashboardView {
        dashboard: Dashboard {
            id: id.into(),
            name: "Energy".into(),
            order: 1,
            widgets: vec![Widget {
                id: "w1".into(),
                kind: WidgetKind::EnergyStat,
                span: Span::X1x1,
                config: WidgetConfig::Energy {
                    metric: "pv_power_now".into(),
                    title: title.map(str::to_owned),
                },
                pos: None,
            }],
            show_header: false,
        },
        synthesized: false,
    }
}

fn click(host: &web_sys::Element, selector: &str) {
    host.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no {selector}"))
        .unchecked_into::<web_sys::HtmlElement>()
        .click();
}

#[wasm_bindgen_test]
async fn an_id_missing_from_the_loaded_list_is_unknown() {
    let house = House::new();
    house.dashboards_loaded.set(true);
    let (host, _app) = mount_at("/dashboards/d_gone", house);
    settle().await;
    assert!(host.query_selector(".dashboard-unknown").unwrap().is_some());
    assert!(host.query_selector(".dashboard-loading").unwrap().is_none());
}

#[wasm_bindgen_test]
async fn a_title_typed_in_the_widget_sheet_shows_on_the_tile_at_once() {
    let house = House::new();
    house.dashboards_loaded.set(true);
    house.dashboards.set(vec![solar_stat("d_title", None)]);
    let (host, _app) = mount_at("/dashboards/d_title?edit=1", house);
    settle().await;
    assert_eq!(text_of(&host, ".widget-stat__label"), "Solar now");

    click(&host, ".widget__edit-overlay");
    settle().await;
    let input: web_sys::HtmlInputElement = host
        .query_selector(".widget-config-sheet input.field")
        .unwrap()
        .expect("title field")
        .unchecked_into();
    input.set_value("Roof");
    input
        .dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();
    settle().await;
    assert_eq!(text_of(&host, ".widget-stat__label"), "Roof");
}

#[wasm_bindgen_test]
async fn picking_another_target_in_the_widget_sheet_swaps_what_the_tile_shows() {
    let house = House::new();
    house.dashboards_loaded.set(true);
    house.dashboards.set(vec![solar_stat("d_target", None)]);
    let (host, _app) = mount_at("/dashboards/d_target?edit=1", house);
    settle().await;
    assert_eq!(text_of(&host, ".widget-stat__label"), "Solar now");

    click(&host, ".widget__edit-overlay");
    settle().await;
    let select: web_sys::HtmlSelectElement = host
        .query_selector(".widget-config-sheet select.select")
        .unwrap()
        .expect("the Shows select")
        .unchecked_into();
    assert_eq!(select.value(), "pv_power_now");
    select.set_value("grid_power_now");
    select
        .dispatch_event(&web_sys::Event::new("change").unwrap())
        .unwrap();
    settle().await;
    assert_eq!(text_of(&host, ".widget-stat__label"), "Grid now");
}

#[wasm_bindgen_test]
async fn a_refetched_copy_replaces_the_shown_dashboard_unless_it_is_being_edited() {
    let house = House::new();
    house.dashboards_loaded.set(true);
    house.dashboards.set(vec![solar_stat("d_sync", None)]);
    let (host, _app) = mount_at("/dashboards/d_sync", house);
    settle().await;

    // A reconnect brings a copy saved on another device.
    house
        .dashboards
        .set(vec![solar_stat("d_sync", Some("Roof"))]);
    settle().await;
    assert_eq!(text_of(&host, ".widget-stat__label"), "Roof");

    // While editing, the draft stays as it is.
    click(&host, ".dashboard-edit-toolbar__pencil");
    settle().await;
    house
        .dashboards
        .set(vec![solar_stat("d_sync", Some("Garage"))]);
    settle().await;
    assert_eq!(text_of(&host, ".widget-stat__label"), "Roof");

    // Leaving edit mode with nothing unsaved takes the copy held back meanwhile.
    click(&host, ".dashboard-edit-toolbar__pencil");
    settle().await;
    assert_eq!(text_of(&host, ".widget-stat__label"), "Garage");
}

#[wasm_bindgen_test]
async fn moving_to_another_dashboard_shows_that_dashboard() {
    let house = House::new();
    house.dashboards_loaded.set(true);
    house.dashboards.set(vec![
        solar_stat("d_one", Some("One")),
        solar_stat("d_two", Some("Two")),
    ]);
    let window = web_sys::window().unwrap();
    window
        .history()
        .unwrap()
        .push_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/dashboards/d_one"))
        .unwrap();
    let document = window.document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    // The router's navigate, as a sidebar link calls it.
    let go = StoredValue::new_local(None::<Box<dyn Fn(&str)>>);
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
                {
                    let navigate = leptos_router::hooks::use_navigate();
                    go.set_value(Some(Box::new(move |to: &str| navigate(to, Default::default()))));
                }
                <Routes fallback=|| ()>
                    <Route path=path!("/dashboards/:id") view=DashboardScreen />
                </Routes>
            </Router>
        }
    });
    let go = |to: &str| go.with_value(|f| f.as_ref().expect("navigate")(to));
    settle().await;
    assert_eq!(text_of(&host, ".widget-stat__label"), "One");

    go("/dashboards/d_two");
    settle().await;
    assert_eq!(text_of(&host, ".widget-stat__label"), "Two");

    go("/dashboards/d_one");
    settle().await;
    assert_eq!(text_of(&host, ".widget-stat__label"), "One");
}

fn text_of(host: &web_sys::Element, selector: &str) -> String {
    host.query_selector(selector)
        .unwrap()
        .map(|e| e.text_content().unwrap_or_default())
        .unwrap_or_default()
}

#[wasm_bindgen_test]
async fn the_dashboard_sheet_and_the_history_axis_follow_the_language() {
    let stub = FetchStub::install();
    let now = (js_sys::Date::now() / 1000.0) as i64;
    stub.reply(
        "GET",
        "/api/v1/history/series?entity=sensor.t&range=24h&",
        200,
        &format!(
            r#"{{"window":86400,"points":[[{},20.0,19.0,21.5]]}}"#,
            now - 3_600
        ),
    );
    let window = web_sys::window().unwrap();
    window
        .history()
        .unwrap()
        .push_state_with_url(
            &wasm_bindgen::JsValue::NULL,
            "",
            Some("/dashboards/d_fr?edit=1"),
        )
        .unwrap();
    let document = window.document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();

    let house = House::new();
    let lang = RwSignal::new(Lang::Fr);
    let mut sensor = light("sensor.t");
    sensor.kind = Kind::Sensor;
    sensor.component = Kind::Sensor.component().to_string();
    sensor.unit = Some("°C".into());
    house
        .entities
        .set([sensor].into_iter().map(|e| (e.id.clone(), e)).collect());
    house.dashboards.set(vec![DashboardView {
        dashboard: Dashboard {
            id: "d_fr".into(),
            name: "Mine".into(),
            order: 1,
            widgets: vec![Widget {
                id: "w1".into(),
                kind: WidgetKind::HistoryChart,
                span: Span::X2x1,
                config: WidgetConfig::History {
                    entity_id: "sensor.t".into(),
                    range: nidoo_core::dashboard::DashboardHistoryRange::H24,
                },
                pos: None,
            }],
            show_header: false,
        },
        synthesized: false,
    }]);
    let _app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(RwSignal::new(Theme::Auto));
        provide_context(lang);
        provide_context(nidoo_web::components::toast::Toasts::new());
        provide_context(nidoo_web::commands::Pending::new());
        provide_context(nidoo_web::components::confirm::Confirming::new());
        nidoo_web::nav::install_nav();
        view! {
            <Router>
                <Routes fallback=|| ()>
                    <Route path=path!("/dashboards/:id") view=DashboardScreen />
                </Routes>
            </Router>
        }
    });
    settle().await;
    let gear: web_sys::HtmlElement = host
        .query_selector(".dashboard-edit-toolbar__gear")
        .unwrap()
        .unwrap()
        .unchecked_into();
    gear.click();
    settle().await;

    // French: the sheet and the chart's axis and legend.
    let sheet = text_of(&host, ".dashboard-settings-sheet");
    assert!(sheet.contains("Réglages du tableau de bord"), "{sheet}");
    assert!(sheet.contains("Afficher l'en-tête"), "{sheet}");
    assert!(!sheet.contains("Show header"), "{sheet}");
    let axis = text_of(&host, ".history-chart__axis-x");
    assert!(axis.contains("maintenant"), "{axis}");
    assert!(!axis.contains("now"), "{axis}");
    let legend = text_of(&host, ".history-chart__legend");
    assert!(legend.contains("24 dernières heures"), "{legend}");
    assert!(legend.contains("max 21,5\u{a0}°C"), "{legend}");

    // The scale carries several round y labels and a gridline per label, and the tooltip waits
    // hidden until the pointer is over the plot.
    let ylabels = host
        .query_selector_all(".history-chart__axis-y > span")
        .unwrap();
    assert!((3..=6).contains(&ylabels.length()), "{}", ylabels.length());
    let grid = host.query_selector_all(".history-chart__grid").unwrap();
    assert!(grid.length() >= ylabels.length(), "{}", grid.length());
    let tip: web_sys::HtmlElement = host
        .query_selector(".history-chart__tip")
        .unwrap()
        .expect("tooltip node")
        .unchecked_into();
    assert!(tip.hidden());
    let plot: web_sys::HtmlElement = host
        .query_selector(".history-chart__plot")
        .unwrap()
        .unwrap()
        .unchecked_into();
    let rect = plot.get_bounding_client_rect();
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_client_x((rect.left() + rect.width() / 2.0) as i32);
    init.set_client_y((rect.top() + rect.height() / 2.0) as i32);
    plot.dispatch_event(
        &web_sys::PointerEvent::new_with_event_init_dict("pointermove", &init).unwrap(),
    )
    .unwrap();
    settle().await;
    assert!(!tip.hidden());
    let said = tip.text_content().unwrap_or_default();
    assert!(said.contains("20,0\u{a0}°C"), "{said}");
    // The one point sits near the right edge, so the card opens to its left.
    assert!(tip.class_list().contains("history-chart__tip--left"));
    plot.dispatch_event(
        &web_sys::PointerEvent::new_with_event_init_dict("pointerleave", &init).unwrap(),
    )
    .unwrap();
    settle().await;
    assert!(tip.hidden());

    // English, with no remount: the same elements read the other language.
    lang.set(Lang::En);
    settle().await;
    let sheet = text_of(&host, ".dashboard-settings-sheet");
    assert!(sheet.contains("Dashboard settings"), "{sheet}");
    assert!(!sheet.contains("Afficher"), "{sheet}");
    let axis = text_of(&host, ".history-chart__axis-x");
    assert!(axis.contains("now"), "{axis}");
    assert!(!axis.contains("maintenant"), "{axis}");
    let legend = text_of(&host, ".history-chart__legend");
    assert!(legend.contains("Last 24 hours"), "{legend}");
    assert!(legend.contains("max 21.5\u{a0}°C"), "{legend}");
    // The chart did not refetch for the language change.
    let fetches = stub
        .requests()
        .iter()
        .filter(|r| r.url.starts_with("/api/v1/history/series"))
        .count();
    assert_eq!(fetches, 1);
}

/// A dashboard showing the house header waits for what sizes the header (the state snapshot,
/// the weather and solar answers) before it paints, so the grid never jumps down as chips and
/// cards fill in.
#[wasm_bindgen_test]
async fn a_dashboard_with_the_header_waits_for_the_house() {
    let house = House::new();
    house.dashboards_loaded.set(true);
    let mut view = solar_stat("d_head", None);
    view.dashboard.show_header = true;
    house.dashboards.set(vec![view]);
    let (host, _app) = mount_at("/dashboards/d_head", house);
    settle().await;
    assert!(host.query_selector(".dashboard-loading").unwrap().is_some());
    assert!(host.query_selector(".home-header").unwrap().is_none());
    assert!(
        host.query_selector(".dashboard-edit-toolbar")
            .unwrap()
            .is_none()
    );

    house.entities.set(
        [light("light.k")]
            .into_iter()
            .map(|e| (e.id.clone(), e))
            .collect(),
    );
    settle().await;
    assert!(host.query_selector(".home-header").unwrap().is_none());

    house.weather_loaded.set(true);
    house.solar_loaded.set(true);
    settle().await;
    assert!(host.query_selector(".dashboard-loading").unwrap().is_none());
    assert!(host.query_selector(".home-header").unwrap().is_some());
    assert!(
        host.query_selector(".dashboard-edit-toolbar")
            .unwrap()
            .is_some()
    );
}
