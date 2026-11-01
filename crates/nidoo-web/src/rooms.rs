//! The rooms as the house shows them: the chip list (Favorites, each room with something on Home,
//! Unassigned last) that the overview, the room pages and the sidebar share.

use leptos::prelude::*;
use nidoo_api::{AreaView, EntityView};
use nidoo_core::model::{Kind, Placement, UNASSIGNED_AREA};
use nidoo_core::types::Placement as TypePlacement;
use std::collections::BTreeMap;

use crate::components::card_frame::GROUP_PREFIX;
use crate::groups::favorites_count;
use crate::house::House;

/// The id of the virtual "Favorites" chip.
pub(crate) const FAVORITES: &str = "__favorites__";

/// True when Home shows `e`: a primary entity that the app's placement rule keeps on Home (an
/// entity with an energy role or shown on Security leaves Home unless it is pinned).
pub(crate) fn shows_on_home(e: &EntityView) -> bool {
    e.is_primary()
        && (e.kind != Kind::Generic || generic_placement(e) == TypePlacement::Card)
        && Placement::on_home(e.kind, e.device_class.as_deref(), &e.ui)
}

/// Where the type table places a generic entity; unknown components stay cards.
pub(crate) fn generic_placement(e: &EntityView) -> TypePlacement {
    nidoo_core::types::type_def(&e.component).map_or(TypePlacement::Card, |d| d.placement)
}

/// How many entities Home shows per area id.
pub(crate) fn home_counts<'a>(
    entities: impl Iterator<Item = &'a EntityView>,
) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for e in entities.filter(|e| shows_on_home(e)) {
        *counts.entry(e.area.clone()).or_insert(0) += 1;
    }
    counts
}

/// The chips in the order they are shown: Favorites first (when `favorites` is non-zero), then
/// the areas exactly as the hub sent them, except that "Unassigned" moves to the end. Each chip
/// carries the count of entities Home shows there (`counts`), not the hub's primary count, and an
/// area where Home shows nothing has no chip.
pub(crate) fn chips(
    areas: &[AreaView],
    favorites: usize,
    counts: &BTreeMap<String, usize>,
) -> Vec<AreaView> {
    let mut chips = Vec::with_capacity(areas.len() + 1);
    if favorites > 0 {
        chips.push(AreaView {
            id: FAVORITES.into(),
            // The room page titles it in the language on screen; no chip shows this name.
            name: String::new(),
            icon: None,
            order: None,
            entity_count: favorites,
            primary_count: favorites,
            group_names: Default::default(),
            group_favorites: Default::default(),
        });
    }
    let counted = |a: &AreaView| AreaView {
        primary_count: counts.get(&a.id).copied().unwrap_or(0),
        ..a.clone()
    };
    chips.extend(
        areas
            .iter()
            .filter(|a| a.id != UNASSIGNED_AREA)
            .map(counted),
    );
    chips.extend(
        areas
            .iter()
            .filter(|a| a.id == UNASSIGNED_AREA)
            .map(counted),
    );
    chips.retain(|c| c.primary_count > 0);
    chips
}

/// The chips that are rooms of their own: `chips` without Favorites and Unassigned. The sidebar
/// lists these.
pub(crate) fn sidebar_rooms(chips: &[AreaView]) -> Vec<AreaView> {
    chips
        .iter()
        .filter(|c| c.id != FAVORITES && c.id != UNASSIGNED_AREA)
        .cloned()
        .collect()
}

/// The chip list built from the house. Memos only notify when their value changes, so a state
/// echo leaves the list alone.
pub(crate) fn use_room_chips() -> Memo<Vec<AreaView>> {
    let house = expect_context::<House>();
    let favorites = Memo::new(move |_| {
        house.areas.with(|a| {
            house
                .entities
                .with(|m| house.groups.with(|g| favorites_count(m.values(), a, g)))
        })
    });
    let counts = Memo::new(move |_| house.entities.with(|m| home_counts(m.values())));
    Memo::new(move |_| {
        house
            .areas
            .with(|a| counts.with(|c| chips(a, favorites.get(), c)))
    })
}

/// Fetches the groups' member counts on mount and again when a group appears or goes.
pub(crate) fn use_group_refresh() {
    let house = expect_context::<House>();
    let group_ids = Memo::new(move |_| {
        house.entities.with(|m| {
            m.keys()
                .filter(|id| id.starts_with(GROUP_PREFIX))
                .cloned()
                .collect::<Vec<_>>()
        })
    });
    Effect::new(move |_| {
        group_ids.track();
        house.refresh_groups();
    });
}

