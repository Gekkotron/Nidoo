#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::Router;
use nidoo_api::DashboardView;
use nidoo_core::dashboard::Dashboard;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::shell::Shell;
use nidoo_web::theme::Theme;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

fn dashboard(id: &str, name: &str, order: u32) -> DashboardView {
    DashboardView {
        dashboard: Dashboard {
            id: id.into(),
            name: name.into(),
            order,
            widgets: Vec::new(),
            show_header: false,
        },
        synthesized: false,
    }
}

/// What a test drives from outside: the house, the engine and badge inputs, the theme.
struct Mounted {
    /// Unmounts on drop, so the next test's router is the only one handling link clicks.
    _app: Box<dyn std::any::Any>,
    host: web_sys::Element,
    house: House,
    climate_on: RwSignal<bool>,
    theme: RwSignal<Theme>,
}

async fn mount(lang: Lang, badge: Option<&str>) -> Mounted {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let house = House::new();
    let climate_on = RwSignal::new(false);
    let theme = RwSignal::new(Theme::Auto);
    let badge = badge.map(str::to_owned);
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(theme);
        provide_context(RwSignal::new(lang));
        nidoo_web::nav::install_nav();
        let badge: Signal<Option<String>> = Signal::stored(badge.clone());
        view! {
            <Router>
                <Shell
                    climate_on=climate_on
                    alarm_armed=Signal::from(false)
                    solar_active=Signal::from(false)
                    alerts_badge=badge
                >
                    <p class="shell-test-child">"child"</p>
                </Shell>
            </Router>
        }
    });
    tick().await;
    Mounted {
        _app: Box::new(app),
        host,
        house,
        climate_on,
        theme,
    }
}

fn text_of(host: &web_sys::Element, selector: &str) -> String {
    host.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("no {selector}"))
        .text_content()
        .unwrap_or_default()
}

#[wasm_bindgen_test]
async fn sidebar_lists_the_main_entries() {
    let m = mount(Lang::En, Some("3")).await;
    let sidebar = text_of(&m.host, "aside.sidebar#sidebar");
    for label in [
        "Nidoo.",
        "Overview",
        "Automations",
        "Groups",
        "My rooms",
        "Energy",
        "Security",
        "Alerts",
        "Settings",
    ] {
        assert!(sidebar.contains(label), "{label} missing from {sidebar}");
    }
    assert!(m.host.query_selector(".topbar").unwrap().is_none());
    assert!(
        m.host
            .query_selector(".sidebar__status .pill[role=\"status\"]")
            .unwrap()
            .is_some()
    );
    assert!(
        m.host
            .query_selector("ul.sidebar__rooms")
            .unwrap()
            .is_some()
    );
    // The count is spoken from hidden text inside the link; the pill itself is hidden from AT.
    assert_eq!(
        text_of(&m.host, ".sidebar a[href=\"/alerts\"] .visually-hidden"),
        "3 unseen alerts"
    );
    assert_eq!(
        m.host
            .query_selector(".sidebar a[href=\"/alerts\"] .nav__badge")
            .unwrap()
            .expect("a badge")
            .get_attribute("aria-hidden")
            .as_deref(),
        Some("true")
    );
    // The page itself sits in the content column, under the header.
    assert!(
        m.host
            .query_selector(".shell__content > header.content-header")
            .unwrap()
            .is_some()
    );
    assert!(
        m.host
            .query_selector(".shell__content > main.screen > .shell-test-child")
            .unwrap()
            .is_some()
    );
}

#[wasm_bindgen_test]
async fn climate_entry_follows_the_engine() {
    let m = mount(Lang::En, None).await;
    assert!(!text_of(&m.host, ".sidebar").contains("Climate"));
    m.climate_on.set(true);
    tick().await;
    let link = m
        .host
        .query_selector(".sidebar a[href=\"/climate\"]")
        .unwrap()
        .expect("a Climate link");
    let text = link.text_content().unwrap();
    assert!(text.contains("Climate"));
    assert!(text.contains("Beta"));
}

