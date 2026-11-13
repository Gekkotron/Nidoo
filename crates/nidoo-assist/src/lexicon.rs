//! The French word tables, folded to ASCII lowercase. Generic only: room and device names come
//! from the house, never from here.

/// What a known word means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    On,
    Off,
    /// "mets", "règle": a value follows.
    Set,
    Raise,
    Lower,
    /// A word for lights in general ("lumière").
    Light,
    /// A word for a kind of light that may also be part of its name ("led", "lampe").
    LightNamed,
    Heating,
    Temperature,
    Degree,
    Percent,
    All,
    Outdoor,
    Question,
    /// "il fait combien".
    Fait,
    /// "un peu", "légèrement".
    Little,
    /// A word for shutters ("volet", "store"; "roulant" of "volet roulant").
    Cover,
    /// "ouvre": opens a shutter.
    Open,
    /// "ferme": closes a shutter.
    Close,
    /// A word for a television ("télé", "tv").
    Tv,
    /// A word for a plug ("prise").
    Plug,
    /// A device this version does not handle (curtains, sirens...).
    OtherDevice,
    /// "ne ... pas": never act on a negated order.
    Negation,
    /// A statement or a past tense ("est", "qui", "j'ai"): describes, never orders.
    Statement,
    /// "peux-tu", "voudrais": a polite request, which is an order even as a question.
    Request,
    /// A word that introduces the room ("du", "dans", "chez").
    Linker,
    /// A word for music ("musique", "chanson", "morceau", "titre").
    Media,
    /// "son", "volume": the speakers' sound. "son" is a possessive unless an article precedes it.
    Sound,
    /// "pause".
    Pause,
    /// "suivante": the next song.
    Next,
    /// "précédente": the previous song.
    Previous,
    /// "reprends", "relance", "remets": resume the music, or the sound.
    Resume,
    /// "lance": start the music.
    Launch,
    /// "stop": pause the music (the player has no stop).
    Stop,
    /// "reviens" of "reviens en arrière".
    Back,
    /// "écoute" of "qu'est-ce qu'on écoute ?".
    Listen,
    Noise,
}

