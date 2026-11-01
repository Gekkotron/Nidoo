//! FR/EN text: the hand-made groups (list, sheet and wizard).

use super::*;
use nidoo_core::model::Kind;

const NBSP: char = '\u{a0}';

/// Fixed words of the groups area. Sentences with values are the helper functions below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupText {
    /// The list's empty state.
    NoGroupsYet,
    /// Sheet: the member list heading.
    Members,
    ShowInFavorites,
    HideMembers,
    DeleteGroup,
    GroupGone,
    CouldntLoad,
    SaveFailed,
    DeleteFailed,
    /// Chip on a member that is not of the group's kind.
    Mismatch,
    /// A member the house no longer knows.
    MemberNotFound,
    NoFlows,
    /// Wizard.
    Loading,
    StepType,
    StepMembers,
    StepNameRoom,
    Progress,
    GroupName,
    GroupNotFound,
    Back,
    Next,
    SaveGroup,
    CreateGroup,
}

/// The text of a fixed word.
pub fn group_text(lang: Lang, k: GroupText) -> &'static str {
    use GroupText as T;
    let (en, fr) = match k {
        T::NoGroupsYet => (
            "No groups yet. Create one to control several entities together.",
            "Aucun groupe pour l'instant. Créez-en un pour piloter plusieurs entités ensemble.",
        ),
        T::Members => ("Members", "Membres"),
        T::ShowInFavorites => ("Show in Favorites", "Afficher dans les favoris"),
        T::HideMembers => ("Hide members on Home", "Masquer les membres sur l'accueil"),
        T::DeleteGroup => ("Delete group", "Supprimer le groupe"),
        T::GroupGone => ("This group is gone.", "Ce groupe n'existe plus."),
        T::CouldntLoad => ("Couldn't load the group", "Impossible de charger le groupe"),
        T::SaveFailed => (
            "Couldn't save the group",
            "Impossible d'enregistrer le groupe",
        ),
        T::DeleteFailed => (
            "Couldn't delete the group",
            "Impossible de supprimer le groupe",
        ),
        T::Mismatch => ("Mismatch", "Incompatible"),
        T::MemberNotFound => ("Not found", "Introuvable"),
        T::NoFlows => (
            "This group is used by no flows.",
            "Ce groupe n'est utilisé par aucune automatisation.",
        ),
        T::Loading => ("Loading…", "Chargement…"),
        T::StepType => ("Type", "Type"),
        T::StepMembers => ("Members", "Membres"),
        T::StepNameRoom => ("Name and room", "Nom et pièce"),
        T::Progress => ("Progress", "Progression"),
        T::GroupName => ("Group name", "Nom du groupe"),
        T::GroupNotFound => ("Group not found", "Groupe introuvable"),
        T::Back => ("Back", "Retour"),
        T::Next => ("Next", "Suivant"),
        T::SaveGroup => ("Save group", "Enregistrer le groupe"),
        T::CreateGroup => ("Create group", "Créer le groupe"),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// `k` as a signal that follows the language (for props that take a `Signal<String>`). Call it
/// where `use_lang` works: in a component body.
pub fn group_text_signal(k: GroupText) -> Signal<String> {
    let lang = use_lang();
    Signal::derive(move || group_text(lang(), k).to_owned())
}

/// A group kind in lower case: "light" / "lumière".
pub fn kind_name(lang: Lang, kind: Kind) -> String {
    states::kind_word(lang, kind).to_lowercase()
}

/// A group kind as a standalone label, in sentence case: "Light" / "Lumière".
pub fn kind_label(lang: Lang, kind: Kind) -> String {
    let name = kind_name(lang, kind);
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => name,
    }
}

/// "3 lights" / "3 lumières", "1 light" / "1 lumière", "0 lumière".
pub fn kind_count(lang: Lang, kind: Kind, n: usize) -> String {
    let one = kind_name(lang, kind);
    let many = match (lang, kind) {
        (Lang::En, Kind::Switch) => "switches".to_owned(),
        (Lang::Fr, Kind::WaterHeater) => one.clone(),
        _ => format!("{one}s"),
    };
    // `count` takes (en, fr) pairs: both slots carry the form of the language asked for.
    format::count(lang, n, (&one, &one), (&many, &many))
}

/// "living · 3 lights", the line under the group's name.
pub fn subtitle(lang: Lang, room: &str, kind: Kind, n: usize) -> String {
    format!("{room} · {}", kind_count(lang, kind, n))
}

/// "3 devices" / "3 appareils": the member count of a group row.
pub fn members_count(lang: Lang, n: usize) -> String {
    format::count(lang, n, ("device", "appareil"), ("devices", "appareils"))
}

/// "2 groups" / "2 groupes".
pub fn groups_count(lang: Lang, n: usize) -> String {
    format::count(lang, n, ("group", "groupe"), ("groups", "groupes"))
}

