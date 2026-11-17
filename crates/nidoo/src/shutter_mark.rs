//! The one-shot marking of the unclassed covers that already exist as shutters. An unclassed
//! cover counts as a shutter only with the `showAs: "shutter"` override, so on upgrade the
//! hub marks the house's current ones once; covers discovered later need the mark by hand.

use std::sync::Arc;

use nidoo_api::{EntityView, commandable};
use nidoo_core::model::Kind;

use crate::hub::Hub;

/// The retained marker that says the existing unclassed covers were marked. It sits under the
/// overrides' prefix; the hub intercepts it before the registry, as it does `_home`.
pub fn marker_topic(ui_prefix: &str) -> String {
    format!("{ui_prefix}/_shutters_marked")
}

/// The covers to mark, sorted by id: protocol cover, no device class, no `showAs` yet, and
/// commandable as a category member would be.
pub fn plan(entities: &[EntityView]) -> Vec<String> {
    let mut ids: Vec<String> = entities
        .iter()
        .filter(|e| {
            e.protocol_kind == Kind::Cover
                && e.device_class.is_none()
                && e.ui.show_as.is_none()
                && commandable(e)
        })
        .map(|e| e.id.clone())
        .collect();
    ids.sort();
    ids
}

/// Boot entry point: waits for the broker link and a quiet retained sync, then marks once
/// and logs the outcome. An empty house after settling only gets the marker.
pub async fn spawn(hub: Arc<Hub>) {
    if !crate::groups::migrate::wait_until_settled(&hub).await {
        return;
    }
    let outcome = hub.run_shutter_mark_once(true).await;
    match outcome.skipped_reason {
        Some(reason) => tracing::info!(reason, "shutter mark skipped"),
        None => tracing::info!(marked = ?outcome.marked, "shutter mark done"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_core::model::{Capabilities, EntityCategory, EntityState, UiOverride};

    fn view(id: &str, kind: Kind, class: Option<&str>, show_as: Option<&str>) -> EntityView {
        EntityView {
            id: id.to_owned(),
            device_id: None,
            name: id.to_owned(),
            device_name: None,
            label: None,
            kind,
            component: kind.component().to_string(),
            protocol_kind: kind,
            device_class: class.map(str::to_owned),
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: "living".to_owned(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride {
                show_as: show_as.map(str::to_owned),
                ..UiOverride::default()
            },
            state: EntityState::default(),
        }
    }

    #[test]
    fn marks_only_unclassed_unmarked_covers() {
        let all = [
            view("b_plain", Kind::Cover, None, None),
            view("a_plain", Kind::Cover, None, None),
            view("shutter", Kind::Cover, Some("shutter"), None),
            view("garage", Kind::Cover, Some("garage"), None),
            view("already", Kind::Cover, None, Some("shutter")),
            view("other_mark", Kind::Cover, None, Some("plug")),
            view("switch", Kind::Switch, None, None),
        ];
        assert_eq!(plan(&all), ["a_plain", "b_plain"]);
    }

    #[test]
    fn skips_covers_a_category_could_not_command() {
        let mut hidden_part = view("part", Kind::Cover, None, None);
        hidden_part.part_of = Some("x".to_owned());
        let mut config = view("config", Kind::Cover, None, None);
        config.category = EntityCategory::Config;
        assert!(plan(&[hidden_part, config]).is_empty());
    }

    #[test]
    fn marker_sits_under_the_ui_prefix() {
        assert_eq!(marker_topic("nidoo/ui"), "nidoo/ui/_shutters_marked");
    }
}
