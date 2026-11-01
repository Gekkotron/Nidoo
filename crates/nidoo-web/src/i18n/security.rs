//! FR/EN text: the Security screen, the alarm, the alert list and the alert detail.

use chrono::NaiveDate;
use nidoo_api::AlertChannel;
use nidoo_core::model::{SensorGroup, SensorSection};

use super::states::{StateCtx, state_word};
use super::{Lang, format};

const NBSP: char = '\u{A0}';

/// Fixed words of the security and alerts area. Sentences with values are the helpers below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecurityText {
    // The alarm panel's state words come from `states::state_word`; only this one is local.
    Unavailable,
    // Arming modes and the alarm's controls.
    ModeHome,
    ModeAway,
    ModeNight,
    ModeVacation,
    ModeCustom,
    Disarm,
    Code,
    // A security sensor's state words.
    WordDetected,
    WordClear,
    WordLeak,
    WordDry,
    WordSos,
    WordOk,
    WordVibration,
    WordStill,
    WordTampered,
    // Sections of the screen.
    SectionOpenings,
    SectionMotion,
    SectionFire,
    SectionWater,
    SectionOther,
    AllClear,
    // The screen.
    NothingYet,
    NoPanelBefore,
    NoPanelAfter,
    Actions,
    NoSensors,
    Cancel,
    // What a tap on an action does.
    VerbPress,
    VerbOpen,
    VerbRing,
    VerbPlay,
    VerbTurnOn,
    VerbLock,
    VerbUnlock,
    Ringing,
    // A lock's state words.
    LockLocked,
    LockUnlocked,
    LockLocking,
    LockUnlocking,
    LockJammed,
    LockOpen,
    LockOpening,
    LockOffline,
    LockUnknown,
    // Alerts.
    JustNow,
    Resolved,
    All,
    Channels,
    NoAlerts,
    NoAlertsInChannel,
    Today,
    Yesterday,
    RecentAlerts,
    SeeAll,
    Sent,
    MediaGone,
    PlayClip,
    Play,
    OpenDevice,
    AlertGone,
    ChannelAlarm,
    ChannelSecurity,
    ChannelCamera,
    ChannelEnergy,
    ChannelClimate,
    ChannelSystem,
    ChannelMisc,
}

impl SecurityText {
    pub const ALL: &'static [SecurityText] = &[
        SecurityText::Unavailable,
        SecurityText::ModeHome,
        SecurityText::ModeAway,
        SecurityText::ModeNight,
        SecurityText::ModeVacation,
        SecurityText::ModeCustom,
        SecurityText::Disarm,
        SecurityText::Code,
        SecurityText::WordDetected,
        SecurityText::WordClear,
        SecurityText::WordLeak,
        SecurityText::WordDry,
        SecurityText::WordSos,
        SecurityText::WordOk,
        SecurityText::WordVibration,
        SecurityText::WordStill,
        SecurityText::WordTampered,
        SecurityText::SectionOpenings,
        SecurityText::SectionMotion,
        SecurityText::SectionFire,
        SecurityText::SectionWater,
        SecurityText::SectionOther,
        SecurityText::AllClear,
        SecurityText::NothingYet,
        SecurityText::NoPanelBefore,
        SecurityText::NoPanelAfter,
        SecurityText::Actions,
        SecurityText::NoSensors,
        SecurityText::Cancel,
        SecurityText::VerbPress,
        SecurityText::VerbOpen,
        SecurityText::VerbRing,
        SecurityText::VerbPlay,
        SecurityText::VerbTurnOn,
        SecurityText::VerbLock,
        SecurityText::VerbUnlock,
        SecurityText::Ringing,
        SecurityText::LockLocked,
        SecurityText::LockUnlocked,
        SecurityText::LockLocking,
        SecurityText::LockUnlocking,
        SecurityText::LockJammed,
        SecurityText::LockOpen,
        SecurityText::LockOpening,
        SecurityText::LockOffline,
        SecurityText::LockUnknown,
        SecurityText::JustNow,
        SecurityText::Resolved,
        SecurityText::All,
        SecurityText::Channels,
        SecurityText::NoAlerts,
        SecurityText::NoAlertsInChannel,
        SecurityText::Today,
        SecurityText::Yesterday,
        SecurityText::RecentAlerts,
        SecurityText::SeeAll,
        SecurityText::Sent,
        SecurityText::MediaGone,
        SecurityText::PlayClip,
        SecurityText::Play,
        SecurityText::OpenDevice,
        SecurityText::AlertGone,
        SecurityText::ChannelAlarm,
        SecurityText::ChannelSecurity,
        SecurityText::ChannelCamera,
        SecurityText::ChannelEnergy,
        SecurityText::ChannelClimate,
        SecurityText::ChannelSystem,
        SecurityText::ChannelMisc,
    ];
}

