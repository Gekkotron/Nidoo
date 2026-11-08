#![cfg(target_arch = "wasm32")]
//! Back from a card's sheet returns to the search with the same words and room. Its own test binary:
//! see `support/search_harness.rs`.

use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

#[path = "support/search_harness.rs"]
mod harness;
use harness::*;

#[wasm_bindgen_test]
async fn back_from_sheet_restores_results() {
    let (areas, entities) = kitchen_house();
    let m = mount("/search?q=kitchen", areas, entities).await;
    let header: web_sys::HtmlElement = m
        .host
        .query_selector("section.room .card__header")
        .unwrap()
        .expect("a card")
        .unchecked_into();
    header.click();
    settle().await;
    assert_eq!(window().location().pathname().unwrap(), "/entity/light.k");
    let back: web_sys::HtmlElement = m
        .host
        .query_selector("a.sheet__back")
        .unwrap()
        .expect("a Back link")
        .unchecked_into();
    back.click();
    settle().await;
    assert_eq!(window().location().pathname().unwrap(), "/search");
    assert_eq!(window().location().search().unwrap(), "?q=kitchen");
    assert_eq!(window().location().hash().unwrap(), "#room-kitchen");
    assert_eq!(input(&m).value(), "kitchen");
    assert_eq!(card_count(&m), 1);
    assert!(text_of(&m).contains("Kitchen lamp"));
}
