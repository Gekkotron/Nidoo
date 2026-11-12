//! FR/EN text: the climate area (screen, room panel, settings sheet, week timeline, water heater).

use super::format::{month_short, weekday_short};
use super::*;

const NBSP: char = '\u{A0}';

/// Fixed words of the climate area. Sentences with values are the helper functions below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClimateText {
    Subtitle,
    HistoryOffBanner,
    NoRoomClimate,
    Rooms,
    ZoneOff,
    ZonePending,
    ZoneActive,
    NoClimateDevice,
    PendingNoSchedule,
    ActiveNoDecision,
    Held,
    LearnOff,
    LearnHistoryOff,
    LearnUpToDate,
    LearnProposed,
    LearnRejected,
    LearnEco,
    Idle,
    SrcSafety,
    SrcOverride,
    SrcBoost,
    SrcHoliday,
    SrcAway,
    SrcEco,
    SrcEmpty,
    SrcSchedule,
    Tomorrow,
    Until,
    SetOnDevice,
    SetHere,
    NoReading,
    Now,
    NoDecision,
    Clear,
    Target,
    ZoneStateLabel,
    UntilNextChange,
    LearnNow,
    RecentDecisions,
    ProposedSchedule,
    ProposedChanges,
    NoChange,
    DiscardEdits,
    ApproveEdited,
    Approve,
    Reject,
    ApprovedRules,
    ThisWeek,
    NoApprovedSchedule,
    CreateSchedule,
    Save,
    Cancel,
    EditSchedule,
    SettingsLabel,
    Season,
    OutdoorTemperature,
    HeatBelow,
    CoolAbove,
    Presence,
    AwayGrace,
    Holiday,
    Frost,
    Overheat,
    Overcool,
    StaleAfter,
    LearningWindow,
    MinimumDays,
    AgreementNeeded,
    LearnAt,
    UnitMin,
    UnitDays,
    UnitZeroToOne,
    None,
    Remove,
    AddEllipsis,
    RoomThermometer,
    AutomaticClimateDevice,
    NoRoom,
    DragStart,
    DragEnd,
    Days,
    From,
    To,
    TemperatureLabel,
    DeleteBlock,
    Done,
    Base,
    BaseTemperature,
    OutsideBlocks,
    TapToAdd,
    Heating,
    Inlet,
    Tank,
    Eco,
    EcoHeat,
    EcoCool,
    ExtraActions,
    FanSpeed,
    DontChange,
    PresenceAlarmHint,
    EcoUntilButton,
    CorrectionTitle,
    CorrectionNoSensor,
    CorrectionStep,
    CorrectionBand,
    CorrectionInterval,
    CorrectionWarmup,
    CorrectionMaxBelow,
    CorrectionMaxAbove,
    CorrectionQuiet,
    RoomPresenceTitle,
    RoomPresenceSensors,
    RoomPresenceEmptyAfter,
    RoomPresenceBackAfter,
    RoomPresenceSetback,
    RoomPresenceOccupied,
    RoomPresenceNoSensor,
    UnitHours,
    EcoUntilDate,
    EcoUntilTime,
    StartEco,
    CancelEco,
    EcoPickDate,
    HouseSettings,
    RoomSettings,
    RoomSettingsTitle,
    AdvancedSettings,
    EcoPill,
    LowerTarget,
    RaiseTarget,
    Learning,
    TargetIn,
}