/// The `(en, fr)` pair of a fixed word, in the requested language.
pub fn security_text(lang: Lang, k: SecurityText) -> &'static str {
    use SecurityText as T;
    let (en, fr) = match k {
        T::Unavailable => ("Unavailable", "Indisponible"),
        T::ModeHome => ("Home", "Présent"),
        T::ModeAway => ("Away", "Absent"),
        T::ModeNight => ("Night", "Nuit"),
        T::ModeVacation => ("Vacation", "Vacances"),
        T::ModeCustom => ("Custom bypass", "Personnalisé"),
        T::Disarm => ("Disarm", "Désactiver"),
        T::Code => ("Code", "Code"),
        T::WordDetected => ("Detected", "Détecté"),
        T::WordClear => ("Clear", "Normal"),
        T::WordLeak => ("Leak", "Fuite"),
        T::WordDry => ("Dry", "Sec"),
        T::WordSos => ("SOS", "SOS"),
        T::WordOk => ("OK", "OK"),
        T::WordVibration => ("Vibration", "Vibration"),
        T::WordStill => ("Still", "Immobile"),
        T::WordTampered => ("Tampered", "Sabotage"),
        T::SectionOpenings => ("Openings", "Ouvertures"),
        T::SectionMotion => ("Motion", "Mouvement"),
        T::SectionFire => ("Fire", "Incendie"),
        T::SectionWater => ("Water", "Eau"),
        T::SectionOther => ("Other", "Autres"),
        T::AllClear => ("All clear", "Tout est calme"),
        T::NothingYet => (
            "Nothing on Security yet.",
            "Rien dans Sécurité pour l'instant.",
        ),
        T::NoPanelBefore => (
            "No alarm panel. Publish an MQTT ",
            "Aucune centrale d'alarme. Publiez un ",
        ),
        T::NoPanelAfter => (" to see it here.", " MQTT pour la voir ici."),
        T::Actions => ("Actions", "Actions"),
        T::NoSensors => ("No security sensors.", "Aucun capteur de sécurité."),
        T::Cancel => ("Cancel", "Annuler"),
        T::VerbPress => ("Press", "Appuyer"),
        T::VerbOpen => ("Open", "Ouvrir"),
        T::VerbRing => ("Ring", "Faire sonner"),
        T::VerbPlay => ("Play", "Sonner"),
        T::VerbTurnOn => ("Turn on", "Activer"),
        T::VerbLock => ("Lock", "Verrouiller"),
        T::VerbUnlock => ("Unlock", "Déverrouiller"),
        T::Ringing => ("Ringing", "Sonnerie en cours"),
        T::LockLocked => ("Locked", "Verrouillé"),
        T::LockUnlocked => ("Unlocked", "Déverrouillé"),
        T::LockLocking => ("Locking…", "Verrouillage…"),
        T::LockUnlocking => ("Unlocking…", "Déverrouillage…"),
        T::LockJammed => ("Jammed", "Bloqué"),
        T::LockOpen => ("Open", "Ouvert"),
        T::LockOpening => ("Opening…", "Ouverture…"),
        T::LockOffline => ("Offline", "Hors ligne"),
        T::LockUnknown => ("Unknown", "Inconnu"),
        T::JustNow => ("Just now", "À l'instant"),
        T::Resolved => ("Resolved", "Résolue"),
        T::All => ("All", "Tous"),
        T::Channels => ("Channels", "Canaux"),
        T::NoAlerts => ("No alerts yet", "Aucune alerte pour l'instant"),
        T::NoAlertsInChannel => ("No alerts in this channel", "Aucune alerte dans ce canal"),
        T::Today => ("Today", "Aujourd'hui"),
        T::Yesterday => ("Yesterday", "Hier"),
        T::RecentAlerts => ("Recent alerts", "Alertes récentes"),
        T::SeeAll => ("See all", "Tout voir"),
        T::Sent => ("Sent", "Envoyé"),
        T::MediaGone => (
            "Media no longer available",
            "Ce média n'est plus disponible",
        ),
        T::PlayClip => ("Play clip", "Lire la séquence"),
        T::Play => ("Play", "Lire"),
        T::OpenDevice => ("Open device", "Ouvrir l'appareil"),
        T::AlertGone => (
            "This alert is no longer available",
            "Cette alerte n'est plus disponible",
        ),
        T::ChannelAlarm => ("Alarm", "Alarme"),
        T::ChannelSecurity => ("Security", "Sécurité"),
        T::ChannelCamera => ("Camera", "Caméra"),
        T::ChannelEnergy => ("Energy", "Énergie"),
        T::ChannelClimate => ("Climate", "Climat"),
        T::ChannelSystem => ("System", "Système"),
        T::ChannelMisc => ("Misc", "Divers"),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// A lock card's state word.
pub fn lock_word(lang: Lang, w: crate::cards::lock::LockWord) -> &'static str {
    use crate::cards::lock::LockWord as W;
    let key = match w {
        W::Locked => SecurityText::LockLocked,
        W::Unlocked => SecurityText::LockUnlocked,
        W::Locking => SecurityText::LockLocking,
        W::Unlocking => SecurityText::LockUnlocking,
        W::Jammed => SecurityText::LockJammed,
        W::Open => SecurityText::LockOpen,
        W::Opening => SecurityText::LockOpening,
        W::Offline => SecurityText::LockOffline,
        W::Unknown => SecurityText::LockUnknown,
    };
    security_text(lang, key)
}

