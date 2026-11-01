//! FR/EN text: the Settings sub-pages, the room page and the ambiance action forms' command
//! words. Also holds the literals of those screens that are not words (the hub's validation
//! phrases that the Home panel matches, the MCP snippets, the theme query), so the bare-text guard
//! sees one place for them.

use super::{Lang, format};

/// The fixed words of the Settings pages, the room page and the command select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsText {
    HintHome,
    HintLocation,
    HintWeather,
    HintSolarForecast,
    HintCameras,
    HintPush,
    HintAlarm,
    HintNetworkPresence,
    HintAppearance,
    HintZigbee,
    HintBroker,
    HintDatabase,
    HintMcp,
    HintAbout,
    NoSuchPage,
    AllSettings,
    Theme,
    ThemeHint,
    ThemeAuto,
    ThemeNight,
    ThemeIvoire,
    EnergyView,
    EnergyViewHint,
    EnergyViewClassic,
    EnergyViewFlux,
    EnergyViewOrbite,
    EnergyPreview,
    Yes,
    No,
    Copy,
    Copied,
    Connecting,
    Online,
    Offline,
    Bridge,
    Version,
    Coordinator,
    PermitJoin,
    State,
    UpdatesAvailable,
    AllUpToDate,
    Counts,
    Devices,
    Entities,
    PrimaryEntities,
    Areas,
    IgnoredTopics,
    DevicesHint,
    OpenZ2mDevices,
    Connection,
    Status,
    Reason,
    Endpoint,
    Transport,
    TransportValue,
    SetUpAssistant,
    ClaudeCode,
    CodexCli,
    ClaudeDesktop,
    Hub,
    Product,
    ProductName,
    HubOrigin,
    Favorites,
    NothingHere,
    Press,
    SetPosition,
    SetTemperature,
    SetValue,
}

impl SettingsText {
    pub const ALL: &'static [SettingsText] = &[
        SettingsText::HintHome,
        SettingsText::HintLocation,
        SettingsText::HintWeather,
        SettingsText::HintSolarForecast,
        SettingsText::HintCameras,
        SettingsText::HintPush,
        SettingsText::HintAlarm,
        SettingsText::HintNetworkPresence,
        SettingsText::HintAppearance,
        SettingsText::HintZigbee,
        SettingsText::HintBroker,
        SettingsText::HintDatabase,
        SettingsText::HintMcp,
        SettingsText::HintAbout,
        SettingsText::NoSuchPage,
        SettingsText::AllSettings,
        SettingsText::Theme,
        SettingsText::ThemeHint,
        SettingsText::ThemeAuto,
        SettingsText::ThemeNight,
        SettingsText::ThemeIvoire,
        SettingsText::EnergyView,
        SettingsText::EnergyViewHint,
        SettingsText::EnergyViewClassic,
        SettingsText::EnergyViewFlux,
        SettingsText::EnergyViewOrbite,
        SettingsText::EnergyPreview,
        SettingsText::Yes,
        SettingsText::No,
        SettingsText::Copy,
        SettingsText::Copied,
        SettingsText::Connecting,
        SettingsText::Online,
        SettingsText::Offline,
        SettingsText::Bridge,
        SettingsText::Version,
        SettingsText::Coordinator,
        SettingsText::PermitJoin,
        SettingsText::State,
        SettingsText::UpdatesAvailable,
        SettingsText::AllUpToDate,
        SettingsText::Counts,
        SettingsText::Devices,
        SettingsText::Entities,
        SettingsText::PrimaryEntities,
        SettingsText::Areas,
        SettingsText::IgnoredTopics,
        SettingsText::DevicesHint,
        SettingsText::OpenZ2mDevices,
        SettingsText::Connection,
        SettingsText::Status,
        SettingsText::Reason,
        SettingsText::Endpoint,
        SettingsText::Transport,
        SettingsText::TransportValue,
        SettingsText::SetUpAssistant,
        SettingsText::ClaudeCode,
        SettingsText::CodexCli,
        SettingsText::ClaudeDesktop,
        SettingsText::Hub,
        SettingsText::Product,
        SettingsText::ProductName,
        SettingsText::HubOrigin,
        SettingsText::Favorites,
        SettingsText::NothingHere,
        SettingsText::Press,
        SettingsText::SetPosition,
        SettingsText::SetTemperature,
        SettingsText::SetValue,
    ];
}

