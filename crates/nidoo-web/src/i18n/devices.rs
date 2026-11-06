//! FR/EN text: the devices area (device list, entity sheet, override editor and the shared
//! card controls).

use super::*;

const NBSP: char = '\u{a0}';

/// Fixed words of the devices area. Sentences with values are the helper functions below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceText {
    /// The cancel button of the inline "are you sure?" bar.
    Cancel,
    /// Save button.
    Save,
    /// The press button of a button group.
    PressAll,
    /// Vacuum action: make the robot beep.
    Locate,
    /// Media player buttons and texts.
    MediaPlay,
    MediaPause,
    MediaNext,
    MediaPrevious,
    PlayByName,
    NothingPlaying,
    PlaySomeMusic,
    /// Button card.
    Press,
    /// Chime card.
    Play,
    /// Cover buttons.
    Open,
    Stop,
    Close,
    Moving,
    Value,
    Option,
    Colour,
    ColourOpen,
    NoColour,
    White,
    FanSpeed,
    Dock,
    Spot,
    Mixed,
    Target,
    /// Cover position slider.
    Position,
    RoomNeedsName,
    EditRoom,
    DragToReorder,
    RoomName,
    NoRoom,
    FirmwareUpdate,
    Entities,
    Zigbee2mqttDevices,
    SearchPlaceholder,
    SearchDevices,
    Online,
    Offline,
    Unknown,
    Device,
    Name,
    Model,
    Vendor,
    IeeeAddress,
    Availability,
    LinkQuality,
    OtherEntities,
    Attributes,
    LastUpdate,
    Controls,
    ViewHistory,
    InGroups,
    DeviceDiagnostics,
    UnknownEntity,
    Details,
    UpdateAvailable,
    Room,
    Default,
    Other,
    EnergyCounter,
    Automatic,
    BrightnessOnTurnOn,
    Reset,
    OtherSaved,
    Solar,
    Grid,
    GridExport,
    Consumer,
    Water,
    Warm,
    Red,
    Green,
    Blue,
    WaterHeater,
    HeatPump,
    Oven,
    Fridge,
    Washer,
    Dryer,
    Dishwasher,
    NoneChoice,
    LockControl,
    LastState,
    Plug,
    Saved,
    OverrideCleared,
    GroupShutters,
    GroupLights,
    General,
    NameHint,
    Favorite,
    HideFromHome,
    ShowOnHome,
    Type,
    ShowAs,
    MainControl,
    Light,
    TurnOnIn,
    ShowOnSecurity,
    AskBeforeActing,
    DisplayAndEnergy,
    CardStyle,
    Standard,
    HeaderTemperature,
    EnergyRole,
    NeedsPowerUnit,
    InvertSign,
    Inverted,
    Icon,
    TargetPresets,
    PresetsHint,
    ResetToDefaults,
}

