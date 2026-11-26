//! Group text and maths, and the cards Home shows. Pure; the cards live elsewhere.

use std::collections::{BTreeMap, HashSet};

use nidoo_api::{AreaView, EntityView, GroupView};
use nidoo_core::model::{Availability, Command, CoverState, StateValue};

use crate::cards::cover::is_moving;
use crate::format::is_on;
use crate::i18n::{GroupState, Lang, group_state};
use crate::rooms::{FAVORITES, shows_on_home};
use crate::screens::home::without_paired_companions;

/// One card on Home: a single entity (a hand-made group is an entity too).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum HomeItem {
    Entity(String),
}

/// The entities Home shows, in Home order (the hub's `order` override first, then by name).
fn shown<'a>(entities: impl Iterator<Item = &'a EntityView>) -> Vec<&'a EntityView> {
    let mut shown: Vec<&EntityView> = entities.filter(|e| shows_on_home(e)).collect();
    shown.sort_by_cached_key(|e| (e.ui.order.unwrap_or(i32::MAX), e.name.to_lowercase()));
    shown
}

/// Members of a group with `hide_members` lose their own card on Home; a member that is in
/// several groups is hidden when any of them says so.
pub(crate) fn hidden_member_ids(groups: &BTreeMap<String, GroupView>) -> HashSet<&str> {
    groups
        .values()
        .filter(|g| g.hide_members)
        .flat_map(|g| g.members.iter().map(|m| m.id.as_str()))
        .collect()
}

/// What Home shows under `chip`, in display order: pinned cards first, then by room. A room
/// chip shows the whole room; Favorites shows the favourite entities.
pub fn home_items<'a>(
    entities: impl Iterator<Item = &'a EntityView>,
    areas: &[AreaView],
    chip: &str,
    groups: &BTreeMap<String, GroupView>,
) -> Vec<HomeItem> {
    let hidden_member_ids = hidden_member_ids(groups);
    let candidates: Vec<&EntityView> = shown(entities)
        .into_iter()
        .filter(|e| {
            if hidden_member_ids.contains(e.id.as_str()) && !(chip == FAVORITES && e.ui.favorite) {
                return false;
            }
            if chip != FAVORITES {
                return e.area == chip;
            }
            e.ui.favorite
        })
        .collect();
    let candidates = without_paired_companions(candidates);

    let mut placed: Vec<(i32, usize, HomeItem)> = candidates
        .into_iter()
        .map(|e| {
            let area_index = areas
                .iter()
                .position(|a| a.id == e.area)
                .unwrap_or(usize::MAX);
            (
                e.ui.order.unwrap_or(i32::MAX),
                area_index,
                HomeItem::Entity(e.id.clone()),
            )
        })
        .collect();
    placed.sort_by_key(|(order, area, _)| (*order, *area));
    placed.into_iter().map(|(_, _, item)| item).collect()
}

/// How many cards the Favorites chip would show.
pub fn favorites_count<'a>(
    entities: impl Iterator<Item = &'a EntityView>,
    areas: &[AreaView],
    groups: &BTreeMap<String, GroupView>,
) -> usize {
    home_items(entities, areas, FAVORITES, groups).len()
}

/// The way a cover group is moving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Opening,
    Closing,
}

/// A group's state boiled down for its card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupSummary {
    /// Members that are on (lights) or open (covers).
    pub on: usize,
    pub total: usize,
    pub moving: Option<Motion>,
    /// Each member's position for the mini bars, 0..=100.
    pub positions: Vec<u8>,
    /// Any member online.
    pub available: bool,
}

fn motion_of(text: &str) -> Option<Motion> {
    if text.eq_ignore_ascii_case("opening") {
        Some(Motion::Opening)
    } else if text.eq_ignore_ascii_case("closing") {
        Some(Motion::Closing)
    } else {
        None
    }
}

/// How `member` moves: by its own state, else by a folded "moving" child (the child's text gives
/// the direction, opening when it gives none).
fn motion(member: &EntityView, entities: &BTreeMap<String, EntityView>) -> Option<Motion> {
    if let StateValue::Cover(CoverState {
        state: Some(state), ..
    }) = &member.state.value
        && let Some(m) = motion_of(state)
    {
        return Some(m);
    }
    let children: Vec<&EntityView> = entities
        .values()
        .filter(|c| c.part_of.as_deref() == Some(member.id.as_str()))
        .collect();
    if !is_moving(children.iter().copied()) {
        return None;
    }
    let from_text = children.iter().find_map(|c| match &c.state.value {
        StateValue::Text(t) => motion_of(t),
        _ => None,
    });
    Some(from_text.unwrap_or(Motion::Opening))
}

