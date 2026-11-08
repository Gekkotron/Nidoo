#![cfg(target_arch = "wasm32")]
//! Back from a card's sheet scrolls to the room the card was in. Its own test binary: see
//! `support/search_harness.rs`.

use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);

#[path = "support/search_harness.rs"]
mod harness;
use harness::*;

/// Back lands on the room the card was in, further down the page, and leaves the field alone.
#[wasm_bindgen_test]
async fn back_from_sheet_returns_to_the_room() {
    let (areas, mut entities) = kitchen_house();
    for i in 0..16 {
        entities.push(light(
            &format!("light.k{i}"),
            &format!("Kitchen lamp {i}"),
            "kitchen",
        ));
    }
    let m = mount("/search?q=lamp", areas, entities).await;
    let header: web_sys::HtmlElement = m
        .host
        .query_selector("#room-garage .card__header")
        .unwrap()
        .expect("the garage card")
        .unchecked_into();
    header.click();
    settle().await;
    assert_eq!(window().location().pathname().unwrap(), "/entity/light.g");
    let back: web_sys::HtmlElement = m
        .host
        .query_selector("a.sheet__back")
        .unwrap()
        .expect("a Back link")
        .unchecked_into();
    back.click();
    settle().await;
    gloo_timers::future::TimeoutFuture::new(100).await;
    assert_eq!(window().location().hash().unwrap(), "#room-garage");
    let garage = window()
        .document()
        .unwrap()
        .get_element_by_id("room-garage")
        .expect("the garage section");
    // The test page has no stylesheet, so it may end before the room reaches the very top: the
    // room is on screen and the page has scrolled to it.
    let top = garage.get_bounding_client_rect().top();
    let height = window().inner_height().unwrap().as_f64().unwrap();
    assert!(
        (0.0..height).contains(&top),
        "the garage room is on screen, not at {top}px"
    );
    assert!(window().scroll_y().unwrap() > 0.0);
    let active = window().document().unwrap().active_element();
    assert!(
        active.is_none_or(|el| !el.class_list().contains("search__input")),
        "the field is not focused on the way back"
    );
}
