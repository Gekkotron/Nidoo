//! Following the System page's links. Alone in its file: every `Router` a test file mounts keeps
//! its click listener after its test, and the oldest one takes the click, so only the first test
//! of a file can navigate by clicking.
#![cfg(target_arch = "wasm32")]

use leptos::prelude::*;
use leptos::task::tick;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_web::house::House;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::system_settings::SystemSettingsPage;
use nidoo_web::theme::Theme;
use serde_json::json;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

fn all(host: &web_sys::Element, selector: &str) -> Vec<web_sys::HtmlElement> {
    let nodes = host.query_selector_all(selector).unwrap();
    (0..nodes.length())
        .map(|i| nodes.item(i).unwrap().unchecked_into())
        .collect()
}

async fn wait_for(host: &web_sys::Element, selector: &str, n: usize) {
    for _ in 0..80 {
        if all(host, selector).len() >= n {
            break;
        }
        gloo_timers::future::TimeoutFuture::new(25).await;
    }
    let found = all(host, selector).len();
    assert!(found >= n, "waited for {n} x {selector:?}, found {found}");
    tick().await;
    gloo_timers::future::TimeoutFuture::new(50).await;
    tick().await;
}

#[wasm_bindgen_test]
async fn back_from_chart_returns_to_tiles() {
    let stub = FetchStub::install();
    let now = (js_sys::Date::now() / 1000.0) as u64;
    let view = json!({
        "startedAt": now - 3600,
        "uptimePct": 100.0,
        "restarts": [],
        "cpu": {"serverPct": 23.0, "hubPct": 4.0},
        "ram": null,
        "disk": null,
        "temperatureC": null,
        "devices": 1,
        "entities": 1,
        "retained": 1,
        "commandsToday": 1,
        "services": [],
        "notify": false,
    });
    stub.reply("GET", "/api/v1/system", 200, &view.to_string());
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    web_sys::window()
        .unwrap()
        .history()
        .unwrap()
        .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/settings"))
        .unwrap();
    let house = House::new();
    let _app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(house);
        provide_context(RwSignal::new(Lang::En));
        provide_context(RwSignal::new(Theme::Auto));
        nidoo_web::nav::install_nav();
        view! {
            <Router>
                <nidoo_web::nav::TrackBackStack />
                <nidoo_web::nav::BackLink />
                <Routes fallback=|| "fallback">
                    <Route
                        path=path!("/settings")
                        view=|| {
                            view! {
                                <leptos_router::components::A href="/settings/system">
                                    "System"
                                </leptos_router::components::A>
                            }
                        }
                    />
                    <Route path=path!("/settings/system") view=SystemSettingsPage />
                </Routes>
            </Router>
        }
    });

    wait_for(&host, "a[href=\"/settings/system\"]", 1).await;
    all(&host, "a[href=\"/settings/system\"]")[0].click();
    wait_for(&host, "a[href=\"/settings/system?show=cpu\"]", 1).await;
    all(&host, "a[href=\"/settings/system?show=cpu\"]")[0].click();
    wait_for(&host, ".system__back", 1).await;
    assert!(all(&host, ".system__tiles").is_empty());
    // The header's Back goes to the tiles too, not past them.
    let header_back = all(&host, ".sheet__back").remove(0);
    assert_eq!(
        header_back.get_attribute("href").as_deref(),
        Some("/settings/system")
    );

    all(&host, ".system__back")[0].click();
    wait_for(&host, ".system__tiles", 1).await;
    assert!(all(&host, ".system__back").is_empty());
    let loc = web_sys::window().unwrap().location();
    assert_eq!(loc.search().unwrap(), "");
}