fn pick(lang: Lang, en: String, fr: String) -> String {
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// The alarm panel's state line, built on the shared alarm words (`states::state_word`), with
/// the hero's own emphasis on the transitional states: "Exit delay…", "Triggered!".
pub fn panel_state(lang: Lang, state: Option<&str>, available: bool) -> String {
    if !available {
        return security_text(lang, SecurityText::Unavailable).to_owned();
    }
    let Some(raw) = state.filter(|s| {
        *s == "disarmed"
            || s.starts_with("armed_")
            || matches!(*s, "arming" | "pending" | "disarming" | "triggered")
    }) else {
        return "—".to_owned();
    };
    let word = state_word(lang, StateCtx::Alarm, raw);
    match (raw, lang) {
        ("arming" | "pending" | "disarming", _) if !word.ends_with('…') => format!("{word}…"),
        ("triggered", Lang::En) => format!("{word}!"),
        ("triggered", Lang::Fr) => format!("{word}{NBSP}!"),
        _ => word,
    }
}

/// A security sensor's state word. Openings and motion follow the device class
/// ("Porte ouverte"); the other groups have their own wording.
pub fn sensor_state(lang: Lang, group: SensorGroup, class: Option<&str>, active: bool) -> String {
    use SecurityText as T;
    let key = match (group, active) {
        (SensorGroup::Opening | SensorGroup::Motion, _) => {
            let raw = if active { "on" } else { "off" };
            return state_word(lang, StateCtx::BinarySensor(class.map(str::to_owned)), raw);
        }
        (SensorGroup::Smoke | SensorGroup::Co | SensorGroup::Gas, true) => T::WordDetected,
        (SensorGroup::Smoke | SensorGroup::Co | SensorGroup::Gas, false) => T::WordClear,
        (SensorGroup::Leak, true) => T::WordLeak,
        (SensorGroup::Leak, false) => T::WordDry,
        (SensorGroup::Sos, true) => T::WordSos,
        (SensorGroup::Sos, false) => T::WordOk,
        (SensorGroup::Vibration, true) => T::WordVibration,
        (SensorGroup::Vibration, false) => T::WordStill,
        (SensorGroup::Tamper, true) => T::WordTampered,
        (SensorGroup::Tamper, false) => T::WordOk,
    };
    security_text(lang, key).to_owned()
}

/// A section's title.
pub fn section_title(lang: Lang, section: SensorSection) -> &'static str {
    use SecurityText as T;
    security_text(
        lang,
        match section {
            SensorSection::Openings => T::SectionOpenings,
            SensorSection::Motion => T::SectionMotion,
            SensorSection::Fire => T::SectionFire,
            SensorSection::Water => T::SectionWater,
            SensorSection::Other => T::SectionOther,
        },
    )
}

