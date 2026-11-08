#![cfg(target_arch = "wasm32")]
//! Esc in the search field clears it, then leaves for the page before. Its own test binary: see
//! `support/search_harness.rs`.

use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

#[path = "support/search_harness.rs"]
mod harness;
use harness::*;

#[wasm_bindgen_test]
async fn escape_clears_then_leaves() {
    let (areas, entities) = kitchen_house();
    let m = mount("/previous", areas, entities).await;
    let link: web_sys::HtmlElement = m
        .host
        .query_selector("#open-search")
        .unwrap()
        .expect("the link to the search")
        .unchecked_into();
    link.click();
    settle().await;
    assert_eq!(window().location().pathname().unwrap(), "/search");
    type_in(&input(&m), "kitchen");
    debounce().await;
    settle().await;
    press_escape(&input(&m));
    settle().await;
    assert_eq!(input(&m).value(), "");
    assert_eq!(window().location().pathname().unwrap(), "/search");
    assert_eq!(window().location().search().unwrap(), "");
    press_escape(&input(&m));
    settle().await;
    assert_eq!(window().location().pathname().unwrap(), "/previous");
}
