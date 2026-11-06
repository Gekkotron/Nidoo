//! The word matching shared by the MCP `search` tool and the web search page, written for
//! people and models that phrase a question ("température du salon") rather than an exact
//! name.
//!
//! The query is lowercased, folded to ASCII (`é` → `e`) and split into words; articles and
//! prepositions are dropped. An entity matches when every remaining word is found in its
//! name, id, device, room (id or name), kind, device class, or the French words for those.
//! A device matches when every word is found in its friendly name, model or vendor.

use std::collections::HashMap;

use crate::{AreaView, DeviceView, EntityView};
use nidoo_core::model::{Kind, SwitchType};

/// Words that carry no meaning for a search, in French and English.
const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "au", "aux", "d", "dans", "de", "des", "du", "en", "et", "in", "l", "la",
    "le", "les", "of", "the", "un", "une",
];

/// A parsed query: folded words, stop words removed.
pub struct Query {
    words: Vec<String>,
}

impl Query {
    pub fn parse(query: &str) -> Self {
        let words = fold(query)
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|word| !word.is_empty() && !STOP_WORDS.contains(word))
            .map(str::to_owned)
            .collect();
        Self { words }
    }

    /// True when the query has no words left (empty, or only stop words).
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// True when every word is in `text` (already folded). A query of stop words only
    /// matches everything, like an empty query did.
    fn all_in(&self, text: &str) -> bool {
        self.words.iter().all(|word| text.contains(word.as_str()))
    }

    pub fn device_matches(&self, device: &DeviceView) -> bool {
        self.all_in(&device_text(device))
    }

    /// True when every word is in the room's id or name.
    pub fn area_matches(&self, area: &AreaView) -> bool {
        self.all_in(&fold(&format!("{}\n{}", area.id, area.name)))
    }
}

/// Matches entities against a query, with the rooms and devices of one snapshot.
pub struct EntityMatcher<'a> {
    query: &'a Query,
    areas: HashMap<&'a str, &'a str>,
    devices: HashMap<&'a str, String>,
}

impl<'a> EntityMatcher<'a> {
    pub fn new(query: &'a Query, areas: &'a [AreaView], devices: &'a [DeviceView]) -> Self {
        Self {
            query,
            areas: areas
                .iter()
                .map(|area| (area.id.as_str(), area.name.as_str()))
                .collect(),
            devices: devices
                .iter()
                .map(|device| (device.id.as_str(), device_text(device)))
                .collect(),
        }
    }

    pub fn matches(&self, entity: &EntityView) -> bool {
        let mut text = entity_text(entity, self.areas.get(entity.area.as_str()).copied());
        if let Some(device) = entity
            .device_id
            .as_deref()
            .and_then(|id| self.devices.get(id))
        {
            text.push('\n');
            text.push_str(device);
        }
        self.query.all_in(&text)
    }
}

/// Lowercases `text` and folds accented letters to ASCII (`Lumière` becomes `lumiere`).
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            'à' | 'á' | 'â' | 'ä' => out.push('a'),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'í' | 'î' | 'ï' => out.push('i'),
            'ó' | 'ô' | 'ö' => out.push('o'),
            'ú' | 'ù' | 'û' | 'ü' => out.push('u'),
            'ç' => out.push('c'),
            'œ' => out.push_str("oe"),
            'æ' => out.push_str("ae"),
            other => out.push(other),
        }
    }
    out
}

/// The folded fields a device is found by, one per line so a word never spans two fields.
fn device_text(device: &DeviceView) -> String {
    let fields = [
        Some(device.friendly_name.as_str()),
        device.model.as_deref(),
        device.vendor.as_deref(),
    ];
    fold(&fields.into_iter().flatten().collect::<Vec<_>>().join("\n"))
}

