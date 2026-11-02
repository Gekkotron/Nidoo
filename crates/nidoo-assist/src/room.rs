//! Which room the words name, from the house's areas only.

use nidoo_api::search::fold;

use crate::house::House;
use crate::lexicon::{Class, class};
use crate::text::{Word, words};

/// The area the room words name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoomMatch {
    One(String),
    /// Equally good areas, in the house's order.
    Many(Vec<String>),
    NoRoomSaid,
    Unknown,
}

/// The significant folded words of a name: linkers and noise dropped ("Chambre de Léa" →
/// `chambre`, `lea`).
pub fn name_words(name: &str) -> Vec<String> {
    words(&name.replace('_', " "))
        .into_iter()
        .filter_map(|w| match w {
            Word::Text(t) => Some(t),
            _ => None,
        })
        .filter(|t| !matches!(class(t), Some(Class::Linker | Class::Noise)))
        .collect()
}

/// How well `said` names an area whose words are `name`: 2 when they are the same words, 1
/// when every said word is one of the name's, 0 otherwise.
fn score(said: &[String], name: &[String]) -> u8 {
    if !said.iter().all(|w| name.contains(w)) {
        return 0;
    }
    if name.iter().all(|w| said.contains(w)) {
        2
    } else {
        1
    }
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

    #[test]
    fn area_id_counts() {
        assert_eq!(r(&["chambre_tom"]), RoomMatch::One("chambre_tom".into()));
    }
}
