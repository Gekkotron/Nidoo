//! FR/EN text: the climate area (screen, room panel, settings sheet, week timeline, water heater).

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
}
