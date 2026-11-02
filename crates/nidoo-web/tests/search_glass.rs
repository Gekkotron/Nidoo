#![cfg(target_arch = "wasm32")]
//! The glass in the top bar opens the search page with its field focused. Its own test binary: see
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
async fn glass_opens_search_page() {
    let m = mount_shell("/room/kitchen").await;
    let glass: web_sys::HtmlElement = m
        .host
        .query_selector(".content-header__tools > :first-child")
        .unwrap()
        .expect("the glass")
        .unchecked_into();
    assert!(glass.class_list().contains("content-header__search"));
    assert_eq!(glass.get_attribute("aria-label").as_deref(), Some("Search"));
    assert_eq!(glass.get_attribute("aria-current"), None);
    glass.click();
    settle().await;
    assert_eq!(window().location().pathname().unwrap(), "/search");
    let active = window().document().unwrap().active_element().unwrap();
    assert!(active.class_list().contains("search__input"));
    let glass = m
        .host
        .query_selector(".content-header__search")
        .unwrap()
        .unwrap();
    assert_eq!(glass.get_attribute("aria-current").as_deref(), Some("page"));
}
