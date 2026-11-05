//! FR/EN words for what an entity is and the state it is in.
//!
//! English keeps the raw prettifier's output (`format::word`), so only French needs a table; a
//! value outside the known vocabulary is prettified the same way in both languages.

use nidoo_core::model::{Kind, SwitchType};

use super::{Key, Lang, text};
use crate::format::word;

/// Which vocabulary a raw state value belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateCtx {
    Cover,
    Lock,
    Alarm,
    /// Climate and water-heater modes.
    Hvac,
    Vacuum,
    Presence,
    /// A binary sensor's `on`/`off`, worded by its device class.
    BinarySensor(Option<String>),
    /// A choice whose vocabulary is not known: presence, lock and cover words are tried.
    Choice,
    Other,
}

/// The word for `raw` in `ctx`: translated when it is known vocabulary, else prettified.
pub fn state_word(lang: Lang, ctx: StateCtx, raw: &str) -> String {
    match raw {
        "unavailable" => return word_pair(lang, "Unavailable", "Indisponible"),
        "unknown" => return word_pair(lang, "Unknown", "Inconnu"),
        _ => {}
    }
    let known = match &ctx {
        StateCtx::BinarySensor(class) => Some(binary(class.as_deref(), raw == "on")),
        StateCtx::Choice => presence(raw).or_else(|| lock(raw)).or_else(|| cover(raw)),
        StateCtx::Cover => cover(raw),
        StateCtx::Lock => lock(raw),
        StateCtx::Alarm => alarm(raw),
        StateCtx::Hvac => hvac(raw),
        StateCtx::Vacuum => vacuum(raw),
        StateCtx::Presence => presence(raw),
        StateCtx::Other => None,
    };
    match (known, lang) {
        (Some((en, _)), Lang::En) => en.map_or_else(|| word(raw), str::to_owned),
        (Some((_, fr)), Lang::Fr) => fr.to_owned(),
        (None, _) => word(raw),
    }
}

fn word_pair(lang: Lang, en: &str, fr: &str) -> String {
    match lang {
        Lang::En => en.to_owned(),
        Lang::Fr => fr.to_owned(),
    }
}

/// `(English override, French)`; `None` in English keeps the prettified raw value.
type Words = (Option<&'static str>, &'static str);

fn cover(raw: &str) -> Option<Words> {
    Some(match raw {
        "open" => (None, "Ouvert"),
        "closed" => (None, "Fermé"),
        "opening" => (None, "Ouverture…"),
        "closing" => (None, "Fermeture…"),
        "stopped" => (None, "Arrêté"),
        _ => return None,
    })
}

fn lock(raw: &str) -> Option<Words> {
    Some(match raw {
        "locked" => (None, "Verrouillé"),
        "unlocked" => (None, "Déverrouillé"),
        "locking" => (None, "Verrouillage…"),
        "unlocking" => (None, "Déverrouillage…"),
        "jammed" => (None, "Bloqué"),
        _ => return None,
    })
}

fn alarm(raw: &str) -> Option<Words> {
    Some(match raw {
        "disarmed" => (None, "Désactivée"),
        "armed_home" => (None, "Activée (présent)"),
        "armed_away" => (None, "Activée (absent)"),
        "armed_night" => (None, "Activée (nuit)"),
        "armed_vacation" => (None, "Activée (vacances)"),
        "armed_custom_bypass" => (None, "Activée (personnalisée)"),
        "arming" => (Some("Exit delay"), "Délai de sortie"),
        "disarming" => (None, "Désactivation…"),
        "pending" => (Some("Entry delay"), "Délai d'entrée"),
        "triggered" => (Some("Triggered"), "Déclenchée"),
        _ => return None,
    })
}

fn hvac(raw: &str) -> Option<Words> {
    Some(match raw {
        "off" => (None, "Arrêt"),
        "heat" => (None, "Chauffage"),
        "cool" => (None, "Climatisation"),
        "auto" => (None, "Auto"),
        "heat_cool" => (None, "Chaud/froid"),
        "dry" => (None, "Déshumidification"),
        "fan_only" => (None, "Ventilation"),
        "eco" => (None, "Éco"),
        "electric" => (None, "Électrique"),
        "gas" => (None, "Gaz"),
        "heat_pump" => (None, "Pompe à chaleur"),
        "performance" => (None, "Performance"),
        "high_demand" => (None, "Forte demande"),
        _ => return None,
    })
}

fn vacuum(raw: &str) -> Option<Words> {
    Some(match raw {
        "docked" => (None, "À la base"),
        "cleaning" => (None, "Nettoyage"),
        "returning" => (None, "Retour à la base"),
        "paused" => (None, "En pause"),
        "idle" => (None, "Inactif"),
        "error" => (None, "Erreur"),
        _ => return None,
    })
}

fn presence(raw: &str) -> Option<Words> {
    Some(match raw {
        "home" => (None, "Présent"),
        "not_home" => (Some("Away"), "Absent"),
        _ => return None,
    })
}