impl ClimateText {
    pub const ALL: &'static [ClimateText] = &[
        ClimateText::Subtitle,
        ClimateText::HistoryOffBanner,
        ClimateText::NoRoomClimate,
        ClimateText::Rooms,
        ClimateText::ZoneOff,
        ClimateText::ZonePending,
        ClimateText::ZoneActive,
        ClimateText::NoClimateDevice,
        ClimateText::PendingNoSchedule,
        ClimateText::ActiveNoDecision,
        ClimateText::Held,
        ClimateText::LearnOff,
        ClimateText::LearnHistoryOff,
        ClimateText::LearnUpToDate,
        ClimateText::LearnProposed,
        ClimateText::LearnRejected,
        ClimateText::LearnEco,
        ClimateText::Idle,
        ClimateText::SrcSafety,
        ClimateText::SrcOverride,
        ClimateText::SrcBoost,
        ClimateText::SrcHoliday,
        ClimateText::SrcAway,
        ClimateText::SrcEco,
        ClimateText::SrcEmpty,
        ClimateText::SrcSchedule,
        ClimateText::Tomorrow,
        ClimateText::Until,
        ClimateText::SetOnDevice,
        ClimateText::SetHere,
        ClimateText::NoReading,
        ClimateText::Now,
        ClimateText::NoDecision,
        ClimateText::Clear,
        ClimateText::Target,
        ClimateText::ZoneStateLabel,
        ClimateText::UntilNextChange,
        ClimateText::LearnNow,
        ClimateText::RecentDecisions,
        ClimateText::ProposedSchedule,
        ClimateText::ProposedChanges,
        ClimateText::NoChange,
        ClimateText::DiscardEdits,
        ClimateText::ApproveEdited,
        ClimateText::Approve,
        ClimateText::Reject,
        ClimateText::ApprovedRules,
        ClimateText::ThisWeek,
        ClimateText::NoApprovedSchedule,
        ClimateText::CreateSchedule,
        ClimateText::Save,
        ClimateText::Cancel,
        ClimateText::EditSchedule,
        ClimateText::SettingsLabel,
        ClimateText::Season,
        ClimateText::OutdoorTemperature,
        ClimateText::HeatBelow,
        ClimateText::CoolAbove,
        ClimateText::Presence,
        ClimateText::AwayGrace,
        ClimateText::Holiday,
        ClimateText::Frost,
        ClimateText::Overheat,
        ClimateText::Overcool,
        ClimateText::StaleAfter,
        ClimateText::LearningWindow,
        ClimateText::MinimumDays,
        ClimateText::AgreementNeeded,
        ClimateText::LearnAt,
        ClimateText::UnitMin,
        ClimateText::UnitDays,
        ClimateText::UnitZeroToOne,
        ClimateText::None,
        ClimateText::Remove,
        ClimateText::AddEllipsis,
        ClimateText::RoomThermometer,
        ClimateText::AutomaticClimateDevice,
        ClimateText::NoRoom,
        ClimateText::DragStart,
        ClimateText::DragEnd,
        ClimateText::Days,
        ClimateText::From,
        ClimateText::To,
        ClimateText::TemperatureLabel,
        ClimateText::DeleteBlock,
        ClimateText::Done,
        ClimateText::Base,
        ClimateText::BaseTemperature,
        ClimateText::OutsideBlocks,
        ClimateText::TapToAdd,
        ClimateText::Heating,
        ClimateText::Inlet,
        ClimateText::Tank,
        ClimateText::Eco,
        ClimateText::EcoHeat,
        ClimateText::EcoCool,
        ClimateText::ExtraActions,
        ClimateText::FanSpeed,
        ClimateText::DontChange,
        ClimateText::PresenceAlarmHint,
        ClimateText::EcoUntilButton,
        ClimateText::EcoUntilDate,
        ClimateText::EcoUntilTime,
        ClimateText::StartEco,
        ClimateText::CancelEco,
        ClimateText::EcoPickDate,
        ClimateText::HouseSettings,
        ClimateText::RoomSettings,
        ClimateText::RoomSettingsTitle,
        ClimateText::AdvancedSettings,
        ClimateText::EcoPill,
        ClimateText::LowerTarget,
        ClimateText::RaiseTarget,
        ClimateText::Learning,
        ClimateText::TargetIn,
        ClimateText::CorrectionTitle,
        ClimateText::CorrectionNoSensor,
        ClimateText::CorrectionStep,
        ClimateText::CorrectionBand,
        ClimateText::CorrectionInterval,
        ClimateText::CorrectionWarmup,
        ClimateText::CorrectionMaxBelow,
        ClimateText::CorrectionMaxAbove,
        ClimateText::CorrectionQuiet,
        ClimateText::RoomPresenceTitle,
        ClimateText::RoomPresenceSensors,
        ClimateText::RoomPresenceEmptyAfter,
        ClimateText::RoomPresenceBackAfter,
        ClimateText::RoomPresenceSetback,
        ClimateText::RoomPresenceOccupied,
        ClimateText::RoomPresenceNoSensor,
        ClimateText::UnitHours,
    ];
}