impl DeviceText {
    pub const ALL: &'static [DeviceText] = &[
        DeviceText::Cancel,
        DeviceText::Save,
        DeviceText::PressAll,
        DeviceText::Locate,
        DeviceText::Press,
        DeviceText::Play,
        DeviceText::Open,
        DeviceText::Stop,
        DeviceText::Close,
        DeviceText::Moving,
        DeviceText::Value,
        DeviceText::Option,
        DeviceText::Colour,
        DeviceText::ColourOpen,
        DeviceText::NoColour,
        DeviceText::White,
        DeviceText::FanSpeed,
        DeviceText::Dock,
        DeviceText::Spot,
        DeviceText::Mixed,
        DeviceText::Target,
        DeviceText::Position,
        DeviceText::RoomNeedsName,
        DeviceText::EditRoom,
        DeviceText::DragToReorder,
        DeviceText::RoomName,
        DeviceText::NoRoom,
        DeviceText::FirmwareUpdate,
        DeviceText::Entities,
        DeviceText::Zigbee2mqttDevices,
        DeviceText::SearchPlaceholder,
        DeviceText::SearchDevices,
        DeviceText::Online,
        DeviceText::Offline,
        DeviceText::Unknown,
        DeviceText::Device,
        DeviceText::Name,
        DeviceText::Model,
        DeviceText::Vendor,
        DeviceText::IeeeAddress,
        DeviceText::Availability,
        DeviceText::LinkQuality,
        DeviceText::OtherEntities,
        DeviceText::Attributes,
        DeviceText::LastUpdate,
        DeviceText::Controls,
        DeviceText::ViewHistory,
        DeviceText::InGroups,
        DeviceText::DeviceDiagnostics,
        DeviceText::UnknownEntity,
        DeviceText::Details,
        DeviceText::UpdateAvailable,
        DeviceText::Room,
        DeviceText::Default,
        DeviceText::Other,
        DeviceText::EnergyCounter,
        DeviceText::Automatic,
        DeviceText::BrightnessOnTurnOn,
        DeviceText::Reset,
        DeviceText::OtherSaved,
        DeviceText::Solar,
        DeviceText::Grid,
        DeviceText::GridExport,
        DeviceText::Consumer,
        DeviceText::Water,
        DeviceText::Warm,
        DeviceText::Red,
        DeviceText::Green,
        DeviceText::Blue,
        DeviceText::WaterHeater,
        DeviceText::HeatPump,
        DeviceText::Oven,
        DeviceText::Fridge,
        DeviceText::Washer,
        DeviceText::Dryer,
        DeviceText::Dishwasher,
        DeviceText::NoneChoice,
        DeviceText::LockControl,
        DeviceText::LastState,
        DeviceText::Plug,
        DeviceText::Saved,
        DeviceText::OverrideCleared,
        DeviceText::GroupShutters,
        DeviceText::GroupLights,
        DeviceText::General,
        DeviceText::NameHint,
        DeviceText::Favorite,
        DeviceText::HideFromHome,
        DeviceText::ShowOnHome,
        DeviceText::Type,
        DeviceText::ShowAs,
        DeviceText::MainControl,
        DeviceText::Light,
        DeviceText::TurnOnIn,
        DeviceText::ShowOnSecurity,
        DeviceText::AskBeforeActing,
        DeviceText::DisplayAndEnergy,
        DeviceText::CardStyle,
        DeviceText::Standard,
        DeviceText::HeaderTemperature,
        DeviceText::EnergyRole,
        DeviceText::NeedsPowerUnit,
        DeviceText::InvertSign,
        DeviceText::Inverted,
        DeviceText::Icon,
        DeviceText::TargetPresets,
        DeviceText::PresetsHint,
        DeviceText::ResetToDefaults,
    ];
}

