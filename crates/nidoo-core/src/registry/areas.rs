//! Area resolution and the effective area list.

use std::collections::BTreeSet;

use crate::model::{Area, Entity, UNASSIGNED_AREA};

/// Resolves a raw area string against the declared areas: a declared id wins, then a declared
/// name ignoring case (giving its id); anything else is an implicit area id as written.
pub(super) fn resolve(raw: &str, declared: &[Area]) -> String {
    declared
        .iter()
        .find(|area| area.id == raw)
        .or_else(|| {
            let wanted = raw.to_lowercase();
            declared
                .iter()
                .find(|area| area.name.to_lowercase() == wanted)
        })
        .map_or_else(|| raw.to_owned(), |area| area.id.clone())
}

/// The effective area of an entity: override → `suggested_area` → `path_area` → `Unassigned`,
/// each resolved against the declared areas. A blank override area is as good as absent.
pub(super) fn effective(entity: &Entity, override_area: Option<&str>, declared: &[Area]) -> String {
    let raw = override_area
        .filter(|area| !area.trim().is_empty())
        .or(entity.suggested_area.as_deref())
        .or(entity.path_area.as_deref())
        .unwrap_or(UNASSIGNED_AREA);
    resolve(raw, declared)
}

/// Declared areas by `order` (unset last) then name, then the implicit areas carried by
/// entities (sorted by id, named by their id), then `Unassigned` when an entity has it.
pub(super) fn effective_list<'a>(
    declared: &[Area],
    entities: impl Iterator<Item = &'a Entity>,
) -> Vec<Area> {
    let mut ordered: Vec<Area> = declared.to_vec();
    ordered.sort_by(|a, b| {
        let order = |area: &Area| area.order.map_or(i64::from(i32::MAX) + 1, i64::from);
        order(a).cmp(&order(b)).then_with(|| a.name.cmp(&b.name))
    });
    let declared_ids: BTreeSet<&str> = declared.iter().map(|area| area.id.as_str()).collect();
    let used: BTreeSet<&str> = entities.map(|entity| entity.area.as_str()).collect();
    for id in &used {
        if *id != UNASSIGNED_AREA && !declared_ids.contains(id) {
            ordered.push(Area {
                id: (*id).to_owned(),
                name: (*id).to_owned(),
                ..Area::default()
            });
        }
    }
    if used.contains(UNASSIGNED_AREA) && !declared_ids.contains(UNASSIGNED_AREA) {
        ordered.push(Area {
            id: UNASSIGNED_AREA.to_owned(),
            name: UNASSIGNED_AREA.to_owned(),
            ..Area::default()
        });
    }
    ordered
}