/// The text for a word in a language.
pub fn climate_text(lang: Lang, k: ClimateText) -> &'static str {
    use ClimateText as T;
    let (en, fr) = match k {
        T::Subtitle => (
            "Still being tested: check what the engine decides before you rely on it.",
            "Encore en test\u{A0}: vérifiez ce que décide le moteur avant de vous y fier.",
        ),
        T::HistoryOffBanner => (
            "History is off: rules can't be learned until a history database is set up.",
            "L'historique est désactivé\u{A0}: les règles ne peuvent pas être apprises tant qu'une base d'historique n'est pas configurée.",
        ),
        T::NoRoomClimate => (
            "No room has a climate device",
            "Aucune pièce n'a d'appareil de climat",
        ),
        T::Rooms => ("Rooms", "Pièces"),
        T::ZoneOff => ("Off", "Arrêt"),
        T::ZonePending => ("Pending", "En attente"),
        T::ZoneActive => ("Active", "Active"),
        T::NoClimateDevice => ("No climate device", "Aucun appareil de climat"),
        T::PendingNoSchedule => ("Pending · no schedule", "En attente · aucune programmation"),
        T::ActiveNoDecision => (
            "Active · no decision yet",
            "Active · pas encore de décision",
        ),
        T::Held => ("held", "dérogation en cours"),
        T::LearnOff => ("Learning is off", "L'apprentissage est désactivé"),
        T::LearnHistoryOff => (
            "History is off: rules can't be learned",
            "L'historique est désactivé\u{A0}: les règles ne peuvent pas être apprises",
        ),
        T::LearnUpToDate => ("Rules up to date", "Règles à jour"),
        T::LearnProposed => ("New rules to review", "Nouvelles règles à examiner"),
        T::LearnRejected => ("Last proposal rejected", "Dernière proposition rejetée"),
        T::LearnEco => (
            "Learning paused while in eco",
            "Apprentissage en pause pendant l'éco",
        ),
        T::Idle => ("Idle", "Au repos"),
        T::SrcSafety => ("Safety", "Sécurité"),
        T::SrcOverride => ("Override", "Dérogation"),
        T::SrcBoost => ("Boost", "Boost"),
        T::SrcHoliday => ("Holiday", "Vacances"),
        T::SrcAway => ("Away", "Absence"),
        T::SrcEco => ("Eco", "Éco"),
        T::SrcEmpty => ("Empty room", "Pièce vide"),
        T::SrcSchedule => ("Schedule", "Programmation"),
        T::Tomorrow => ("tomorrow", "demain"),
        T::Until => ("until", "jusqu'à"),
        T::SetOnDevice => ("set on a device", "réglé sur l'appareil"),
        T::SetHere => ("set here", "réglé ici"),
        T::NoReading => ("No reading", "Aucune mesure"),
        T::Now => ("Now", "Maintenant"),
        T::NoDecision => ("No decision yet", "Pas encore de décision"),
        T::Clear => ("Clear", "Effacer"),
        T::Target => ("Target", "Consigne"),
        T::ZoneStateLabel => ("Zone state", "État de la zone"),
        T::UntilNextChange => ("Until next change", "Jusqu'au prochain changement"),
        T::LearnNow => ("Learn now", "Apprendre maintenant"),
        T::RecentDecisions => ("Recent decisions", "Décisions récentes"),
        T::ProposedSchedule => ("Proposed schedule", "Programmation proposée"),
        T::ProposedChanges => ("Proposed changes", "Modifications proposées"),
        T::NoChange => (
            "No change from the approved rules",
            "Aucune différence avec les règles approuvées",
        ),
        T::DiscardEdits => ("Discard edits", "Annuler les modifications"),
        T::ApproveEdited => ("Approve edited", "Approuver la version modifiée"),
        T::Approve => ("Approve", "Approuver"),
        T::Reject => ("Reject", "Rejeter"),
        T::ApprovedRules => ("Approved rules", "Règles approuvées"),
        T::ThisWeek => ("This week", "Cette semaine"),
        T::NoApprovedSchedule => (
            "No approved schedule yet",
            "Aucune programmation approuvée pour l'instant",
        ),
        T::CreateSchedule => ("Create schedule", "Créer une programmation"),
        T::Save => ("Save", "Enregistrer"),
        T::Cancel => ("Cancel", "Annuler"),
        T::EditSchedule => ("Edit schedule", "Modifier la programmation"),
        T::SettingsLabel => ("Climate settings", "Réglages du climat"),
        T::Season => ("Season", "Saison"),
        T::OutdoorTemperature => ("Outdoor temperature", "Température extérieure"),
        T::HeatBelow => ("Heat below", "Chauffer en dessous de"),
        T::CoolAbove => ("Cool above", "Refroidir au-dessus de"),
        T::Presence => ("Presence", "Présence"),
        T::AwayGrace => ("Away grace", "Délai d'absence"),
        T::Holiday => ("Holiday", "Vacances"),
        T::Frost => ("Frost", "Hors-gel"),
        T::Overheat => ("Overheat", "Surchauffe"),
        T::Overcool => ("Overcool", "Refroidissement excessif"),
        T::StaleAfter => ("Stale after", "Mesure périmée après"),
        T::LearningWindow => ("Learning window", "Fenêtre d'apprentissage"),
        T::MinimumDays => ("Minimum days", "Jours minimum"),
        T::AgreementNeeded => ("Agreement needed", "Accord requis"),
        T::LearnAt => ("Learn at", "Heure d'apprentissage"),
        T::UnitMin => ("min", "min"),
        T::CorrectionTitle => ("Thermometer correction", "Correction par le thermomètre"),
        T::CorrectionNoSensor => (
            "No room thermometer: pick one above",
            "Pas de thermomètre : choisissez-en un ci-dessus",
        ),
        T::CorrectionStep => ("Step", "Pas"),
        T::CorrectionBand => ("Band", "Écart toléré"),
        T::CorrectionInterval => ("Interval", "Intervalle"),
        T::CorrectionWarmup => ("Warm-up", "Mise en route"),
        T::CorrectionMaxBelow => ("Max below target", "Max sous la cible"),
        T::CorrectionMaxAbove => ("Max above target", "Max au-dessus de la cible"),
        T::CorrectionQuiet => ("Quiet limit", "Silence maximal"),
        T::RoomPresenceTitle => ("Room presence", "Présence dans la pièce"),
        T::RoomPresenceSensors => ("Presence sensors", "Détecteurs de présence"),
        T::RoomPresenceEmptyAfter => ("Empty after", "Vide après"),
        T::RoomPresenceBackAfter => ("Occupied again after", "Occupée de nouveau après"),
        T::RoomPresenceSetback => ("Lower by", "Baisser de"),
        T::RoomPresenceOccupied => ("Occupied", "Occupée"),
        T::RoomPresenceNoSensor => (
            "Pick a presence sensor: without one the room counts as occupied",
            "Choisissez un détecteur : sans lui, la pièce compte comme occupée",
        ),
        T::UnitHours => ("h", "h"),
        T::UnitDays => ("days", "jours"),
        T::UnitZeroToOne => ("0 to 1", "0 à 1"),
        T::None => ("None", "Aucun"),
        T::Remove => ("Remove", "Retirer"),
        T::AddEllipsis => ("Add…", "Ajouter…"),
        T::RoomThermometer => ("Room thermometer", "Thermomètre de la pièce"),
        T::AutomaticClimateDevice => (
            "Automatic · climate device",
            "Automatique · appareil de climat",
        ),
        T::NoRoom => ("No room", "Sans pièce"),
        T::DragStart => ("Drag the start", "Faire glisser le début"),
        T::DragEnd => ("Drag the end", "Faire glisser la fin"),
        T::Days => ("Days", "Jours"),
        T::From => ("From", "De"),
        T::To => ("To", "À"),
        T::TemperatureLabel => ("Temperature", "Température"),
        T::DeleteBlock => ("Delete block", "Supprimer la plage"),
        T::Done => ("Done", "Terminé"),
        T::Base => ("Base", "Base"),
        T::BaseTemperature => ("Base temperature", "Température de base"),
        T::OutsideBlocks => ("outside the blocks", "hors des plages"),
        T::TapToAdd => (
            "Tap an empty spot to add a block",
            "Touchez un emplacement vide pour ajouter une plage",
        ),
        T::Heating => ("Heating", "En chauffe"),
        T::Inlet => ("Inlet", "Arrivée"),
        T::Tank => ("Tank", "Ballon"),
        T::Eco => ("Eco", "Éco"),
        T::EcoHeat => ("Eco heating", "Éco chauffage"),
        T::EcoCool => ("Eco cooling", "Éco climatisation"),
        T::ExtraActions => ("Extra actions", "Actions en plus"),
        T::FanSpeed => ("Fan speed", "Vitesse du ventilateur"),
        T::DontChange => ("Don't change", "Ne pas changer"),
        T::PresenceAlarmHint => (
            "The hub's alarm armed away always means nobody home; another alarm panel can be added",
            "L'alarme du hub armée absent veut toujours dire personne à la maison\u{A0}; une autre alarme peut être ajoutée",
        ),
        T::EcoUntilButton => ("Eco until…", "Éco jusqu'au…"),
        T::EcoUntilDate => ("Until", "Jusqu'au"),
        T::EcoUntilTime => ("Time", "Heure"),
        T::StartEco => ("Start eco", "Lancer l'éco"),
        T::CancelEco => ("Cancel eco", "Annuler l'éco"),
        T::EcoPickDate => (
            "Pick an end date in the future",
            "Choisissez une date de fin à venir",
        ),
        T::HouseSettings => ("House settings", "Réglages de la maison"),
        T::RoomSettings => ("Room settings", "Réglages de la pièce"),
        T::RoomSettingsTitle => ("Room settings ·", "Réglages ·"),
        T::AdvancedSettings => ("Advanced settings", "Réglages avancés"),
        T::EcoPill => ("Eco…", "Éco…"),
        T::LowerTarget => ("Lower target", "Baisser la consigne"),
        T::RaiseTarget => ("Raise target", "Monter la consigne"),
        T::Learning => ("Learning", "Apprentissage"),
        T::TargetIn => ("Target ·", "Consigne ·"),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// A translator for [`ClimateText`] bound to the language in context (reactive).
pub fn use_climate_text() -> impl Fn(ClimateText) -> &'static str + Copy + 'static {
    let lang = use_lang();
    move |k| climate_text(lang(), k)
}

fn pick(lang: Lang, en: String, fr: String) -> String {
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// The correction's main numbers, already formatted: "Step 0.5 °C · tolerance 0.5 °C · every
/// 15 min" / "Pas 0,5 °C · écart 0,5 °C · toutes les 15 min".
pub fn correction_summary_line(lang: Lang, step: &str, band: &str, interval_min: u32) -> String {
    pick(
        lang,
        format!("Step {step} °C · tolerance {band} °C · every {interval_min} min"),
        format!("Pas {step} °C · écart {band} °C · toutes les {interval_min} min"),
    )
}

/// "Learning: 6 of 10 days so far" / "Apprentissage : 6 jours sur 10 pour l'instant".
pub fn learning_progress(lang: Lang, days: u32, needed: u32) -> String {
    let fr_days = format::count(Lang::Fr, days as usize, ("", "jour"), ("", "jours"));
    pick(
        lang,
        format!("Learning: {days} of {needed} days so far"),
        format!("Apprentissage{NBSP}: {fr_days} sur {needed} pour l'instant"),
    )
}

/// A hold line: "Heat 21° until 07:00 · set here" / "Chauffage 21° jusqu'à 07:00 · réglé ici".
pub fn hold_until(lang: Lang, what: &str, until: &str, by: ClimateText) -> String {
    let word = climate_text(lang, ClimateText::Until);
    format!("{what} {word} {until} · {}", climate_text(lang, by))
}

/// "Now · 10 min ago" / "Maintenant · il y a 10 min".
pub fn now_with_age(lang: Lang, age: &str) -> String {
    format!("{} · {age}", climate_text(lang, ClimateText::Now))
}

/// "Automatic · Bedroom thermometer" / "Automatique · Thermomètre de la chambre".
pub fn automatic_with(lang: Lang, what: &str) -> String {
    pick(
        lang,
        format!("Automatic · {what}"),
        format!("Automatique · {what}"),
    )
}

/// Since when a room counts as empty ("Empty since 14:20").
pub fn empty_since(lang: Lang, clock: &str) -> String {
    pick(
        lang,
        format!("Empty since {clock}"),
        format!("Vide depuis {clock}"),
    )
}

/// "Could not save: reason" / "Enregistrement impossible : reason" (the reason is the hub's).
pub fn could_not_save(lang: Lang, hub: &str) -> String {
    pick(
        lang,
        format!("Could not save: {hub}"),
        format!("Enregistrement impossible{NBSP}: {hub}"),
    )
}

/// The activity chip of a heater drawing power: "Heating · 43 %" / "En chauffe · 43 %".
pub fn heating_power(lang: Lang, percent: u8) -> String {
    format!(
        "{} · {percent}{NBSP}%",
        climate_text(lang, ClimateText::Heating)
    )
}

/// A fan speed choice: "low (on 2 of 4 units)" / "low (sur 2 des 4 appareils)".
pub fn fan_choice(lang: Lang, mode: &str, n: usize, of: usize) -> String {
    pick(
        lang,
        format!("{mode} (on {n} of {of} units)"),
        format!("{mode} (sur {n} des {of} appareils)"),
    )
}

/// A room's eco line: "Eco until Sun 1 Nov 00:00" / "Éco jusqu'au dim. 1 nov. 00:00".
pub fn eco_until_line(lang: Lang, until: &str) -> String {
    pick(
        lang,
        format!("Eco until {until}"),
        format!("Éco jusqu'au {until}"),
    )
}

/// The engine's English decision reason as the user's language. English passes through; French
/// translates each "; "-separated segment by the engine's known phrasings, and a segment it does
/// not recognise stays as the engine wrote it.
pub fn reason_text(lang: Lang, raw: &str) -> String {
    match lang {
        Lang::En => raw.to_owned(),
        Lang::Fr => raw
            .split("; ")
            .map(|seg| reason_segment_fr(seg).unwrap_or_else(|| seg.to_owned()))
            .collect::<Vec<_>>()
            .join("; "),
    }
}

/// "19.0 °C" as "19,0 °C": the decimal point between two digits becomes a comma.
fn fr_decimal(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    chars
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let between_digits = i > 0
                && chars[i - 1].is_ascii_digit()
                && chars.get(i + 1).is_some_and(char::is_ascii_digit);
            if c == '.' && between_digits { ',' } else { c }
        })
        .collect()
}

