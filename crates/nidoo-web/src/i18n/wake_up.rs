//! FR/EN text: the Wake-up feature (its Settings page, the play-a-playlist command card and the
//! markers on the flows it manages).

use super::Lang;

const NBSP: char = '\u{a0}';

/// The fixed words of the Wake-up feature. Sentences with values are the helper functions below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WakeText {
    Lede,
    Enable,
    EnableHint,
    Cancel,
    TurnOff,
    Pinson,
    NoPinson,
    OpenPage,
    ManagedBy,
    OpenManaged,
    CanvasBanner,
    CanvasBannerBody,
    EditInWakeUp,
    SaveAnyway,
    Playlist,
    Shuffle,
    Output,
    FadeIn,
    CurrentOutput,
    UnknownPlaylistHint,
    FadeOff,
}

impl WakeText {
    pub const ALL: &'static [WakeText] = &[
        WakeText::Lede,
        WakeText::Enable,
        WakeText::EnableHint,
        WakeText::Cancel,
        WakeText::TurnOff,
        WakeText::Pinson,
        WakeText::NoPinson,
        WakeText::OpenPage,
        WakeText::ManagedBy,
        WakeText::OpenManaged,
        WakeText::CanvasBanner,
        WakeText::CanvasBannerBody,
        WakeText::EditInWakeUp,
        WakeText::SaveAnyway,
        WakeText::Playlist,
        WakeText::Shuffle,
        WakeText::Output,
        WakeText::FadeIn,
        WakeText::CurrentOutput,
        WakeText::UnknownPlaylistHint,
        WakeText::FadeOff,
    ];
}

/// The text of `k` in `lang`.
pub fn wake_text(lang: Lang, k: WakeText) -> &'static str {
    use WakeText as T;
    let (en, fr) = match k {
        T::Lede => (
            "Wake up to music with Pinson, at a fixed time, on the speakers of the house.",
            "Se réveiller en musique avec Pinson, à heure fixe, sur les enceintes de la maison.",
        ),
        T::Enable => ("Turn on Wake-up", "Activer le réveil"),
        T::EnableHint => (
            "Adds \u{201c}Wake-up\u{201d} to the Home menu. Off by default.",
            "Ajoute \u{ab}\u{a0}Réveil\u{a0}\u{bb} au menu Maison. Désactivé par défaut.",
        ),
        T::Cancel => ("Cancel", "Annuler"),
        T::TurnOff => ("Turn off", "Désactiver"),
        T::Pinson => ("Pinson", "Pinson"),
        T::NoPinson => ("Music needs Pinson", "La musique a besoin de Pinson"),
        T::OpenPage => ("Open the Wake-up page", "Ouvrir la page Réveil"),
        T::ManagedBy => ("Managed by Wake-up", "Géré par Réveil"),
        T::OpenManaged => ("open", "ouvrir"),
        T::CanvasBanner => ("Managed by Wake-up.", "Géré par Réveil."),
        T::CanvasBannerBody => (
            "This flow is rewritten each time you save from the Wake-up page: your changes here would be lost.",
            "Ce flux est réécrit à chaque enregistrement depuis la page Réveil\u{a0}: vos modifications ici seraient perdues.",
        ),
        T::EditInWakeUp => ("Edit in Wake-up", "Modifier dans Réveil"),
        T::SaveAnyway => ("Save anyway", "Enregistrer quand même"),
        T::Playlist => ("Playlist", "Playlist"),
        T::Shuffle => ("Shuffle", "Aléatoire"),
        T::Output => ("Output", "Sortie"),
        T::FadeIn => ("Fade-in", "Montée du son"),
        T::CurrentOutput => ("Current output", "Sortie actuelle"),
        T::UnknownPlaylistHint => (
            "This playlist no longer exists in Pinson: when it rings, Pinson will play Favorites.",
            "Cette playlist n'existe plus dans Pinson\u{a0}: au moment voulu, Pinson jouera les Favoris.",
        ),
        T::FadeOff => ("Off", "Sans"),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// The saved playlist the player no longer lists: "unknown playlist (x7f)".
pub fn unknown_playlist(lang: Lang, id: &str) -> String {
    match lang {
        Lang::En => format!("unknown playlist ({id})"),
        Lang::Fr => format!("playlist inconnue ({id})"),
    }
}

/// The saved output the player no longer lists: "unknown output (x7f)".
pub fn unknown_output(lang: Lang, id: &str) -> String {
    match lang {
        Lang::En => format!("unknown output ({id})"),
        Lang::Fr => format!("sortie inconnue ({id})"),
    }
}

/// The fade-in length: "Off", "30 s", "1 min", "2 min", "5 min" (any other count of seconds
/// reads as seconds, or whole minutes when it is a multiple of 60).
pub fn fade_text(lang: Lang, secs: u32) -> String {
    match secs {
        0 => wake_text(lang, WakeText::FadeOff).to_owned(),
        s if s % 60 == 0 => format!("{} min", s / 60),
        s => format!("{s} s"),
    }
}

/// The confirm banner's title: "1 active alarm will be turned off." (singular and plural).
pub fn disable_title(lang: Lang, n: usize) -> String {
    match (lang, n) {
        (Lang::En, 1) => "1 active alarm will be turned off.".to_owned(),
        (Lang::En, n) => format!("{n} active alarms will be turned off."),
        (Lang::Fr, 1) => "1 réveil actif sera désactivé.".to_owned(),
        (Lang::Fr, n) => format!("{n} réveils actifs seront désactivés."),
    }
}

/// The confirm banner's body: what turning the feature off does to the alarms.
pub fn disable_body(lang: Lang, n: usize) -> &'static str {
    match (lang, n) {
        (Lang::En, 1) => {
            "It will stop ringing. If you turn Wake-up back on, your alarms stay off until you switch them on again."
        }
        (Lang::En, _) => {
            "They will stop ringing. If you turn Wake-up back on, your alarms stay off until you switch them on again."
        }
        (Lang::Fr, 1) => {
            "Il ne sonnera plus. En réactivant la fonction, vos réveils restent éteints jusqu'à ce que vous les rallumiez."
        }
        (Lang::Fr, _) => {
            "Ils ne sonneront plus. En réactivant la fonction, vos réveils restent éteints jusqu'à ce que vous les rallumiez."
        }
    }
}

