//! Classification of an MQTT topic under the discovery prefixes.

use crate::model::{IGNORED_COMPONENTS, Kind};

/// The parts of `<prefix>/<component>/[<node_id>/]<object_id>/config`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DiscoveryTopic {
    pub prefix: String,
    pub component: String,
    pub node_id: Option<String>,
    pub object_id: String,
}

/// What a topic under a discovery prefix is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TopicClass {
    /// A discovery config topic for a supported or ignored component.
    Discovery(DiscoveryTopic),
    /// A reserved sub-tree directly under a prefix (e.g. `ui` for `oikos/ui/<unique_id>`).
    Reserved(String),
    /// Anything else, including topics outside every prefix.
    Other,
}

/// Classifies `topic`: it must start with `<prefix>/`; the next segment is a reserved sub-tree
/// → [`TopicClass::Reserved`]; it is a known component and the topic ends with `/config` with
/// one or two segments between → [`TopicClass::Discovery`]; otherwise [`TopicClass::Other`].
pub fn classify(topic: &str, prefixes: &[String], reserved: &[String]) -> TopicClass {
    let Some((prefix, rest)) = prefixes.iter().find_map(|prefix| {
        topic
            .strip_prefix(prefix.as_str())?
            .strip_prefix('/')
            .map(|rest| (prefix, rest))
    }) else {
        return TopicClass::Other;
    };
    let segments: Vec<&str> = rest.split('/').collect();
    let Some(&first) = segments.first() else {
        return TopicClass::Other;
    };
    if reserved.iter().any(|r| r == first) {
        return TopicClass::Reserved(first.to_owned());
    }
    let well_formed = (3..=4).contains(&segments.len())
        && segments.last() == Some(&"config")
        && segments.iter().all(|segment| !segment.is_empty());
    if !well_formed || !is_component(first) {
        return TopicClass::Other;
    }
    let node_id = (segments.len() == 4).then(|| segments[1].to_owned());
    TopicClass::Discovery(DiscoveryTopic {
        prefix: prefix.clone(),
        component: first.to_owned(),
        node_id,
        object_id: segments[segments.len() - 2].to_owned(),
    })
}

fn is_component(name: &str) -> bool {
    Kind::from_component(name).is_some()
        || IGNORED_COMPONENTS.contains(&name)
        || crate::types::type_def(name).is_some()
}
