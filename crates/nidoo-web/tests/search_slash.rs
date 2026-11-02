#![cfg(target_arch = "wasm32")]
//! `/` opens the search from a page, and only types inside a text field. See
//! `support/search_harness.rs`.

use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/search_harness.rs"]
mod harness;
#[path = "support/shell_search_harness.rs"]
mod shell_harness;
use harness::*;
use shell_harness::*;

#[wasm_bindgen_test]
async fn slash_in_a_text_field_types() {
    let m = mount_shell("/room/kitchen").await;
    let field: web_sys::HtmlInputElement = m
        .host
        .query_selector("#rename")
        .unwrap()
        .expect("the rename field")
        .unchecked_into();
    field.focus().unwrap();
    press_slash_on(&field);
    settle().await;
    assert_eq!(window().location().pathname().unwrap(), "/room/kitchen");
}

#[wasm_bindgen_test]
async fn slash_on_page_opens_search() {
    let m = mount_shell("/room/kitchen").await;
    press_slash_on(&window().document().unwrap().body().unwrap());
    settle().await;
    assert_eq!(window().location().pathname().unwrap(), "/search");
    drop(m);
}
