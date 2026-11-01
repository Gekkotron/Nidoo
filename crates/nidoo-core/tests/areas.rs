//! `AreasDocument` round-trips the app's retained `oikos/ui/_areas` JSON.

mod common;

use nidoo_core::model::{AreasDocument, UNASSIGNED_AREA};

#[test]
fn recorded_areas_round_trip() {
    let raw = common::fixture("ui/_areas.json");
    let doc = AreasDocument::decode(raw.as_bytes()).expect("recorded areas decode");
    assert!(!doc.areas.is_empty());
    assert_eq!(doc.areas[0].id, "living");
    let encoded = doc.encode();
    let again = AreasDocument::decode(encoded.as_bytes()).expect("re-decode");
    assert_eq!(doc, again);
}

#[test]
fn group_names_and_extra_survive() {
    let raw = br#"{"areas":[{"id":"a","name":"A","groupNames":{"cover":"Volets"},"newKey":true}]}"#;
    let doc = AreasDocument::decode(raw).unwrap();
    assert_eq!(
        doc.areas[0].group_names.get("cover").map(String::as_str),
        Some("Volets")
    );
    assert_eq!(doc.encode(), std::str::from_utf8(raw).unwrap());
}

#[test]
fn rename_keeps_the_id_and_every_other_field() {
    let raw = br#"{"areas":[{"id":"dehors","name":"Dehors","order":7,"groupNames":{"cover":"Volets"},"newKey":true}]}"#;
    let mut doc = AreasDocument::decode(raw).unwrap();
    doc.rename("dehors", "Outdoor");
    assert_eq!(
        doc.encode(),
        r#"{"areas":[{"id":"dehors","name":"Outdoor","order":7,"groupNames":{"cover":"Volets"},"newKey":true}]}"#
    );
}

#[test]
fn rename_declares_an_implicit_area() {
    let mut doc = AreasDocument::decode(br#"{"areas":[{"id":"a","name":"A","order":0}]}"#).unwrap();
    doc.rename("garden", "Garden");
    assert_eq!(
        doc.encode(),
        r#"{"areas":[{"id":"a","name":"A","order":0},{"id":"garden","name":"Garden"}]}"#
    );
}

#[test]
fn unassigned_area_constant() {
    assert_eq!(UNASSIGNED_AREA, "Unassigned");
}
