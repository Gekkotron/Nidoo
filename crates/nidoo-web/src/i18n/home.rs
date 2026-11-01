//! FR/EN text: Home header: date, clock, greeting, mood, chips and the home settings panel.

use chrono::{Datelike, NaiveDate, NaiveTime, Timelike};

use crate::home_header::mood::{Activity, Detail, Greeting, TempBand};
use crate::home_header::summary::HeatingChip;

use super::*;

/// The long date, weekday first: "Vendredi 23 octobre" / "Friday 23 October".
pub fn home_date(lang: Lang, date: NaiveDate) -> String {
    format!(
        "{} {} {}",
        format::weekday_long(lang, date.weekday()),
        date.day(),
        format::month_long(lang, date.month())
    )
}

/// A clock time: "18 h 42" / "18:42".
pub fn home_time(lang: Lang, t: NaiveTime) -> String {
    match lang {
        Lang::Fr => format!("{} h {:02}", t.hour(), t.minute()),
        Lang::En => format::time(t),
    }
}

/// A number with `decimals` places and the language's decimal separator.
pub fn home_number(lang: Lang, v: f64, decimals: usize) -> String {
    format::decimal(lang, v, decimals)
}

/// The greeting for a band of the day.
pub fn greeting_text(lang: Lang, g: Greeting) -> &'static str {
    match (lang, g) {
        (Lang::Fr, Greeting::Morning) => "Bonjour",
        (Lang::Fr, Greeting::Afternoon) => "Bon après-midi",
        (Lang::Fr, Greeting::Evening) => "Bonsoir",
        (Lang::En, Greeting::Morning) => "Good morning",
        (Lang::En, Greeting::Afternoon) => "Good afternoon",
        (Lang::En, Greeting::Evening) => "Good evening",
    }
}

/// The mood sentence, ending with a full stop; the temperature half is left out without a band.
pub fn mood_sentence(lang: Lang, a: &Activity, band: Option<TempBand>) -> String {
    let fr = lang == Lang::Fr;
    let activity = match a {
        Activity::Attention if fr => "Quelque chose demande ton attention".to_owned(),
        Activity::Attention => "Something needs your attention".to_owned(),
        Activity::Open(o) => {
            let label = if fr { "Ouverture" } else { "Open" };
            let space = if fr { " : " } else { ": " };
            match &o.room {
                Some(room) => format!("{label}{space}{} ({room})", o.title),
                None => format!("{label}{space}{}", o.title),
            }
        }
        Activity::OpenMany(n) if fr => format!("{n} ouvertures"),
        Activity::OpenMany(n) => format!("{n} doors or windows open"),
        Activity::Asleep if fr => "La maison dort".to_owned(),
        Activity::Asleep => "The house is asleep".to_owned(),
        Activity::Quiet if fr => "La maison est calme".to_owned(),
        Activity::Quiet => "The house is quiet".to_owned(),
        Activity::Awake if fr => "La maison est éveillée".to_owned(),
        Activity::Awake => "The house is awake".to_owned(),
    };
    let temperature = match (lang, band) {
        (_, None) => "",
        (Lang::Fr, Some(TempBand::Cold)) => ", il fait froid",
        (Lang::Fr, Some(TempBand::Comfortable)) => ", il fait bon",
        (Lang::Fr, Some(TempBand::Warm)) => ", il fait chaud",
        (Lang::En, Some(TempBand::Cold)) => ", it's cold",
        (Lang::En, Some(TempBand::Comfortable)) => ", it's comfortable",
        (Lang::En, Some(TempBand::Warm)) => ", it's warm",
    };
    format!("{activity}{temperature}.")
}

/// One fragment of the detail line.
pub fn detail_text(lang: Lang, d: &Detail) -> String {
    match (lang, d) {
        (Lang::Fr, Detail::RainSoon) => "Pluie attendue dans l'heure".to_owned(),
        (Lang::En, Detail::RainSoon) => "Rain expected within the hour".to_owned(),
        (Lang::Fr, Detail::FrostTonight) => "Gel cette nuit".to_owned(),
        (Lang::En, Detail::FrostTonight) => "Frost tonight".to_owned(),
        (Lang::Fr, Detail::Sunset(at)) => format!("Coucher du soleil à {}", home_time(lang, *at)),
        (Lang::En, Detail::Sunset(at)) => format!("Sunset at {}", home_time(lang, *at)),
        (Lang::Fr, Detail::SunsetFallback) => {
            "Coucher du soleil : position de la maison non réglée".to_owned()
        }
        (Lang::En, Detail::SunsetFallback) => "Sunset: house location not set".to_owned(),
    }
}

