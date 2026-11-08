//! FR/EN strings: language choice, storage, the shell's text table, and one child module per area.

use leptos::prelude::*;

mod alarm;
mod ambiances;
mod assist_settings;
mod assistant;
pub mod climate;
mod dashboards;
pub mod devices;
pub mod energy;
pub mod errors;
pub mod flows;
pub mod format;
pub mod groups;
pub mod history;
mod home;
mod mobile;
mod network;
pub mod picker;
mod search;
pub mod security;
pub mod settings;
pub mod setup;
mod solar;
mod sound;
mod speech_input;
mod speech_languages;
pub mod states;
mod system;
mod weather;

pub use alarm::*;
pub use ambiances::*;
pub use assist_settings::*;
pub use assistant::*;
pub use dashboards::*;
pub use home::*;
pub use mobile::*;
pub use network::*;
pub use search::*;
pub use solar::*;
pub use sound::*;
pub use speech_input::*;
pub use speech_languages::*;
pub use system::*;
pub use weather::*;

/// Checks a text table: every entry has both languages, and they differ unless listed in `same`.
#[cfg(test)]
pub(crate) fn assert_table<K: Copy + PartialEq + std::fmt::Debug>(
    all: &[K],
    text: impl Fn(Lang, K) -> &'static str,
    same: &[K],
) {
    let bad: Vec<String> = all
        .iter()
        .filter_map(|&k| {
            let (en, fr) = (text(Lang::En, k), text(Lang::Fr, k));
            if en.is_empty() || fr.is_empty() {
                Some(format!("{k:?} is empty in a language"))
            } else if en == fr && !same.contains(&k) {
                Some(format!("{k:?} is \"{en}\" in both languages"))
            } else {
                None
            }
        })
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

/// Storage key for the persisted choice.
#[cfg(target_arch = "wasm32")]
const STORAGE_KEY: &str = "nidoo.lang";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
}

impl Lang {
    /// The same language as the shared flow code names it.
    pub fn core(self) -> nidoo_core::flow::Lang {
        match self {
            Lang::En => nidoo_core::flow::Lang::En,
            Lang::Fr => nidoo_core::flow::Lang::Fr,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Fr => "fr",
        }
    }

    /// The other language, for the toggle.
    pub fn next(self) -> Lang {
        match self {
            Lang::En => Lang::Fr,
            Lang::Fr => Lang::En,
        }
    }
}

/// Reads a stored value; `None` when missing or unknown.
pub fn parse(value: Option<&str>) -> Option<Lang> {
    match value {
        Some("en") => Some(Lang::En),
        Some("fr") => Some(Lang::Fr),
        _ => None,
    }
}

/// French for any `fr*` browser tag (case-insensitive), English otherwise.
pub fn from_browser(tag: &str) -> Lang {
    if tag.to_ascii_lowercase().starts_with("fr") {
        Lang::Fr
    } else {
        Lang::En
    }
}

/// `localStorage`, or `None` when it is missing, blocked or throws.
#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

