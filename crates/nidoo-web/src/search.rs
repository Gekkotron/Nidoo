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
mod tests {
    use std::collections::BTreeMap;

    use super::{SEARCH_CAP, search_results};
    use nidoo_api::{AreaView, EntityView, GroupMemberView, GroupView};
    use nidoo_core::model::{
        Capabilities, EntityCategory, EntityState, Kind, UNASSIGNED_AREA, UiOverride,
    };

    fn entity(id: &str, name: &str, kind: Kind, area: &str) -> EntityView {
        EntityView {
            id: id.into(),
            device_id: None,
            name: name.into(),
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
            area: area.into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride::default(),
            state: EntityState::default(),
        }
    }

    fn light(id: &str, name: &str, area: &str) -> EntityView {
        entity(id, name, Kind::Light, area)
    }

    fn area(id: &str, name: &str) -> AreaView {
        AreaView {
            id: id.into(),
            name: name.into(),
            icon: None,
            order: None,
            entity_count: 1,
            primary_count: 1,
            group_names: Default::default(),
            group_favorites: Default::default(),
        }
    }

    fn two_rooms() -> Vec<AreaView> {
        vec![area("kitchen", "Kitchen"), area("garage", "Garage")]
    }

    fn ids(r: &super::SearchResults) -> Vec<&str> {
        r.sections
            .iter()
            .flat_map(|s| s.ids.iter().map(String::as_str))
            .collect()
    }

    #[test]
    fn stop_words_only_is_none() {
        let all = [light("a", "Lamp", "kitchen")];
        assert!(search_results("le", all.iter(), &two_rooms(), &[], &BTreeMap::new()).is_none());
        assert!(search_results("the", all.iter(), &two_rooms(), &[], &BTreeMap::new()).is_none());
    }

    #[test]
    fn matches_name_room_and_french_kind() {
        let all = [light("a", "Lamp", "kitchen"), light("b", "Lamp", "garage")];
        let r = search_results(
            "kitchen lampe",
            all.iter(),
            &two_rooms(),
            &[],
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(ids(&r), ["a"]);
    }

    #[test]
    fn energy_and_security_entities_are_included() {
        let mut grid = entity("grid", "Grid power", Kind::Sensor, "kitchen");
        grid.device_class = Some("power".into());
        grid.ui.energy_role = Some("grid".into());
        let mut door = entity("door", "Door", Kind::BinarySensor, "kitchen");
        door.device_class = Some("door".into());
        let all = [grid, door];
        let r = search_results("kitchen", all.iter(), &two_rooms(), &[], &BTreeMap::new()).unwrap();
        assert_eq!(r.total, 2);
        assert!(ids(&r).contains(&"grid") && ids(&r).contains(&"door"));
    }

    #[test]
    fn hidden_and_non_primary_are_out() {
        let mut hidden = light("h", "Lamp hidden", "kitchen");
        hidden.ui.hidden = true;
        let mut diag = light("d", "Lamp diag", "kitchen");
        diag.category = EntityCategory::Diagnostic;
        let mut generic = entity("g", "Lamp generic", Kind::Generic, "kitchen");
        generic.component = "update".into();
        let all = [hidden, diag, generic, light("ok", "Lamp ok", "kitchen")];
        let r = search_results("lamp", all.iter(), &two_rooms(), &[], &BTreeMap::new()).unwrap();
        assert_eq!(ids(&r), ["ok"]);
    }

    #[test]
    fn hide_members_group_shows_group_only() {
        let mut member = light("a", "Lamp A", "kitchen");
        member.ui.favorite = true;
        let group_entity = light("group_x", "Lamp group", "kitchen");
        let group = GroupView {
            id: "x".into(),
            name: "Lamp group".into(),
            kind: Kind::Light,
            members: vec![GroupMemberView {
                id: "a".into(),
                name: "Lamp A".into(),
                present: true,
                mismatch: false,
            }],
            hide_members: true,
            entity: group_entity.clone(),
            used_by_flows: Vec::new(),
        };
        let groups = BTreeMap::from([("x".to_string(), group)]);
        let all = [member, group_entity];
        let r = search_results("lamp", all.iter(), &two_rooms(), &[], &groups).unwrap();
        assert_eq!(ids(&r), ["group_x"]);
    }

    #[test]
    fn paired_humidity_folded_only_when_both_match() {
        let mut temp = entity("t", "Probe temperature", Kind::Sensor, "kitchen");
        temp.device_class = Some("temperature".into());
        temp.device_id = Some("d1".into());
        let mut hum = entity("h", "Probe humidity", Kind::Sensor, "kitchen");
        hum.device_class = Some("humidity".into());
        hum.device_id = Some("d1".into());
        let all = [temp, hum];
        let both =
            search_results("kitchen", all.iter(), &two_rooms(), &[], &BTreeMap::new()).unwrap();
        assert_eq!(ids(&both), ["t"]);
        let humidity =
            search_results("humidite", all.iter(), &two_rooms(), &[], &BTreeMap::new()).unwrap();
        assert_eq!(ids(&humidity), ["h"]);
    }

    #[test]
    fn sections_in_sidebar_order_unknown_area_last() {
        let areas = vec![area("b", "Room B"), area("a", "Room A")];
        let all = [
            light("la", "Lamp", "a"),
            light("lgone", "Lamp", "gone"),
            light("lb", "Lamp", "b"),
            light("lu", "Lamp", UNASSIGNED_AREA),
        ];
        let r = search_results("lamp", all.iter(), &areas, &[], &BTreeMap::new()).unwrap();
        let order: Vec<&str> = r.sections.iter().map(|s| s.area.as_str()).collect();
        assert_eq!(order, ["b", "a", UNASSIGNED_AREA]);
        let last = &r.sections[2].ids;
        assert!(last.contains(&"lgone".to_string()) && last.contains(&"lu".to_string()));
    }

    #[test]
    fn cap_cuts_flattened_list() {
        let mut all = Vec::new();
        for i in 0..40 {
            all.push(light(&format!("k{i}"), &format!("Lamp {i:02}"), "kitchen"));
        }
        for i in 0..30 {
            all.push(light(&format!("g{i}"), &format!("Lamp {i:02}"), "garage"));
        }
        let r = search_results("lamp", all.iter(), &two_rooms(), &[], &BTreeMap::new()).unwrap();
        assert_eq!(r.total, 70);
        assert_eq!(r.shown, SEARCH_CAP);
        assert_eq!(ids(&r).len(), SEARCH_CAP);
        assert_eq!(r.sections[0].ids.len(), 40);
        assert_eq!(r.sections[1].ids.len(), 20);
    }

    #[test]
    fn room_hits_with_sidebar_count_never_unassigned() {
        let areas = vec![
            area("kitchen", "Kitchen"),
            area(UNASSIGNED_AREA, "Unassigned"),
        ];
        let all = [
            light("a", "Lamp", "kitchen"),
            light("b", "Lamp", "kitchen"),
            light("c", "Lamp", UNASSIGNED_AREA),
        ];
        let r = search_results("kitchen", all.iter(), &areas, &[], &BTreeMap::new()).unwrap();
        assert_eq!(r.rooms.len(), 1);
        assert_eq!((r.rooms[0].id.as_str(), r.rooms[0].count), ("kitchen", 2));
        let r = search_results("unassigned", all.iter(), &areas, &[], &BTreeMap::new()).unwrap();
        assert!(r.rooms.is_empty());
    }
}
