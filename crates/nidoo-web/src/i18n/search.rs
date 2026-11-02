//! FR/EN text: the search page.

use super::*;

/// A fixed word or sentence of the search page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchText {
    /// The top bar's magnifying glass.
    Open,
    Placeholder,
    Hint,
    Clear,
    Rooms,
}

/// A fixed word or sentence of the search page.
pub fn search_text(lang: Lang, k: SearchText) -> &'static str {
    use SearchText::*;
    let (en, fr) = match k {
        Open => ("Search", "Rechercher"),
        Placeholder => (
            "Search a device, a room…",
            "Chercher un appareil, une pièce…",
        ),
        Hint => (
            "Type a name, a room or a kind of device",
            "Tapez un nom, une pièce ou un type d'appareil",
        ),
        Clear => ("Clear", "Effacer"),
        Rooms => ("Rooms", "Pièces"),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// "3 devices · 2 rooms" / "3 appareils · 2 pièces": singular at 1, the room part left out at 0.
pub fn search_count(lang: Lang, devices: usize, rooms: usize) -> String {
    let (device, room) = match lang {
        Lang::En => ("device", "room"),
        Lang::Fr => ("appareil", "pièce"),
    };
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    let mut out = plural(devices, device);
    if rooms > 0 {
        out.push_str(" · ");
        out.push_str(&plural(rooms, room));
    }
    out
}

/// "Nothing matches “lamp”" / "Rien ne correspond à « lamp »".
pub fn search_no_match(lang: Lang, query: &str) -> String {
    match lang {
        Lang::En => format!("Nothing matches “{query}”"),
        Lang::Fr => format!("Rien ne correspond à «\u{a0}{query}\u{a0}»"),
    }
}

/// "Showing 60 of 70 — type more to narrow it down".
pub fn search_capped(lang: Lang, shown: usize, total: usize) -> String {
    match lang {
        Lang::En => format!("Showing {shown} of {total} — type more to narrow it down"),
        Lang::Fr => format!("{shown} sur {total} affichés — précisez la recherche"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_text_has_both_languages() {
        use SearchText::*;
        assert_table(&[Open, Placeholder, Hint, Clear, Rooms], search_text, &[]);
    }

    #[test]
    fn the_count_follows_each_number() {
        assert_eq!(search_count(Lang::En, 1, 0), "1 device");
        assert_eq!(search_count(Lang::En, 3, 1), "3 devices · 1 room");
        assert_eq!(search_count(Lang::En, 0, 2), "0 devices · 2 rooms");
        assert_eq!(search_count(Lang::Fr, 1, 1), "1 appareil · 1 pièce");
        assert_eq!(search_count(Lang::Fr, 12, 2), "12 appareils · 2 pièces");
    }

    #[test]
    fn the_sentences_carry_their_values() {
        assert_eq!(search_no_match(Lang::En, "zzz"), "Nothing matches “zzz”");
        assert_eq!(
            search_capped(Lang::En, 60, 70),
            "Showing 60 of 70 — type more to narrow it down"
        );
        assert_eq!(
            search_capped(Lang::Fr, 60, 70),
            "60 sur 70 affichés — précisez la recherche"
        );
    }
}