/// Loads the saved language (else the browser's), keeps `<html lang>` and storage in sync with the
/// returned signal. Every storage access is fallible and silent: a private window must never panic.
pub fn install_lang() -> RwSignal<Lang> {
    #[cfg(target_arch = "wasm32")]
    let initial = parse(
        storage()
            .and_then(|s| s.get_item(STORAGE_KEY).ok().flatten())
            .as_deref(),
    )
    .unwrap_or_else(|| {
        from_browser(
            &web_sys::window()
                .and_then(|w| w.navigator().language())
                .unwrap_or_default(),
        )
    });
    #[cfg(not(target_arch = "wasm32"))]
    let initial = Lang::En;

    let lang = RwSignal::new(initial);

    #[cfg(target_arch = "wasm32")]
    Effect::new(move |_| {
        let current = lang.get();
        remember(current);
        if let Some(root) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.document_element())
        {
            let _ = root.set_attribute("lang", current.as_str());
        }
        if let Some(storage) = storage() {
            let _ = storage.set_item(STORAGE_KEY, current.as_str());
        }
    });

    lang
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    MyHome,
    Overview,
    House,
    Automations,
    Groups,
    MyRooms,
    HomeSection,
    Location,
    Database,
    Energy,
    Security,
    Climate,
    Weather,
    SolarForecast,
    Cameras,
    PushNotifications,
    Solar,
    Beta,
    Ambiances,
    NewAmbiance,
    AmbianceMissing,
    Edit,
    Delete,
    MoveUp,
    MoveDown,
    AmbianceApply,
    AmbianceShowOnHome,
    AmbianceOnlyFour,
    AmbianceEmpty,
    AmbianceAddExamples,
    AmbiancesRowTitle,
    AmbianceInvite,
    AmbianceChoose,
    AmbianceAddLamp,
    AmbianceLastRun,
    AmbianceNoReport,
    OutcomeDone,
    OutcomeFailed,
    OutcomeSkipped,
    AmbiancesHowItWorks,
    AmbiancesAbout1,
    AmbiancesAbout2,
    AmbiancesAbout3,
    AmbName,
    AmbSubtitle,
    AmbIcon,
    AmbActions,
    AmbAddAction,
    AmbCategoryType,
    AmbDeviceType,
    AmbHeatingType,
    AmbAutomationType,
    AmbWhere,
    AmbWholeHouse,
    Unassigned,
    AmbSeeWhich,
    AmbHold,
    AmbBackToSchedule,
    AmbHours,
    AmbEveryZone,
    AmbSwitchOn,
    AmbSwitchOff,
    AmbRun,
    AmbSave,
    AmbTryIt,
    AmbSaveFirst,
    AmbWhat,
    AmbTo,
    AmbLights,
    AmbPlugs,
    AmbCovers,
    AmbOpen,
    AmbClose,
    AmbTemperature,
    AmbZone,
    AmbCommand,
    AmbValue,
    Alerts,
    Search,
    Settings,
    Devices,
    Appearance,
    Alarm,
    NetworkPresence,
    AssistantVoice,
    AssistantListening,
    Sound,
    System,
    Zigbee2mqtt,
    Broker,
    AiAssistants,
    About,
    NewDashboard,
    NewFlow,
    NewGroup,
    EditGroup,
    History,
    Alert,
    Online,
    Reconnecting,
    Offline,
    OpenMenu,
    CloseMenu,
    ChangeTheme,
    /// Names the language you switch TO.
    ChangeLanguage,
    Navigation,
    Breadcrumb,
    RoomGone,
    LightOn,
    LightOff,
    SwitchOn,
    SwitchOff,
    Brightness,
    More,
    Live,
    AgoHours,
    AgoDays,
    ModeHeat,
    ModeCool,
    ModeAuto,
    ModeDry,
    ModeFan,
    ModeOff,
    Mode,
    ItIs,
    TargetTemperature,
    Decrease,
    Increase,
    VacDocked,
    VacCleaning,
    VacReturning,
    VacPaused,
    VacIdle,
    VacError,
    VacOnDock,
    VacRunning,
    VacFan,
    VacOffDock,
    VacCheck,
    Start,
    Pause,
    Resume,
    SendHome,
    Battery,
}

