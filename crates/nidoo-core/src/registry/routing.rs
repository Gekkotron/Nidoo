//! From a broker message to an [`Inbound`], and the application of discovery messages
//! (including device-based configs, which the registry expands itself).

use super::{Change, Inbound, Registry};
use crate::discovery::{
    DEVICE_BASED_REASON, DiscoveryTopic, Parsed, TopicClass, classify, expand_device_based, parse,
};

/// Classifies `topic` for [`Registry::apply`]; see [`Registry::route`].
pub(super) fn route(registry: &Registry, topic: &str, payload: Vec<u8>, at: u64) -> Inbound {
    let config = &registry.config;
    if let Some(rest) = topic
        .strip_prefix(config.ui_prefix.as_str())
        .and_then(|rest| rest.strip_prefix('/'))
    {
        if rest == "_areas" {
            return Inbound::Areas(payload);
        }
        if rest == "_types" {
            return Inbound::Ignored;
        }
        if !rest.is_empty() {
            return Inbound::Override {
                unique_id: rest.to_owned(),
                payload,
            };
        }
    }
    if let Some(rest) = topic
        .strip_prefix(config.z2m_base.as_str())
        .and_then(|rest| rest.strip_prefix("/bridge/"))
    {
        match rest {
            "devices" => return Inbound::Z2mDevices(payload),
            "info" => return Inbound::Z2mBridgeInfo(payload),
            // Entities list `bridge/state` as an availability topic: once one is subscribed,
            // the message is a state message for them too and carries the caller's `at`.
            "state" if !registry.subscriptions.contains(topic) => {
                return Inbound::Z2mBridgeState(payload);
            }
            _ => {}
        }
    }
    if let TopicClass::Discovery(discovery) =
        classify(topic, &config.discovery_prefixes, &config.reserved)
    {
        return Inbound::Discovery(discovery, payload);
    }
    if registry.subscriptions.contains(topic) {
        return Inbound::State {
            topic: topic.to_owned(),
            payload,
            at,
        };
    }
    Inbound::Irrelevant
}

/// `<prefix>/<component>/[<node_id>/]<object_id>/config`.
pub(super) fn topic_string(topic: &DiscoveryTopic) -> String {
    match &topic.node_id {
        Some(node_id) => format!(
            "{}/{}/{}/{}/config",
            topic.prefix, topic.component, node_id, topic.object_id
        ),
        None => format!(
            "{}/{}/{}/config",
            topic.prefix, topic.component, topic.object_id
        ),
    }
}

/// Applies a discovery message: upserts or removes the entities it describes, records ignored
/// topics, and drops the entities this topic produced before but no longer does.
pub(super) fn apply_discovery(
    registry: &mut Registry,
    topic: &DiscoveryTopic,
    payload: Vec<u8>,
    changes: &mut Vec<Change>,
) {
    let topic_str = topic_string(topic);
    if registry.last_discovery.get(&topic_str) == Some(&payload) {
        return;
    }
    let mut produced = Vec::new();
    let mut removed = false;
    match parse(topic, &payload, &topic_str) {
        Ok(Parsed::Entity(entity)) => {
            forget_ignored(registry, &topic_str, changes);
            produced.push(entity.unique_id.clone());
            registry.upsert(*entity, &topic_str, changes);
        }
        Ok(Parsed::DeviceAutomation(fragment)) => {
            forget_ignored(registry, &topic_str, changes);
            registry.upsert_device_automation(fragment, &topic_str, changes);
        }
        Ok(Parsed::Removed) => {
            removed = true;
            forget_ignored(registry, &topic_str, changes);
            registry.remove_device_automation(&topic_str, changes);
        }
        Ok(Parsed::Ignored { reason }) if reason == DEVICE_BASED_REASON => {
            forget_ignored(registry, &topic_str, changes);
            expand(
                registry,
                topic,
                &payload,
                &topic_str,
                &mut produced,
                changes,
            );
        }
        Ok(Parsed::Ignored { reason }) => note_ignored(registry, &topic_str, reason, changes),
        Err(error) => note_ignored(registry, &topic_str, error.to_string(), changes),
    }

    // Entities this topic produced last time and does not any more are gone, unless another
    // topic has since taken them over.
    let previous = registry.produced.remove(&topic_str).unwrap_or_default();
    for id in previous {
        if !produced.contains(&id) && registry.origin.get(&id) == Some(&topic_str) {
            registry.remove_entity(&id, changes);
        }
    }
    if !produced.is_empty() {
        registry.produced.insert(topic_str.clone(), produced);
    }
    if removed {
        registry.last_discovery.remove(&topic_str);
    } else {
        registry.last_discovery.insert(topic_str, payload);
    }
}

/// Expands a device-based config and applies each per-entity config as if it had been
/// published on its own topic (`<prefix>/<platform>/<device object_id>/<component id>/config`).
///
/// HA marks a removed component with an entry holding only `platform`; `expand_device_based`
/// skips those, so the entity disappears from `produced` and the diff in [`apply_discovery`]
/// removes it. The same happens when the entry is simply absent.
fn expand(
    registry: &mut Registry,
    topic: &DiscoveryTopic,
    payload: &[u8],
    topic_str: &str,
    produced: &mut Vec<String>,
    changes: &mut Vec<Change>,
) {
    let entries = match expand_device_based(topic, payload) {
        Ok(entries) => entries,
        Err(error) => {
            note_ignored(registry, topic_str, error.to_string(), changes);
            return;
        }
    };
    for (entity_topic, config) in entries {
        let entity_topic_str = topic_string(&entity_topic);
        let Ok(bytes) = serde_json::to_vec(&config) else {
            continue;
        };
        match parse(&entity_topic, &bytes, &entity_topic_str) {
            Ok(Parsed::Entity(entity)) => {
                forget_ignored(registry, &entity_topic_str, changes);
                produced.push(entity.unique_id.clone());
                registry.upsert(*entity, topic_str, changes);
            }
            Ok(Parsed::DeviceAutomation(fragment)) => {
                forget_ignored(registry, &entity_topic_str, changes);
                registry.upsert_device_automation(fragment, &entity_topic_str, changes);
            }
            Ok(Parsed::Removed) => {}
            Ok(Parsed::Ignored { reason }) => {
                note_ignored(registry, &entity_topic_str, reason, changes);
            }
            Err(error) => note_ignored(registry, &entity_topic_str, error.to_string(), changes),
        }
    }
}

fn note_ignored(registry: &mut Registry, topic: &str, reason: String, changes: &mut Vec<Change>) {
    if registry.ignored.insert(topic, reason) {
        changes.push(Change::IgnoredChanged);
    }
}

fn forget_ignored(registry: &mut Registry, topic: &str, changes: &mut Vec<Change>) {
    if registry.ignored.remove(topic) {
        changes.push(Change::IgnoredChanged);
    }
}