/// "Step 2 of 3" / "Étape 2 sur 3".
pub fn step_of(lang: Lang, step: u8, total: u8) -> String {
    match lang {
        Lang::En => format!("Step {step} of {total}"),
        Lang::Fr => format!("Étape {step} sur {total}"),
    }
}

/// "Delete Salon?" / "Supprimer Salon ?".
pub fn delete_question(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("Delete {name}?"),
        Lang::Fr => format!("Supprimer {name}{NBSP}?"),
    }
}

/// The paragraph of the delete confirmation: the flows that would be left with an unknown entity.
/// The flow names are shown as they are.
pub fn delete_message(lang: Lang, flows: &[String]) -> String {
    if flows.is_empty() {
        return group_text(lang, GroupText::NoFlows).to_owned();
    }
    let list = flows.join(", ");
    match lang {
        Lang::En => format!(
            "This group is used by the following flows: {list}. Deleting it will leave them with an unknown entity."
        ),
        Lang::Fr => format!(
            "Ce groupe est utilisé par les automatisations suivantes{NBSP}: {list}. Le supprimer leur laissera une entité inconnue."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: &[GroupText] = &[
        GroupText::NoGroupsYet,
        GroupText::Members,
        GroupText::ShowInFavorites,
        GroupText::HideMembers,
        GroupText::DeleteGroup,
        GroupText::GroupGone,
        GroupText::CouldntLoad,
        GroupText::SaveFailed,
        GroupText::DeleteFailed,
        GroupText::Mismatch,
        GroupText::MemberNotFound,
        GroupText::NoFlows,
        GroupText::Loading,
        GroupText::StepType,
        GroupText::StepMembers,
        GroupText::StepNameRoom,
        GroupText::Progress,
        GroupText::GroupName,
        GroupText::GroupNotFound,
        GroupText::Back,
        GroupText::Next,
        GroupText::SaveGroup,
        GroupText::CreateGroup,
    ];
    const SAME: &[GroupText] = &[GroupText::StepType];

    #[test]
    fn every_group_text_has_two_languages() {
        assert_table(ALL, group_text, SAME);
    }

    #[test]
    fn member_counts_agree_at_0_1_and_2() {
        assert_eq!(members_count(Lang::Fr, 0), "0 appareil");
        assert_eq!(members_count(Lang::Fr, 1), "1 appareil");
        assert_eq!(members_count(Lang::Fr, 3), "3 appareils");
        assert_eq!(members_count(Lang::En, 0), "0 devices");
        assert_eq!(members_count(Lang::En, 1), "1 device");
        assert_eq!(groups_count(Lang::Fr, 0), "0 groupe");
        assert_eq!(groups_count(Lang::Fr, 2), "2 groupes");
    }

    #[test]
    fn kind_counts_pluralise_per_language() {
        assert_eq!(kind_count(Lang::En, Kind::Light, 3), "3 lights");
        assert_eq!(kind_count(Lang::En, Kind::Light, 1), "1 light");
        assert_eq!(kind_count(Lang::En, Kind::Switch, 2), "2 switches");
        assert_eq!(
            kind_count(Lang::En, Kind::WaterHeater, 2),
            "2 water heaters"
        );
        assert_eq!(kind_count(Lang::Fr, Kind::Light, 3), "3 lumières");
        assert_eq!(kind_count(Lang::Fr, Kind::Light, 0), "0 lumière");
        assert_eq!(kind_count(Lang::Fr, Kind::Cover, 2), "2 volets");
        assert_eq!(kind_count(Lang::Fr, Kind::WaterHeater, 2), "2 chauffe-eau");
        assert_eq!(kind_name(Lang::Fr, Kind::WaterHeater), "chauffe-eau");
        assert_eq!(kind_label(Lang::Fr, Kind::Light), "Lumière");
        assert_eq!(kind_label(Lang::Fr, Kind::WaterHeater), "Chauffe-eau");
        assert_eq!(kind_label(Lang::En, Kind::Light), "Light");
    }

    #[test]
    fn sentences_follow_french_typography() {
        assert_eq!(
            subtitle(Lang::Fr, "salon", Kind::Light, 1),
            "salon · 1 lumière"
        );
        assert_eq!(step_of(Lang::Fr, 2, 3), "Étape 2 sur 3");
        assert_eq!(step_of(Lang::En, 2, 3), "Step 2 of 3");
        assert_eq!(delete_question(Lang::Fr, "Salon"), "Supprimer Salon\u{a0}?");
        assert_eq!(delete_question(Lang::En, "Salon"), "Delete Salon?");
        let msg = delete_message(Lang::Fr, &["a".to_owned(), "b".to_owned()]);
        assert!(msg.contains("suivantes\u{a0}: a, b."), "{msg}");
        assert_eq!(
            delete_message(Lang::En, &[]),
            "This group is used by no flows."
        );
    }
}