impl Key {
    pub const ALL: &'static [Key] = &[
        Key::MyHome,
        Key::Overview,
        Key::House,
        Key::Automations,
        Key::Groups,
        Key::MyRooms,
        Key::HomeSection,
        Key::Location,
        Key::Database,
        Key::Energy,
        Key::Security,
        Key::Climate,
        Key::Weather,
        Key::SolarForecast,
        Key::Cameras,
        Key::PushNotifications,
        Key::Solar,
        Key::Beta,
        Key::Ambiances,
        Key::NewAmbiance,
        Key::AmbianceMissing,
        Key::Edit,
        Key::Delete,
        Key::MoveUp,
        Key::MoveDown,
        Key::AmbianceApply,
        Key::AmbianceShowOnHome,
        Key::AmbianceOnlyFour,
        Key::AmbianceEmpty,
        Key::AmbianceAddExamples,
        Key::AmbiancesRowTitle,
        Key::AmbianceInvite,
        Key::AmbianceChoose,
        Key::AmbianceAddLamp,
        Key::AmbianceLastRun,
        Key::AmbianceNoReport,
        Key::OutcomeDone,
        Key::OutcomeFailed,
        Key::OutcomeSkipped,
        Key::AmbiancesHowItWorks,
        Key::AmbiancesAbout1,
        Key::AmbiancesAbout2,
        Key::AmbiancesAbout3,
        Key::AmbName,
        Key::AmbSubtitle,
        Key::AmbIcon,
        Key::AmbActions,
        Key::AmbAddAction,
        Key::AmbCategoryType,
        Key::AmbDeviceType,
        Key::AmbHeatingType,
        Key::AmbAutomationType,
        Key::AmbWhere,
        Key::AmbWholeHouse,
        Key::Unassigned,
        Key::AmbSeeWhich,
        Key::AmbHold,
        Key::AmbBackToSchedule,
        Key::AmbHours,
        Key::AmbEveryZone,
        Key::AmbSwitchOn,
        Key::AmbSwitchOff,
        Key::AmbRun,
        Key::AmbSave,
        Key::AmbTryIt,
        Key::AmbSaveFirst,
        Key::AmbWhat,
        Key::AmbTo,
        Key::AmbLights,
        Key::AmbPlugs,
        Key::AmbCovers,
        Key::AmbOpen,
        Key::AmbClose,
        Key::AmbTemperature,
        Key::AmbZone,
        Key::AmbCommand,
        Key::AmbValue,
        Key::Alerts,
        Key::Search,
        Key::Settings,
        Key::Devices,
        Key::Appearance,
        Key::Alarm,
        Key::NetworkPresence,
        Key::AssistantVoice,
        Key::AssistantListening,
        Key::Sound,
        Key::System,
        Key::Zigbee2mqtt,
        Key::Broker,
        Key::AiAssistants,
        Key::About,
        Key::NewDashboard,
        Key::NewFlow,
        Key::NewGroup,
        Key::EditGroup,
        Key::History,
        Key::Alert,
        Key::Online,
        Key::Reconnecting,
        Key::Offline,
        Key::OpenMenu,
        Key::CloseMenu,
        Key::ChangeTheme,
        Key::ChangeLanguage,
        Key::Navigation,
        Key::Breadcrumb,
        Key::RoomGone,
        Key::LightOn,
        Key::LightOff,
        Key::SwitchOn,
        Key::SwitchOff,
        Key::Brightness,
        Key::More,
        Key::Live,
        Key::AgoHours,
        Key::AgoDays,
        Key::ModeHeat,
        Key::ModeCool,
        Key::ModeAuto,
        Key::ModeDry,
        Key::ModeFan,
        Key::ModeOff,
        Key::Mode,
        Key::ItIs,
        Key::TargetTemperature,
        Key::Decrease,
        Key::Increase,
        Key::VacDocked,
        Key::VacCleaning,
        Key::VacReturning,
        Key::VacPaused,
        Key::VacIdle,
        Key::VacError,
        Key::VacOnDock,
        Key::VacRunning,
        Key::VacFan,
        Key::VacOffDock,
        Key::VacCheck,
        Key::Start,
        Key::Pause,
        Key::Resume,
        Key::SendHome,
        Key::Battery,
    ];
}