const TABLE: &[(Class, &[&str])] = &[
    (
        Class::On,
        &[
            "allume", "allumer", "allumes", "allumez", "rallume", "rallumer", "active", "activer",
            "actives", "activez", "demarre", "demarrer",
        ],
    ),
    (
        Class::Off,
        &[
            "eteins",
            "eteint",
            "eteindre",
            "eteignez",
            "coupe",
            "couper",
            "coupes",
            "coupez",
            "desactive",
            "desactiver",
            "desactives",
            "desactivez",
            "arrete",
            "arreter",
            "arretes",
            "arretez",
        ],
    ),
    (
        Class::Set,
        &[
            "mets", "met", "mettre", "mettez", "regle", "regler", "regles", "reglez", "passe",
            "passer", "passes", "passez",
            // Speech to text writes the homophone "mais" for "mets"; "but" has no use in an order.
            "mais",
        ],
    ),
    (
        Class::Raise,
        &[
            "monte",
            "monter",
            "montes",
            "montez",
            "remonte",
            "remonter",
            "remontez",
            "augmente",
            "augmenter",
            "augmentes",
            "augmentez",
            "hausse",
            "hausser",
        ],
    ),
    (
        Class::Lower,
        &[
            "baisse",
            "baisser",
            "baisses",
            "baissez",
            "rebaisse",
            "descends",
            "descend",
            "descendre",
            "descendez",
            "diminue",
            "diminuer",
            "diminues",
            "diminuez",
        ],
    ),
    (
        Class::Light,
        &[
            "lumiere",
            "lumieres",
            "eclairage",
            "eclairages",
            "luminaire",
            "luminaires",
        ],
    ),
    (
        Class::LightNamed,
        &[
            "lampe",
            "lampes",
            "led",
            "leds",
            "plafonnier",
            "plafonniers",
            "ampoule",
            "ampoules",
            "guirlande",
            "guirlandes",
            "lampadaire",
            "lampadaires",
            "applique",
            "appliques",
            "spot",
            "spots",
            "veilleuse",
        ],
    ),
    (
        Class::Heating,
        &[
            "chauffage",
            "chauffages",
            "radiateur",
            "radiateurs",
            "clim",
            "climatisation",
            "climatiseur",
            "thermostat",
            "consigne",
        ],
    ),
    (
        Class::Cover,
        &["volet", "volets", "store", "stores", "roulant", "roulants"],
    ),
    (
        Class::Open,
        &["ouvre", "ouvrir", "ouvres", "ouvrez", "rouvre", "rouvrir"],
    ),
    (
        Class::Close,
        &["ferme", "fermer", "fermes", "fermez", "referme", "refermer"],
    ),
    (Class::Temperature, &["temperature", "temperatures"]),
    (Class::Degree, &["degre", "degres", "°"]),
    (Class::Percent, &["%", "pourcent", "pourcents"]),
    (Class::All, &["tout", "tous", "toute", "toutes"]),
    (Class::Outdoor, &["dehors", "exterieur"]),
    (
        Class::Question,
        &["?", "quelle", "quel", "quels", "quelles", "combien", "quoi"],
    ),
    (
        Class::Media,
        &[
            "musique", "musiques", "chanson", "chansons", "morceau", "morceaux", "titre", "titres",
        ],
    ),
    (Class::Sound, &["son", "volume"]),
    (Class::Pause, &["pause"]),
    (Class::Next, &["suivante", "suivant"]),
    (Class::Previous, &["precedente", "precedent"]),
    (
        Class::Resume,
        &[
            "reprends",
            "reprend",
            "reprendre",
            "reprenez",
            "relance",
            "relancer",
            "relances",
            "relancez",
            "remets",
            "remet",
            "remettre",
            "remettez",
        ],
    ),
    (Class::Launch, &["lance", "lancer", "lances", "lancez"]),
    (Class::Stop, &["stop"]),
    (Class::Back, &["reviens", "revenir", "revient", "retourne"]),
    (Class::Listen, &["ecoute", "ecoutes", "ecouter", "ecoutez"]),
    (Class::Fait, &["fait"]),
    (Class::Little, &["legerement"]),
    (
        Class::OtherDevice,
        &[
            "rideau",
            "rideaux",
            "sirene",
            "sirenes",
            "ventilateur",
            "ventilateurs",
            "alarme",
            "alarmes",
        ],
    ),
    (
        Class::Tv,
        &["tele", "teles", "television", "televisions", "tv", "tvs"],
    ),
    (Class::Plug, &["prise", "prises"]),
    (Class::Negation, &["ne", "n", "pas", "jamais"]),
    (
        Class::Statement,
        &[
            "est", "sont", "etait", "etaient", "ete", "qui", "pourquoi", "ai", "as", "avons",
            "avez", "ont",
        ],
    ),
    (
        Class::Request,
        &[
            "peux", "peut", "pouvez", "pourrais", "pourriez", "veux", "voudrais", "veuillez",
        ],
    ),
    (
        Class::Linker,
        &[
            "du", "de", "des", "d", "dans", "au", "aux", "chez", "a", "sur", "en",
        ],
    ),
    (
        Class::Noise,
        &[
            "le",
            "la",
            "les",
            "l",
            "et",
            "ou",
            "il",
            "elle",
            "on",
            "te",
            "tu",
            "je",
            "j",
            "me",
            "m",
            "moi",
            "nous",
            "vous",
            "ce",
            "ca",
            "cela",
            "qu",
            "que",
            "s",
            "y",
            "si",
            "svp",
            "stp",
            "plait",
            "merci",
            "bonjour",
            "salut",
            // Fillers heard by a satellite ("ah salut ? quelle est…"): never a room word.
            "ah",
            "oh",
            "euh",
            "bah",
            "hein",
            "maintenant",
            "ok",
            "vas",
            "dis",
            "dites",
            "donne",
            "donnez",
            "affiche",
            "indique",
            "montre",
            "montrez",
            "fais",
            "faire",
            "piece",
            "dire",
            "donner",
            "savoir",
            "laisse",
            "laisser",
            "laissez",
            "laisses",
            "montrer",
            "monde",
            "couple",
            "couples",
            "allure",
            "metre",
            "metres",
            "allumee",
            "allumees",
            "eteinte",
            "eteintes",
            "coupee",
            "coupees",
        ],
    ),
];

