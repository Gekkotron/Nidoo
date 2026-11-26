//! The search page's results: the shown entities that match the words, grouped by room, with the
//! rooms that match as tiles. Pure; the screen lives in `screens::search`.

use std::collections::BTreeMap;

use nidoo_api::search::{EntityMatcher, Query};
use nidoo_api::{AreaView, DeviceView, EntityView, GroupView};
use nidoo_core::model::{Kind, UNASSIGNED_AREA};
use nidoo_core::types::Placement as TypePlacement;

use crate::groups::hidden_member_ids;
use crate::rooms::{generic_placement, home_counts};
use crate::screens::home::without_paired_companions;

/// The most cards a search draws.
pub const SEARCH_CAP: usize = 60;

/// One room's matching cards, in display order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchSection {
    /// The area id; `UNASSIGNED_AREA` for entities whose area is not in the house.
    pub area: String,
    pub ids: Vec<String>,
}

/// A room whose name or id matches the query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomHit {
    pub id: String,
    pub name: String,
    /// How many entities Home shows in the room.
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchResults {
    /// Capped, in display order.
    pub sections: Vec<SearchSection>,
    /// Never `UNASSIGNED_AREA`.
    pub rooms: Vec<RoomHit>,
    /// The number of matching cards before the cap.
    pub total: usize,
    /// `min(total, SEARCH_CAP)`.
    pub shown: usize,
}

/// What `query` finds, or `None` when it has no words (the caller shows Favorites and a hint).
pub fn search_results<'a>(
    query: &str,
    entities: impl Iterator<Item = &'a EntityView>,
    areas: &[AreaView],
    devices: &[DeviceView],
    groups: &BTreeMap<String, GroupView>,
) -> Option<SearchResults> {
    let query = Query::parse(query);
    if query.is_empty() {
        return None;
    }
    let hidden = hidden_member_ids(groups);
    let matcher = EntityMatcher::new(&query, areas, devices);
    let entities: Vec<&EntityView> = entities.collect();

    let matched: Vec<&EntityView> = entities
        .iter()
        .copied()
        .filter(|e| {
            e.is_primary()
                && (e.kind != Kind::Generic || generic_placement(e) == TypePlacement::Card)
                && !hidden.contains(e.id.as_str())
                && matcher.matches(e)
        })
        .collect();
    let mut matched = without_paired_companions(matched);
    matched.sort_by_cached_key(|e| (e.ui.order.unwrap_or(i32::MAX), e.name.to_lowercase()));

    // Rooms in the hub's order; "Unassigned" and areas the house no longer has fold into the last.
    let mut order: Vec<&str> = areas
        .iter()
        .map(|a| a.id.as_str())
        .filter(|id| *id != UNASSIGNED_AREA)
        .collect();
    order.push(UNASSIGNED_AREA);
    let slot = |area: &str| {
        order
            .iter()
            .position(|id| *id == area)
            .unwrap_or(order.len() - 1)
    };
    let mut sections: Vec<SearchSection> = order
        .iter()
        .map(|id| SearchSection {
            area: (*id).to_owned(),
            ids: Vec::new(),
        })
        .collect();
    for e in &matched {
        sections[slot(&e.area)].ids.push(e.id.clone());
    }
    sections.retain(|s| !s.ids.is_empty());

    let total = matched.len();
    let mut left = SEARCH_CAP;
    for section in &mut sections {
        section.ids.truncate(left);
        left -= section.ids.len();
    }
    sections.retain(|s| !s.ids.is_empty());

    let counts = home_counts(entities.iter().copied());
    let rooms = areas
        .iter()
        .filter(|a| a.id != UNASSIGNED_AREA && query.area_matches(a))
        .filter_map(|a| {
            let count = counts.get(&a.id).copied().unwrap_or(0);
            (count > 0).then(|| RoomHit {
                id: a.id.clone(),
                name: a.name.clone(),
                count,
            })
        })
        .collect();

    Some(SearchResults {
        sections,
        rooms,
        total,
        shown: total.min(SEARCH_CAP),
    })
}

#[cfg(test)]
mod tests;
