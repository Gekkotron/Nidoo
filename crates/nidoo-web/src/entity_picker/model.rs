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
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use nidoo_api::{AreaView, EntityView};
    use nidoo_core::model::{
        Capabilities, EntityCategory, EntityState, Kind, StateValue, UiOverride,
    };

    use super::*;
    use crate::entity_picker::needs::{is_siren, is_thermometer, keep};
    use crate::i18n::Lang;

    fn ent(
        id: &str,
        name: &str,
        device: &str,
        area: &str,
        kind: Kind,
        class: Option<&str>,
    ) -> EntityView {
        EntityView {
            id: id.into(),
            device_id: None,
            name: name.into(),
            device_name: (!device.is_empty()).then(|| device.to_owned()),
            label: None,
            kind,
            component: kind.component().to_string(),
            protocol_kind: kind,
            device_class: class.map(Into::into),
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

    fn area(id: &str, name: &str) -> AreaView {
        AreaView {
            id: id.into(),
            name: name.into(),
            icon: None,
            order: None,
            entity_count: 0,
            primary_count: 0,
            group_names: BTreeMap::new(),
            group_favorites: Default::default(),
        }
    }

    fn house(list: Vec<EntityView>) -> BTreeMap<String, EntityView> {
        list.into_iter().map(|e| (e.id.clone(), e)).collect()
    }

    fn filter<'a>(query: &'a str) -> Filter<'a> {
        Filter {
            pool: None,
            suggest: None,
            chip: Chip::All,
            diagnostic: false,
            query,
            room: None,
            current: &[],
        }
    }

    fn ids(l: &Listing) -> Vec<&str> {
        l.sections
            .iter()
            .flat_map(|s| s.rows.iter().map(|r| r.id.as_str()))
            .collect()
    }

    #[test]
    fn search_folds_accents_and_both_languages() {
        let h = house(vec![
            ent(
                "t",
                "Température",
                "",
                "a",
                Kind::Sensor,
                Some("temperature"),
            ),
            ent("s", "Plug", "", "a", Kind::Switch, None),
        ]);
        let areas = [area("a", "Room A")];
        for q in ["temperature", "TEMPÉ"] {
            assert_eq!(
                ids(&listing(Lang::En, &h, &areas, &filter(q))),
                ["t"],
                "{q}"
            );
        }
        assert_eq!(
            ids(&listing(Lang::En, &h, &areas, &filter("interrupteur"))),
            ["s"]
        );
        assert_eq!(
            ids(&listing(Lang::Fr, &h, &areas, &filter("switch"))),
            ["s"]
        );
    }

    #[test]
    fn weather_entities_match_french_words() {
        let h = house(vec![
            ent(
                "weather_rain_next_hour",
                "Rain next hour",
                "Weather",
                "",
                Kind::Sensor,
                Some("precipitation"),
            ),
            ent(
                "weather_pressure",
                "Pressure",
                "Weather",
                "",
                Kind::Sensor,
                None,
            ),
            ent("s", "Plug", "", "", Kind::Switch, None),
        ]);
        let fr = |q| ids(&listing(Lang::Fr, &h, &[], &filter(q))).join(",");
        assert_eq!(fr("météo"), "weather_rain_next_hour,weather_pressure");
        assert_eq!(fr("meteo"), "weather_rain_next_hour,weather_pressure");
        assert_eq!(fr("pluie"), "weather_rain_next_hour");
        // The hub's English names still match.
        assert_eq!(fr("weather"), "weather_rain_next_hour,weather_pressure");
        let rows = listing(Lang::Fr, &h, &[], &filter("pluie")).sections;
        assert_eq!(rows[0].rows[0].title, "Pluie dans l'heure");
        assert_eq!(rows[0].rows[0].device.as_deref(), Some("Météo"));
    }

    #[test]
    fn device_and_room_names_match() {
        let h = house(vec![
            ent("a", "Mode", "Plug A", "k", Kind::Switch, None),
            ent("b", "Mode", "Plug B", "j", Kind::Switch, None),
        ]);
        let areas = [area("k", "Kitchen"), area("j", "Hall")];
        assert_eq!(
            ids(&listing(Lang::En, &h, &areas, &filter("plug a"))),
            ["a"]
        );
        assert_eq!(ids(&listing(Lang::En, &h, &areas, &filter("kitch"))), ["a"]);
        assert_eq!(
            ids(&listing(Lang::En, &h, &areas, &filter("PLUG"))),
            ["a", "b"]
        );
    }

    #[test]
    fn look_alikes_differ_by_device() {
        let h = house(vec![
            ent("a", "Backlight mode", "Plug A", "a", Kind::Switch, None),
            ent("b", "Backlight mode", "Plug B", "a", Kind::Switch, None),
        ]);
        let l = listing(Lang::En, &h, &[area("a", "Room A")], &filter(""));
        let devices: Vec<_> = l.sections[0]
            .rows
            .iter()
            .map(|r| r.device.clone())
            .collect();
        assert_eq!(devices, [Some("Plug A".into()), Some("Plug B".into())]);
        assert_eq!(l.sections[0].rows[0].room.as_deref(), Some("Room A"));
    }

    #[test]
    fn device_line_skips_a_device_already_in_the_title() {
        // Named like its device: the title ("Plug A · Temperature") already leads with it.
        let named = ent(
            "t",
            "Plug A",
            "Plug A",
            "a",
            Kind::Sensor,
            Some("temperature"),
        );
        let own = ent("b", "Backlight mode", "Plug A", "a", Kind::Switch, None);
        let row = row_of(Lang::En, &named, Some("Room A".into()));
        assert!(row.title.starts_with("Plug A"), "{}", row.title);
        assert_eq!(row.device, None);
        assert_eq!(
            row_of(Lang::En, &own, None).device.as_deref(),
            Some("Plug A")
        );
    }

    #[test]
    fn suggested_chip_narrows_and_all_widens_to_pool_only() {
        let h = house(vec![
            ent("t", "Temp", "", "a", Kind::Sensor, Some("temperature")),
            ent("h", "Hum", "", "a", Kind::Sensor, Some("humidity")),
            ent("s", "Siren", "", "a", Kind::Switch, None),
        ]);
        let areas = [area("a", "Room A")];
        let thermo = keep(is_thermometer);
        let siren = keep(is_siren);
        let mut f = filter("");
        f.suggest = Some(&thermo);
        f.chip = Chip::Suggested;
        assert_eq!(ids(&listing(Lang::En, &h, &areas, &f)), ["t"]);
        f.chip = Chip::All;
        assert_eq!(ids(&listing(Lang::En, &h, &areas, &f)), ["h", "s", "t"]);
        let mut g = filter("");
        g.pool = Some(&siren);
        g.chip = Chip::All;
        assert_eq!(ids(&listing(Lang::En, &h, &areas, &g)), ["s"]);
    }

    #[test]
    fn diagnostic_hidden_unless_flag_or_current() {
        let mut d = ent("d", "Link quality", "", "a", Kind::Sensor, None);
        d.category = EntityCategory::Diagnostic;
        let h = house(vec![d, ent("p", "Plug", "", "a", Kind::Switch, None)]);
        let areas = [area("a", "Room A")];
        assert_eq!(ids(&listing(Lang::En, &h, &areas, &filter(""))), ["p"]);
        let mut f = filter("");
        f.diagnostic = true;
        assert_eq!(ids(&listing(Lang::En, &h, &areas, &f)), ["d", "p"]);
        let cur = ["d".to_owned()];
        let mut g = filter("");
        g.current = &cur;
        assert_eq!(ids(&listing(Lang::En, &h, &areas, &g)), ["d", "p"]);
        g.query = "plug";
        assert_eq!(ids(&listing(Lang::En, &h, &areas, &g)), ["p"]);
    }

    #[test]
    fn current_ignores_pool_and_chip() {
        let h = house(vec![
            ent("t", "Temp", "", "a", Kind::Sensor, Some("temperature")),
            ent("s", "Siren", "", "a", Kind::Switch, None),
        ]);
        let thermo = keep(is_thermometer);
        let cur = ["s".to_owned()];
        let mut f = filter("");
        f.pool = Some(&thermo);
        f.current = &cur;
        assert_eq!(
            ids(&listing(Lang::En, &h, &[area("a", "A")], &f)),
            ["s", "t"]
        );
    }

    #[test]
    fn a_chosen_group_stays_listed_though_the_pool_refuses_it() {
        let h = house(vec![
            ent("lamp", "Lamp", "", "a", Kind::Light, None),
            ent("group_all", "All", "", "a", Kind::Light, None),
        ]);
        let pool: Keep = Arc::new(|e: &EntityView| !e.id.starts_with("group_"));
        let cur = ["group_all".to_owned()];
        let areas = [area("a", "A")];
        let mut f = filter("");
        f.pool = Some(&pool);
        assert_eq!(ids(&listing(Lang::En, &h, &areas, &f)), ["lamp"]);
        f.current = &cur;
        assert_eq!(
            ids(&listing(Lang::En, &h, &areas, &f)),
            ["group_all", "lamp"]
        );
    }

    #[test]
    fn missing_ids_lead() {
        let h = house(vec![ent("a", "Plug", "", "a", Kind::Switch, None)]);
        let cur = ["gone".to_owned(), "a".to_owned()];
        let mut f = filter("");
        f.current = &cur;
        let l = listing(Lang::En, &h, &[area("a", "Room A")], &f);
        assert_eq!(l.missing, ["gone"]);
        assert_eq!(ids(&l), ["a"]);
    }

    #[test]
    fn sections_follow_area_order_no_room_last() {
        let h = house(vec![
            ent("n", "N", "", "Unassigned", Kind::Switch, None),
            ent("z", "Z", "", "zz", Kind::Switch, None),
            ent("a", "A", "", "a", Kind::Switch, None),
            ent("b", "B", "", "b", Kind::Switch, None),
            ent("e", "E", "", "", Kind::Switch, None),
        ]);
        let areas = [area("b", "Bed"), area("a", "Attic")];
        let l = listing(Lang::En, &h, &areas, &filter(""));
        let names: Vec<_> = l.sections.iter().map(|s| s.room.as_str()).collect();
        assert_eq!(names, ["Bed", "Attic", "zz", "No room"]);
        assert_eq!(ids(&l), ["b", "a", "z", "e", "n"]);
        assert_eq!(l.sections[3].area, nidoo_core::model::UNASSIGNED_AREA);
    }

    #[test]
    fn room_narrows_sections_but_not_counts() {
        let h = house(vec![
            ent("a", "A", "", "a", Kind::Switch, None),
            ent("b", "B", "", "b", Kind::Switch, None),
            ent("c", "C", "", "b", Kind::Switch, None),
        ]);
        let areas = [area("a", "Attic"), area("b", "Bed")];
        let all = listing(Lang::En, &h, &areas, &filter(""));
        let mut f = filter("");
        f.room = Some("a");
        let l = listing(Lang::En, &h, &areas, &f);
        assert_eq!(ids(&l), ["a"]);
        assert_eq!(l.rooms, all.rooms);
        assert_eq!(l.rooms[1].count, 2);
        assert_eq!((l.total, l.shown), (3, 1));
    }

    #[test]
    fn all_without_query_caps_at_200() {
        let list = (0..1000)
            .map(|i| {
                ent(
                    &format!("e{i}"),
                    &format!("Plug {i}"),
                    "",
                    "a",
                    Kind::Switch,
                    None,
                )
            })
            .collect();
        let h = house(list);
        let areas = [area("a", "Room A")];
        let l = listing(Lang::En, &h, &areas, &filter(""));
        assert_eq!((l.shown, l.total), (CAP, 1000));
        let l = listing(Lang::En, &h, &areas, &filter("1"));
        assert_eq!(l.shown, l.total);
        assert!(l.shown > CAP);
        let mut f = filter("");
        f.room = Some("a");
        assert_eq!(listing(Lang::En, &h, &areas, &f).shown, 1000);
    }

    #[test]
    fn suggested_chip_is_never_capped() {
        let list = (0..300)
            .map(|i| {
                ent(
                    &format!("e{i}"),
                    "T",
                    "",
                    "a",
                    Kind::Sensor,
                    Some("temperature"),
                )
            })
            .collect();
        let thermo = keep(is_thermometer);
        let mut f = filter("");
        f.suggest = Some(&thermo);
        f.chip = Chip::Suggested;
        let l = listing(Lang::En, &house(list), &[area("a", "A")], &f);
        assert_eq!(l.shown, 300);
    }

    #[test]
    fn listing_ignores_state_changes() {
        let mut a = ent("a", "Temp", "", "a", Kind::Sensor, Some("temperature"));
        let mut b = a.clone();
        a.state.value = StateValue::Number(20.0);
        b.state.value = StateValue::Number(21.5);
        let areas = [area("a", "Room A")];
        let f = filter("");
        assert_eq!(
            listing(Lang::En, &house(vec![a]), &areas, &f),
            listing(Lang::En, &house(vec![b]), &areas, &f)
        );
    }

    #[test]
    fn keep_is_shareable() {
        let k: Keep = Arc::new(|e: &EntityView| e.id == "x");
        assert!(k(&ent("x", "X", "", "a", Kind::Switch, None)));
    }
}