/// True once the house has said anything (rooms, entities or an online socket).
pub(crate) fn use_loaded() -> Memo<bool> {
    let house = expect_context::<House>();
    Memo::new(move |_| {
        house.areas.with(|a| !a.is_empty())
            || house.entities.with(|m| !m.is_empty())
            || matches!(house.connection.get(), nidoo_api::ConnectionStatus::Online)
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{FAVORITES, chips, home_counts, sidebar_rooms};
    use nidoo_api::{AreaView, EntityView};
    use nidoo_core::model::{
        Capabilities, EntityCategory, EntityState, Kind, UNASSIGNED_AREA, UiOverride,
    };

    fn entity(id: &str, kind: Kind, device_class: Option<&str>, ui: UiOverride) -> EntityView {
        EntityView {
            id: id.into(),
            device_id: None,
            name: id.into(),
            device_name: None,
            label: None,
            kind,
            component: kind.component().to_string(),
            protocol_kind: kind,
            device_class: device_class.map(Into::into),
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: "living".into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui,
            state: EntityState::default(),
        }
    }

    fn role(role: &str, pinned: bool, hidden: bool) -> UiOverride {
        UiOverride {
            energy_role: Some(role.into()),
            home_pinned: pinned,
            hidden,
            ..Default::default()
        }
    }

    #[test]
    fn chip_counts_follow_home_rule() {
        let mut outside = entity(
            "door",
            Kind::BinarySensor,
            Some("door"),
            UiOverride::default(),
        );
        outside.area = UNASSIGNED_AREA.into();
        let all = [
            entity("lamp", Kind::Light, None, UiOverride::default()),
            entity(
                "grid",
                Kind::Sensor,
                Some("power"),
                role("grid", false, false),
            ),
            outside,
        ];
        let counts = home_counts(all.iter());
        assert_eq!(counts.get("living"), Some(&1));
        assert_eq!(counts.get(UNASSIGNED_AREA), None);
        let areas = [area("living"), area(UNASSIGNED_AREA)];
        let list = chips(&areas, 0, &counts);
        assert_eq!(ids(&list), ["living"]);
        assert_eq!(list[0].primary_count, 1);
    }

    /// One card in each of `ids`, as `home_counts` would count them.
    fn counts(ids: &[&str]) -> BTreeMap<String, usize> {
        ids.iter().map(|id| ((*id).to_owned(), 1)).collect()
    }

    #[test]
    fn rooms_with_nothing_on_home_have_no_chip() {
        let areas = [area("upstairs"), area("living"), area(UNASSIGNED_AREA)];
        assert_eq!(ids(&chips(&areas, 0, &counts(&["living"]))), ["living"]);
        assert!(chips(&areas, 0, &BTreeMap::new()).is_empty());
    }

    fn area(id: &str) -> AreaView {
        AreaView {
            id: id.into(),
            name: id.into(),
            icon: None,
            order: None,
            entity_count: 1,
            primary_count: 1,
            group_names: Default::default(),
            group_favorites: Default::default(),
        }
    }

    fn ids(list: &[AreaView]) -> Vec<&str> {
        list.iter().map(|a| a.id.as_str()).collect()
    }

    #[test]
    fn keeps_hub_order_and_moves_unassigned_last() {
        let areas = [area("zeta"), area("Unassigned"), area("alpha")];
        assert_eq!(
            ids(&chips(&areas, 0, &counts(&["zeta", "Unassigned", "alpha"]))),
            ["zeta", "alpha", "Unassigned"]
        );
    }

    #[test]
    fn favorites_chip_comes_first_when_any() {
        let areas = [area("kitchen")];
        let kitchen = counts(&["kitchen"]);
        assert_eq!(ids(&chips(&areas, 2, &kitchen)), [FAVORITES, "kitchen"]);
        assert_eq!(ids(&chips(&areas, 0, &kitchen)), ["kitchen"]);
    }

    #[test]
    fn sidebar_rooms_drop_favorites_and_unassigned() {
        let areas = [area("living"), area("kitchen"), area(UNASSIGNED_AREA)];
        let all = chips(&areas, 2, &counts(&["living", "kitchen", UNASSIGNED_AREA]));
        assert_eq!(ids(&all), [FAVORITES, "living", "kitchen", UNASSIGNED_AREA]);
        let rooms = sidebar_rooms(&all);
        assert_eq!(ids(&rooms), ["living", "kitchen"]);
        assert!(rooms.iter().all(|r| r.primary_count == 1));
    }
}
