#![cfg(target_arch = "wasm32")]

use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn app_renders() {
    let window = web_sys::window().unwrap();
    // The shell follows the browser language unless one is saved; pin English for the labels below.
    if let Ok(Some(storage)) = window.local_storage() {
        storage.set_item("nidoo.lang", "en").unwrap();
    }
    let document = window.document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    // The handle unmounts the app when dropped, so it must live until the assertions run.
    let _mounted = leptos::mount::mount_to(host.clone().unchecked_into(), nidoo_web::app::App);
    let html = host.inner_html();
    assert!(html.contains("Nidoo."));
    assert!(html.contains("pill"));
    assert!(html.contains("Alerts"));
    assert!(html.contains("Automations"));
}