/// A fixed word of these screens.
pub fn settings_text(lang: Lang, k: SettingsText) -> &'static str {
    use SettingsText as T;
    let (en, fr) = match k {
        T::HintHome => (
            "Eco setpoint, house mode, indoor temperature, off-peak hours",
            "Consigne éco, mode de la maison, température intérieure, heures creuses",
        ),
        T::HintLocation => (
            "City, coordinates and time zone",
            "Ville, coordonnées et fuseau horaire",
        ),
        T::HintWeather => ("Forecast for your home", "Prévisions pour votre maison"),
        T::HintSolarForecast => (
            "Solcast forecast for your panels",
            "Prévision Solcast pour vos panneaux",
        ),
        T::HintCameras => (
            "Frigate events, snapshots and live view",
            "Événements, images et vue en direct de Frigate",
        ),
        T::HintPush => ("Alerts on your phone", "Alertes sur votre téléphone"),
        T::HintAlarm => (
            "Sensors, sirens, remotes and presence",
            "Capteurs, sirènes, télécommandes et présence",
        ),
        T::HintNetworkPresence => (
            "Who is home, by pinging devices",
            "Qui est là, en envoyant un ping aux appareils",
        ),
        T::HintAppearance => ("Theme, energy view", "Thème, vue énergie"),
        T::HintZigbee => (
            "Bridge, firmware updates and devices",
            "Passerelle, mises à jour du micrologiciel et appareils",
        ),
        T::HintBroker => ("MQTT connection", "Connexion MQTT"),
        T::HintDatabase => ("TimescaleDB connection", "Connexion à TimescaleDB"),
        T::HintMcp => (
            "Connect Claude, Codex and other assistants",
            "Connecter Claude, Codex et d'autres assistants",
        ),
        T::HintAbout => ("Version and hub address", "Version et adresse du hub"),
        T::NoSuchPage => (
            "There is no such settings page.",
            "Cette page de réglages n'existe pas.",
        ),
        T::AllSettings => ("All settings", "Tous les réglages"),
        T::Theme => ("Theme", "Thème"),
        T::ThemeHint => (
            "The theme button in the header switches it too.",
            "Le bouton de thème de l'en-tête le change aussi.",
        ),
        T::ThemeAuto => ("Auto", "Auto"),
        T::ThemeNight => ("Night Glass", "Night Glass"),
        T::ThemeIvoire => ("Ivoire", "Ivoire"),
        T::EnergyView => ("Energy view", "Vue énergie"),
        T::EnergyViewHint => (
            "How the Energy page draws the live power.",
            "Comment la page Énergie dessine la puissance en direct.",
        ),
        T::EnergyViewClassic => ("Classic", "Classique"),
        T::EnergyViewFlux => ("Flux", "Flux"),
        T::EnergyViewOrbite => ("Orbite", "Orbite"),
        T::EnergyPreview => (
            "Preview with sample values",
            "Aperçu avec des valeurs d'exemple",
        ),
        T::Yes => ("Yes", "Oui"),
        T::No => ("No", "Non"),
        T::Copy => ("Copy", "Copier"),
        T::Copied => ("Copied", "Copié"),
        T::Connecting => ("Connecting…", "Connexion…"),
        T::Online => ("Online", "En ligne"),
        T::Offline => ("Offline", "Hors ligne"),
        T::Bridge => ("Bridge", "Passerelle"),
        T::Version => ("Version", "Version"),
        T::Coordinator => ("Coordinator", "Coordinateur"),
        T::PermitJoin => ("Permit join", "Appairage autorisé"),
        T::State => ("State", "État"),
        T::UpdatesAvailable => ("Updates available", "Mises à jour disponibles"),
        T::AllUpToDate => (
            "All devices are up to date",
            "Tous les appareils sont à jour",
        ),
        T::Counts => ("Counts", "Totaux"),
        T::Devices => ("Devices", "Appareils"),
        T::Entities => ("Entities", "Entités"),
        T::PrimaryEntities => ("Primary entities", "Entités principales"),
        T::Areas => ("Areas", "Pièces"),
        T::IgnoredTopics => ("Ignored topics", "Sujets de découverte ignorés"),
        T::DevicesHint => (
            "Zigbee2MQTT devices, firmware updates and ignored discovery topics.",
            "Appareils Zigbee2MQTT, mises à jour du micrologiciel et sujets de découverte ignorés.",
        ),
        T::OpenZ2mDevices => ("Open Z2M devices", "Ouvrir les appareils Z2M"),
        T::Connection => ("Connection", "Connexion"),
        T::Status => ("Status", "Statut"),
        T::Reason => ("Reason", "Raison"),
        T::Endpoint => ("Endpoint", "Point d'accès"),
        T::Transport => ("Transport", "Transport"),
        T::TransportValue => (
            "Streamable HTTP, no auth (LAN only)",
            "HTTP en flux, sans authentification (réseau local uniquement)",
        ),
        T::SetUpAssistant => ("Set up an assistant", "Configurer un assistant"),
        T::ClaudeCode => ("Claude Code", "Claude Code"),
        T::CodexCli => (
            "Codex CLI (~/.codex/config.toml)",
            "Codex CLI (~/.codex/config.toml)",
        ),
        T::ClaudeDesktop => (
            "Claude Desktop (stdio proxy)",
            "Claude Desktop (proxy stdio)",
        ),
        T::Hub => ("Hub", "Hub"),
        T::Product => ("Product", "Produit"),
        T::ProductName => ("Oikos-os", "Oikos-os"),
        T::HubOrigin => (
            "Hub: this page's origin",
            "Hub\u{A0}: origine de cette page",
        ),
        T::Favorites => ("Favorites", "Favoris"),
        T::NothingHere => ("Nothing here yet.", "Rien ici pour l'instant."),
        T::Press => ("Press", "Appuyer"),
        T::SetPosition => ("Set position", "Régler la position"),
        T::SetTemperature => ("Set temperature", "Régler la température"),
        T::SetValue => ("Set value", "Régler la valeur"),
    };
    if lang == Lang::Fr { fr } else { en }
}