/// A binary sensor's state, agreed with what it watches ("Porte ouverte", "Volet ouvert").
fn binary(class: Option<&str>, on: bool) -> Words {
    // English keeps the wording the binary-sensor card has always had.
    let (en_on, en_off, fr_on, fr_off) = match class.unwrap_or("") {
        "door" | "window" | "garage_door" => ("Open", "Closed", "Ouverte", "Fermée"),
        "opening" => ("Open", "Closed", "Ouvert", "Fermé"),
        "motion" => ("Motion", "Clear", "Mouvement", "Aucun mouvement"),
        "occupancy" => ("Motion", "Clear", "Occupé", "Libre"),
        "presence" => ("Motion", "Clear", "Présent", "Absent"),
        "smoke" => ("Smoke", "Clear", "Fumée", "Normal"),
        "gas" | "carbon_monoxide" => ("On", "Off", "Détecté", "Normal"),
        "moisture" => ("On", "Off", "Fuite", "Sec"),
        "battery" => ("On", "Off", "Faible", "Normale"),
        _ => ("On", "Off", "Activé", "Désactivé"),
    };
    if on {
        (Some(en_on), fr_on)
    } else {
        (Some(en_off), fr_off)
    }
}

/// "Allumé"/"Éteint" for a light, "Activé"/"Désactivé" for anything else that switches.
pub fn on_off(lang: Lang, light: bool, on: bool) -> &'static str {
    let key = match (light, on) {
        (true, true) => Key::LightOn,
        (true, false) => Key::LightOff,
        (false, true) => Key::SwitchOn,
        (false, false) => Key::SwitchOff,
    };
    text(lang, key)
}

/// What an entity is: "Light" / "Lumière".
pub fn kind_word(lang: Lang, kind: Kind) -> &'static str {
    let (en, fr) = match kind {
        Kind::Light => ("Light", "Lumière"),
        Kind::Switch => ("Switch", "Interrupteur"),
        Kind::Cover => ("Cover", "Volet"),
        Kind::Climate => ("Climate", "Thermostat"),
        Kind::Sensor => ("Sensor", "Capteur"),
        Kind::BinarySensor => ("Binary sensor", "Détecteur"),
        Kind::AlarmPanel => ("Alarm panel", "Alarme"),
        Kind::Number => ("Number", "Nombre"),
        Kind::Select => ("Select", "Liste"),
        Kind::Button => ("Button", "Bouton"),
        Kind::Vacuum => ("Vacuum", "Aspirateur"),
        Kind::WaterHeater => ("Water heater", "Chauffe-eau"),
        Kind::Generic => ("Generic", "Appareil"),
    };
    pick(lang, en, fr)
}

/// What a switch is shown as: "Plug" / "Prise".
pub fn switch_type_word(lang: Lang, t: SwitchType) -> &'static str {
    let fr = match t {
        SwitchType::Switch => "Interrupteur",
        SwitchType::Light => "Lumière",
        SwitchType::Door => "Gâche de porte",
        SwitchType::Siren => "Sirène",
        SwitchType::Chime => "Carillon",
        SwitchType::Plug => "Prise",
        SwitchType::Tv => "Télé",
    };
    pick(lang, t.label(), fr)
}

/// What a sensor measures or a detector watches: "Door" / "Porte"; `None` for a class outside
/// the hub's vocabulary.
pub fn device_class_word(lang: Lang, class: &str) -> Option<&'static str> {
    let (en, fr) = match class {
        "door" => ("Door", "Porte"),
        "window" => ("Window", "Fenêtre"),
        "garage_door" => ("Garage door", "Porte de garage"),
        "opening" => ("Opening", "Ouverture"),
        "motion" => ("Motion", "Mouvement"),
        "occupancy" => ("Occupancy", "Présence"),
        "presence" => ("Presence", "Présence"),
        "moisture" => ("Moisture", "Fuite d'eau"),
        "smoke" => ("Smoke", "Fumée"),
        "gas" => ("Gas", "Gaz"),
        "carbon_monoxide" => ("Carbon monoxide", "Monoxyde de carbone"),
        "battery" => ("Battery", "Batterie"),
        "temperature" => ("Temperature", "Température"),
        "humidity" => ("Humidity", "Humidité"),
        "illuminance" => ("Illuminance", "Luminosité"),
        "power" => ("Power", "Puissance"),
        "energy" => ("Energy", "Énergie"),
        "voltage" => ("Voltage", "Tension"),
        "current" => ("Current", "Courant"),
        "pressure" => ("Pressure", "Pression"),
        "co2" => ("CO2", "CO2"),
        "pm25" => ("PM2.5", "PM2,5"),
        "pm10" => ("PM10", "PM10"),
        "atmospheric_pressure" => ("Atmospheric pressure", "Pression atmosphérique"),
        "frequency" => ("Frequency", "Fréquence"),
        "precipitation" => ("Precipitation", "Précipitations"),
        "wind_speed" => ("Wind speed", "Vitesse du vent"),
        "water" => ("Water", "Eau"),
        "vibration" => ("Vibration", "Vibration"),
        "safety" => ("Safety", "Sécurité"),
        "problem" => ("Problem", "Problème"),
        "connectivity" => ("Connectivity", "Connexion"),
        "tamper" => ("Tamper", "Sabotage"),
        "firmware" => ("Firmware", "Micrologiciel"),
        "timestamp" => ("Timestamp", "Horodatage"),
        "restart" => ("Restart", "Redémarrage"),
        "identify" => ("Identify", "Identification"),
        "apparent_power" => ("Apparent power", "Puissance apparente"),
        "power_factor" => ("Power factor", "Facteur de puissance"),
        "distance" => ("Distance", "Distance"),
        "speed" => ("Speed", "Vitesse"),
        "duration" => ("Duration", "Durée"),
        "sound_pressure" => ("Sound pressure", "Pression acoustique"),
        "signal_strength" => ("Signal strength", "Puissance du signal"),
        "irradiance" => ("Irradiance", "Rayonnement"),
        _ => return None,
    };
    Some(pick(lang, en, fr))
}

