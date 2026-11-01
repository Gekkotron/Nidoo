//! Discovery topics that did not become entities, kept so the Devices view can list them.

use std::collections::BTreeMap;

/// A discovery topic the registry recognised but could not turn into an entity: an ignored
/// component, an unsupported one, or an invalid payload.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Ignored {
    pub topic: String,
    pub reason: String,
}

/// One reason per topic, exposed as a slice sorted by topic.
#[derive(Debug, Default)]
pub(super) struct IgnoredSet {
    by_topic: BTreeMap<String, String>,
    list: Vec<Ignored>,
}

impl IgnoredSet {
    /// Records `reason` for `topic`; true when the set changed.
    pub(super) fn insert(&mut self, topic: &str, reason: String) -> bool {
        if self.by_topic.get(topic) == Some(&reason) {
            return false;
        }
        self.by_topic.insert(topic.to_owned(), reason);
        self.rebuild();
        true
    }

    /// Forgets `topic`; true when it was listed.
    pub(super) fn remove(&mut self, topic: &str) -> bool {
        if self.by_topic.remove(topic).is_none() {
            return false;
        }
        self.rebuild();
        true
    }

    pub(super) fn as_slice(&self) -> &[Ignored] {
        &self.list
    }

    fn rebuild(&mut self) {
        self.list = self
            .by_topic
            .iter()
            .map(|(topic, reason)| Ignored {
                topic: topic.clone(),
                reason: reason.clone(),
            })
            .collect();
    }
}
