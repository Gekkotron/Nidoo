#![cfg(target_arch = "wasm32")]
//! `/` on the search page itself. Its own test binary: see `support/search_harness.rs`.

use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/search_harness.rs"]
mod harness;
#[path = "support/shell_search_harness.rs"]
mod shell_harness;
use harness::*;
use shell_harness::*;

/// On the search itself, `/` puts the cursor back in the field and keeps the words.
#[wasm_bindgen_test]
async fn slash_on_the_search_page_focuses_the_field() {
    let m = mount_shell("/search?q=lamp").await;
    let field: web_sys::HtmlInputElement = m
        .host
        .query_selector("input.search__input")
        .unwrap()
        .expect("the search field")
        .unchecked_into();
    field.blur().unwrap();
    press_slash_on(&window().document().unwrap().body().unwrap());
    settle().await;
    assert_eq!(window().location().search().unwrap(), "?q=lamp");
    assert_eq!(field.value(), "lamp");
    let active = window()
        .document()
        .unwrap()
        .active_element()
        .expect("a focused element");
    assert!(active.class_list().contains("search__input"));
}