#[wasm_bindgen_test]
async fn dashboards_unfold_from_two() {
    let m = mount(Lang::En, None).await;
    m.house
        .dashboards
        .set(vec![dashboard(nidoo_api::HOUSE_ID, "House", 0)]);
    tick().await;
    assert!(
        m.host
            .query_selector(".sidebar__dashboards")
            .unwrap()
            .is_none()
    );
    // The first authored dashboard is made from here, so the action is there with one dashboard.
    assert!(text_of(&m.host, "button.sidebar__new").contains("New dashboard"));
    // Overview goes to the house when it is the only dashboard.
    assert!(
        m.host
            .query_selector(".sidebar a[href=\"/dashboards/house\"]")
            .unwrap()
            .is_some()
    );

    // Listed authored first by the hub; the sidebar still puts the house on top.
    m.house.dashboards.set(vec![
        dashboard("d1", "Wall tablet", 1),
        dashboard(nidoo_api::HOUSE_ID, "House", 0),
    ]);
    tick().await;
    let links = m.host.query_selector_all(".sidebar__dashboards a").unwrap();
    assert_eq!(links.length(), 2);
    let first = links.item(0).unwrap().text_content().unwrap();
    let second = links.item(1).unwrap().text_content().unwrap();
    assert_eq!((first.as_str(), second.as_str()), ("House", "Wall tablet"));
    assert_eq!(
        m.host
            .query_selector_all("button.sidebar__new")
            .unwrap()
            .length(),
        1
    );
    // Overview still lands on the house, as `/` does: an authored dashboard never takes over.
    assert!(
        m.host
            .query_selector(".sidebar__link[href=\"/dashboards/house\"]")
            .unwrap()
            .is_some()
    );
}

#[wasm_bindgen_test]
async fn overview_targets_the_open_dashboard() {
    let m = mount(Lang::En, None).await;
    m.house.dashboards.set(vec![
        dashboard(nidoo_api::HOUSE_ID, "House", 0),
        dashboard("d1", "Wall tablet", 1),
        dashboard("d2", "Kitchen screen", 2),
    ]);
    let overview = || {
        m.host
            .query_selector(".sidebar__link")
            .unwrap()
            .expect("the Overview link")
            .get_attribute("href")
    };
    go("/dashboards/d2").await;
    assert_eq!(overview().as_deref(), Some("/dashboards/d2"));
    go("/dashboards/house").await;
    assert_eq!(overview().as_deref(), Some("/dashboards/house"));
    // Away from the dashboards, or on one that does not exist, it goes where "/" lands: the house.
    go("/flows").await;
    assert_eq!(overview().as_deref(), Some("/dashboards/house"));
    go("/dashboards/gone").await;
    assert_eq!(overview().as_deref(), Some("/dashboards/house"));
    go("/").await;
}

#[wasm_bindgen_test]
async fn the_built_in_house_is_named_in_the_shell_language() {
    let m = mount(Lang::Fr, None).await;
    let mut house = dashboard(nidoo_api::HOUSE_ID, "House", 0);
    house.synthesized = true;
    m.house
        .dashboards
        .set(vec![house, dashboard("d1", "Wall tablet", 1)]);
    go("/dashboards/house").await;
    assert_eq!(
        text_of(&m.host, ".sidebar__sublink[href=\"/dashboards/house\"]"),
        "Maison"
    );
    assert_eq!(
        text_of(&m.host, ".breadcrumb [aria-current=\"page\"]"),
        "Maison"
    );
    // A dashboard the user named keeps its name.
    assert_eq!(
        text_of(&m.host, ".sidebar__sublink[href=\"/dashboards/d1\"]"),
        "Wall tablet"
    );
    go("/").await;
}

#[wasm_bindgen_test]
async fn french_shell() {
    let m = mount(Lang::Fr, Some("2")).await;
    let sidebar = text_of(&m.host, ".sidebar");
    for label in [
        "Vue d'ensemble",
        "Automatisations",
        "Mes pièces",
        "Réglages",
    ] {
        assert!(sidebar.contains(label), "{label} missing from {sidebar}");
    }
    assert!(
        text_of(&m.host, "nav.breadcrumb")
            .trim_start()
            .starts_with("Ma maison")
    );
    assert_eq!(
        text_of(&m.host, ".sidebar a[href=\"/alerts\"] .visually-hidden"),
        "2 alertes non lues"
    );
}

