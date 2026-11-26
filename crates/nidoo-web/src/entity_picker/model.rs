//! What the picker lists: the entities that pass a field's rules and the search, grouped by room
//! and capped. Pure, so a state echo (which changes no row) yields an equal [`Listing`].

use std::collections::{BTreeMap, BTreeSet};

use nidoo_api::{AreaView, EntityView};
use nidoo_core::model::UNASSIGNED_AREA;

use super::Keep;
use crate::flow_edit::fold_accents;
use crate::format::{
    area_name, entity_title, kind_search_words, list_label, room_name, shown_device_name,
    title_suffix,
};
use crate::i18n::Lang;
use crate::i18n::picker::{PickerText, picker_text};
use crate::icons::entity_icon;

/// Most rows listed while nothing narrows the list; a notice tells the rest is behind a search.
pub const CAP: usize = 200;

/// The picker's chip: the field's suggestion, or everything the field accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip {
    Suggested,
    All,
}

/// Everything that decides what the picker lists.
pub struct Filter<'a> {
    /// Hard: what the field accepts; `None` is every entity.
    pub pool: Option<&'a Keep>,
    /// Soft: the Suggested chip.
    pub suggest: Option<&'a Keep>,
    /// Ignored when `suggest` is `None`.
    pub chip: Chip,
    /// Also list entities the UI hides by default.
    pub diagnostic: bool,
    pub query: &'a str,
    /// The area id picked in the room column; `None` is all rooms.
    pub room: Option<&'a str>,
    /// The chosen ids: always listed (pool, chip and diagnostic ignored; the query applies).
    pub current: &'a [String],
}

/// One entity in the list. It carries no state, so a value change never changes a row.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub title: String,
    /// The device name, when it tells look-alikes apart.
    pub device: Option<String>,
    pub room: Option<String>,
    pub icon: &'static str,
}

/// The rows of one room.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub area: String,
    pub room: String,
    pub rows: Vec<Row>,
}

/// How many matches a room holds, for the room column.
#[derive(Clone, Debug, PartialEq)]
pub struct RoomCount {
    pub area: String,
    pub room: String,
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Listing {
    /// Chosen ids the house lacks, in `current` order.
    pub missing: Vec<String>,
    /// After the room narrowing and the cap.
    pub sections: Vec<Section>,
    /// Matches per room, before the room narrowing.
    pub rooms: Vec<RoomCount>,
    /// Matches before the cap (and before the room narrowing).
    pub total: usize,
    /// Rows in `sections`.
    pub shown: usize,
}

/// Whether `e` matches the folded `needle`: it is part of the entity's list label, name, device
/// (as shown and as named), room, kind words (both languages) or id. An empty needle matches
/// everything.
pub fn matches(lang: Lang, e: &EntityView, room: Option<&str>, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let [kind_en, kind_fr] = kind_search_words(e);
    [
        list_label(lang, e),
        e.name.clone(),
        e.device_name.clone().unwrap_or_default(),
        shown_device_name(lang, e).unwrap_or_default().to_owned(),
        room.unwrap_or_default().to_owned(),
        kind_en,
        kind_fr,
        e.id.clone(),
    ]
    .iter()
    .any(|hay| fold_accents(hay).contains(needle))
}

/// The room key of `area`: the catch-all and a blank area are one.
fn area_key(area: &str) -> &str {
    if area.is_empty() {
        UNASSIGNED_AREA
    } else {
        area
    }
}

/// The row for `e`: its title, the device when that adds something, its room and icon.
pub fn row_of(lang: Lang, e: &EntityView, room: Option<String>) -> Row {
    let title = entity_title(lang, e);
    // A title with a suffix is "device · kind": it already names the device.
    let device = shown_device_name(lang, e)
        .filter(|d| !d.is_empty() && *d != title && title_suffix(lang, e).is_none())
        .map(str::to_owned);
    Row {
        id: e.id.clone(),
        title,
        device,
        room,
        icon: entity_icon(e),
    }
}

/// The picker's list for `f`: sections in the hub's room order, each sorted by title, unknown
/// rooms next and "No room" last.
pub fn listing(
    lang: Lang,
    entities: &BTreeMap<String, EntityView>,
    areas: &[AreaView],
    f: &Filter,
) -> Listing {
    let needle = fold_accents(f.query.trim());
    let suggested = f.chip == Chip::Suggested && f.suggest.is_some();
    let current: BTreeSet<&str> = f.current.iter().map(String::as_str).collect();
    let missing = f
        .current
        .iter()
        .filter(|id| !entities.contains_key(*id))
        .cloned()
        .collect();

    // Rows by area key, each with its folded title for the sort.
    let mut by_area: BTreeMap<String, Vec<(String, Row)>> = BTreeMap::new();
    for e in entities.values() {
        let chosen = current.contains(e.id.as_str());
        let candidate = f.pool.is_none_or(|p| p(e))
            && (!suggested || f.suggest.is_some_and(|s| s(e)))
            && (f.diagnostic || e.is_primary());
        let room = room_name(areas, &e.area).filter(|r| !r.is_empty());
        if !(candidate || chosen) || !matches(lang, e, room.as_deref(), &needle) {
            continue;
        }
        let row = row_of(lang, e, room);
        by_area
            .entry(area_key(&e.area).to_owned())
            .or_default()
            .push((fold_accents(&row.title), row));
    }
    for rows in by_area.values_mut() {
        rows.sort_by(|a, b| (&a.0, &a.1.id).cmp(&(&b.0, &b.1.id)));
    }

    let mut sections = Vec::new();
    let mut push = |area: &str, room: String, rows: Vec<(String, Row)>| {
        sections.push(Section {
            area: area.to_owned(),
            room,
            rows: rows.into_iter().map(|(_, r)| r).collect(),
        });
    };
    for a in areas.iter().filter(|a| a.id != UNASSIGNED_AREA) {
        if let Some(rows) = by_area.remove(&a.id) {
            push(&a.id, area_name(lang, &a.id, &a.name), rows);
        }
    }
    let unassigned = by_area.remove(UNASSIGNED_AREA);
    for (area, rows) in by_area {
        push(&area, area.clone(), rows);
    }
    if let Some(rows) = unassigned {
        push(
            UNASSIGNED_AREA,
            picker_text(lang, PickerText::NoRoom).to_owned(),
            rows,
        );
    }

    let rooms = sections
        .iter()
        .map(|s| RoomCount {
            area: s.area.clone(),
            room: s.room.clone(),
            count: s.rows.len(),
        })
        .collect();
    let total = sections.iter().map(|s| s.rows.len()).sum();
    if let Some(room) = f.room {
        sections.retain(|s| s.area == room);
    }
    if (f.chip == Chip::All || f.suggest.is_none()) && needle.is_empty() && f.room.is_none() {
        let mut left = CAP;
        for s in &mut sections {
            s.rows.truncate(left);
            left -= s.rows.len();
        }
        sections.retain(|s| !s.rows.is_empty());
    }
    let shown = sections.iter().map(|s| s.rows.len()).sum();
    Listing {
        missing,
        sections,
        rooms,
        total,
        shown,
    }
}

#[cfg(test)]
mod tests;