/// A setpoint as the engine prints it ("19.0 °C" or "off"), in French.
fn fr_setpoint(s: &str) -> String {
    if s == "off" {
        "arrêt".to_owned()
    } else {
        fr_decimal(s)
    }
}

/// "Wed 21 Oct 11:00" as "mer. 21 oct. 11:00"; anything else is returned as it came.
fn fr_day_clock(s: &str) -> String {
    let parts: Vec<&str> = s.split_whitespace().collect();
    let [day, date, month, clock] = parts[..] else {
        return s.to_owned();
    };
    let weekday = day.parse::<chrono::Weekday>().ok();
    let month = (1..=12).find(|m| month_short(Lang::En, *m) == month);
    match (weekday, month) {
        (Some(w), Some(m)) => format!(
            "{} {date} {} {clock}",
            weekday_short(Lang::Fr, w).to_lowercase(),
            month_short(Lang::Fr, m)
        ),
        _ => s.to_owned(),
    }
}

/// "head: tail" with the French space before the colon.
fn fr_colon(head: &str, tail: &str) -> String {
    format!("{head}{NBSP}: {tail}")
}

/// One segment of an engine reason in French, or `None` when it is not a known shape.
fn reason_segment_fr(seg: &str) -> Option<String> {
    match seg {
        "Zone is off" => return Some("Pièce arrêtée".to_owned()),
        "Waiting for approved rules" => return Some("En attente de règles approuvées".to_owned()),
        "Safety cleared: restoring previous settings" => {
            return Some(fr_colon("Sécurité levée", "réglages précédents rétablis"));
        }
        "No heating or cooling season" => {
            return Some("Hors saison de chauffage ou de climatisation".to_owned());
        }
        "no room thermometer" => return Some("pas de thermomètre dans la pièce".to_owned()),
        "room thermometer has not reported: no correction" => {
            return Some(fr_colon(
                "le thermomètre de la pièce n'a encore rien envoyé",
                "aucune correction",
            ));
        }
        "room thermometer silent: no correction" => {
            return Some(fr_colon(
                "thermomètre de la pièce muet",
                "aucune correction",
            ));
        }
        "all units held: correction paused" => {
            return Some(fr_colon(
                "tous les appareils sont maintenus",
                "correction en pause",
            ));
        }
        "schedule already lower" => {
            return Some("la programmation est déjà plus basse".to_owned());
        }
        "schedule already higher" => {
            return Some("la programmation est déjà plus haute".to_owned());
        }
        _ => {}
    }
    protection_fr(seg)
        .or_else(|| stale_fr(seg))
        .or_else(|| until_fr(seg))
        .or_else(|| setpoint_reason_fr(seg))
        .or_else(|| note_fr(seg))
}