/// The lights chip: "3 lights on" / "3 lumières allumées"; `short` drops the "on" part.
pub fn lights_chip(lang: Lang, n: usize, short: bool) -> String {
    let (one, other) = if short {
        (("light", "lumière"), ("lights", "lumières"))
    } else {
        (
            ("light on", "lumière allumée"),
            ("lights on", "lumières allumées"),
        )
    };
    crate::i18n::format::count(lang, n, one, other)
}

/// The indoor chip: "19.6 °C inside" / "19,6 °C à l'intérieur"; `short` is the temperature alone.
pub fn indoor_chip(lang: Lang, t: f64, short: bool) -> String {
    let value = format!("{} °C", home_number(lang, t, 1));
    match (lang, short) {
        (_, true) => value,
        (Lang::Fr, false) => format!("{value} à l'intérieur"),
        (Lang::En, false) => format!("{value} inside"),
    }
}

/// The heating chip; `short` is one word.
pub fn heating_chip_text(lang: Lang, h: HeatingChip, short: bool) -> &'static str {
    match (lang, h, short) {
        (Lang::Fr, HeatingChip::Off, false) => "Chauffage éteint",
        (Lang::Fr, HeatingChip::Off, true) => "Éteint",
        (Lang::Fr, HeatingChip::Eco, false) => "Chauffage en éco",
        (Lang::Fr, HeatingChip::Eco, true) => "Éco",
        (Lang::Fr, HeatingChip::Cooling, false) => "Clim en marche",
        (Lang::Fr, HeatingChip::Cooling, true) => "Clim",
        (Lang::En, HeatingChip::Off, false) => "Heating off",
        (Lang::En, HeatingChip::Off, true) => "Off",
        (Lang::En, HeatingChip::Eco, false) => "Heating in eco",
        (Lang::En, HeatingChip::Eco, true) => "Eco",
        (Lang::En, HeatingChip::Cooling, false) => "Air conditioning on",
        (Lang::En, HeatingChip::Cooling, true) => "A/C",
    }
}

/// The accessible name of the house mode menu.
pub fn house_mode_label(lang: Lang) -> &'static str {
    match lang {
        Lang::Fr => "Mode de la maison",
        Lang::En => "House mode",
    }
}

/// The weather card's accessible name.
pub fn weather_link_label(lang: Lang) -> &'static str {
    match lang {
        Lang::Fr => "Voir la météo",
        Lang::En => "Open the weather",
    }
}

/// The fixed words of the Home panel in Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HomeText {
    Title,
    Hint,
    EcoSetpoint,
    ModeSelector,
    IndoorSensor,
    ColdBelow,
    HotAbove,
    NoneChoice,
    AutoMean,
    EcoRange,
    ThresholdRange,
    OrderRule,
    EntityRule,
    Save,
    Saved,
    NotANumber,
    CouldNotSave,
    OffPeakHours,
    OffPeakHint,
    AddWindow,
    RemoveWindow,
    BadTime,
    StartEqualsEnd,
    WindowsOverlap,
    OffPeakDevice,
    OffPeakDeviceHint,
    OffPeakValues,
    OffPeakAddValue,
    OffPeakBackup,
    OffPeakClearDevice,
}

