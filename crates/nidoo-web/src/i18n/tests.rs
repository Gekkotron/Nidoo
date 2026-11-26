use super::*;
#[test]
fn every_key_has_both_languages() {
    for &k in Key::ALL {
        assert!(
            !text(Lang::En, k).is_empty() && !text(Lang::Fr, k).is_empty(),
            "{k:?}"
        );
    }
}
#[test]
fn browser_language_picks_french_only_for_fr() {
    assert_eq!(from_browser("fr-FR"), Lang::Fr);
    assert_eq!(from_browser("FR"), Lang::Fr);
    assert_eq!(from_browser("en-US"), Lang::En);
    assert_eq!(from_browser("de-DE"), Lang::En);
    assert_eq!(from_browser(""), Lang::En);
}
#[test]
fn stored_values_parse() {
    assert_eq!(parse(Some("fr")), Some(Lang::Fr));
    assert_eq!(parse(Some("en")), Some(Lang::En));
    assert_eq!(parse(Some("xx")), None);
    assert_eq!(parse(None), None);
}
#[test]
fn shell_copy() {
    assert_eq!(text(Lang::Fr, Key::Overview), "Vue d'ensemble");
    assert_eq!(text(Lang::Fr, Key::Online), "La maison est connectée");
    assert_eq!(unseen_alerts(Lang::Fr, "3"), "3 alertes non lues");
    assert_eq!(unseen_alerts(Lang::En, "3"), "3 unseen alerts");
    assert_eq!(unseen_alerts(Lang::En, "1"), "1 unseen alert");
    assert_eq!(unseen_alerts(Lang::Fr, "1"), "1 alerte non lue");
    assert_eq!(unseen_alerts(Lang::En, "99+"), "99+ unseen alerts");
    assert_eq!(unseen_alerts(Lang::Fr, "99+"), "99+ alertes non lues");
}
#[test]
fn toggles() {
    assert_eq!(Lang::En.next(), Lang::Fr);
    assert_eq!(Lang::Fr.next(), Lang::En);
    assert_eq!(Lang::Fr.as_str(), "fr");
}
/// Adding a `Key` variant without listing it here fails to compile.
fn listed(k: Key) -> bool {
    match k {
        Key::MyHome
        | Key::Overview
        | Key::House
        | Key::Automations
        | Key::Groups
        | Key::MyRooms
        | Key::HomeSection
        | Key::Location
        | Key::Database
        | Key::Energy
        | Key::Security
        | Key::Climate
        | Key::Weather
        | Key::SolarForecast
        | Key::Cameras
        | Key::PushNotifications
        | Key::MobileApp
        | Key::Solar
        | Key::Beta
        | Key::Ambiances
        | Key::NewAmbiance
        | Key::AmbianceMissing
        | Key::Edit
        | Key::Delete
        | Key::MoveUp
        | Key::MoveDown
        | Key::AmbianceApply
        | Key::AmbianceShowOnHome
        | Key::AmbianceOnlyFour
        | Key::AmbianceEmpty
        | Key::AmbianceAddExamples
        | Key::AmbiancesRowTitle
        | Key::AmbianceInvite
        | Key::AmbianceChoose
        | Key::AmbianceAddLamp
        | Key::AmbianceLastRun
        | Key::AmbianceNoReport
        | Key::OutcomeDone
        | Key::OutcomeFailed
        | Key::OutcomeSkipped
        | Key::AmbiancesHowItWorks
        | Key::AmbiancesAbout1
        | Key::AmbiancesAbout2
        | Key::AmbiancesAbout3
        | Key::AmbName
        | Key::AmbSubtitle
        | Key::AmbIcon
        | Key::AmbActions
        | Key::AmbAddAction
        | Key::AmbCategoryType
        | Key::AmbDeviceType
        | Key::AmbHeatingType
        | Key::AmbAutomationType
        | Key::AmbWhere
        | Key::AmbWholeHouse
        | Key::Unassigned
        | Key::AmbSeeWhich
        | Key::AmbHold
        | Key::AmbBackToSchedule
        | Key::AmbHours
        | Key::AmbEveryZone
        | Key::AmbSwitchOn
        | Key::AmbSwitchOff
        | Key::AmbRun
        | Key::AmbSave
        | Key::AmbTryIt
        | Key::AmbSaveFirst
        | Key::AmbWhat
        | Key::AmbTo
        | Key::AmbLights
        | Key::AmbPlugs
        | Key::AmbCovers
        | Key::AmbOpen
        | Key::AmbClose
        | Key::AmbTemperature
        | Key::AmbZone
        | Key::AmbCommand
        | Key::AmbValue
        | Key::Alerts
        | Key::Search
        | Key::Settings
        | Key::Devices
        | Key::Appearance
        | Key::Alarm
        | Key::WakeUp
        | Key::NetworkPresence
        | Key::AssistantVoice
        | Key::AssistantListening
        | Key::Sound
        | Key::System
        | Key::Backup
        | Key::Zigbee2mqtt
        | Key::Broker
        | Key::AiAssistants
        | Key::About
        | Key::NewDashboard
        | Key::NewFlow
        | Key::NewGroup
        | Key::EditGroup
        | Key::History
        | Key::Alert
        | Key::Online
        | Key::Reconnecting
        | Key::Offline
        | Key::OpenMenu
        | Key::CloseMenu
        | Key::ChangeTheme
        | Key::ChangeLanguage
        | Key::Navigation
        | Key::Breadcrumb
        | Key::RoomGone
        | Key::LightOn
        | Key::LightOff
        | Key::SwitchOn
        | Key::SwitchOff
        | Key::Brightness
        | Key::More
        | Key::Live
        | Key::AgoHours
        | Key::AgoDays
        | Key::ModeHeat
        | Key::ModeCool
        | Key::ModeAuto
        | Key::ModeDry
        | Key::ModeFan
        | Key::ModeOff
        | Key::Mode
        | Key::ItIs
        | Key::TargetTemperature
        | Key::Decrease
        | Key::Increase
        | Key::VacDocked
        | Key::VacCleaning
        | Key::VacReturning
        | Key::VacPaused
        | Key::VacIdle
        | Key::VacError
        | Key::VacOnDock
        | Key::VacRunning
        | Key::VacFan
        | Key::VacOffDock
        | Key::VacCheck
        | Key::Start
        | Key::Pause
        | Key::Resume
        | Key::SendHome
        | Key::Battery => Key::ALL.contains(&k),
    }
}
#[test]
fn all_is_complete() {
    for &k in Key::ALL {
        assert!(listed(k));
    }
    assert_eq!(Key::ALL.len(), 147);
}
#[test]
fn every_key_differs_between_languages() {
    // Words spelled the same in both languages.
    const SAME: &[Key] = &[
        Key::AmbActions,
        Key::AmbZone,
        Key::Zigbee2mqtt,
        Key::ModeAuto,
        Key::Mode,
        Key::Pause,
        Key::PushNotifications,
    ];
    crate::i18n::assert_table(Key::ALL, text, SAME);
}