/// "Frost protection: 6.5 °C < 7.0 °C" and its overheat / overcool / "holding until" forms.
fn protection_fr(seg: &str) -> Option<String> {
    let (name, rest, cmp) = if let Some(r) = seg.strip_prefix("Frost protection: ") {
        ("Protection antigel", r, '<')
    } else if let Some(r) = seg.strip_prefix("Overheat protection: ") {
        ("Protection surchauffe", r, '>')
    } else {
        (
            "Protection surrefroidissement",
            seg.strip_prefix("Overcool protection: ")?,
            '<',
        )
    };
    if let Some((value, limit)) = rest.split_once(&format!(" {cmp} ")) {
        return Some(fr_colon(
            name,
            &format!("{} {cmp} {}", fr_decimal(value), fr_decimal(limit)),
        ));
    }
    let (value, target) = rest.split_once(", holding until ")?;
    Some(fr_colon(
        name,
        &format!(
            "{}, maintenu jusqu'à {}",
            fr_decimal(value),
            fr_decimal(target)
        ),
    ))
}

/// "Reading is stale (last 18.0 °C): not acting until approved rules exist" and its siblings.
fn stale_fr(seg: &str) -> Option<String> {
    let (what, rest) = if let Some(r) = seg.strip_prefix("Reading is stale (last ") {
        let (value, after) = r.split_once(')')?;
        (
            format!("Mesure périmée (dernière valeur {})", fr_decimal(value)),
            after,
        )
    } else if let Some(r) = seg.strip_prefix("Reading is stale") {
        ("Mesure périmée".to_owned(), r)
    } else {
        let r = seg.strip_prefix("No temperature reading")?;
        ("Aucune mesure de température".to_owned(), r)
    };
    if rest.is_empty() {
        return Some(what);
    }
    let tail = match rest.strip_prefix(": ")? {
        "not acting until approved rules exist" => {
            "aucune action tant qu'il n'y a pas de règles approuvées"
        }
        "no rules to fall back on, not acting" => "aucune règle de repli, aucune action",
        "holding the safer of the current target and base" => {
            "on garde la plus prudente de la consigne actuelle et de la base"
        }
        _ => return None,
    };
    Some(fr_colon(&what, tail))
}

