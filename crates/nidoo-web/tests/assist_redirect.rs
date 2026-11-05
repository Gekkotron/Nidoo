#![cfg(target_arch = "wasm32")]
//! The old speech settings address lands on Settings → Assistant → Voice, replacing the history
//! entry so Back does not bounce off it. Alone in its file: it mounts a `Router`.

use leptos::prelude::*;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use nidoo_web::i18n::Lang;
use nidoo_web::screens::assist_settings::AssistSettingsRedirect;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
async fn old_address_redirects_to_voice() {
    let window = web_sys::window().unwrap();
    let history = window.history().unwrap();
    history
        .replace_state_with_url(
            &wasm_bindgen::JsValue::NULL,
            "",
            Some("/assistant/settings"),
        )
        .unwrap();
    let before = history.length().unwrap();
    let document = window.document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    let _app = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(RwSignal::new(Lang::En));
        view! {
            <Router>
                <Routes fallback=|| "fallback">
                    <Route path=path!("/assistant/settings") view=AssistSettingsRedirect />
                    <Route path=path!("/settings/assistant-voice") view=|| view! { <p class="voice">"voice"</p> } />
                </Routes>
            </Router>
        }
    });
    for _ in 0..80 {
        if host.query_selector(".voice").unwrap().is_some() {
            break;
        }
        gloo_timers::future::TimeoutFuture::new(25).await;
    }
    assert!(
        host.query_selector(".voice").unwrap().is_some(),
        "the Voice page shows"
    );
    assert_eq!(
        window.location().pathname().unwrap(),
        "/settings/assistant-voice"
    );
    assert_eq!(history.length().unwrap(), before, "replaced, not pushed");
}