/// One group's part of the summary line: "2 open" / "2 ouverts".
pub fn group_count(lang: Lang, group: SensorGroup, n: usize) -> String {
    let (one, other) = match group {
        SensorGroup::Smoke => (("smoke", "fumée"), ("smoke", "fumées")),
        SensorGroup::Co => (("CO", "CO"), ("CO", "CO")),
        SensorGroup::Gas => (("gas", "gaz"), ("gas", "gaz")),
        SensorGroup::Leak => (("leak", "fuite"), ("leaks", "fuites")),
        SensorGroup::Sos => (("SOS", "SOS"), ("SOS", "SOS")),
        SensorGroup::Opening => (("open", "ouvert"), ("open", "ouverts")),
        SensorGroup::Motion => (("motion", "mouvement"), ("motion", "mouvements")),
        SensorGroup::Vibration => (("vibration", "vibration"), ("vibration", "vibrations")),
        SensorGroup::Tamper => (("tamper", "sabotage"), ("tamper", "sabotages")),
    };
    format::count(lang, n, one, other)
}

/// "Open Gate?" / "Ouvrir Gate ?": what a tile asks before acting. `key` is a `Verb…` word.
pub fn action_question(lang: Lang, key: SecurityText, name: &str) -> String {
    let verb = security_text(lang, key);
    match (lang, key) {
        (Lang::Fr, SecurityText::VerbPress) => format!("{verb} sur {name}{NBSP}?"),
        (Lang::Fr, _) => format!("{verb} {name}{NBSP}?"),
        (Lang::En, _) => format!("{verb} {name}?"),
    }
}

/// "Garage door, garage, open": the accessible name of a sensor tile.
pub fn sensor_label(name: &str, area: Option<&str>, state: &str) -> String {
    let state = state.to_lowercase();
    match area {
        Some(area) => format!("{name}, {}, {state}", area.to_lowercase()),
        None => format!("{name}, {state}"),
    }
}

/// "Garage door, off": the accessible name of an action tile.
pub fn action_label(name: &str, state: Option<&str>) -> String {
    match state {
        Some(s) => format!("{name}, {}", s.to_lowercase()),
        None => name.to_owned(),
    }
}

/// "yesterday 09:41" / "hier 09:41".
pub fn since_yesterday(lang: Lang, hhmm: &str) -> String {
    pick(lang, format!("yesterday {hhmm}"), format!("hier {hhmm}"))
}

pub fn since_days(lang: Lang, days: i64) -> String {
    let n = usize::try_from(days).unwrap_or_default();
    let form = format::count(lang, n, ("day", "jour"), ("days", "jours"));
    pick(lang, format!("{form} ago"), format!("il y a {form}"))
}

/// How long ago an alert was sent: "Just now", "5 min ago", "3 h ago", "2 d ago" and in French
/// "À l'instant", "il y a 5 min", "il y a 3 h", "il y a 2 j". A time in the future is "just now".
pub fn alert_age(lang: Lang, sent_at: u64, now: u64) -> String {
    let age = now.saturating_sub(sent_at);
    if age < 60 {
        return security_text(lang, SecurityText::JustNow).to_owned();
    }
    let (n, unit_en, unit_fr) = match age {
        60..3600 => (age / 60, "min", "min"),
        3600..86_400 => (age / 3600, "h", "h"),
        _ => (age / 86_400, "d", "j"),
    };
    match lang {
        Lang::En => format!("{n}{NBSP}{unit_en} ago"),
        Lang::Fr => format!("il y a {n}{NBSP}{unit_fr}"),
    }
}

