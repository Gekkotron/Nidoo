#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::tick;
use nidoo_web::app::Root;
use nidoo_web::setup::{Applying, ApplyingOverlay, wait_for_hub};
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

#[path = "support/fetch_stub.rs"]
mod fetch_stub;
use fetch_stub::FetchStub;

thread_local! {
    static RELOADS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn record(target: &str) {
    RELOADS.with(|r| r.borrow_mut().push(target.to_owned()));
}

fn status(mode: &str, restarting: bool) -> String {
    format!(
        r#"{{"mode":"{mode}","step":"welcome","finished":false,"bundled":false,"location":null,"mqtt":null,"database":null,"restarting":{restarting}}}"#
    )
}

fn host() -> web_sys::Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let host = document.create_element("div").unwrap();
    document.body().unwrap().append_child(&host).unwrap();
    host
}

fn pathname() -> String {
    web_sys::window().unwrap().location().pathname().unwrap()
}

fn replace_state(path: &str) {
    web_sys::window()
        .unwrap()
        .history()
        .unwrap()
        .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(path))
        .unwrap();
}

#[wasm_bindgen_test]
async fn setup_mode_sends_any_path_to_setup() {
    let original = pathname();
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/setup", 200, &status("setup", false));
    replace_state("/dashboards/house");
    let host = host();
    let handle = leptos::mount::mount_to(host.clone().unchecked_into(), Root);
    for _ in 0..10 {
        TimeoutFuture::new(20).await;
        tick().await;
    }
    assert_eq!(pathname(), "/setup");
    assert!(
        stub.requests()
            .iter()
            .all(|r| !r.url.starts_with("/api/v1/dashboards")),
        "{:?}",
        stub.requests()
    );
    drop(handle);
    replace_state(&original);
}

#[wasm_bindgen_test]
async fn wait_for_hub_returns_once_the_restart_is_over() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/setup", 200, &status("running", true));
    let done = std::rc::Rc::new(RefCell::new(None));
    let slot = done.clone();
    wasm_bindgen_futures::spawn_local(async move {
        *slot.borrow_mut() = Some(wait_for_hub(10, 50).await);
    });
    TimeoutFuture::new(30).await;
    stub.reply("GET", "/api/v1/setup", 200, &status("running", false));
    for _ in 0..40 {
        TimeoutFuture::new(20).await;
        if done.borrow().is_some() {
            break;
        }
    }
    assert!(matches!(&*done.borrow(), Some(Some(_))));
}

#[wasm_bindgen_test]
async fn overlay_reloads_to_its_target() {
    let stub = FetchStub::install();
    stub.reply("GET", "/api/v1/setup", 200, &status("running", false));
    RELOADS.with(|r| r.borrow_mut().clear());
    let applying = Applying::new(record);
    let host = host();
    let handle = leptos::mount::mount_to(host.clone().unchecked_into(), move || {
        provide_context(applying);
        view! { <ApplyingOverlay /> }
    });
    applying.start("/setup");
    tick().await;
    assert!(host.text_content().unwrap().contains("Applying…"));
    // The overlay sleeps 1 s, then polls every second.
    for _ in 0..150 {
        TimeoutFuture::new(20).await;
        if !RELOADS.with(|r| r.borrow().is_empty()) {
            break;
        }
    }
    assert_eq!(
        RELOADS.with(|r| r.borrow().clone()),
        vec!["/setup".to_owned()]
    );
    drop(handle);
}