/// The Pinson status line: the player's name and what it lists, or that none is connected.
pub fn pinson_status(lang: Lang, lists: Option<(usize, usize)>) -> String {
    let Some((playlists, outputs)) = lists else {
        return match lang {
            Lang::En => "No Pinson player connected".to_owned(),
            Lang::Fr => "Aucun lecteur Pinson connecté".to_owned(),
        };
    };
    match lang {
        Lang::En => format!(
            "Player \u{201c}Pinson\u{201d} connected \u{b7} {playlists} {} \u{b7} {outputs} {}",
            if playlists == 1 {
                "playlist"
            } else {
                "playlists"
            },
            if outputs == 1 { "output" } else { "outputs" },
        ),
        Lang::Fr => format!(
            "Lecteur \u{ab}{NBSP}Pinson{NBSP}\u{bb} connecté \u{b7} {playlists} playlist{} \u{b7} {outputs} sortie{}",
            if playlists > 1 { "s" } else { "" },
            if outputs > 1 { "s" } else { "" },
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Words spelled the same in both languages.
    const SAME: &[WakeText] = &[WakeText::Pinson, WakeText::Playlist];

    #[test]
    fn every_word_has_both_languages() {
        super::super::assert_table(WakeText::ALL, wake_text, SAME);
    }

    #[test]
    fn disable_banner_follows_the_count() {
        assert_eq!(disable_title(Lang::Fr, 1), "1 réveil actif sera désactivé.");
        assert_eq!(
            disable_title(Lang::Fr, 2),
            "2 réveils actifs seront désactivés."
        );
        assert_eq!(
            disable_title(Lang::En, 1),
            "1 active alarm will be turned off."
        );
        assert_eq!(
            disable_title(Lang::En, 3),
            "3 active alarms will be turned off."
        );
    }

    #[test]
    fn pinson_status_reads_both_ways() {
        assert_eq!(
            pinson_status(Lang::Fr, Some((4, 4))),
            "Lecteur \u{ab}\u{a0}Pinson\u{a0}\u{bb} connecté \u{b7} 4 playlists \u{b7} 4 sorties"
        );
        assert_eq!(pinson_status(Lang::En, None), "No Pinson player connected");
    }
}