/// "{age} · Resolved" once the producer cancelled the alert.
pub fn age_resolved(lang: Lang, age: &str) -> String {
    format!("{age} · {}", security_text(lang, SecurityText::Resolved))
}

/// The heading of a day group: the date, with the year when it is not `current_year`.
pub fn day_heading(lang: Lang, year: i32, month: u8, day: u8, current_year: i32) -> String {
    let Some(d) = NaiveDate::from_ymd_opt(year, u32::from(month), u32::from(day)) else {
        return String::new();
    };
    let short = format::date_short(lang, d);
    if year == current_year {
        short
    } else {
        format!("{short} {year}")
    }
}

/// The channel's name.
pub fn channel_name(lang: Lang, channel: AlertChannel) -> &'static str {
    use SecurityText as T;
    security_text(
        lang,
        match channel {
            AlertChannel::Alarm => T::ChannelAlarm,
            AlertChannel::Security => T::ChannelSecurity,
            AlertChannel::Camera => T::ChannelCamera,
            AlertChannel::Energy => T::ChannelEnergy,
            AlertChannel::Climate => T::ChannelClimate,
            AlertChannel::System => T::ChannelSystem,
            AlertChannel::Misc => T::ChannelMisc,
        },
    )
}

/// "Couldn't send Disarm" / "Impossible d'envoyer Disarm".
pub fn send_failed(lang: Lang, label: &str) -> String {
    pick(
        lang,
        format!("Couldn't send {label}"),
        format!("Impossible d'envoyer {label}"),
    )
}