/// Boost, override, eco and the thermometer-silent note: the shapes that carry a time.
fn until_fr(seg: &str) -> Option<String> {
    for (prefix, word) in [("Boost to ", "Boost à"), ("Override to ", "Dérogation à")] {
        if let Some(rest) = seg.strip_prefix(prefix) {
            let (sp, clock) = rest.split_once(" until ")?;
            return Some(format!("{word} {} jusqu'à {clock}", fr_setpoint(sp)));
        }
    }
    if let Some(rest) = seg.strip_prefix("Eco until ") {
        let (when, sp) = rest.split_once(": ")?;
        let (when, nobody) = match when.strip_suffix(" (nobody home)") {
            Some(w) => (w, " (personne à la maison)"),
            None => (when, ""),
        };
        return Some(fr_colon(
            &format!("Éco jusqu'au {}{nobody}", fr_day_clock(when)),
            &fr_setpoint(sp),
        ));
    }
    let rest = seg.strip_prefix("the room thermometer has been silent for over ")?;
    let (duration, tail) = rest.split_once(", ")?;
    (tail == "using the devices' own readings").then(|| {
        format!(
            "le thermomètre de la pièce est muet depuis plus de {duration}, mesures des appareils utilisées"
        )
    })
}

/// Holiday, away, schedule, presence setback, "would set" and "No rules for … mode".
fn setpoint_reason_fr(seg: &str) -> Option<String> {
    if let Some(sp) = seg.strip_prefix("Holiday: ") {
        return Some(fr_colon("Vacances", &fr_setpoint(sp)));
    }
    if let Some(sp) = seg.strip_prefix("Nobody home: eco ") {
        return Some(fr_colon(
            "Personne à la maison",
            &format!("éco {}", fr_setpoint(sp)),
        ));
    }
    if let Some(sp) = seg.strip_prefix("Schedule base: ") {
        return Some(fr_colon("Base de la programmation", &fr_setpoint(sp)));
    }
    if let Some(rest) = seg.strip_prefix("Schedule block ") {
        let (n, sp) = rest.split_once(": ")?;
        n.parse::<u32>().ok()?;
        return Some(fr_colon(
            &format!("Plage {n} de la programmation"),
            &fr_setpoint(sp),
        ));
    }
    if let Some(rest) = seg.strip_prefix("No one in the room for ") {
        let (minutes, rest) = rest.split_once(" min: ")?;
        let (target, detail) = rest.split_once(" (")?;
        let detail = detail.strip_suffix(')')?;
        let detail = match detail {
            "base" => "base".to_owned(),
            "frost" => "antigel".to_owned(),
            other => fr_decimal(other),
        };
        return Some(fr_colon(
            &format!("Personne dans la pièce depuis {minutes} min"),
            &format!("{} ({detail})", fr_decimal(target)),
        ));
    }
    if let Some(sp) = seg.strip_prefix("would set ") {
        return Some(if sp == "off" {
            "arrêterait".to_owned()
        } else {
            format!("réglerait sur {}", fr_decimal(sp))
        });
    }
    let mode = seg.strip_prefix("No rules for ")?.strip_suffix(" mode")?;
    let mode = match mode {
        "heat" => "chauffage",
        "cool" => "climatisation",
        _ => return None,
    };
    Some(format!("Aucune règle en mode {mode}"))
}

