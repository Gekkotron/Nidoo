//! Which room the words name, from the house's areas only.

use nidoo_api::search::fold;
use nidoo_core::model::UNASSIGNED_AREA;

use crate::house::House;
use crate::lexicon::{BEDROOM, Class, NOT_BEDROOM, class};
use crate::text::{Word, number_text, words};

/// The area the room words name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoomMatch {
    One(String),
    /// Equally good areas, in the house's order.
    Many(Vec<String>),
    NoRoomSaid,
    Unknown,
}

/// Whether a room's words say it is something other than a bedroom ("Salon", "Salle de bain").
fn not_bedroom(name: &[String]) -> bool {
    name.iter().any(|w| NOT_BEDROOM.contains(&w.as_str()))
}

fn is_bedroom_word(w: &str) -> bool {
    BEDROOM.contains(&w)
}

/// The significant folded words of a name: linkers and noise dropped ("Chambre de Léa" →
/// `chambre`, `lea`).
pub fn name_words(name: &str) -> Vec<String> {
    words(&name.replace('_', " "))
        .into_iter()
        .filter_map(|w| match w {
            Word::Text(t) => Some(t),
            Word::Number(n) => Some(number_text(n)),
            Word::Signed(_) => None,
        })
        .filter(|t| !matches!(class(t), Some(Class::Linker | Class::Noise)))
        .collect()
}

/// How well `said` names an area whose words are `name`: 2 when they are the same words, 1
/// when every said word is one of the name's, 0 otherwise. A bedroom word the name lacks may be
/// left out ("chambre de Léa" names "Léa", partly), never for a room whose name says it is
/// something else ("chambre du salon" names no room).
pub(crate) fn score(said: &[String], name: &[String]) -> u8 {
    if said.iter().all(|w| name.contains(w)) {
        return if name.iter().all(|w| said.contains(w)) {
            2
        } else {
            1
        };
    }
    let rest: Vec<&String> = said.iter().filter(|w| !is_bedroom_word(w)).collect();
    let dropped = rest.len() < said.len();
    u8::from(
        dropped && !rest.is_empty() && !not_bedroom(name) && rest.iter().all(|w| name.contains(w)),
    )
}

/// Resolves room words against the areas' ids and names (folded). An exact name beats a name
/// that only contains the words; equally good areas are all returned.
pub fn resolve_room(house: &House, room_words: &[String]) -> RoomMatch {
    let said: Vec<String> = room_words
        .iter()
        .map(|w| fold(w))
        .filter(|w| !matches!(class(w), Some(Class::Linker | Class::Noise)))
        .collect();
    if said.is_empty() {
        return RoomMatch::NoRoomSaid;
    }
    let said_flat: Vec<String> = said.iter().flat_map(|w| name_words(w)).collect();
    let mut best = 0;
    let mut found: Vec<String> = Vec::new();
    for area in house.areas {
        let s = score(&said_flat, &name_words(&area.name))
            .max(score(&said_flat, &name_words(&area.id)));
        if s == 0 || s < best {
            continue;
        }
        if s > best {
            best = s;
            found.clear();
        }
        found.push(area.id.clone());
    }
    // A bare "chambre" no room name holds: every room that may be a bedroom, to ask among, never
    // to act on.
    if found.is_empty() && said_flat.iter().all(|w| is_bedroom_word(w)) {
        found = house
            .areas
            .iter()
            .filter(|a| {
                a.id != UNASSIGNED_AREA
                    && !not_bedroom(&name_words(&a.name))
                    && !not_bedroom(&name_words(&a.id))
            })
            .map(|a| a.id.clone())
            .collect();
        return if found.is_empty() {
            RoomMatch::Unknown
        } else {
            RoomMatch::Many(found)
        };
    }
    match found.len() {
        0 => RoomMatch::Unknown,
        1 => RoomMatch::One(found.remove(0)),
        _ => RoomMatch::Many(found),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;

    fn r(words: &[&str]) -> RoomMatch {
        let f = fixture::house();
        let words: Vec<String> = words.iter().map(|w| w.to_string()).collect();
        resolve_room(&f.house(), &words)
    }

    #[test]
    fn exact_name_beats_substring() {
        assert_eq!(r(&["salon"]), RoomMatch::One("salon".into()));
        assert_eq!(r(&["salon", "tv"]), RoomMatch::One("salon_tv".into()));
        assert_eq!(r(&["tv", "salon"]), RoomMatch::One("salon_tv".into()));
    }

    #[test]
    fn person_room() {
        assert_eq!(r(&["lea"]), RoomMatch::One("chambre_lea".into()));
        assert_eq!(r(&["chambre", "lea"]), RoomMatch::One("chambre_lea".into()));
    }

    #[test]
    fn bare_chambre_is_ambiguous() {
        assert_eq!(
            r(&["chambre"]),
            RoomMatch::Many(vec!["chambre_lea".into(), "chambre_tom".into()])
        );
    }

    #[test]
    fn no_room_words() {
        assert_eq!(r(&[]), RoomMatch::NoRoomSaid);
    }

    #[test]
    fn unknown_room() {
        assert_eq!(r(&["cuisine"]), RoomMatch::Unknown);
        assert_eq!(r(&["chambre", "cuisine"]), RoomMatch::Unknown);
    }

    #[test]
    fn accents_folded() {
        assert_eq!(r(&["léa"]), r(&["lea"]));
    }

    fn person(words: &[&str]) -> RoomMatch {
        let f = fixture::person_rooms();
        let words: Vec<String> = words.iter().map(|w| w.to_string()).collect();
        resolve_room(&f.house(), &words)
    }

    #[test]
    fn chambre_may_be_left_out_of_a_person_named_room() {
        assert_eq!(
            person(&["chambre", "de", "lea"]),
            RoomMatch::One("lea".into())
        );
        assert_eq!(person(&["lea"]), RoomMatch::One("lea".into()));
        assert_eq!(person(&["chambre", "cuisine"]), RoomMatch::Unknown);
    }

    #[test]
    fn chambre_never_names_a_room_that_is_something_else() {
        assert_eq!(person(&["chambre", "du", "salon"]), RoomMatch::Unknown);
        assert_eq!(person(&["chambre", "garage"]), RoomMatch::Unknown);
    }

    #[test]
    fn a_bare_chambre_offers_only_the_rooms_that_may_be_bedrooms() {
        assert_eq!(
            person(&["la", "chambre"]),
            RoomMatch::Many(vec!["lea".into(), "tom".into()])
        );
        let mut f = fixture::person_rooms();
        f.areas.push(fixture::area(UNASSIGNED_AREA, "Unassigned"));
        let words = vec!["chambre".to_string()];
        assert_eq!(
            resolve_room(&f.house(), &words),
            RoomMatch::Many(vec!["lea".into(), "tom".into()])
        );
    }

    #[test]
    fn area_id_counts() {
        assert_eq!(r(&["chambre_tom"]), RoomMatch::One("chambre_tom".into()));
    }
}