/// "From frigate · Oct 19 2026, 09:41" / "Via frigate · 19 oct. 2026, 09:41".
pub fn source_line(lang: Lang, source: &str, date: &str) -> String {
    pick(
        lang,
        format!("From {source} · {date}"),
        format!("Via {source} · {date}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::assert_table;

    #[test]
    fn every_word_has_both_languages() {
        use SecurityText as T;
        const SAME: &[SecurityText] =
            &[T::Code, T::WordSos, T::WordOk, T::WordVibration, T::Actions];
        assert_table(SecurityText::ALL, security_text, SAME);
    }

    #[test]
    fn lock_words() {
        use crate::cards::lock::LockWord;
        assert_eq!(lock_word(Lang::Fr, LockWord::Jammed), "Bloqué");
        assert_eq!(lock_word(Lang::En, LockWord::Locking), "Locking…");
        assert_eq!(
            security_text(Lang::Fr, SecurityText::VerbUnlock),
            "Déverrouiller"
        );
    }

    #[test]
    fn panel_states_in_both_languages() {
        let state = |lang, raw| panel_state(lang, Some(raw), true);
        assert_eq!(state(Lang::En, "disarmed"), "Disarmed");
        assert_eq!(state(Lang::Fr, "disarmed"), "Désactivée");
        assert_eq!(state(Lang::En, "armed_away"), "Armed away");
        assert_eq!(state(Lang::Fr, "armed_away"), "Activée (absent)");
        assert_eq!(state(Lang::En, "arming"), "Exit delay…");
        assert_eq!(state(Lang::Fr, "arming"), "Délai de sortie…");
        assert_eq!(state(Lang::En, "pending"), "Entry delay…");
        assert_eq!(state(Lang::Fr, "pending"), "Délai d'entrée…");
        assert_eq!(state(Lang::En, "triggered"), "Triggered!");
        assert_eq!(state(Lang::Fr, "triggered"), "Déclenchée\u{A0}!");
        assert_eq!(state(Lang::En, "disarming"), "Disarming…");
        assert_eq!(state(Lang::Fr, "disarming"), "Désactivation…");
        assert_eq!(
            panel_state(Lang::Fr, Some("disarmed"), false),
            "Indisponible"
        );
        assert_eq!(panel_state(Lang::Fr, None, true), "—");
    }

    #[test]
    fn sensor_words_follow_the_group() {
        let word = |lang, g, class, on| sensor_state(lang, g, class, on);
        assert_eq!(
            word(Lang::Fr, SensorGroup::Opening, Some("door"), true),
            "Ouverte"
        );
        assert_eq!(
            word(Lang::Fr, SensorGroup::Opening, Some("opening"), false),
            "Fermé"
        );
        assert_eq!(
            word(Lang::Fr, SensorGroup::Motion, Some("motion"), false),
            "Aucun mouvement"
        );
        assert_eq!(
            word(Lang::Fr, SensorGroup::Smoke, Some("smoke"), true),
            "Détecté"
        );
        assert_eq!(word(Lang::En, SensorGroup::Leak, None, false), "Dry");
        assert_eq!(word(Lang::Fr, SensorGroup::Tamper, None, true), "Sabotage");
    }

    #[test]
    fn group_counts_agree_at_0_1_2() {
        assert_eq!(group_count(Lang::Fr, SensorGroup::Opening, 0), "0 ouvert");
        assert_eq!(group_count(Lang::Fr, SensorGroup::Opening, 1), "1 ouvert");
        assert_eq!(group_count(Lang::Fr, SensorGroup::Opening, 2), "2 ouverts");
        assert_eq!(group_count(Lang::En, SensorGroup::Opening, 2), "2 open");
        assert_eq!(group_count(Lang::En, SensorGroup::Leak, 1), "1 leak");
        assert_eq!(group_count(Lang::En, SensorGroup::Leak, 2), "2 leaks");
    }

    #[test]
    fn questions_keep_french_typography() {
        assert_eq!(
            action_question(Lang::En, SecurityText::VerbOpen, "Gate"),
            "Open Gate?"
        );
        assert_eq!(
            action_question(Lang::Fr, SecurityText::VerbOpen, "Gate"),
            "Ouvrir Gate\u{A0}?"
        );
        assert_eq!(
            action_question(Lang::Fr, SecurityText::VerbPress, "Bell"),
            "Appuyer sur Bell\u{A0}?"
        );
    }

    #[test]
    fn alert_ages_in_both_languages() {
        assert_eq!(alert_age(Lang::En, 90, 100), "Just now");
        assert_eq!(alert_age(Lang::Fr, 90, 100), "À l'instant");
        assert_eq!(alert_age(Lang::En, 0, 120), "2\u{A0}min ago");
        assert_eq!(alert_age(Lang::Fr, 0, 120), "il y a 2\u{A0}min");
        assert_eq!(alert_age(Lang::Fr, 0, 7200), "il y a 2\u{A0}h");
        assert_eq!(alert_age(Lang::Fr, 0, 172_800), "il y a 2\u{A0}j");
        assert_eq!(alert_age(Lang::En, 700, 100), "Just now");
    }

    #[test]
    fn since_days_counts() {
        assert_eq!(since_days(Lang::En, 3), "3 days ago");
        assert_eq!(since_days(Lang::Fr, 3), "il y a 3 jours");
        assert_eq!(since_days(Lang::Fr, 1), "il y a 1 jour");
    }

    #[test]
    fn day_headings_use_the_language_date() {
        assert_eq!(day_heading(Lang::En, 2026, 10, 5, 2026), "Oct 5");
        assert_eq!(day_heading(Lang::Fr, 2026, 10, 5, 2026), "5 oct.");
        assert_eq!(day_heading(Lang::En, 2025, 10, 5, 2026), "Oct 5 2025");
        assert_eq!(day_heading(Lang::Fr, 2025, 10, 5, 2026), "5 oct. 2025");
    }

    #[test]
    fn channels_and_failures() {
        assert_eq!(channel_name(Lang::Fr, AlertChannel::Camera), "Caméra");
        assert_eq!(channel_name(Lang::En, AlertChannel::Misc), "Misc");
        assert_eq!(
            send_failed(Lang::Fr, "Désactiver"),
            "Impossible d'envoyer Désactiver"
        );
    }
}
