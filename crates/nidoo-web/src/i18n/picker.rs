//! FR/EN text: the searchable entity picker.

use super::*;

/// Fixed words of the picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerText {
    /// The search box placeholder.
    Search,
    /// The chip that widens the list to everything the field accepts.
    All,
    /// The chip that lists diagnostic entities too.
    Diagnostic,
    /// The room column's first entry.
    AllRooms,
    /// The section of entities outside any room.
    NoRoom,
    /// The empty state when nothing matches.
    NoMatch,
    /// The empty state's button that drops the room and chip narrowing.
    SearchAll,
    /// Marks a chosen entity that is offline.
    Unavailable,
    /// Chip on a chosen entity the house no longer lists, after the raw id.
    Missing,
    Close,
    /// A list chip's remove label.
    Remove,
    /// The trigger's text when nothing is chosen.
    Choose,
    /// Suggested-chip labels.
    Thermometers,
    Presence,
    OnOff,
    /// The flow command field's suggestion: entities a flow can send a command to.
    Commandable,
}

/// The text of a fixed word.
pub fn picker_text(lang: Lang, k: PickerText) -> &'static str {
    use PickerText as T;
    let (en, fr) = match k {
        T::Search => (
            "Search a device, a room, a type…",
            "Rechercher un appareil, une pièce, un type…",
        ),
        T::All => ("All", "Tout"),
        T::Diagnostic => ("+ Diagnostic", "+ Diagnostic"),
        T::AllRooms => ("All rooms", "Toutes les pièces"),
        T::NoRoom => ("No room", "Sans pièce"),
        T::NoMatch => ("Nothing matches.", "Aucun résultat."),
        T::SearchAll => ("Search all entities", "Chercher dans toutes les entités"),
        T::Unavailable => ("Unavailable", "Indisponible"),
        T::Missing => ("missing", "introuvable"),
        T::Close => ("Close", "Fermer"),
        T::Remove => ("Remove", "Retirer"),
        T::Choose => ("Choose an entity", "Choisir une entité"),
        T::Thermometers => ("Thermometers", "Thermomètres"),
        T::Presence => ("Presence", "Présence"),
        T::OnOff => ("On / off", "Marche / arrêt"),
        T::Commandable => ("Controllable", "Pilotables"),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// The picker's confirm button: "Done (3)".
pub fn done_label(lang: Lang, n: usize) -> String {
    match lang {
        Lang::En => format!("Done ({n})"),
        Lang::Fr => format!("Terminé ({n})"),
    }
}

/// The cap notice: "200 of 812 shown, type to narrow".
pub fn capped_label(lang: Lang, shown: usize, total: usize) -> String {
    match lang {
        Lang::En => format!("{shown} of {total} shown, type to narrow"),
        Lang::Fr => format!("{shown} sur {total} affichés, tapez pour affiner"),
    }
}

/// A translator for [`PickerText`] bound to the language in context (reactive).
pub fn use_picker_text() -> impl Fn(PickerText) -> &'static str + Copy {
    let lang = use_lang();
    move |k| picker_text(lang(), k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use PickerText as T;

    #[test]
    fn table_complete() {
        let all = [
            T::Search,
            T::All,
            T::Diagnostic,
            T::AllRooms,
            T::NoRoom,
            T::NoMatch,
            T::SearchAll,
            T::Unavailable,
            T::Missing,
            T::Close,
            T::Remove,
            T::Choose,
            T::Thermometers,
            T::Presence,
            T::OnOff,
            T::Commandable,
        ];
        assert_table(&all, picker_text, &[T::Diagnostic]);
    }

    #[test]
    fn done_and_capped_labels() {
        assert_eq!(done_label(Lang::En, 3), "Done (3)");
        assert_eq!(done_label(Lang::Fr, 3), "Terminé (3)");
        assert_eq!(
            capped_label(Lang::En, 200, 812),
            "200 of 812 shown, type to narrow"
        );
        assert_eq!(
            capped_label(Lang::Fr, 200, 812),
            "200 sur 812 affichés, tapez pour affiner"
        );
    }
}
