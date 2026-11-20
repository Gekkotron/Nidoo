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
    PageTitle,
    PageLede,
    AddAlarm,
    Disabled,
    DisabledBody,
    OpenSettings,
    NoPinsonBody,
    Unreadable,
    Delete,
    NotFound,
    BackToList,
    Name,
    Hour,
    Minute,
    Music,
    Speakers,
    MutedNote,
    Rise,
    RiseHint,
    PickDay,
    NoDayNext,
    Save,
    OnLabel,
    OffLabel,
    AlarmOn,
    Volume,
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
        WakeText::PageTitle,
        WakeText::PageLede,
        WakeText::AddAlarm,
        WakeText::Disabled,
        WakeText::DisabledBody,
        WakeText::OpenSettings,
        WakeText::NoPinsonBody,
        WakeText::Unreadable,
        WakeText::Delete,
        WakeText::NotFound,
        WakeText::BackToList,
        WakeText::Name,
        WakeText::Hour,
        WakeText::Minute,
        WakeText::Music,
        WakeText::Speakers,
        WakeText::MutedNote,
        WakeText::Rise,
        WakeText::RiseHint,
        WakeText::PickDay,
        WakeText::NoDayNext,
        WakeText::Save,
        WakeText::OnLabel,
        WakeText::OffLabel,
        WakeText::AlarmOn,
        WakeText::Volume,
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
        T::PageTitle => ("Wake-up", "Réveil"),
        T::PageLede => (
            "Wake up to music: Pinson plays a playlist on the speakers, raising the volume slowly.",
            "Se réveiller en musique\u{a0}: Pinson lance une playlist sur les enceintes, en montant doucement le son.",
        ),
        T::AddAlarm => ("Add an alarm", "Ajouter un réveil"),
        T::Disabled => ("Wake-up is off", "Le réveil est désactivé"),
        T::DisabledBody => (
            "Turn it on in Settings \u{2192} Home \u{2192} Wake-up to set a music alarm.",
            "Activez-le dans Réglages \u{2192} Maison \u{2192} Réveil pour régler une heure de réveil en musique.",
        ),
        T::OpenSettings => ("Open settings", "Ouvrir les réglages"),
        T::NoPinsonBody => (
            "No Pinson player is connected. Connect Pinson to the home MQTT broker (Settings \u{2192} Media players), then come back to set an alarm.",
            "Aucun lecteur Pinson n\u{2019}est connecté. Branchez Pinson au broker MQTT de la maison (Réglages \u{2192} Lecteurs multimédias), puis revenez ici pour régler un réveil.",
        ),
        T::Unreadable => ("Unreadable alarm", "Réveil illisible"),
        T::Delete => ("Delete", "Supprimer"),
        T::NotFound => (
            "This alarm no longer exists.",
            "Ce réveil n\u{2019}existe plus.",
        ),
        T::BackToList => ("Back to Wake-up", "Retour au réveil"),
        T::Name => ("Name", "Nom"),
        T::Hour => ("Hour", "Heure"),
        T::Minute => ("Minute", "Minute"),
        T::Music => ("Music", "Musique"),
        T::Speakers => ("Speakers", "Enceintes"),
        T::MutedNote => (
            "muted during the alarm (and after)",
            "coupée pendant le réveil (et après)",
        ),
        T::Rise => ("Fade-in", "Montée"),
        T::RiseHint => (
            "The sound rises from silence to the set volume.",
            "Le son part du silence jusqu\u{2019}au volume réglé.",
        ),
        T::PickDay => ("Pick at least one day.", "Choisissez au moins un jour."),
        T::NoDayNext => (
            "No day picked: this alarm will not ring.",
            "Aucun jour choisi\u{a0}: ce réveil ne sonnera pas.",
        ),
        T::Save => ("Save", "Enregistrer"),
        T::OnLabel => ("On", "Activé"),
        T::OffLabel => ("Off", "Désactivé"),
        T::AlarmOn => ("Alarm on", "Réveil activé"),
        T::Volume => ("Volume", "Volume"),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// "Next alarm: Nov 23 2026, 07:00" (French puts a no-break space before the colon).
pub fn next_alarm(lang: Lang, when: &str) -> String {
    match lang {
        Lang::En => format!("Next alarm: {when}"),
        Lang::Fr => format!("Prochain réveil{NBSP}: {when}"),
    }
}

/// "Delete Wake-up 07:00?" / « Supprimer Réveil 07:00 ? »: asked before an alarm is deleted.
pub fn delete_question(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("Delete {name}?"),
        Lang::Fr => format!("Supprimer {name}{NBSP}?"),
    }
}

/// The note under an unreadable alarm: its flow id and what can still be done.
pub fn unreadable_body(lang: Lang, id: &str) -> String {
    match lang {
        Lang::En => format!(
            "The settings of \u{201c}{id}\u{201d} can no longer be read (edited by hand?). It can only be deleted."
        ),
        Lang::Fr => format!(
            "Les réglages de \u{ab}{NBSP}{id}{NBSP}\u{bb} ne se lisent plus (modifiés à la main{NBSP}?). Il ne peut être que supprimé."
        ),
    }
}

/// A slider value: "25 %" (French puts a no-break space before the sign).
pub fn percent(lang: Lang, v: f64) -> String {
    match lang {
        Lang::En => format!("{} %", v.round()),
        Lang::Fr => format!("{}{NBSP}%", v.round()),
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
    const SAME: &[WakeText] = &[
        WakeText::Pinson,
        WakeText::Playlist,
        WakeText::Minute,
        WakeText::Volume,
    ];

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
    fn delete_question_reads_both_ways() {
        assert_eq!(
            delete_question(Lang::En, "Wake-up 07:00"),
            "Delete Wake-up 07:00?"
        );
        assert_eq!(
            delete_question(Lang::Fr, "Réveil 07:00"),
            format!("Supprimer Réveil 07:00{NBSP}?")
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
