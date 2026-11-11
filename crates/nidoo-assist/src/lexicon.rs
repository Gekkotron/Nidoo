//! The French and English word tables, folded to ASCII lowercase. Generic only: room and device
//! names come from the house, never from here.

use nidoo_core::flow::Lang;
use nidoo_core::model::Hs;

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

/// The English table: the same classes, read by the same grammar. "turn" and "switch" are
/// noise: "on", "off", "up" and "down" carry the order ("turn the lights off").
const TABLE_EN: &[(Class, &[&str])] = &[
    (Class::On, &["on", "start", "starts", "enable", "activate"]),
    (
        Class::Off,
        &["off", "disable", "deactivate", "kill", "extinguish"],
    ),
    (Class::Set, &["set", "put", "make", "change", "adjust"]),
    (
        Class::Raise,
        &["raise", "increase", "up", "higher", "boost"],
    ),
    (
        Class::Lower,
        &["lower", "decrease", "reduce", "down", "drop"],
    ),
    (Class::Light, &["light", "lights", "lighting"]),
    (
        Class::LightNamed,
        &[
            "lamp",
            "lamps",
            "led",
            "leds",
            "bulb",
            "bulbs",
            "spotlight",
            "spotlights",
            "spot",
            "spots",
            "chandelier",
            "chandeliers",
            "nightlight",
            "sconce",
            "sconces",
            "garland",
            "garlands",
        ],
    ),
    (
        Class::Heating,
        &[
            "heating",
            "heat",
            "heater",
            "heaters",
            "radiator",
            "radiators",
            "thermostat",
            "thermostats",
            "ac",
            "aircon",
            "setpoint",
        ],
    ),
    (
        Class::Cover,
        &[
            "shutter", "shutters", "blind", "blinds", "shade", "shades", "roller", "rollers",
        ],
    ),
    (Class::Open, &["open", "opens", "reopen"]),
    (Class::Close, &["close", "closes", "shut", "reclose"]),
    (Class::Temperature, &["temperature", "temperatures", "temp"]),
    (Class::Degree, &["degree", "degrees", "°", "celsius"]),
    (Class::Percent, &["%", "percent"]),
    (Class::All, &["all", "every", "everything", "whole"]),
    (Class::Outdoor, &["outside", "outdoors", "outdoor", "out"]),
    (Class::Question, &["?", "what", "whats", "which", "how"]),
    (
        Class::Media,
        &["music", "song", "songs", "track", "tracks", "tune", "tunes"],
    ),
    (Class::Sound, &["sound", "volume"]),
    (Class::Pause, &["pause"]),
    (Class::Next, &["next", "skip"]),
    (Class::Previous, &["previous"]),
    (Class::Resume, &["resume", "unpause", "restart"]),
    (Class::Launch, &["launch"]),
    (Class::Stop, &["stop"]),
    (Class::Listen, &["listening", "listen", "playing"]),
    (Class::Fait, &["warm", "hot", "cold", "cool", "chilly"]),
    (Class::Little, &["slightly", "bit", "little", "tad"]),
    (
        Class::OtherDevice,
        &[
            "curtain", "curtains", "siren", "sirens", "fan", "fans", "alarm", "alarms", "lock",
            "locks", "door", "doors", "gate", "gates",
        ],
    ),
    (
        Class::Tv,
        &["tv", "tvs", "television", "televisions", "telly"],
    ),
    (
        Class::Plug,
        &["plug", "plugs", "socket", "sockets", "outlet", "outlets"],
    ),
    (
        Class::Negation,
        &[
            "not", "dont", "don", "never", "doesnt", "doesn", "didnt", "didn", "isnt", "isn",
            "arent", "aren", "no",
        ],
    ),
    (
        Class::Statement,
        &[
            "is", "are", "was", "were", "been", "who", "why", "did", "does", "have", "has", "had",
            "ve",
        ],
    ),
    (
        Class::Request,
        &[
            "can", "could", "would", "will", "want", "wanna", "need", "like",
        ],
    ),
    (
        Class::Linker,
        &[
            "in", "of", "at", "to", "by", "from", "inside", "for", "near",
        ],
    ),
    (
        Class::Noise,
        &[
            "the",
            "a",
            "an",
            "my",
            "our",
            "your",
            "his",
            "her",
            "their",
            "its",
            "and",
            "or",
            "it",
            "this",
            "that",
            "these",
            "those",
            "there",
            "here",
            "i",
            "me",
            "we",
            "us",
            "you",
            "s",
            "t",
            "d",
            "ll",
            "m",
            "re",
            "some",
            "please",
            "pls",
            "plz",
            "thanks",
            "thank",
            "hey",
            "hi",
            "hello",
            "ok",
            "okay",
            "now",
            "just",
            "again",
            "too",
            "also",
            "turn",
            "turns",
            "switch",
            "go",
            "tell",
            "show",
            "give",
            "let",
            "know",
            "be",
            "do",
            "get",
            "keep",
            "much",
            "right",
            "currently",
            "room",
            "back",
            "pull",
            "bring",
            "roll",
        ],
    ),
];