/// The folded fields an entity is found by, one per line.
fn entity_text(entity: &EntityView, area_name: Option<&str>) -> String {
    let switch_word = SwitchType::of(entity.protocol_kind, &entity.ui)
        .filter(|t| *t != SwitchType::Switch)
        .map(SwitchType::label);
    let mut fields: Vec<&str> = vec![&entity.name, &entity.id, &entity.area, &entity.component];
    fields.extend(entity.device_name.as_deref());
    fields.extend(area_name);
    fields.extend(switch_word);
    fields.extend(entity.device_class.as_deref());
    fields.extend(kind_words(entity.kind));
    fields.extend(
        entity
            .device_class
            .as_deref()
            .map(class_words)
            .unwrap_or_default(),
    );
    fold(&fields.join("\n"))
}

/// The English and French words for a kind, folded.
fn kind_words(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Light => &["light", "lumiere", "lampe"],
        Kind::Switch => &["switch", "interrupteur", "prise"],
        Kind::Cover => &["cover", "volet", "store"],
        Kind::Climate => &["climate", "climatisation", "chauffage", "thermostat"],
        Kind::Sensor => &["sensor", "capteur"],
        Kind::BinarySensor => &["binary_sensor", "capteur", "detecteur"],
        Kind::AlarmPanel => &["alarm", "alarme"],
        Kind::Number => &["number", "nombre"],
        Kind::Select => &["select", "choix"],
        Kind::Button => &["button", "bouton"],
        Kind::Vacuum => &["vacuum", "aspirateur"],
        Kind::WaterHeater => &["water_heater", "chauffe-eau", "ballon"],
        Kind::MediaPlayer => &["media_player"],
        Kind::Generic => &[],
    }
}

/// The French words for the device classes a house commonly reports, folded; the class
/// itself is matched as is.
fn class_words(class: &str) -> &'static [&'static str] {
    match class {
        "humidity" => &["humidite"],
        "power" => &["puissance"],
        "energy" => &["energie", "consommation"],
        "battery" => &["batterie", "pile"],
        "illuminance" => &["luminosite"],
        "motion" | "occupancy" | "presence" => &["presence", "mouvement"],
        "door" => &["porte"],
        "window" => &["fenetre"],
        "opening" => &["ouverture"],
        "pressure" | "atmospheric_pressure" => &["pression"],
        "voltage" => &["tension"],
        "current" => &["courant"],
        "smoke" => &["fumee"],
        "moisture" => &["fuite", "humidite"],
        "gas" => &["gaz"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_folds_accents_and_drops_stop_words() {
        let query = Query::parse("Température du Salon");
        assert_eq!(query.words, ["temperature", "salon"]);
        let query = Query::parse("lumière de l'entrée");
        assert_eq!(query.words, ["lumiere", "entree"]);
    }

    #[test]
    fn parse_keeps_snake_case_ids_whole() {
        let query = Query::parse("water_heater_power");
        assert_eq!(query.words, ["water_heater_power"]);
    }

    #[test]
    fn every_word_must_match() {
        let query = Query::parse("temperature salon");
        assert!(query.all_in("temperature\nliving\nsalon"));
        assert!(!query.all_in("temperature\nkitchen\ncuisine"));
    }

    #[test]
    fn a_query_of_stop_words_matches_everything() {
        assert!(Query::parse("de la").all_in("anything"));
    }

    #[test]
    fn french_words_cover_kinds_and_classes() {
        assert!(kind_words(Kind::Light).contains(&"lumiere"));
        assert!(kind_words(Kind::Cover).contains(&"volet"));
        assert!(class_words("humidity").contains(&"humidite"));
        assert!(class_words("unknown_class").is_empty());
    }

    fn area(id: &str, name: &str) -> AreaView {
        AreaView {
            id: id.to_owned(),
            name: name.to_owned(),
            icon: None,
            order: None,
            entity_count: 0,
            primary_count: 0,
            group_names: Default::default(),
            group_favorites: Default::default(),
        }
    }

    #[test]
    fn area_matches_name_or_id_every_word() {
        let a = area("kitchen_1", "Cuisine d'été");
        assert!(Query::parse("cuisine ete").area_matches(&a));
        assert!(Query::parse("KITCHEN_1").area_matches(&a));
        assert!(!Query::parse("cuisine garage").area_matches(&a));
    }

    #[test]
    fn is_empty_after_stop_words() {
        assert!(Query::parse("  le de the ").is_empty());
        assert!(!Query::parse("le salon").is_empty());
    }
}