/// A fixed word of the Home panel.
pub fn home_text(lang: Lang, k: HomeText) -> &'static str {
    use HomeText::*;
    let (en, fr) = match k {
        Title => ("Home", "Maison"),
        Hint => (
            "What the home header says and when it calls it cold or hot.",
            "Ce que dit l'en-tête de la maison et quand il parle de froid ou de chaud.",
        ),
        EcoSetpoint => ("Eco setpoint (°C)", "Consigne éco (°C)"),
        ModeSelector => ("Mode selector", "Sélecteur de mode"),
        IndoorSensor => ("Indoor sensor", "Capteur intérieur"),
        ColdBelow => ("Cold below (°C)", "Froid en dessous de (°C)"),
        HotAbove => ("Hot above (°C)", "Chaud au-dessus de (°C)"),
        NoneChoice => ("None", "Aucun"),
        AutoMean => (
            "Automatic (median of rooms)",
            "Automatique (médiane des pièces)",
        ),
        EcoRange => ("Between 5 and 30 °C", "Entre 5 et 30 °C"),
        ThresholdRange => ("Between 5 and 35 °C", "Entre 5 et 35 °C"),
        OrderRule => (
            "Must be lower than the warm threshold",
            "Doit être inférieur au seuil chaud",
        ),
        EntityRule => (
            "Entity missing or of another type",
            "Entité introuvable ou d'un autre type",
        ),
        Save => ("Save", "Enregistrer"),
        Saved => ("Saved", "Enregistré"),
        NotANumber => ("Enter a number", "Saisissez un nombre"),
        CouldNotSave => ("Could not save", "Enregistrement impossible"),
        OffPeakHours => ("Off-peak hours", "Heures creuses"),
        OffPeakHint => (
            "From your electricity contract. The Energy chart splits what you buy by these hours; past days keep the hours they had.",
            "Celles de votre contrat d'électricité. Le graphique Énergie sépare vos achats selon ces heures ; les jours passés gardent les leurs.",
        ),
        AddWindow => ("Add a period", "Ajouter une plage"),
        OffPeakDevice => ("Follow a device", "Suivre un appareil"),
        OffPeakDeviceHint => (
            "Pick the entity that reports the tariff, then the values that mean off-peak.",
            "Choisissez l'entité qui indique le tarif, puis les valeurs qui signifient heures creuses.",
        ),
        OffPeakValues => ("Off-peak values", "Valeurs heures creuses"),
        OffPeakAddValue => ("Add a value", "Ajouter une valeur"),
        OffPeakBackup => (
            "Backup windows — used when the device has no reading",
            "Plages de secours — utilisées quand l'appareil ne donne rien",
        ),
        OffPeakClearDevice => ("Remove device", "Retirer l'appareil"),
        RemoveWindow => ("Remove", "Retirer"),
        BadTime => ("Enter a time as HH:MM", "Saisissez une heure HH:MM"),
        StartEqualsEnd => (
            "A period must not start and end at the same time",
            "Une plage ne peut pas commencer et finir à la même heure",
        ),
        WindowsOverlap => ("Periods overlap", "Des plages se chevauchent"),
    };
    if lang == Lang::Fr { fr } else { en }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::home_header::summary::Opening;

    #[test]
    fn lights_chip_agrees_at_zero_and_one() {
        assert_eq!(lights_chip(Lang::Fr, 0, false), "0 lumière allumée");
        assert_eq!(lights_chip(Lang::Fr, 1, true), "1 lumière");
        assert_eq!(lights_chip(Lang::Fr, 2, false), "2 lumières allumées");
        assert_eq!(lights_chip(Lang::En, 0, true), "0 lights");
        assert_eq!(lights_chip(Lang::En, 1, false), "1 light on");
    }

    #[test]
    fn home_panel_copy_has_both_languages() {
        use HomeText::*;
        for k in [
            Title,
            Hint,
            EcoSetpoint,
            ModeSelector,
            IndoorSensor,
            ColdBelow,
            HotAbove,
            NoneChoice,
            AutoMean,
            EcoRange,
            ThresholdRange,
            OrderRule,
            EntityRule,
            Save,
            Saved,
            NotANumber,
            CouldNotSave,
        ] {
            assert!(!home_text(Lang::En, k).is_empty() && !home_text(Lang::Fr, k).is_empty());
        }
        assert_eq!(home_text(Lang::Fr, Title), "Maison");
    }
    fn window(room: Option<&str>) -> Activity {
        Activity::Open(Opening {
            title: "Fenêtre".into(),
            room: room.map(Into::into),
        })
    }
    #[test]
    fn mood_sentence_fr_en() {
        use TempBand::*;
        let fr = |a: &Activity, b| mood_sentence(Lang::Fr, a, Some(b));
        assert_eq!(
            fr(&Activity::Quiet, Comfortable),
            "La maison est calme, il fait bon."
        );
        assert_eq!(
            fr(&Activity::Asleep, Cold),
            "La maison dort, il fait froid."
        );
        assert_eq!(
            fr(&window(Some("Cuisine")), Cold),
            "Ouverture : Fenêtre (Cuisine), il fait froid."
        );
        assert_eq!(
            fr(&window(None), Cold),
            "Ouverture : Fenêtre, il fait froid."
        );
        assert_eq!(
            fr(&Activity::OpenMany(2), Warm),
            "2 ouvertures, il fait chaud."
        );
        assert_eq!(
            fr(&Activity::Attention, Comfortable),
            "Quelque chose demande ton attention, il fait bon."
        );
        assert_eq!(
            fr(&Activity::Awake, Warm),
            "La maison est éveillée, il fait chaud."
        );
        let en = |a: &Activity, b| mood_sentence(Lang::En, a, Some(b));
        assert_eq!(
            en(&Activity::Quiet, Comfortable),
            "The house is quiet, it's comfortable."
        );
        assert_eq!(
            en(&Activity::Asleep, Cold),
            "The house is asleep, it's cold."
        );
        assert_eq!(en(&Activity::Awake, Warm), "The house is awake, it's warm.");
        assert_eq!(
            en(&Activity::OpenMany(2), Cold),
            "2 doors or windows open, it's cold."
        );
        assert_eq!(
            en(&window(Some("Kitchen")), Cold),
            "Open: Fenêtre (Kitchen), it's cold."
        );
        assert_eq!(
            en(&Activity::Attention, Cold),
            "Something needs your attention, it's cold."
        );
    }
    #[test]
    fn sentence_without_temperature() {
        assert_eq!(
            mood_sentence(Lang::Fr, &Activity::Quiet, None),
            "La maison est calme."
        );
        assert_eq!(
            mood_sentence(Lang::En, &Activity::Quiet, None),
            "The house is quiet."
        );
    }
    #[test]
    fn home_date_fr_en() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 23).unwrap();
        assert_eq!(home_date(Lang::Fr, d), "Vendredi 23 octobre");
        assert_eq!(home_date(Lang::En, d), "Friday 23 October");
    }
    #[test]
    fn home_time_fr_en() {
        let t = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
        assert_eq!(home_time(Lang::Fr, t(18, 42)), "18 h 42");
        assert_eq!(home_time(Lang::En, t(18, 42)), "18:42");
        assert_eq!(home_time(Lang::Fr, t(9, 5)), "9 h 05");
        assert_eq!(home_time(Lang::En, t(0, 7)), "00:07");
        assert_eq!(home_time(Lang::En, t(12, 0)), "12:00");
    }
    #[test]
    fn home_number_follows_language() {
        assert_eq!(home_number(Lang::Fr, 19.64, 1), "19,6");
        assert_eq!(home_number(Lang::En, 19.64, 1), "19.6");
        assert_eq!(home_number(Lang::Fr, 5.0, 0), "5");
    }
    #[test]
    fn greetings_fr_en() {
        assert_eq!(greeting_text(Lang::Fr, Greeting::Morning), "Bonjour");
        assert_eq!(
            greeting_text(Lang::Fr, Greeting::Afternoon),
            "Bon après-midi"
        );
        assert_eq!(greeting_text(Lang::Fr, Greeting::Evening), "Bonsoir");
        assert_eq!(greeting_text(Lang::En, Greeting::Morning), "Good morning");
        assert_eq!(
            greeting_text(Lang::En, Greeting::Afternoon),
            "Good afternoon"
        );
        assert_eq!(greeting_text(Lang::En, Greeting::Evening), "Good evening");
    }
    #[test]
    fn detail_text_fr() {
        let sunset = NaiveTime::from_hms_opt(18, 42, 0).unwrap();
        assert_eq!(
            detail_text(Lang::Fr, &Detail::RainSoon),
            "Pluie attendue dans l'heure"
        );
        assert_eq!(
            detail_text(Lang::Fr, &Detail::FrostTonight),
            "Gel cette nuit"
        );
        assert_eq!(
            detail_text(Lang::Fr, &Detail::Sunset(sunset)),
            "Coucher du soleil à 18 h 42"
        );
        assert_eq!(
            detail_text(Lang::Fr, &Detail::SunsetFallback),
            "Coucher du soleil : position de la maison non réglée"
        );
    }
    #[test]
    fn detail_text_en() {
        let sunset = NaiveTime::from_hms_opt(18, 42, 0).unwrap();
        assert_eq!(
            detail_text(Lang::En, &Detail::RainSoon),
            "Rain expected within the hour"
        );
        assert_eq!(
            detail_text(Lang::En, &Detail::FrostTonight),
            "Frost tonight"
        );
        assert_eq!(
            detail_text(Lang::En, &Detail::Sunset(sunset)),
            "Sunset at 18:42"
        );
        assert_eq!(
            detail_text(Lang::En, &Detail::SunsetFallback),
            "Sunset: house location not set"
        );
    }
    #[test]
    fn every_home_text_has_two_languages() {
        const ALL: &[HomeText] = &[
            HomeText::Title,
            HomeText::Hint,
            HomeText::EcoSetpoint,
            HomeText::ModeSelector,
            HomeText::IndoorSensor,
            HomeText::ColdBelow,
            HomeText::HotAbove,
            HomeText::NoneChoice,
            HomeText::AutoMean,
            HomeText::EcoRange,
            HomeText::ThresholdRange,
            HomeText::OrderRule,
            HomeText::EntityRule,
            HomeText::Save,
            HomeText::Saved,
            HomeText::NotANumber,
            HomeText::CouldNotSave,
            HomeText::OffPeakHours,
            HomeText::OffPeakHint,
            HomeText::AddWindow,
            HomeText::RemoveWindow,
            HomeText::BadTime,
            HomeText::StartEqualsEnd,
            HomeText::WindowsOverlap,
            HomeText::OffPeakDevice,
            HomeText::OffPeakDeviceHint,
            HomeText::OffPeakValues,
            HomeText::OffPeakAddValue,
            HomeText::OffPeakBackup,
            HomeText::OffPeakClearDevice,
        ];
        crate::i18n::assert_table(ALL, home_text, &[]);
    }
}