/// The text of `k` in `lang`.
pub fn device_text(lang: Lang, k: DeviceText) -> &'static str {
    use DeviceText as T;
    let (en, fr) = match k {
        T::Cancel => ("Cancel", "Annuler"),
        T::Save => ("Save", "Enregistrer"),
        T::PressAll => ("Press all", "Tout appuyer"),
        T::Locate => ("Locate", "Localiser"),
        T::MediaPlay => ("Play", "Lecture"),
        T::MediaPause => ("Pause", "Pause"),
        T::MediaNext => ("Next", "Suivant"),
        T::MediaPrevious => ("Previous", "Précédent"),
        T::PlayByName => ("Play by name", "Jouer un titre"),
        T::NothingPlaying => ("Nothing playing", "Rien ne joue"),
        T::PlaySomeMusic => ("Play some music", "Lancer de la musique"),
        T::Press => ("Press", "Appuyer"),
        T::Play => ("Play", "Sonner"),
        T::Open => ("Open", "Ouvrir"),
        T::Stop => ("Stop", "Arrêter"),
        T::Close => ("Close", "Fermer"),
        T::Moving => ("Moving", "En mouvement"),
        T::Value => ("Value", "Valeur"),
        T::Option => ("Option", "Option"),
        T::Colour => ("Colour", "Couleur"),
        T::ColourOpen => (
            "Colour: open to change it",
            "Couleur : ouvrir pour la modifier",
        ),
        T::NoColour => ("No colour set", "Aucune couleur définie"),
        T::White => ("White", "Blanc"),
        T::FanSpeed => ("Fan speed", "Vitesse du ventilateur"),
        T::Dock => ("Dock", "Retour à la base"),
        T::Spot => ("Spot", "Nettoyage localisé"),
        T::Mixed => ("Mixed", "Mixte"),
        T::Target => ("Target", "Consigne"),
        T::Position => ("Position", "Position"),
        T::RoomNeedsName => ("A room needs a name", "Une pièce doit avoir un nom"),
        T::EditRoom => ("Edit room", "Modifier la pièce"),
        T::DragToReorder => (
            "Drag the cards to reorder",
            "Faites glisser les cartes pour les réordonner",
        ),
        T::RoomName => ("Room name", "Nom de la pièce"),
        T::NoRoom => ("No room", "Sans pièce"),
        T::FirmwareUpdate => (
            "Firmware update available",
            "Mise à jour du firmware disponible",
        ),
        T::Entities => ("Entities", "Entités"),
        T::Zigbee2mqttDevices => ("Zigbee2MQTT devices", "Appareils Zigbee2MQTT"),
        T::SearchPlaceholder => (
            "Name, room, model, maker or IEEE",
            "Nom, pièce, modèle, fabricant ou IEEE",
        ),
        T::SearchDevices => ("Search devices", "Rechercher des appareils"),
        T::Online => ("Online", "En ligne"),
        T::Offline => ("Offline", "Hors ligne"),
        T::Unknown => ("Unknown", "Inconnu"),
        T::Device => ("Device", "Appareil"),
        T::Name => ("Name", "Nom"),
        T::Model => ("Model", "Modèle"),
        T::Vendor => ("Vendor", "Fabricant"),
        T::IeeeAddress => ("IEEE address", "Adresse IEEE"),
        T::Availability => ("Availability", "Disponibilité"),
        T::LinkQuality => ("Link quality", "Qualité du lien"),
        T::OtherEntities => ("Other entities", "Autres entités"),
        T::Attributes => ("Attributes", "Attributs"),
        T::LastUpdate => ("Last update:", "Dernière mise à jour :"),
        T::Controls => ("Controls", "Commandes"),
        T::ViewHistory => ("View history →", "Voir l'historique →"),
        T::InGroups => ("In groups", "Dans les groupes"),
        T::DeviceDiagnostics => ("Device & diagnostics", "Appareil et diagnostic"),
        T::UnknownEntity => ("Unknown entity", "Entité inconnue"),
        T::Details => ("Details", "Détails"),
        T::UpdateAvailable => ("Update available", "Mise à jour disponible"),
        T::Room => ("Room", "Pièce"),
        T::Default => ("Default", "Par défaut"),
        T::Other => ("Other…", "Autre…"),
        T::EnergyCounter => ("Energy counter", "Compteur d'énergie"),
        T::Automatic => ("Automatic", "Automatique"),
        T::BrightnessOnTurnOn => ("Brightness when turned on", "Luminosité à l'allumage"),
        T::Reset => ("Reset", "Réinitialiser"),
        T::OtherSaved => ("Other saved settings", "Autres réglages enregistrés"),
        T::Solar => ("Solar", "Solaire"),
        T::Grid => ("Grid", "Réseau"),
        T::GridExport => ("Grid export", "Injection réseau"),
        T::Consumer => ("Consumer", "Consommateur"),
        T::Water => ("Water", "Eau"),
        T::Warm => ("Warm", "Blanc chaud"),
        T::Red => ("Red", "Rouge"),
        T::Green => ("Green", "Vert"),
        T::Blue => ("Blue", "Bleu"),
        T::WaterHeater => ("Water heater", "Chauffe-eau"),
        T::HeatPump => ("Heat pump", "Pompe à chaleur"),
        T::Oven => ("Oven", "Four"),
        T::Fridge => ("Fridge", "Réfrigérateur"),
        T::Washer => ("Washer", "Lave-linge"),
        T::Dryer => ("Dryer", "Sèche-linge"),
        T::Dishwasher => ("Dishwasher", "Lave-vaisselle"),
        T::NoneChoice => ("None", "Aucun"),
        T::LockControl => ("Lock", "Verrou"),
        T::LastState => ("Last state", "Dernier état"),
        T::Plug => ("Plug", "Prise"),
        T::Saved => ("Saved", "Enregistré"),
        T::OverrideCleared => ("Override cleared", "Réglages réinitialisés"),
        T::GroupShutters => (
            "Group with the other shutters in this room",
            "Grouper avec les autres volets de cette pièce",
        ),
        T::GroupLights => (
            "Group with the other lights in this room",
            "Grouper avec les autres lumières de cette pièce",
        ),
        T::General => ("General", "Général"),
        T::NameHint => (
            "Leave empty to use the device's own name.",
            "Laisser vide pour utiliser le nom de l'appareil.",
        ),
        T::Favorite => ("Favorite", "Favori"),
        T::HideFromHome => ("Hide from Home", "Masquer de l'accueil"),
        T::ShowOnHome => ("Show on Home", "Afficher sur l'accueil"),
        T::Type => ("Type", "Type"),
        T::ShowAs => ("Show as", "Afficher comme"),
        T::MainControl => ("Main control", "Commande principale"),
        T::Light => ("Light", "Lumière"),
        T::TurnOnIn => ("Turn on in", "Allumer en"),
        T::ShowOnSecurity => ("Show on Security", "Afficher dans Sécurité"),
        T::AskBeforeActing => ("Ask before acting", "Demander avant d'agir"),
        T::DisplayAndEnergy => ("Display and energy", "Affichage et énergie"),
        T::CardStyle => ("Card style", "Style de carte"),
        T::Standard => ("Standard", "Standard"),
        T::HeaderTemperature => ("Header temperature", "Température d'en-tête"),
        T::EnergyRole => ("Energy role", "Rôle énergétique"),
        T::NeedsPowerUnit => (
            "Needs a power unit (W, kW)",
            "Nécessite une unité de puissance (W, kW)",
        ),
        T::InvertSign => ("Invert sign", "Inverser le signe"),
        T::Inverted => (
            "Inverted (on and off swapped)",
            "Inversé (marche et arrêt échangés)",
        ),
        T::Icon => ("Icon", "Icône"),
        T::TargetPresets => ("Target presets (°C)", "Consignes prédéfinies (°C)"),
        T::PresetsHint => (
            "Comma-separated, e.g. 19, 21, 23. Up to 6 chips.",
            "Séparées par des points-virgules, par ex. 19 ; 21,5 ; 23. 6 valeurs au maximum.",
        ),
        T::ResetToDefaults => ("Reset to defaults", "Rétablir les valeurs par défaut"),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// A translator for [`DeviceText`] bound to the language in context (reactive).
pub fn use_device_text() -> impl Fn(DeviceText) -> &'static str + Copy + 'static {
    let lang = use_lang();
    move |k| device_text(lang(), k)
}

/// `k` as a signal that follows the language (for props that take a `Signal<String>`). Call it
/// where `use_lang` works: in a component body.
pub fn text_signal(k: DeviceText) -> Signal<String> {
    let lang = use_lang();
    Signal::derive(move || device_text(lang(), k).to_owned())
}

/// A shared [`Key`] word as a signal that follows the language.
pub fn key_signal(k: Key) -> Signal<String> {
    let lang = use_lang();
    Signal::derive(move || text(lang(), k).to_owned())
}

/// What a device's availability reads as: "Online" / "En ligne".
pub fn availability_text(
    lang: Lang,
    availability: nidoo_core::model::Availability,
) -> &'static str {
    use nidoo_core::model::Availability as A;
    device_text(
        lang,
        match availability {
            A::Online => DeviceText::Online,
            A::Offline => DeviceText::Offline,
            A::Unknown => DeviceText::Unknown,
        },
    )
}