/// A light colour said after "en" ("allume la lampe en rouge"). The hues are the colour
/// picker's swatches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colour {
    White,
    Red,
    Orange,
    Yellow,
    Green,
    Cyan,
    Blue,
    Purple,
    Magenta,
    Pink,
}

impl Colour {
    /// The French name, as the reply says it.
    pub fn name(self) -> &'static str {
        self.name_in(Lang::Fr)
    }

    /// The name in `lang`, as the reply says it.
    pub fn name_in(self, lang: Lang) -> &'static str {
        if lang == Lang::En {
            return match self {
                Colour::White => "white",
                Colour::Red => "red",
                Colour::Orange => "orange",
                Colour::Yellow => "yellow",
                Colour::Green => "green",
                Colour::Cyan => "cyan",
                Colour::Blue => "blue",
                Colour::Purple => "purple",
                Colour::Magenta => "magenta",
                Colour::Pink => "pink",
            };
        }
        match self {
            Colour::White => "blanc",
            Colour::Red => "rouge",
            Colour::Orange => "orange",
            Colour::Yellow => "jaune",
            Colour::Green => "vert",
            Colour::Cyan => "cyan",
            Colour::Blue => "bleu",
            Colour::Purple => "violet",
            Colour::Magenta => "magenta",
            Colour::Pink => "rose",
        }
    }

    /// The hue and saturation sent to the light.
    pub fn hs(self) -> Hs {
        let (h, s) = match self {
            Colour::White => (0.0, 0.0),
            Colour::Red => (0.0, 100.0),
            Colour::Orange => (30.0, 100.0),
            Colour::Yellow => (55.0, 100.0),
            Colour::Green => (120.0, 100.0),
            Colour::Cyan => (180.0, 100.0),
            Colour::Blue => (225.0, 100.0),
            Colour::Purple => (270.0, 100.0),
            Colour::Magenta => (300.0, 100.0),
            Colour::Pink => (335.0, 100.0),
        };
        Hs { h, s }
    }
}

/// The colour words, folded, with every gender and number form.
const COLOURS: &[(&[&str], Colour)] = &[
    (&["blanc", "blanche", "blancs", "blanches"], Colour::White),
    (&["rouge", "rouges"], Colour::Red),
    (&["orange", "oranges"], Colour::Orange),
    (&["jaune", "jaunes"], Colour::Yellow),
    (&["vert", "verte", "verts", "vertes"], Colour::Green),
    (&["cyan", "cyans", "turquoise", "turquoises"], Colour::Cyan),
    (&["bleu", "bleue", "bleus", "bleues"], Colour::Blue),
    (
        &[
            "violet",
            "violette",
            "violets",
            "violettes",
            "mauve",
            "mauves",
        ],
        Colour::Purple,
    ),
    (&["magenta", "magentas"], Colour::Magenta),
    (&["rose", "roses"], Colour::Pink),
];

