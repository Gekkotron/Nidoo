//! Shared by the glass and `/` shortcut browser tests: the real shell over the search page, a room
//! page with a text field, and keyboard helpers. Pulled in with
//! `#[path = "support/shell_search_harness.rs"] mod shell_harness;` next to `search_harness.rs`.
#![allow(dead_code)]

use leptos::prelude::*;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::search::SearchScreen;
use nidoo_web::shell::Shell;
use nidoo_web::theme::Theme;
use wasm_bindgen::JsCast;

use super::harness::{Mounted, settle, window};

/// Opens `url`, then mounts the shell around the search page and a room page holding a rename field.
pub async fn mount_shell(url: &str) -> Mounted {
    window()
        .history()
        .unwrap()
        .push_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(url))
        .unwrap();
    let document = window().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let house = House::new();
    let app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(RwSignal::new(Theme::Auto));
        provide_context(RwSignal::new(Lang::En));
        provide_context(nidoo_web::components::toast::Toasts::new());
        provide_context(nidoo_web::commands::Pending::new());
        provide_context(nidoo_web::components::confirm::Confirming::new());
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
                    <Routes fallback=|| ()>
                        <Route path=path!("/search") view=SearchScreen />
                        <Route
                            path=path!("/room/kitchen")
                            view=|| view! { <input id="rename" type="text" /> }
                        />
                    </Routes>
                </Shell>
            </Router>
        }
    });
    settle().await;
    Mounted::new(app, host)
}

pub fn press_slash_on(target: &web_sys::EventTarget) {
    let init = web_sys::KeyboardEventInit::new();
    init.set_key("/");
    init.set_bubbles(true);
    init.set_cancelable(true);
    target
        .dispatch_event(
            &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap(),
        )
        .unwrap();
}
