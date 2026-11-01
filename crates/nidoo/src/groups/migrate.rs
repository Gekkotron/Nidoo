//! The one-shot changeover from the derived room groups (one card per room and kind) to
//! hand-made group definitions.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use nidoo_api::{AreaView, ConnectionStatus, EntityView, GroupSpec};
use nidoo_core::model::{Kind, UNASSIGNED_AREA};
use tokio::sync::broadcast::error::RecvError;
use tokio::time::timeout;

use crate::hub::Hub;

/// How long the retained sync must stay silent before the migration starts.
const SETTLE_QUIET: Duration = Duration::from_millis(1500);

/// A room holds a derived group only from this many members.
const MIN_MEMBERS: usize = 2;

/// The kind a derived room group takes, with its `groupNames` key and default card name;
/// `None` for kinds that never grouped.
fn derived_kind(kind: Kind) -> Option<(&'static str, &'static str)> {
    match kind {
        Kind::Cover => Some(("cover", "Shutters")),
        Kind::Light => Some(("light", "Lights")),
        _ => None,
    }
}

/// The groups the derived rule showed, as `(id, spec)` sorted by id. The id is
/// `<kind key>_<area id>`; area ids are already `[a-z0-9_]` slugs.
pub fn plan(
    entities: &BTreeMap<String, EntityView>,
    areas: &[AreaView],
) -> Vec<(String, GroupSpec)> {
    let mut rosters: BTreeMap<(String, Kind), Vec<String>> = BTreeMap::new();
    for (id, entity) in entities {
        if entity.ui.ungrouped || entity.area == UNASSIGNED_AREA {
            continue;
        }
        if derived_kind(entity.kind).is_some() {
            rosters
                .entry((entity.area.clone(), entity.kind))
                .or_default()
                .push(id.clone());
        }
    }
    let mut out: Vec<(String, GroupSpec)> = rosters
        .into_iter()
        .filter(|(_, members)| members.len() >= MIN_MEMBERS)
        .filter_map(|((area, kind), mut members)| {
            let (key, default_name) = derived_kind(kind)?;
            members.sort();
            let custom = areas
                .iter()
                .find(|a| a.id == area)
                .and_then(|a| a.group_names.get(key))
                .map(|name| name.trim())
                .filter(|name| !name.is_empty());
            let name = custom.unwrap_or(default_name).to_owned();
            Some((
                format!("{key}_{area}"),
                GroupSpec {
                    kind,
                    name,
                    members,
                    hide_members: true,
                },
            ))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Boot entry point: waits for the broker link, then for the retained sync to go quiet,
/// then migrates once and logs the outcome.
pub async fn spawn(hub: Arc<Hub>) {
    let mut connection = hub.connection();
    while !matches!(*connection.borrow_and_update(), ConnectionStatus::Online) {
        if connection.changed().await.is_err() {
            return;
        }
    }
    let mut seen = hub.subscribe_state_seen();
    loop {
        match timeout(SETTLE_QUIET, seen.recv()).await {
            Err(_) => break,
            Ok(Ok(_)) | Ok(Err(RecvError::Lagged(_))) => {}
            Ok(Err(RecvError::Closed)) => return,
        }
    }
    let outcome = hub.run_group_migration_once().await;
    match outcome.skipped_reason {
        Some(reason) => tracing::info!(reason, "group migration skipped"),
        None => tracing::info!(created = ?outcome.created, "group migration done"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_core::model::{
        Area, Capabilities, EntityCategory, EntityState, UNASSIGNED_AREA, UiOverride,
    };

    fn view(id: &str, area: &str, kind: Kind, ungrouped: bool) -> EntityView {
        EntityView {
            id: id.to_owned(),
            device_id: None,
            name: id.to_owned(),
            device_name: None,
            label: None,
            kind,
            component: kind.component().to_string(),
            protocol_kind: kind,
            device_class: None,
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: area.to_owned(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride {
                ungrouped,
                ..UiOverride::default()
            },
            state: EntityState::default(),
        }
    }

    fn map(views: Vec<EntityView>) -> BTreeMap<String, EntityView> {
        views.into_iter().map(|v| (v.id.clone(), v)).collect()
    }

    fn area(id: &str, names: &[(&str, &str)]) -> AreaView {
        let mut a = AreaView::new(
            &Area {
                id: id.to_owned(),
                name: id.to_owned(),
                ..Area::default()
            },
            0,
            0,
        );
        a.group_names = names
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        a
    }

    #[test]
    fn ungrouped_member_is_skipped() {
        let e = map(vec![
            view("a", "living", Kind::Light, false),
            view("b", "living", Kind::Light, false),
            view("c", "living", Kind::Light, true),
        ]);
        let p = plan(&e, &[]);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].1.members, ["a", "b"]);
    }

    #[test]
    fn fewer_than_two_members_makes_no_group() {
        let e = map(vec![
            view("a", "living", Kind::Light, false),
            view("b", "living", Kind::Cover, false),
            view("c", "hall", Kind::Light, false),
            view("d", "living", Kind::Switch, false),
            view("e", "living", Kind::Switch, false),
        ]);
        assert!(plan(&e, &[]).is_empty());
    }

    #[test]
    fn unassigned_area_makes_no_group() {
        let e = map(vec![
            view("a", UNASSIGNED_AREA, Kind::Light, false),
            view("b", UNASSIGNED_AREA, Kind::Light, false),
        ]);
        assert!(plan(&e, &[]).is_empty());
    }

    #[test]
    fn spec_hides_members_and_uses_default_names() {
        let e = map(vec![
            view("a", "living", Kind::Cover, false),
            view("b", "living", Kind::Cover, false),
            view("c", "living", Kind::Light, false),
            view("d", "living", Kind::Light, false),
        ]);
        let p = plan(&e, &[area("living", &[])]);
        let ids: Vec<&str> = p.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["cover_living", "light_living"]);
        assert!(p.iter().all(|(_, s)| s.hide_members));
        assert_eq!(p[0].1.name, "Shutters");
        assert_eq!(p[1].1.name, "Lights");
        assert_eq!(p[1].1.kind, Kind::Light);
    }

    #[test]
    fn name_honours_the_area_group_names() {
        let e = map(vec![
            view("a", "living", Kind::Light, false),
            view("b", "living", Kind::Light, false),
        ]);
        let p = plan(&e, &[area("living", &[("light", "Main lamps")])]);
        assert_eq!(p[0].1.name, "Main lamps");
        let blank = plan(&e, &[area("living", &[("light", "  ")])]);
        assert_eq!(blank[0].1.name, "Lights");
    }
}