fn position_of(member: &EntityView) -> u8 {
    match &member.state.value {
        StateValue::Cover(CoverState {
            position: Some(p), ..
        }) => (*p).min(100),
        _ => {
            if is_on(&member.state) {
                100
            } else {
                0
            }
        }
    }
}

/// `members` boiled down; `entities` supplies the folded children that say a cover moves.
pub fn summary(members: &[EntityView], entities: &BTreeMap<String, EntityView>) -> GroupSummary {
    let motions: Vec<Motion> = members.iter().filter_map(|m| motion(m, entities)).collect();
    let moving = if motions.contains(&Motion::Opening) {
        Some(Motion::Opening)
    } else {
        motions.first().copied()
    };
    GroupSummary {
        on: members.iter().filter(|m| is_on(&m.state)).count(),
        total: members.len(),
        moving,
        positions: members.iter().map(position_of).collect(),
        available: members.iter().any(is_available),
    }
}

/// A member that can take a command: anything not known to be offline.
fn is_available(e: &EntityView) -> bool {
    e.state.availability != Availability::Offline
}

/// What a cover group says: unavailable when no member is online, else moving, all open, all
/// closed or how many are open and closed.
pub fn cover_text(lang: Lang, s: &GroupSummary) -> String {
    let state = if !s.available {
        GroupState::Unavailable
    } else {
        match s.moving {
            Some(Motion::Opening) => GroupState::Opening,
            Some(Motion::Closing) => GroupState::Closing,
            None if s.on == s.total => GroupState::AllOpen,
            None if s.on == 0 => GroupState::AllClosed,
            None => GroupState::Covers {
                open: s.on,
                closed: s.total - s.on,
            },
        }
    };
    group_state(lang, state)
}

/// What a light group says: unavailable when no member is online, else all on, all off or
/// how many of them are on.
pub fn light_text(lang: Lang, s: &GroupSummary) -> String {
    let state = if !s.available {
        GroupState::Unavailable
    } else if s.on == s.total {
        GroupState::AllOn
    } else if s.on == 0 {
        GroupState::AllOff
    } else {
        GroupState::SomeOn {
            on: s.on,
            total: s.total,
        }
    };
    group_state(lang, state)
}

/// The members `cmd` goes to: the available ones, and for a position or a brightness only those
/// that support it.
pub fn targets(members: &[EntityView], cmd: &Command) -> Vec<String> {
    members
        .iter()
        .filter(|m| is_available(m))
        .filter(|m| match cmd {
            Command::SetPosition { .. } => m.capabilities.position,
            Command::TurnOn {
                brightness: Some(_),
                ..
            } => m.capabilities.brightness,
            _ => true,
        })
        .map(|m| m.id.clone())
        .collect()
}

/// The light group's toggle is absolute: off when any light is on, else on.
pub fn light_toggle(s: &GroupSummary) -> Command {
    if s.on > 0 {
        Command::TurnOff
    } else {
        Command::TurnOn {
            brightness: None,
            color: None,
            effect: None,
        }
    }
}

/// The rounded mean of `values`, `None` when there are none.
fn mean(values: impl Iterator<Item = u8>) -> Option<u8> {
    let (sum, n) = values.fold((0u32, 0u32), |(s, n), v| (s + u32::from(v), n + 1));
    (n > 0).then(|| (f64::from(sum) / f64::from(n)).round() as u8)
}

/// The "All to" slider's start: the mean of the known positions of position-capable members.
pub fn mean_position(members: &[EntityView]) -> u8 {
    mean(
        members
            .iter()
            .filter(|m| m.capabilities.position)
            .filter_map(|m| match &m.state.value {
                StateValue::Cover(c) => c.position.map(|p| p.min(100)),
                _ => None,
            }),
    )
    .unwrap_or(0)
}

/// The brightness slider's start: the mean brightness of the dimmable lights that are on.
pub fn mean_brightness(members: &[EntityView]) -> u8 {
    mean(
        members
            .iter()
            .filter(|m| m.capabilities.brightness)
            .filter_map(|m| match &m.state.value {
                StateValue::Light(l) if l.on => l.brightness,
                _ => None,
            }),
    )
    .unwrap_or(255)
}

#[cfg(test)]
mod tests;
