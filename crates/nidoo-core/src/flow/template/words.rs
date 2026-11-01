//! Numbers, units and state words as a notify text writes them, in English or French.

use super::Lang;

/// An on/off value: open/closed for openings, detected/clear for presence, else on/off.
pub fn on_off_word(on: bool, device_class: Option<&str>, lang: Lang) -> &'static str {
    let (en, fr) = match (device_class, on) {
        (Some("door" | "window" | "opening" | "garage_door"), true) => ("Open", "Ouvert"),
        (Some("door" | "window" | "opening" | "garage_door"), false) => ("Closed", "Fermé"),
        (Some("motion" | "occupancy" | "presence"), true) => ("Detected", "Détecté"),
        (Some("motion" | "occupancy" | "presence"), false) => ("Clear", "Rien"),
        (_, true) => ("On", "Allumé"),
        (_, false) => ("Off", "Éteint"),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// The state words a notify translates, as (raw, English, French).
const WORDS: &[(&str, &str, &str)] = &[
    // Weather conditions, the same words as the web app's weather card.
    ("clear", "Clear", "Dégagé"),
    ("partly_cloudy", "Partly cloudy", "Éclaircies"),
    ("cloudy", "Cloudy", "Nuageux"),
    ("fog", "Fog", "Brouillard"),
    ("drizzle", "Drizzle", "Bruine"),
    ("rain", "Rain", "Pluie"),
    ("freezing_rain", "Freezing rain", "Pluie verglaçante"),
    ("snow", "Snow", "Neige"),
    ("showers", "Showers", "Averses"),
    ("thunderstorm", "Thunderstorm", "Orage"),
    // Covers.
    ("open", "Open", "Ouvert"),
    ("closed", "Closed", "Fermé"),
    ("opening", "Opening", "Ouverture"),
    ("closing", "Closing", "Fermeture"),
    ("stopped", "Stopped", "Arrêté"),
    // Alarm panels.
    ("disarmed", "Disarmed", "Désarmée"),
    ("armed_home", "Armed (home)", "Armée (présence)"),
    ("armed_away", "Armed (away)", "Armée (absence)"),
    ("armed_night", "Armed (night)", "Armée (nuit)"),
    ("arming", "Arming", "Armement"),
    ("pending", "Pending", "En attente"),
    ("triggered", "Triggered", "Déclenchée"),
    // Climate modes.
    ("off", "Off", "Arrêt"),
    ("heat", "Heat", "Chauffage"),
    ("cool", "Cool", "Climatisation"),
    ("auto", "Auto", "Auto"),
    ("dry", "Dry", "Déshumidification"),
    ("fan_only", "Fan", "Ventilation"),
    ("heat_cool", "Heat/cool", "Chaud/froid"),
    ("on", "On", "Allumé"),
];

/// A text state in `lang`: a known word is translated, anything else is tidied
/// (`eco_mode` → `Eco mode`).
pub fn text_word(raw: &str, lang: Lang) -> String {
    if let Some(&(_, en, fr)) = WORDS.iter().find(|(word, ..)| *word == raw) {
        return match lang {
            Lang::En => en,
            Lang::Fr => fr,
        }
        .to_owned();
    }
    let spaced = raw.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => spaced,
    }
}

/// A number with `decimals` exact decimals (half away from zero), or at most 2 with trailing
/// zeros dropped; `.` or `,` as the decimal mark, never `-0`.
pub fn format_number(n: f64, decimals: Option<u8>, lang: Lang) -> String {
    let places = decimals.unwrap_or(2);
    let scale = 10f64.powi(i32::from(places));
    let mut rounded = (n * scale).round() / scale;
    if rounded == 0.0 {
        rounded = 0.0;
    }
    let mut text = format!("{rounded:.*}", usize::from(places));
    if decimals.is_none() && text.contains('.') {
        text.truncate(text.trim_end_matches('0').trim_end_matches('.').len());
    }
    match lang {
        Lang::En => text,
        Lang::Fr => text.replace('.', ","),
    }
}

/// `number` followed by its unit after a non-breaking space; `%` sticks to the number in English.
pub fn with_unit(number: String, unit: Option<&str>, lang: Lang) -> String {
    match unit.filter(|u| !u.is_empty()) {
        None => number,
        Some("%") if lang == Lang::En => number + "%",
        Some(unit) => format!("{number}\u{a0}{unit}"),
    }
}
