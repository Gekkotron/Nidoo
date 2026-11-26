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