/// The notes the engine appends: held devices, "cannot stop cooling" and the room correction.
fn note_fr(seg: &str) -> Option<String> {
    if let Some(ids) = seg.strip_prefix("cannot stop cooling: ") {
        return Some(fr_colon("impossible d'arrêter la climatisation", ids));
    }
    if let Some((id, clock)) = seg.split_once(" held as it is until ") {
        return Some(format!("{id} laissé tel quel jusqu'à {clock}"));
    }
    if let Some((id, clock)) = seg.split_once(" held until ") {
        return Some(format!("{id} maintenu jusqu'à {clock}"));
    }
    let rest = seg.strip_prefix("room ")?;
    let (word, rest) = rest.split_once(' ')?;
    let word = match word {
        "above" => "au-dessus de",
        "below" => "en dessous de",
        "near" => "proche de",
        _ => return None,
    };
    let (edge, ac) = rest.split_once(": AC ")?;
    let (ac, offset) = match ac.split_once(" (") {
        Some((ac, o)) => (ac, Some(o.strip_suffix(')')?)),
        None => (ac, None),
    };
    let tail = match offset {
        Some(o) => format!("clim {} ({})", fr_decimal(ac), fr_decimal(o)),
        None => format!("clim {}", fr_decimal(ac)),
    };
    Some(fr_colon(
        &format!("pièce {word} {}", fr_decimal(edge)),
        &tail,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eco_sentences() {
        assert_eq!(fan_choice(Lang::En, "low", 2, 4), "low (on 2 of 4 units)");
        assert_eq!(
            fan_choice(Lang::Fr, "low", 2, 4),
            "low (sur 2 des 4 appareils)"
        );
        assert_eq!(
            eco_until_line(Lang::En, "Sun 1 Nov 00:00"),
            "Eco until Sun 1 Nov 00:00"
        );
        assert_eq!(
            eco_until_line(Lang::Fr, "dim. 1 nov. 00:00"),
            "Éco jusqu'au dim. 1 nov. 00:00"
        );
    }

    /// Same word in both languages.
    const SAME: &[ClimateText] = &[
        ClimateText::ZoneActive,
        ClimateText::UnitMin,
        ClimateText::Base,
        ClimateText::UnitHours,
        ClimateText::SrcBoost,
    ];

    #[test]
    fn every_word_has_both_languages() {
        assert_table(ClimateText::ALL, climate_text, SAME);
    }

    #[test]
    fn learning_progress_agrees_with_the_count() {
        assert_eq!(
            learning_progress(Lang::En, 6, 10),
            "Learning: 6 of 10 days so far"
        );
        assert_eq!(
            learning_progress(Lang::Fr, 6, 10),
            "Apprentissage\u{A0}: 6 jours sur 10 pour l'instant"
        );
        assert_eq!(
            learning_progress(Lang::Fr, 1, 10),
            "Apprentissage\u{A0}: 1 jour sur 10 pour l'instant"
        );
        assert_eq!(
            learning_progress(Lang::Fr, 0, 10),
            "Apprentissage\u{A0}: 0 jour sur 10 pour l'instant"
        );
    }

    #[test]
    fn sentences_read_in_both_languages() {
        assert_eq!(
            hold_until(Lang::En, "Heat 21°", "07:00", ClimateText::SetHere),
            "Heat 21° until 07:00 · set here"
        );
        assert_eq!(
            hold_until(Lang::Fr, "Chauffage 21°", "07:00", ClimateText::SetOnDevice),
            "Chauffage 21° jusqu'à 07:00 · réglé sur l'appareil"
        );
        assert_eq!(
            now_with_age(Lang::Fr, "à l'instant"),
            "Maintenant · à l'instant"
        );
        assert_eq!(automatic_with(Lang::Fr, "Salon"), "Automatique · Salon");
        assert_eq!(
            could_not_save(Lang::Fr, "refusé"),
            "Enregistrement impossible\u{A0}: refusé"
        );
        assert_eq!(heating_power(Lang::En, 43), "Heating · 43\u{A0}%");
        assert_eq!(heating_power(Lang::Fr, 43), "En chauffe · 43\u{A0}%");
    }

    /// Every reason shape the engine writes, exactly as decide.rs and feedback.rs format it.
    #[test]
    fn engine_reasons_in_french() {
        let n = "\u{a0}:";
        let cases: Vec<(&str, String)> = vec![
            ("Zone is off", "Pièce arrêtée".into()),
            (
                "Frost protection: 6.5 °C < 7.0 °C",
                format!("Protection antigel{n} 6,5 °C < 7,0 °C"),
            ),
            (
                "Frost protection: 8.0 °C, holding until 9.0 °C",
                format!("Protection antigel{n} 8,0 °C, maintenu jusqu'à 9,0 °C"),
            ),
            (
                "Overheat protection: 31.0 °C > 30.0 °C",
                format!("Protection surchauffe{n} 31,0 °C > 30,0 °C"),
            ),
            (
                "Overheat protection: 29.8 °C, holding until 29.5 °C",
                format!("Protection surchauffe{n} 29,8 °C, maintenu jusqu'à 29,5 °C"),
            ),
            (
                "Overcool protection: 14.0 °C < 15.0 °C",
                format!("Protection surrefroidissement{n} 14,0 °C < 15,0 °C"),
            ),
            (
                "Overcool protection: 15.2 °C, holding until 16.5 °C",
                format!("Protection surrefroidissement{n} 15,2 °C, maintenu jusqu'à 16,5 °C"),
            ),
            (
                "Overcool protection: 14.0 °C < 15.0 °C; cannot stop cooling: ac1, ac2",
                format!(
                    "Protection surrefroidissement{n} 14,0 °C < 15,0 °C; impossible d'arrêter la climatisation{n} ac1, ac2"
                ),
            ),
            (
                "Reading is stale (last 18.0 °C): not acting until approved rules exist",
                format!(
                    "Mesure périmée (dernière valeur 18,0 °C){n} aucune action tant qu'il n'y a pas de règles approuvées"
                ),
            ),
            (
                "Reading is stale: no rules to fall back on, not acting",
                format!("Mesure périmée{n} aucune règle de repli, aucune action"),
            ),
            (
                "No temperature reading: holding the safer of the current target and base",
                format!(
                    "Aucune mesure de température{n} on garde la plus prudente de la consigne actuelle et de la base"
                ),
            ),
            (
                "Safety cleared: restoring previous settings",
                format!("Sécurité levée{n} réglages précédents rétablis"),
            ),
            (
                "Waiting for approved rules",
                "En attente de règles approuvées".into(),
            ),
            (
                "Waiting for approved rules; would set 19.0 °C",
                "En attente de règles approuvées; réglerait sur 19,0 °C".into(),
            ),
            (
                "Waiting for approved rules; would set off",
                "En attente de règles approuvées; arrêterait".into(),
            ),
            (
                "Boost to 22.0 °C until 11:00",
                "Boost à 22,0 °C jusqu'à 11:00".into(),
            ),
            (
                "Override to 20.5 °C until 11:00",
                "Dérogation à 20,5 °C jusqu'à 11:00".into(),
            ),
            (
                "Override to off until 11:00",
                "Dérogation à arrêt jusqu'à 11:00".into(),
            ),
            (
                "No heating or cooling season",
                "Hors saison de chauffage ou de climatisation".into(),
            ),
            (
                "No rules for heat mode",
                "Aucune règle en mode chauffage".into(),
            ),
            (
                "No rules for cool mode",
                "Aucune règle en mode climatisation".into(),
            ),
            ("Holiday: 7.0 °C", format!("Vacances{n} 7,0 °C")),
            ("Holiday: off", format!("Vacances{n} arrêt")),
            (
                "Eco until Wed 21 Oct 11:00: 16.0 °C",
                format!("Éco jusqu'au mer. 21 oct. 11:00{n} 16,0 °C"),
            ),
            (
                "Eco until Wed 21 Oct 11:00 (nobody home): 16.0 °C",
                format!("Éco jusqu'au mer. 21 oct. 11:00 (personne à la maison){n} 16,0 °C"),
            ),
            (
                "Eco until Wed 21 Oct 11:00: 14.0 °C; schedule already lower",
                format!(
                    "Éco jusqu'au mer. 21 oct. 11:00{n} 14,0 °C; la programmation est déjà plus basse"
                ),
            ),
            (
                "Eco until Sun 1 Nov 00:00: 26.0 °C; schedule already higher",
                format!(
                    "Éco jusqu'au dim. 1 nov. 00:00{n} 26,0 °C; la programmation est déjà plus haute"
                ),
            ),
            (
                "Nobody home: eco 16.0 °C",
                format!("Personne à la maison{n} éco 16,0 °C"),
            ),
            (
                "Nobody home: eco off",
                format!("Personne à la maison{n} éco arrêt"),
            ),
            (
                "Schedule block 2: 19.0 °C",
                format!("Plage 2 de la programmation{n} 19,0 °C"),
            ),
            (
                "Schedule base: 18.0 °C",
                format!("Base de la programmation{n} 18,0 °C"),
            ),
            (
                "No one in the room for 30 min: 18.0 °C (19.0 − 1.0)",
                format!("Personne dans la pièce depuis 30 min{n} 18,0 °C (19,0 − 1,0)"),
            ),
            (
                "No one in the room for 30 min: 26.0 °C (base)",
                format!("Personne dans la pièce depuis 30 min{n} 26,0 °C (base)"),
            ),
            (
                "No one in the room for 30 min: 7.0 °C (frost)",
                format!("Personne dans la pièce depuis 30 min{n} 7,0 °C (antigel)"),
            ),
            (
                "trv2 held until 11:00",
                "trv2 maintenu jusqu'à 11:00".into(),
            ),
            (
                "trv held as it is until 11:00",
                "trv laissé tel quel jusqu'à 11:00".into(),
            ),
            (
                "the room thermometer has been silent for over 6 h, using the devices' own readings",
                "le thermomètre de la pièce est muet depuis plus de 6 h, mesures des appareils utilisées"
                    .into(),
            ),
            (
                "the room thermometer has been silent for over 6 h 10 min, using the devices' own readings",
                "le thermomètre de la pièce est muet depuis plus de 6 h 10 min, mesures des appareils utilisées"
                    .into(),
            ),
            (
                "room above 19.5 °C: AC 18.5 °C (-0.5)",
                format!("pièce au-dessus de 19,5 °C{n} clim 18,5 °C (-0,5)"),
            ),
            (
                "room below 20.5 °C: AC 21.0 °C",
                format!("pièce en dessous de 20,5 °C{n} clim 21,0 °C"),
            ),
            (
                "room near 19.0 °C: AC 17.5 °C (-1.5)",
                format!("pièce proche de 19,0 °C{n} clim 17,5 °C (-1,5)"),
            ),
            (
                "room below 20.5 °C: AC 21.5 °C (+0.5)",
                format!("pièce en dessous de 20,5 °C{n} clim 21,5 °C (+0,5)"),
            ),
            (
                "no room thermometer",
                "pas de thermomètre dans la pièce".into(),
            ),
            (
                "room thermometer has not reported: no correction",
                format!("le thermomètre de la pièce n'a encore rien envoyé{n} aucune correction"),
            ),
            (
                "room thermometer silent: no correction",
                format!("thermomètre de la pièce muet{n} aucune correction"),
            ),
            (
                "all units held: correction paused",
                format!("tous les appareils sont maintenus{n} correction en pause"),
            ),
        ];
        for (raw, want) in cases {
            assert_eq!(reason_text(Lang::Fr, raw), want, "{raw}");
        }
    }

    #[test]
    fn reason_english_passes_through() {
        let raw = "Schedule block 2: 19.0 °C; trv2 held until 11:00";
        assert_eq!(reason_text(Lang::En, raw), raw);
    }

    #[test]
    fn reason_unknown_segments_stay_as_they_are() {
        assert_eq!(
            reason_text(Lang::Fr, "Something new: 1.5 x"),
            "Something new: 1.5 x"
        );
        assert_eq!(reason_text(Lang::Fr, ""), "");
        assert_eq!(
            reason_text(Lang::Fr, "Schedule block x: 19.0 °C"),
            "Schedule block x: 19.0 °C"
        );
        assert_eq!(
            reason_text(Lang::Fr, "Zone is off; whatever"),
            "Pièce arrêtée; whatever"
        );
    }

    #[test]
    fn reason_segments_rejoin_with_semicolons() {
        assert_eq!(
            reason_text(
                Lang::Fr,
                "Schedule block 1: 19.0 °C; room above 19.5 °C: AC 18.5 °C (-0.5); trv held until 11:00"
            ),
            "Plage 1 de la programmation\u{a0}: 19,0 °C; pièce au-dessus de 19,5 °C\u{a0}: clim 18,5 °C (-0,5); trv maintenu jusqu'à 11:00"
        );
    }

    #[test]
    fn reason_colons_take_a_non_breaking_space() {
        let out = reason_text(Lang::Fr, "Holiday: 7.0 °C");
        assert!(out.contains("\u{a0}:"));
        assert!(!out.contains(" :"));
        // The clock keeps its bare colon.
        assert_eq!(
            reason_text(Lang::Fr, "Boost to 22.0 °C until 11:00"),
            "Boost à 22,0 °C jusqu'à 11:00"
        );
    }
}