/// The English colour words, folded.
const COLOURS_EN: &[(&[&str], Colour)] = &[
    (&["white"], Colour::White),
    (&["red"], Colour::Red),
    (&["orange"], Colour::Orange),
    (&["yellow"], Colour::Yellow),
    (&["green"], Colour::Green),
    (&["cyan", "turquoise"], Colour::Cyan),
    (&["blue"], Colour::Blue),
    (&["purple", "violet", "mauve"], Colour::Purple),
    (&["magenta"], Colour::Magenta),
    (&["pink"], Colour::Pink),
];

/// The colour a folded French word names, exact words only: a typo never picks a colour.
pub fn colour_of(word: &str) -> Option<Colour> {
    colour_in(word, Lang::Fr)
}

/// The colour a folded word of `lang` names, exact words only.
pub fn colour_in(word: &str, lang: Lang) -> Option<Colour> {
    let table = match lang {
        Lang::Fr => COLOURS,
        Lang::En => COLOURS_EN,
    };
    table
        .iter()
        .find(|(words, _)| words.contains(&word))
        .map(|(_, c)| *c)
}

/// Words for a bedroom, which a room named after its person leaves out ("chambre de Léa" for a
/// room named "Léa"). Kept out of the table: they still reach the room words.
pub const BEDROOM: &[&str] = &["chambre", "chambres", "bedroom", "bedrooms"];

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
    "lounge",
    "dining",
    "study",
    "hallway",
    "corridor",
    "landing",
    "staircase",
    "toilet",
    "restroom",
    "laundry",
    "utility",
    "pantry",
    "basement",
    "cellar",
    "attic",
    "loft",
    "porch",
    "patio",
    "deck",
    "yard",
    "terrace",
    "balcony",
    "conservatory",
    "library",
    "gym",
    "workshop",
    "shed",
    "entrance",
    "entry",
    "foyer",
    "mudroom",
    "closet",
    "playroom",
    "den",
    "downstairs",
    "pool",
    "house",
    "home",
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
    class_in(word, Lang::Fr)
}

/// [`class`] in the table of `lang`.
pub fn class_in(word: &str, lang: Lang) -> Option<Class> {
    let table = match lang {
        Lang::Fr => TABLE,
        Lang::En => TABLE_EN,
    };
    for (class, words) in table {
        if words.contains(&word) {
            return Some(*class);
        }
    }
    let mut found = None;
    for (class, words) in table {
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
    fn colour_words_fold_every_form() {
        assert_eq!(colour_of("bleue").map(Colour::name), Some("bleu"));
        assert_eq!(colour_of("rouges").map(Colour::name), Some("rouge"));
        assert_eq!(colour_of("blanche").map(|c| c.hs().s), Some(0.0));
        assert_eq!(colour_of("rougee"), None);
        assert_eq!(colour_of("salon"), None);
    }

    #[test]
    fn no_word_is_in_two_tables() {
        for table in [TABLE, TABLE_EN] {
            let mut seen = std::collections::BTreeMap::new();
            for (class, words) in table {
                for w in *words {
                    assert_eq!(seen.insert(*w, *class), None, "{w} twice");
                }
            }
        }
    }

    #[test]
    fn english_words_and_typos() {
        assert_eq!(class_in("lights", Lang::En), Some(Class::Light));
        assert_eq!(class_in("ligts", Lang::En), Some(Class::Light));
        assert_eq!(class_in("off", Lang::En), Some(Class::Off));
        assert_eq!(class_in("eteins", Lang::En), None);
        assert_eq!(class_in("lights", Lang::Fr), None);
        assert_eq!(colour_in("blue", Lang::En), Some(Colour::Blue));
        assert_eq!(colour_in("bleu", Lang::En), None);
        assert_eq!(Colour::Blue.name_in(Lang::En), "blue");
    }
}
