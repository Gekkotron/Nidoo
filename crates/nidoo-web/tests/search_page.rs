#![cfg(target_arch = "wasm32")]
//! The Search page: the hint and Favorites, the live filter from the `?q=` of the address, the
//! accented query, the no-match and cap lines.

use nidoo_core::model::Kind;
use wasm_bindgen_test::*;

#[path = "support/search_harness.rs"]
mod harness;
use harness::*;

#[wasm_bindgen_test]
async fn empty_shows_hint_and_favorites() {
    let (areas, mut entities) = kitchen_house();
    entities[1].ui.favorite = true;
    let m = mount("/search", areas, entities).await;
    assert!(text_of(&m).contains("Type a name, a room or a kind of device"));
    assert_eq!(card_count(&m), 1, "only the favorite is drawn");
    assert!(text_of(&m).contains("Garage lamp"));
}

#[wasm_bindgen_test]
async fn empty_without_favorites_shows_only_the_hint() {
    let (areas, entities) = kitchen_house();
    let m = mount("/search", areas, entities).await;
    assert!(text_of(&m).contains("Type a name, a room or a kind of device"));
    assert!(!text_of(&m).contains("Favorites"));
    assert!(!text_of(&m).contains("Nothing here"));
    assert_eq!(card_count(&m), 0);
}

#[wasm_bindgen_test]
async fn typing_filters_after_debounce() {
    let (areas, entities) = kitchen_house();
    let m = mount("/search", areas, entities).await;
    let before = window().history().unwrap().length().unwrap();
    type_in(&input(&m), "kitchen");
    // Not before the debounce: the address and the results still show the empty search.
    settle().await;
    assert_eq!(window().location().search().unwrap(), "");
    assert!(text_of(&m).contains("Type a name, a room or a kind of device"));
    debounce().await;
    settle().await;
    assert_eq!(window().location().pathname().unwrap(), "/search");
    assert_eq!(window().location().search().unwrap(), "?q=kitchen");
    assert_eq!(window().history().unwrap().length().unwrap(), before);
    assert!(text_of(&m).contains("Kitchen lamp"));
    assert!(!text_of(&m).contains("Garage lamp"));
    assert_eq!(card_count(&m), 1);
}

#[wasm_bindgen_test]
async fn url_query_restores_accented_field() {
    let (areas, mut entities) = kitchen_house();
    entities[0].name = "Lumière".into();
    // "lumière" is also the French word for a light: the other device must not be one.
    entities[1].name = "Garage plug".into();
    entities[1].kind = Kind::Switch;
    entities[1].protocol_kind = Kind::Switch;
    entities[1].component = Kind::Switch.component().to_string();
    let m = mount("/search?q=lumi%C3%A8re", areas, entities).await;
    assert_eq!(input(&m).value(), "lumière");
    assert!(text_of(&m).contains("Lumière"));
    assert_eq!(card_count(&m), 1);
}

#[wasm_bindgen_test]
async fn no_match_and_cap_lines() {
    let (areas, mut entities) = kitchen_house();
    for i in 0..68 {
        entities.push(light(
            &format!("light.n{i}"),
            &format!("Spare lamp {i}"),
            "garage",
        ));
    }
    let m = mount("/search", areas, entities).await;
    type_in(&input(&m), "zzz");
    debounce().await;
    settle().await;
    assert!(text_of(&m).contains("Nothing matches “zzz”"));
    assert_eq!(card_count(&m), 0);
    type_in(&input(&m), "lamp");
    debounce().await;
    settle().await;
    assert!(
        text_of(&m).contains("Showing 60 of 70 — type more to narrow it down"),
        "{}",
        text_of(&m)
    );
    assert_eq!(card_count(&m), 60);
}