/// The text for a key in a language.
pub fn text(lang: Lang, key: Key) -> &'static str {
    let (en, fr) = match key {
        Key::MyHome => ("My home", "Ma maison"),
        Key::Overview => ("Overview", "Vue d'ensemble"),
        Key::House => ("House", "Maison"),
        Key::Automations => ("Automations", "Automatisations"),
        Key::Groups => ("Groups", "Groupes"),
        Key::MyRooms => ("My rooms", "Mes pièces"),
        Key::HomeSection => ("Home", "Maison"),
        Key::Location => ("Location", "Localisation"),
        Key::Database => ("Database", "Base de données"),
        Key::Energy => ("Energy", "Énergie"),
        Key::Security => ("Security", "Sécurité"),
        Key::Climate => ("Climate", "Climat"),
        Key::Weather => ("Weather", "Météo"),
        Key::SolarForecast => ("Solar forecast", "Prévision solaire"),
        Key::Cameras => ("Cameras", "Caméras"),
        Key::PushNotifications => ("Notifications", "Notifications"),
        Key::Solar => ("Solar", "Solaire"),
        Key::Beta => ("Beta", "Bêta"),
        Key::Ambiances => ("My moods", "Mes ambiances"),
        Key::NewAmbiance => ("New mood", "Nouvelle ambiance"),
        Key::AmbianceMissing => ("(missing)", "(introuvable)"),
        Key::Edit => ("Edit", "Modifier"),
        Key::Delete => ("Delete", "Supprimer"),
        Key::MoveUp => ("Move up", "Monter"),
        Key::MoveDown => ("Move down", "Descendre"),
        Key::AmbianceApply => ("Apply", "Lancer"),
        Key::AmbianceShowOnHome => ("Show on Home", "Afficher sur l'accueil"),
        Key::AmbianceOnlyFour => (
            "Only the first 4 pinned moods show on Home.",
            "Seules les 4 premières ambiances épinglées s'affichent sur l'accueil.",
        ),
        Key::AmbianceEmpty => ("No mood yet.", "Aucune ambiance pour l'instant."),
        Key::AmbianceAddExamples => ("Add the four examples", "Ajouter les quatre exemples"),
        Key::AmbiancesRowTitle => ("One mood, one tap", "Une ambiance, un geste"),
        Key::AmbianceInvite => ("Create a mood", "Créer une ambiance"),
        Key::AmbianceChoose => ("Choose your moods", "Choisir vos ambiances"),
        Key::AmbianceAddLamp => ("Add a lamp", "Ajoutez une lampe"),
        Key::AmbianceLastRun => ("Last run", "Dernier lancement"),
        Key::AmbianceNoReport => (
            "Not run since this page was opened.",
            "Pas lancée depuis l'ouverture de la page.",
        ),
        Key::OutcomeDone => ("Done", "Fait"),
        Key::OutcomeFailed => ("Failed", "Échec"),
        Key::OutcomeSkipped => ("Skipped", "Ignoré"),
        Key::AmbiancesHowItWorks => ("How it works", "Comment ça marche"),
        Key::AmbiancesAbout1 => (
            "A mood puts the house in a known state with one tap: it runs its actions once, in order.",
            "Une ambiance met la maison dans un état connu d'un seul geste : elle lance ses actions une fois, dans l'ordre.",
        ),
        Key::AmbiancesAbout2 => (
            "\"Every light\", \"every plug\" and \"every shutter\" include devices you add later. Only switches shown as a plug count as plugs.",
            "«\u{A0}Tous les éclairages\u{A0}», «\u{A0}toutes les prises\u{A0}» et «\u{A0}tous les volets\u{A0}» incluent les appareils ajoutés plus tard. Seuls les interrupteurs affichés comme prise comptent comme des prises.",
        ),
        Key::AmbiancesAbout3 => (
            "After a tap, a message says it is done or how many devices did not respond; the last run of each mood is on its page.",
            "Après un geste, un message dit si c'est fait ou combien d'appareils n'ont pas répondu ; le dernier lancement de chaque ambiance est sur sa page.",
        ),
        Key::AmbName => ("Name", "Nom"),
        Key::AmbSubtitle => ("Subtitle", "Sous-titre"),
        Key::AmbIcon => ("Icon", "Icône"),
        Key::AmbActions => ("Actions", "Actions"),
        Key::AmbAddAction => ("Add an action", "Ajouter une action"),
        Key::AmbCategoryType => (
            "Every light, plug or shutter",
            "Éclairages, prises ou volets",
        ),
        Key::AmbDeviceType => ("One device", "Un appareil"),
        Key::AmbHeatingType => ("Heating", "Chauffage"),
        Key::AmbAutomationType => ("Automation", "Automatisation"),
        Key::AmbWhere => ("Where", "Où"),
        Key::AmbWholeHouse => ("Whole house", "Toute la maison"),
        Key::Unassigned => ("Unassigned", "Sans pièce"),
        Key::AmbSeeWhich => ("See which", "Voir lesquels"),
        Key::AmbHold => ("Hold", "Maintenir"),
        Key::AmbBackToSchedule => ("Back to the schedule", "Revenir au programme"),
        Key::AmbHours => ("Hours", "Heures"),
        Key::AmbEveryZone => ("Every zone", "Toutes les zones"),
        Key::AmbSwitchOn => ("Switch on", "Activer"),
        Key::AmbSwitchOff => ("Switch off", "Désactiver"),
        Key::AmbRun => ("Run", "Lancer"),
        Key::AmbSave => ("Save", "Enregistrer"),
        Key::AmbTryIt => ("Try it", "Essayer"),
        Key::AmbSaveFirst => ("Save first", "Enregistrez d'abord"),
        Key::AmbWhat => ("What", "Quoi"),
        Key::AmbTo => ("To", "Sur"),
        Key::AmbLights => ("Lights", "Éclairages"),
        Key::AmbPlugs => ("Plugs", "Prises"),
        Key::AmbCovers => ("Shutters", "Volets"),
        Key::AmbOpen => ("Open", "Ouvrir"),
        Key::AmbClose => ("Close", "Fermer"),
        Key::AmbTemperature => ("Temperature", "Température"),
        Key::AmbZone => ("Zone", "Zone"),
        Key::AmbCommand => ("Command", "Commande"),
        Key::AmbValue => ("Value", "Valeur"),
        Key::Alerts => ("Alerts", "Alertes"),
        Key::Search => ("Search", "Recherche"),
        Key::Settings => ("Settings", "Réglages"),
        Key::Devices => ("Devices", "Appareils"),
        Key::Appearance => ("Appearance", "Apparence"),
        Key::Alarm => ("Alarm", "Alarme"),
        Key::NetworkPresence => ("Network presence", "Présence réseau"),
        Key::AssistantVoice => ("Voice", "Voix"),
        Key::AssistantListening => ("Listening", "Écoute"),
        Key::Sound => ("Sound", "Son"),
        Key::System => ("System", "Système"),
        Key::Zigbee2mqtt => ("Zigbee2MQTT", "Zigbee2MQTT"),
        Key::Broker => ("Broker", "Broker MQTT"),
        Key::AiAssistants => ("AI assistants (MCP)", "Assistants IA (MCP)"),
        Key::About => ("About", "À propos"),
        Key::NewDashboard => ("New dashboard", "Nouveau tableau"),
        Key::NewFlow => ("New flow", "Nouvelle automatisation"),
        Key::NewGroup => ("New group", "Nouveau groupe"),
        Key::EditGroup => ("Edit group", "Modifier le groupe"),
        Key::History => ("History", "Historique"),
        Key::Alert => ("Alert", "Alerte"),
        Key::Online => ("Connected", "La maison est connectée"),
        Key::Reconnecting => ("Reconnecting…", "Reconnexion…"),
        Key::Offline => ("Offline", "Hors ligne"),
        Key::OpenMenu => ("Open menu", "Ouvrir le menu"),
        Key::CloseMenu => ("Close menu", "Fermer le menu"),
        Key::ChangeTheme => ("Change theme", "Changer de thème"),
        Key::ChangeLanguage => ("Français", "English"),
        Key::Navigation => ("Main", "Navigation principale"),
        Key::Breadcrumb => ("Breadcrumb", "Fil d'Ariane"),
        Key::RoomGone => ("This room is gone.", "Cette pièce n'existe plus."),
        Key::LightOn => ("On", "Allumé"),
        Key::LightOff => ("Off", "Éteint"),
        Key::SwitchOn => ("On", "Activé"),
        Key::SwitchOff => ("Off", "Désactivé"),
        Key::Brightness => ("Brightness", "Luminosité"),
        Key::More => ("More options", "Plus d'options"),
        Key::Live => ("Live", "En direct"),
        Key::AgoHours => ("{n} h ago", "Il y a {n} h"),
        Key::AgoDays => ("{n} d ago", "Il y a {n} j"),
        Key::ModeHeat => ("Heat", "Chauffage"),
        Key::ModeCool => ("Cool", "Clim"),
        Key::ModeAuto => ("Auto", "Auto"),
        Key::ModeDry => ("Dry", "Déshumidification"),
        Key::ModeFan => ("Fan", "Ventilation"),
        Key::ModeOff => ("Off", "Arrêt"),
        Key::Mode => ("Mode", "Mode"),
        Key::ItIs => ("It's", "Il fait"),
        Key::TargetTemperature => ("Target temperature", "Température souhaitée"),
        Key::Decrease => ("Decrease", "Baisser"),
        Key::Increase => ("Increase", "Monter"),
        Key::VacDocked => ("At home.", "À la maison."),
        Key::VacCleaning => ("Cleaning.", "En plein ménage."),
        Key::VacReturning => ("Heading home.", "Rentre à la base."),
        Key::VacPaused => ("Paused.", "En pause."),
        Key::VacIdle => ("Idle.", "Au repos."),
        Key::VacError => ("Needs help.", "Besoin d'aide."),
        Key::VacOnDock => (
            "On its dock · Ready to clean",
            "Sur sa base · Prêt à nettoyer",
        ),
        Key::VacRunning => ("Running", "En cours"),
        Key::VacFan => ("fan", "ventilateur"),
        Key::VacOffDock => ("Off its dock", "Hors de sa base"),
        Key::VacCheck => ("Check the robot", "Vérifiez le robot"),
        Key::Start => ("Start", "Démarrer"),
        Key::Pause => ("Pause", "Pause"),
        Key::Resume => ("Resume", "Reprendre"),
        Key::SendHome => ("Send home", "Renvoyer à la base"),
        Key::Battery => ("Battery", "Batterie"),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// The accessible label of the Alerts badge: "3 unseen alerts" / "3 alertes non lues", singular
/// for a badge of "1" ("99+" stays plural).
pub fn unseen_alerts(lang: Lang, badge: &str) -> String {
    match (lang, badge == "1") {
        (Lang::En, true) => format!("{badge} unseen alert"),
        (Lang::En, false) => format!("{badge} unseen alerts"),
        (Lang::Fr, true) => format!("{badge} alerte non lue"),
        (Lang::Fr, false) => format!("{badge} alertes non lues"),
    }
}

/// The toast after a flow button started its flow.
pub fn flow_started(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("Started {name}"),
        Lang::Fr => format!("Lancement de «\u{A0}{name}\u{A0}»"),
    }
}