/// The English and French words for a kind (or the type a switch is shown as), lower case, so
/// a search finds "cover" and "volet" in either language.
pub fn search_words(kind: Kind, switch_type: Option<SwitchType>) -> [String; 2] {
    [Lang::En, Lang::Fr].map(|lang| {
        match switch_type {
            Some(t) => switch_type_word(lang, t),
            None => kind_word(lang, kind),
        }
        .to_lowercase()
    })
}

fn pick(lang: Lang, en: &'static str, fr: &'static str) -> &'static str {
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

#[cfg(test)]
mod tests {
    use nidoo_core::model::{Kind, SwitchType};

    use super::*;
    use crate::i18n::Lang;

    #[test]
    fn sensor_classes_the_house_reports_have_french_words() {
        for (class, fr) in [
            ("frequency", "Fréquence"),
            ("atmospheric_pressure", "Pression atmosphérique"),
            ("wind_speed", "Vitesse du vent"),
            ("precipitation", "Précipitations"),
        ] {
            assert_eq!(device_class_word(Lang::Fr, class), Some(fr));
        }
        assert_eq!(device_class_word(Lang::En, "frequency"), Some("Frequency"));
    }

    #[test]
    fn agreement_follows_the_device_class() {
        let door = StateCtx::BinarySensor(Some("door".into()));
        assert_eq!(state_word(Lang::Fr, door.clone(), "on"), "Ouverte");
        assert_eq!(state_word(Lang::Fr, door, "off"), "Fermée");
        assert_eq!(state_word(Lang::Fr, StateCtx::Cover, "open"), "Ouvert");
        assert_eq!(state_word(Lang::En, StateCtx::Cover, "open"), "Open");
        assert_eq!(state_word(Lang::Fr, StateCtx::Hvac, "heat"), "Chauffage");
        assert_eq!(
            state_word(Lang::Fr, StateCtx::Alarm, "armed_away"),
            "Activée (absent)"
        );
        assert_eq!(
            state_word(Lang::Fr, StateCtx::Presence, "not_home"),
            "Absent"
        );
    }

    #[test]
    fn binary_sensor_english_words() {
        let en = |class: &str, raw| {
            state_word(
                Lang::En,
                StateCtx::BinarySensor(Some(class.to_owned())),
                raw,
            )
        };
        assert_eq!(
            (en("door", "on"), en("door", "off")),
            ("Open".into(), "Closed".into())
        );
        assert_eq!(
            (en("motion", "on"), en("motion", "off")),
            ("Motion".into(), "Clear".into())
        );
        assert_eq!(en("moisture", "on"), "On");
        assert_eq!(en("smoke", "on"), "Smoke");
    }

    #[test]
    fn unknown_values_keep_the_raw_prettifier() {
        assert_eq!(
            state_word(Lang::Fr, StateCtx::Other, "power_on_behavior"),
            "Power on behavior"
        );
        assert_eq!(
            state_word(Lang::Fr, StateCtx::Cover, "half_way"),
            "Half way"
        );
    }

    #[test]
    fn every_kind_has_two_words() {
        for &kind in Kind::ALL {
            let [en, fr] = search_words(kind, None);
            assert!(!en.is_empty() && !fr.is_empty(), "{kind:?}");
        }
        assert_eq!(search_words(Kind::Cover, None), ["cover", "volet"]);
        assert_eq!(
            search_words(Kind::Switch, Some(SwitchType::Plug)),
            ["plug", "prise"]
        );
        assert_eq!(
            search_words(Kind::Switch, Some(SwitchType::Tv)),
            ["tv", "télé"]
        );
    }
}