/// The Zigbee2MQTT row on the Settings list: "3 updates" / "3 mises à jour".
pub fn updates_waiting(lang: Lang, n: usize) -> String {
    format::count(
        lang,
        n,
        ("update", "mise à jour"),
        ("updates", "mises à jour"),
    )
}

/// "Select low" / "Choisir low": a select option as a command. The option is the device's own.
pub fn select_option(lang: Lang, option: &str) -> String {
    match lang {
        Lang::Fr => format!("Choisir {option}"),
        Lang::En => format!("Select {option}"),
    }
}

/// "Set mode heat" / "Régler le mode heat": a climate mode as a command.
pub fn set_mode(lang: Lang, mode: &str) -> String {
    match lang {
        Lang::Fr => format!("Régler le mode {mode}"),
        Lang::En => format!("Set mode {mode}"),
    }
}

// ---- literals that are not words -------------------------------------------------------------

/// The phrases of the hub's Home-settings validation messages, matched to pick our own text.
pub const RULE_START_EQUALS_END: &str = "start equals end";
pub const RULE_BAD_TIME: &str = "bad time";
pub const RULE_OVERLAP: &str = "overlap";
pub const RULE_BETWEEN: &str = "must be between";
pub const RULE_LOWER_THAN: &str = "must be lower than";
pub const RULE_NOT_A: &str = " is not a";

/// What the origin row reads off the browser, where there is no page.
pub const NO_ORIGIN: &str = "unavailable outside the browser";

/// The media query that tells a light system theme.
pub const LIGHT_SCHEME_QUERY: &str = "(prefers-color-scheme: light)";

/// The media query that tells the user asked for less motion.
pub const REDUCED_MOTION_QUERY: &str = "(prefers-reduced-motion: reduce)";

/// The MCP set-up commands, the same in both languages; `{origin}` is the hub's address.
pub const CLAUDE_CODE_SNIPPET: &str = "claude mcp add --transport http oikos {origin}/mcp";
pub const CODEX_SNIPPET: &str = "[mcp_servers.oikos]\nurl = \"{origin}/mcp\"";
/// Keys in the order of docs/users/assistants-mcp.md (`command` before `args`); `{origin}` is the
/// address as a JSON string.
pub const CLAUDE_DESKTOP_SNIPPET: &str = "{\n  \"mcpServers\": {\n    \"oikos\": {\n      \"command\": \"<path>/oikos-os\",\n      \"args\": [\"mcp-stdio\", \"--server\", {origin}]\n    }\n  }\n}";

#[cfg(test)]
mod tests {
    use super::*;

    /// Names and words that are the same in both languages.
    const SAME: &[SettingsText] = &[
        SettingsText::ThemeAuto,
        SettingsText::ThemeNight,
        SettingsText::ThemeIvoire,
        SettingsText::EnergyViewFlux,
        SettingsText::EnergyViewOrbite,
        SettingsText::Version,
        SettingsText::Transport,
        SettingsText::ClaudeCode,
        SettingsText::CodexCli,
        SettingsText::Hub,
        SettingsText::ProductName,
    ];

    #[test]
    fn every_word_has_both_languages() {
        super::super::assert_table(SettingsText::ALL, settings_text, SAME);
    }

    #[test]
    fn all_lists_every_variant() {
        // The match in `settings_text` is exhaustive, so a new variant only needs listing here.
        assert_eq!(SettingsText::ALL.len(), 69);
    }

    #[test]
    fn updates_follow_the_plural_rule() {
        assert_eq!(updates_waiting(Lang::En, 1), "1 update");
        assert_eq!(updates_waiting(Lang::En, 3), "3 updates");
        assert_eq!(updates_waiting(Lang::Fr, 1), "1 mise à jour");
        assert_eq!(updates_waiting(Lang::Fr, 2), "2 mises à jour");
    }

    #[test]
    fn command_sentences_in_both_languages() {
        assert_eq!(select_option(Lang::En, "low"), "Select low");
        assert_eq!(select_option(Lang::Fr, "low"), "Choisir low");
        assert_eq!(set_mode(Lang::Fr, "heat"), "Régler le mode heat");
    }

    #[test]
    fn french_has_a_no_break_space_before_a_colon() {
        assert!(settings_text(Lang::Fr, SettingsText::HubOrigin).contains("\u{A0}:"));
    }
}
