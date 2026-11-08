#![cfg(target_arch = "wasm32")]
//! The phone drawer. Its own test binary: a router left over from an earlier test in the same page
//! would swallow the link click, and this one checks that the page really navigates.

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::shell::Shell;
use nidoo_web::theme::Theme;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

struct Mounted {
    _app: Box<dyn std::any::Any>,
    host: web_sys::Element,
}

async fn mount() -> Mounted {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(House::new());
        provide_context(RwSignal::new(Theme::Auto));
        provide_context(RwSignal::new(Lang::En));
        nidoo_web::nav::install_nav();
        let badge: Signal<Option<String>> = Signal::stored(None);
        view! {
            <Router>
                <Shell
                    climate_on=Signal::from(false)
                    alarm_armed=Signal::from(false)
                    solar_active=Signal::from(false)
                    alerts_badge=badge
                >
                    // Routes make the router commit a navigation to the address bar.
                    <Routes fallback=|| ()>
                        <Route path=path!("/*any") view=|| ()/>
                    </Routes>
                </Shell>
            </Router>
        }
    });
    tick().await;
    Mounted {
        _app: Box::new(app),
        host,
    }
}

/// Moves the page to `path` the way Back/Forward does.
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

/// What the drawer shows: whether it is out, whether the scrim is up, and the menu button's state.
fn drawer_state(m: &Mounted) -> (bool, bool, Option<String>) {
    let menu = m
        .host
        .query_selector(".content-header__menu")
        .unwrap()
        .expect("a menu button");
    (
        m.host.query_selector(".sidebar--open").unwrap().is_some(),
        m.host.query_selector(".shell__scrim").unwrap().is_some(),
        menu.get_attribute("aria-expanded"),
    )
}

#[wasm_bindgen_test]
async fn drawer_closes_on_navigation_and_escape() {
    let m = mount().await;
    let closed = (false, false, Some("false".to_owned()));
    let menu: web_sys::HtmlElement = m
        .host
        .query_selector(".content-header__menu")
        .unwrap()
        .expect("a menu button")
        .unchecked_into();
    assert_eq!(
        menu.get_attribute("aria-controls").as_deref(),
        Some("sidebar")
    );
    assert_eq!(drawer_state(&m), closed);

    let active_is = |selector: &str| {
        let active = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .active_element();
        let wanted = m.host.query_selector(selector).unwrap();
        active.is_some() && active == wanted
    };
    let inert = || {
        m.host
            .query_selector(".shell__content")
            .unwrap()
            .unwrap()
            .has_attribute("inert")
    };
    assert!(!inert());

    menu.click();
    tick().await;
    assert_eq!(drawer_state(&m), (true, true, Some("true".to_owned())));
    // Focus moves into the drawer, and the page behind it cannot be reached.
    assert!(active_is(".sidebar a"));
    assert!(inert());
    let sidebar = m.host.query_selector(".sidebar").unwrap().unwrap();
    assert_eq!(sidebar.get_attribute("role").as_deref(), Some("dialog"));
    assert_eq!(sidebar.get_attribute("aria-modal").as_deref(), Some("true"));
    assert_eq!(
        m.host
            .query_selector(".shell__scrim")
            .unwrap()
            .unwrap()
            .get_attribute("aria-hidden")
            .as_deref(),
        Some("true")
    );

    let init = web_sys::KeyboardEventInit::new();
    init.set_key("Escape");
    let escape =
        web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    web_sys::window().unwrap().dispatch_event(&escape).unwrap();
    tick().await;
    assert_eq!(drawer_state(&m), closed);
    // Focus goes back to the menu button, and the page is live again.
    assert!(active_is(".content-header__menu"));
    assert!(!inert());
    assert_eq!(sidebar.get_attribute("role"), None);

    // Following a sidebar link closes it.
    menu.click();
    tick().await;
    assert!(drawer_state(&m).0);
    let flows: web_sys::HtmlElement = m
        .host
        .query_selector(".sidebar a[href=\"/flows\"]")
        .unwrap()
        .expect("the Automations link")
        .unchecked_into();
    flows.click();
    tick().await;
    gloo_timers::future::TimeoutFuture::new(50).await;
    assert_eq!(drawer_state(&m), closed);
    assert_eq!(
        web_sys::window().unwrap().location().pathname().unwrap(),
        "/flows"
    );

    // A route change closes it too (Back/Forward, a link inside the page).
    menu.click();
    tick().await;
    assert!(drawer_state(&m).0);
    go("/devices").await;
    assert_eq!(drawer_state(&m), closed);

    menu.click();
    tick().await;
    let scrim: web_sys::HtmlElement = m
        .host
        .query_selector(".shell__scrim")
        .unwrap()
        .expect("a scrim")
        .unchecked_into();
    scrim.click();
    tick().await;
    assert_eq!(drawer_state(&m), closed);
    go("/").await;
}