#[wasm_bindgen_test]
async fn theme_button_cycles() {
    let m = mount(Lang::En, None).await;
    let button: web_sys::HtmlElement = m
        .host
        .query_selector(".content-header__theme")
        .unwrap()
        .expect("a theme button")
        .unchecked_into();
    assert_eq!(
        button.get_attribute("aria-label").as_deref(),
        Some("Change theme (Auto)")
    );
    let mut seen = Vec::new();
    for _ in 0..3 {
        button.click();
        tick().await;
        let theme = m.theme.get_untracked();
        seen.push(theme);
        assert_eq!(
            button.get_attribute("title").as_deref(),
            Some(theme.label())
        );
        assert_eq!(
            button.get_attribute("aria-label"),
            Some(format!("Change theme ({})", theme.label()))
        );
    }
    assert_eq!(seen, [Theme::Night, Theme::Ivoire, Theme::Auto]);
}

#[wasm_bindgen_test]
async fn language_button_switches() {
    let m = mount(Lang::En, None).await;
    let button: web_sys::HtmlElement = m
        .host
        .query_selector(".content-header__lang")
        .unwrap()
        .expect("a language button")
        .unchecked_into();
    // No aria-label: the name is the visible "FR", "EN" plus the hidden target language.
    assert_eq!(button.get_attribute("aria-label"), None);
    let hidden = |lang: &str| {
        let span = button
            .query_selector(".visually-hidden")
            .unwrap()
            .expect("the hidden target language");
        assert_eq!(span.get_attribute("lang").as_deref(), Some(lang));
        span.text_content().unwrap()
    };
    let text = button.text_content().unwrap();
    assert!(text.contains("FR") && text.contains("EN"), "{text}");
    assert_eq!(hidden("fr"), "Français");
    button.click();
    tick().await;
    assert!(text_of(&m.host, ".sidebar").contains("Vue d'ensemble"));
    assert_eq!(hidden("en"), "English");
}

/// Moves the page to `path` the way Back/Forward does; every mounted router follows popstate.
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
async fn active_entry_follows_the_route() {
    let m = mount(Lang::En, None).await;
    go("/flows/f1").await;
    let current = m
        .host
        .query_selector_all(".sidebar [aria-current]")
        .unwrap();
    assert_eq!(current.length(), 1);
    assert_eq!(
        current
            .item(0)
            .unwrap()
            .unchecked_into::<web_sys::Element>()
            .get_attribute("href")
            .as_deref(),
        Some("/flows")
    );
    let trail = text_of(&m.host, "nav.breadcrumb");
    assert!(
        trail.contains("Automations") && trail.contains("f1"),
        "{trail}"
    );

    // An open dashboard listed under Overview is the page; Overview is only its section.
    m.house.dashboards.set(vec![
        dashboard(nidoo_api::HOUSE_ID, "House", 0),
        dashboard("d1", "Wall tablet", 1),
    ]);
    go("/dashboards/d1").await;
    let mark = |selector: &str| {
        m.host
            .query_selector(selector)
            .unwrap()
            .unwrap_or_else(|| panic!("no {selector}"))
            .get_attribute("aria-current")
    };
    assert_eq!(
        mark(".sidebar__sublink[href=\"/dashboards/d1\"]").as_deref(),
        Some("page")
    );
    assert_eq!(mark(".sidebar__sublink[href=\"/dashboards/house\"]"), None);
    assert_eq!(
        mark(".sidebar__link[href=\"/dashboards/d1\"]").as_deref(),
        Some("true")
    );

    go("/devices").await;
    assert!(
        m.host
            .query_selector(".sidebar a[href=\"/settings\"][aria-current=\"page\"]")
            .unwrap()
            .is_some()
    );
    go("/").await;
}
