#![allow(dead_code)]
//! Shared test helpers: load recorded broker fixtures and build entities from discovery configs.

use std::path::PathBuf;

use nidoo_core::discovery::{DiscoveryTopic, Parsed, TopicClass, classify, parse};
use nidoo_core::model::Entity;
use serde_json::Value;

/// Directory holding the recorded fixtures.
pub fn fixture_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/"))
}

/// Read a fixture file (path relative to the fixtures directory) as UTF-8.
pub fn fixture(rel: &str) -> String {
    let path = fixture_dir().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", path.display()))
}

pub fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

pub fn discovery_topic(topic: &str) -> DiscoveryTopic {
    match classify(topic, &strings(&["oikos"]), &strings(&["ui", "automation"])) {
        TopicClass::Discovery(t) => t,
        other => panic!("{topic} is not a discovery topic: {other:?}"),
    }
}

pub fn entity_from_payload(topic_str: &str, payload: &str) -> Entity {
    let topic = discovery_topic(topic_str);
    match parse(&topic, payload.as_bytes(), topic_str) {
        Ok(Parsed::Entity(entity)) => *entity,
        other => panic!("{topic_str} did not parse as an entity: {other:?}"),
    }
}

/// Parses one recorded discovery fixture (file name without `.json`) into an entity.
pub fn parse_fixture(file_stem: &str) -> Entity {
    let topic_str = format!("oikos/{}/config", file_stem.replace("__", "/"));
    let payload = fixture(&format!("discovery/{file_stem}.json"));
    entity_from_payload(&topic_str, &payload)
}

/// Builds an entity from a hand-written config for `component`.
pub fn entity(component: &str, config: Value) -> Entity {
    entity_from_payload(
        &format!("oikos/{component}/test/config"),
        &config.to_string(),
    )
}

/// Every recorded discovery fixture that parses as an entity.
pub fn all_entities() -> Vec<Entity> {
    let dir = fixture_dir().join("discovery");
    let mut entities = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("discovery fixture dir") {
        let path = entry.expect("dir entry").path();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("fixture stem")
            .to_owned();
        let topic_str = format!("oikos/{}/config", stem.replace("__", "/"));
        let topic = discovery_topic(&topic_str);
        let payload = std::fs::read(&path).expect("fixture");
        if let Ok(Parsed::Entity(entity)) = parse(&topic, &payload, &topic_str) {
            entities.push(*entity);
        }
    }
    entities
}