/// "3 of 10 devices" / "3 sur 10 appareils": the plural follows the total.
pub fn devices_count(lang: Lang, shown: usize, total: usize) -> String {
    match lang {
        Lang::En => format!(
            "{shown} of {total} {}",
            if total == 1 { "device" } else { "devices" }
        ),
        Lang::Fr => format!(
            "{shown} sur {total} {}",
            if total <= 1 { "appareil" } else { "appareils" }
        ),
    }
}

/// "Ignored discovery topics (4)".
pub fn ignored_topics(lang: Lang, n: usize) -> String {
    match lang {
        Lang::En => format!("Ignored discovery topics ({n})"),
        Lang::Fr => format!("Sujets de découverte ignorés ({n})"),
    }
}

/// The link quality badge of a device row: "LQI 120" (the Zigbee term, in both languages).
pub fn link_quality_badge(q: u32) -> String {
    format!("LQI {q}")
}

/// The climate presets as the text field shows them. They are stored as comma-separated numbers
/// ("19, 21.5"); French shows "19 ; 21,5" since a comma is its decimal mark.
pub fn presets_text(lang: Lang, stored: &str) -> String {
    let pieces = stored
        .split([',', ';'])
        .map(str::trim)
        .filter(|p| !p.is_empty());
    match lang {
        Lang::En => pieces.collect::<Vec<_>>().join(", "),
        Lang::Fr => pieces
            .map(|p| {
                if p.parse::<f64>().is_ok() {
                    p.replace('.', ",")
                } else {
                    p.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join(&format!("{NBSP}; ")),
    }
}

/// The stored presets for what was typed: numbers (a comma or a point as decimal mark) joined
/// by ", ". A `;` is the separator when there is one, else a comma is. A piece that is not a
/// number stays as typed, so nothing is dropped silently.
pub fn presets_stored(text: &str) -> String {
    let separator = if text.contains(';') { ';' } else { ',' };
    text.split(separator)
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| match format::parse_decimal(p) {
            Some(n) => n.to_string(),
            None => p.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The button that clears one saved setting: "Clear order" / "Effacer order". `key` is a raw
/// field name and stays as it is.
pub fn clear_setting(lang: Lang, key: &str) -> String {
    match lang {
        Lang::En => format!("Clear {key}"),
        Lang::Fr => format!("Effacer {key}"),
    }
}

/// One saved setting as "key: value" (a no-break space before the colon in French).
pub fn key_value(lang: Lang, key: &str, value: &str) -> String {
    match lang {
        Lang::En => format!("{key}: {value}"),
        Lang::Fr => format!("{key}{NBSP}: {value}"),
    }
}

/// The installed and latest firmware of an update entity.
pub fn firmware_versions(installed: &str, latest: &str) -> String {
    format!("Firmware {installed} → {latest}")
}

/// The line of a signal entity: "Last: ring, 5 min ago" / "Dernier : ring, il y a 5 min".
pub fn last_signal(lang: Lang, last: &str, ago: Option<&str>) -> String {
    let lead = match lang {
        Lang::En => "Last:".to_owned(),
        Lang::Fr => format!("Dernier{NBSP}:"),
    };
    match ago {
        Some(ago) => format!("{lead} {last}, {ago}"),
        None => format!("{lead} {last}"),
    }
}

/// The readout under the colour wheel: "Hue 120° · Saturation 80 %".
pub fn colour_readout(lang: Lang, hue: f64, saturation: f64) -> String {
    let (h, s) = (hue.round(), saturation.round());
    match lang {
        Lang::En => format!("Hue {h}° · Saturation {s}{NBSP}%"),
        Lang::Fr => format!("Teinte {h}° · Saturation {s}{NBSP}%"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same word in both languages.
    const SAME: &[DeviceText] = &[
        DeviceText::Position,
        DeviceText::Option,
        DeviceText::Type,
        DeviceText::Standard,
    ];

    #[test]
    fn every_word_has_both_languages() {
        assert_table(DeviceText::ALL, device_text, SAME);
    }

    #[test]
    fn the_device_count_follows_the_total() {
        assert_eq!(devices_count(Lang::En, 1, 1), "1 of 1 device");
        assert_eq!(devices_count(Lang::En, 0, 0), "0 of 0 devices");
        assert_eq!(devices_count(Lang::En, 3, 10), "3 of 10 devices");
        assert_eq!(devices_count(Lang::Fr, 0, 0), "0 sur 0 appareil");
        assert_eq!(devices_count(Lang::Fr, 1, 1), "1 sur 1 appareil");
        assert_eq!(devices_count(Lang::Fr, 3, 10), "3 sur 10 appareils");
    }

    #[test]
    fn colons_take_a_no_break_space_in_french() {
        assert_eq!(key_value(Lang::En, "order", "3"), "order: 3");
        assert_eq!(key_value(Lang::Fr, "order", "3"), "order\u{a0}: 3");
        assert_eq!(
            last_signal(Lang::Fr, "ring", Some("il y a 5\u{a0}min")),
            "Dernier\u{a0}: ring, il y a 5\u{a0}min"
        );
        assert_eq!(last_signal(Lang::En, "ring", None), "Last: ring");
        assert_eq!(
            device_text(Lang::Fr, DeviceText::LastUpdate),
            "Dernière mise à jour\u{a0}:"
        );
    }

    #[test]
    fn presets_read_and_write_in_both_languages() {
        assert_eq!(presets_stored("19,5 ; 21"), "19.5, 21");
        assert_eq!(presets_stored("19, 21.5, 23"), "19, 21.5, 23");
        assert_eq!(presets_stored("19,5; ;21;"), "19.5, 21");
        assert_eq!(presets_text(Lang::Fr, "19.5, 21"), "19,5\u{a0}; 21");
        assert_eq!(presets_text(Lang::En, "19.5, 21"), "19.5, 21");
        assert_eq!(
            presets_stored(&presets_text(Lang::Fr, "19.5, 21, 23")),
            "19.5, 21, 23"
        );
    }

    #[test]
    fn the_colour_readout_rounds_and_keeps_the_unit_apart() {
        assert_eq!(
            colour_readout(Lang::En, 119.6, 80.2),
            "Hue 120° · Saturation 80\u{a0}%"
        );
        assert_eq!(
            colour_readout(Lang::Fr, 0.0, 0.0),
            "Teinte 0° · Saturation 0\u{a0}%"
        );
    }
}