/// The language in context, read reactively; English when there is none.
pub fn use_lang() -> impl Fn() -> Lang + Copy + 'static {
    let lang = use_context::<RwSignal<Lang>>();
    move || {
        lang.map_or(Lang::En, |l| {
            let current = l.get();
            remember(current);
            current
        })
    }
}

thread_local! {
    /// The language last read from the context signal: what `current_lang` returns where there
    /// is no reactive owner to find the context through.
    static LAST_SEEN: std::cell::Cell<Lang> = const { std::cell::Cell::new(Lang::En) };
}

fn remember(lang: Lang) {
    LAST_SEEN.with(|c| c.set(lang));
}

/// The language in context right now, without tracking it: for a view built once that does not
/// follow a later toggle (cards rebuilt from the entity). With an owner but no language context
/// it is English; with no owner at all (an async task, a timer or a window listener) it is the
/// language last rendered, since the context cannot be reached from there.
pub fn current_lang() -> Lang {
    match use_context::<RwSignal<Lang>>() {
        Some(l) => {
            let current = l.get_untracked();
            remember(current);
            current
        }
        None if Owner::current().is_none() => LAST_SEEN.with(|c| c.get()),
        None => Lang::En,
    }
}

/// A translator bound to the language signal in context (reactive), or English when there is none.
pub fn use_text() -> impl Fn(Key) -> &'static str + Copy + 'static {
    let lang = use_context::<RwSignal<Lang>>();
    move |key| text(lang.map_or(Lang::En, |l| l.get()), key)
}

#[cfg(test)]
mod tests {
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
            | Key::NetworkPresence
            | Key::AssistantVoice
            | Key::AssistantListening
            | Key::Sound
            | Key::System
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
        assert_eq!(Key::ALL.len(), 144);
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
}