/// Words for a bedroom, which a room named after its person leaves out ("chambre de Léa" for a
/// room named "Léa"). Kept out of the table: they still reach the room words.
pub const BEDROOM: &[&str] = &["chambre", "chambres"];

/// Words that say a room is not a bedroom (French, plus the English of discovered area ids), so
/// that a bedroom word never lands on the living room. Generic room kinds, never a house's names.
pub const NOT_BEDROOM: &[&str] = &[
    "salon",
    "sejour",
    "living",
    "cuisine",
    "kitchen",
    "bureau",
    "office",
    "salle",
    "bain",
    "bath",
    "bathroom",
    "douche",
    "shower",
    "wc",
    "toilette",
    "toilettes",
    "garage",
    "dehors",
    "exterieur",
    "outside",
    "jardin",
    "garden",
    "terrasse",
    "balcon",
    "escalier",
    "stair",
    "stairs",
    "etage",
    "upstairs",
    "couloir",
    "hall",
    "entree",
    "cave",
    "cellier",
    "grenier",
    "buanderie",
    "veranda",
    "dressing",
];

/// Classes a typo may reach. Set, raise and lower never do: a typo must not invent a setpoint
/// ("laisse" is not "baisse").
fn fuzzy(class: Class) -> bool {
    matches!(
        class,
        Class::On
            | Class::Off
            | Class::Light
            | Class::LightNamed
            | Class::Heating
            | Class::Temperature
            | Class::Degree
            | Class::OtherDevice
            | Class::Tv
            | Class::Plug
    )
}

/// The class of a folded word: an exact entry, else a one-edit typo of a 5+ letter entry when
/// every such entry agrees on the class (so a typo never decides between on and off, or
/// between two intents).
pub fn class(word: &str) -> Option<Class> {
    for (class, words) in TABLE {
        if words.contains(&word) {
            return Some(*class);
        }
    }
    let mut found = None;
    for (class, words) in TABLE {
        if !fuzzy(*class) {
            continue;
        }
        if words.iter().any(|w| near(w, word)) {
            match found {
                None => found = Some(*class),
                Some(c) if c == *class => {}
                Some(_) => return None,
            }
        }
    }
    found
}

/// Equal, or one edit apart with both words at least 5 letters long.
pub fn near(a: &str, b: &str) -> bool {
    a == b || (a.chars().count() >= 5 && b.chars().count() >= 5 && levenshtein(a, b) <= 1)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_is_one_edit_on_long_words() {
        assert!(near("allumme", "allume"));
        assert!(near("etein", "eteins"));
        assert!(!near("led", "les"));
        assert!(!near("allume", "allumerez"));
    }

    #[test]
    fn typos_reach_one_class_only() {
        assert_eq!(class("allumme"), Some(Class::On));
        assert_eq!(class("etein"), Some(Class::Off));
        assert_eq!(class("salon"), None);
        // Exact entries beat typos: "montre" is not "monte".
        assert_eq!(class("montre"), Some(Class::Noise));
    }

    #[test]
    fn no_word_is_in_two_tables() {
        let mut seen = std::collections::BTreeMap::new();
        for (class, words) in TABLE {
            for w in *words {
                assert_eq!(seen.insert(*w, *class), None, "{w} twice");
            }
        }
    }
}
