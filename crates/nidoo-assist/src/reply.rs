//! The replies, written from templates, in French or in English (the house's language). Every
//! reply names the room, the action and the value.

use nidoo_api::search::fold;

use nidoo_core::flow::Lang;
use nidoo_core::model::{MediaPlayerState, SwitchType};

use crate::house::House;
use crate::intent::MediaIntent;
use crate::lexicon::{Class, class, class_in};
use crate::plan::Done;
use crate::room::{name_words, named_after_person};

/// What happened to one action, in the plan's order.
#[derive(Clone, Debug, PartialEq)]
pub enum ActionResult {
    Ok,
    /// Nothing to do: already in the wanted state.
    Skipped,
    /// The device is offline: nothing was sent, and the reply says it did not answer.
    Offline,
    Failed(String),
    /// Done, and the media player's state its echo carried.
    Echo(MediaPlayerState),
}

/// The French reason the hub gives when a device never echoed (see the hub's `reason_fr`).
pub const NO_ANSWER: &str = "pas de réponse de l'appareil";

/// [`NO_ANSWER`] in English.
pub const NO_ANSWER_EN: &str = "no answer from the device";

/// A number with a decimal comma, one decimal at most, no ",0": `18,2`, `20`.
pub fn number(x: f64) -> String {
    number_in(x, Lang::Fr)
}

/// [`number`] in `lang`: a decimal point in English (`18.2`), a comma in French (`18,2`).
pub fn number_in(x: f64, lang: Lang) -> String {
    let r = (x * 10.0).round() / 10.0;
    if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else if lang == Lang::En {
        format!("{r:.1}")
    } else {
        format!("{r:.1}").replace('.', ",")
    }
}

/// A temperature: `18,2 °C`, `20 °C`.
pub fn temp(c: f64) -> String {
    temp_in(c, Lang::Fr)
}

/// [`temp`] in `lang`: `18.2 °C` in English.
pub fn temp_in(c: f64, lang: Lang) -> String {
    format!("{} °C", number_in(c, lang))
}

/// A name in the quotes of `lang`: « Baie » in French, "Desk" in English.
pub fn quoted(name: &str, lang: Lang) -> String {
    match lang {
        Lang::En => format!("\"{name}\""),
        Lang::Fr => format!("« {name} »"),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Gender {
    Masculine,
    Feminine,
    /// Starts with a vowel: `l'`.
    Elided,
    /// A room named after its person: `la chambre de Léa`.
    Person,
    /// Not a common room word, yet not a bedroom ("Toilettes"): no article.
    Bare,
}

const FEMININE: &[&str] = &[
    "chambre",
    "cuisine",
    "salle",
    "buanderie",
    "cave",
    "terrasse",
    "veranda",
    "mezzanine",
    "piscine",
    "bibliotheque",
    "lingerie",
    "cour",
    "maison",
    "suite",
    "chaufferie",
    "loggia",
    "serre",
    "remise",
    "cabane",
    "dependance",
    "laverie",
    "piece",
];
const MASCULINE: &[&str] = &[
    "salon",
    "bureau",
    "garage",
    "sejour",
    "couloir",
    "grenier",
    "cellier",
    "dressing",
    "jardin",
    "hall",
    "palier",
    "escalier",
    "studio",
    "debarras",
    "sous",
    "wc",
    "cabinet",
    "balcon",
    "patio",
    "porche",
    "rez",
    "living",
    "dortoir",
    "vestibule",
    "local",
    "spa",
    "sauna",
    "parking",
    "abri",
    "coin",
    "grand",
    "petit",
];

fn gender(name: &str) -> Gender {
    let folded = fold(name);
    let first = folded
        .split(|c: char| !c.is_alphanumeric())
        .find(|w| !w.is_empty())
        .unwrap_or("");
    if FEMININE.contains(&first) {
        Gender::Feminine
    } else if MASCULINE.contains(&first) {
        Gender::Masculine
    } else if first.starts_with(['a', 'e', 'i', 'o', 'u', 'y']) {
        Gender::Elided
    } else if named_after_person(name) {
        Gender::Person
    } else {
        Gender::Bare
    }
}

/// The room as a sentence starts it: `Salon`, `Chambre de Léa` for a room named after its
/// person.
pub fn room(name: &str) -> String {
    match gender(name) {
        Gender::Person => format!("Chambre de {name}"),
        _ => name.to_string(),
    }
}

/// [`room`] in `lang`: `Kitchen`, `Tom's room` in English.
pub fn room_in(name: &str, lang: Lang) -> String {
    match lang {
        Lang::Fr => room(name),
        Lang::En if person_en(name) => format!("{name}'s room"),
        Lang::En => name.to_string(),
    }
}

/// `le Salon`, `la Chambre Léa`, `l'Entrée`, `la chambre de Léa`.
pub fn le(name: &str) -> String {
    match gender(name) {
        Gender::Masculine => format!("le {name}"),
        Gender::Feminine => format!("la {name}"),
        Gender::Elided => format!("l'{name}"),
        Gender::Person => format!("la chambre de {name}"),
        Gender::Bare => name.to_string(),
    }
}

/// The English room with its article: `the Kitchen`, `Tom's room` for a room named after its
/// person, and the bare name when it has its own article or is a possessive (`The Den`,
/// `Tom's Den`).
pub fn the(name: &str) -> String {
    if own_article(name) {
        name.to_string()
    } else if person_en(name) {
        format!("{name}'s room")
    } else {
        format!("the {name}")
    }
}

/// An English room named after its person ("Tom"): not one with its own article, nor one
/// that says it is a room ("TV room").
fn person_en(name: &str) -> bool {
    !own_article(name)
        && named_after_person(name)
        && !name_words(name)
            .iter()
            .any(|w| matches!(w.as_str(), "room" | "rooms"))
}

/// An English name that takes no article: it starts with "the" or is a possessive.
fn own_article(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    lower == "the" || lower.starts_with("the ") || lower.contains("'s") || lower.contains("’s")
}

/// [`the`] as a sentence starts it: `The Kitchen`, `Tom's room`.
pub fn the_start(name: &str) -> String {
    capitalised(&the(name))
}

/// `in the Kitchen`, `in Tom's room`.
pub fn in_the(name: &str) -> String {
    format!("in {}", the(name))
}

/// Where in a room, in `lang`: `dans le Salon`, `in the Kitchen`.
pub fn place(name: &str, lang: Lang) -> String {
    match lang {
        Lang::Fr => dans(name),
        Lang::En => in_the(name),
    }
}

/// `dans le Salon`, `dans la Chambre Léa`.
pub fn dans(name: &str) -> String {
    format!("dans {}", le(name))
}

/// `du Salon`, `de la Chambre Léa`, `de l'Entrée`, `de la chambre de Léa`.
pub fn de(name: &str) -> String {
    match gender(name) {
        Gender::Masculine => format!("du {name}"),
        Gender::Feminine => format!("de la {name}"),
        Gender::Elided => format!("de l'{name}"),
        Gender::Person => format!("de la chambre de {name}"),
        Gender::Bare => format!("de {name}"),
    }
}

/// The masculine or feminine form, after the room's gender ("chambre" for a person's room).
pub fn agree<'a>(name: &str, masculine: &'a str, feminine: &'a str) -> &'a str {
    if matches!(gender(name), Gender::Feminine | Gender::Person) {
        feminine
    } else {
        masculine
    }
}

/// The first letter in upper case.
fn capitalised(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The reply after acting: what was done, what was already so, what did not answer, in the
/// house's language.
pub fn done(done: &Done, results: &[ActionResult], house: &House, simulate: bool) -> String {
    let lang = house.lang;
    let en = lang == Lang::En;
    let text = match done {
        Done::Lights {
            area,
            on,
            names,
            colour,
            unable,
        } => {
            let mut text = lights(
                house.area_name(area),
                *on,
                colour.as_deref(),
                names,
                results,
                lang,
            );
            match (unable.as_slice(), en) {
                ([], _) => {}
                ([one], false) => text.push_str(&format!(" « {one} » ne prend pas de couleur.")),
                ([one], true) => text.push_str(&format!(" \"{one}\" can't take a colour.")),
                (many, false) => text.push_str(&format!(
                    " Ne prennent pas de couleur : {}.",
                    many.join(", ")
                )),
                (many, true) => {
                    text.push_str(&format!(" Can't take a colour: {}.", many.join(", ")))
                }
            }
            text
        }
        Done::Setpoint { zone, from, to } => {
            setpoint(house.area_name(zone), *from, *to, results, lang)
        }
        Done::Covers {
            area,
            open,
            position,
            names,
            unable,
        } => covers(
            area.as_deref().map(|a| house.area_name(a)),
            *open,
            *position,
            names,
            unable,
            results,
            lang,
        ),
        Done::Switches {
            what,
            area,
            on,
            names,
        } => switches(
            *what,
            area.as_deref().map(|a| house.area_name(a)),
            *on,
            names,
            results,
            lang,
        ),
        Done::Media { what, .. } => media(what, results.first(), lang),
        Done::Volume { area, level } => {
            let (changed, already) = if en {
                ("Volume changed", "Volume already set")
            } else {
                ("Volume changé", "Volume déjà là")
            };
            let at = if en { "Volume at" } else { "Volume à" };
            speakers(
                house.area_name(area),
                &level.map_or_else(|| changed.into(), |l| format!("{at} {l} %")),
                already,
                results,
                lang,
            )
        }
        Done::Mute { area, on } => {
            let (done, already) = match (*on, en) {
                (true, false) => ("Son coupé", "Son déjà coupé"),
                (false, false) => ("Son remis", "Son déjà remis"),
                (true, true) => ("Sound muted", "Sound already muted"),
                (false, true) => ("Sound back on", "Sound already on"),
            };
            speakers(house.area_name(area), done, already, results, lang)
        }
    };
    match (simulate, en) {
        (false, _) => text,
        (true, false) => format!("Simulation : {text}"),
        (true, true) => format!("Simulation: {text}"),
    }
}

/// Whether a hub failure reason means the device never echoed.
fn no_answer(why: &str) -> bool {
    why == NO_ANSWER || why == NO_ANSWER_EN
}

/// A media player's reply, from its echo when there is one.
fn media(what: &MediaIntent, result: Option<&ActionResult>, lang: Lang) -> String {
    if lang == Lang::En {
        return media_en(what, result);
    }
    let plain = || match what {
        MediaIntent::Pause => "Musique en pause.".to_string(),
        MediaIntent::Play { resume: true } => "Je reprends la musique.".into(),
        MediaIntent::Play { resume: false } => "Je lance la musique.".into(),
        MediaIntent::PlayMedia { query } => format!("Je cherche « {query} »."),
        MediaIntent::Next => "Chanson suivante.".into(),
        MediaIntent::Previous => "Chanson précédente.".into(),
        MediaIntent::NowPlaying => String::new(),
    };
    match result {
        Some(ActionResult::Echo(state)) => match (what, &state.title) {
            (MediaIntent::Pause, _) | (_, None) => plain(),
            (MediaIntent::Play { resume: true }, Some(title)) => format!("Je reprends {title}."),
            (MediaIntent::Next | MediaIntent::Previous, Some(title)) => match &state.artist {
                Some(artist) => format!("{title}, {artist}."),
                None => format!("{title}."),
            },
            (_, Some(title)) => format!("Je joue {title}."),
        },
        Some(ActionResult::Skipped) => match what {
            MediaIntent::Pause => "La musique est déjà en pause.".into(),
            _ => "La musique joue déjà.".into(),
        },
        Some(ActionResult::Offline) => "Le lecteur ne répond pas.".into(),
        Some(ActionResult::Failed(why)) if no_answer(why) => "Le lecteur n'a pas répondu.".into(),
        Some(ActionResult::Failed(why)) => format!("Le lecteur n'a pas répondu : {why}."),
        Some(ActionResult::Ok) | None => plain(),
    }
}

/// [`media`] in English.
fn media_en(what: &MediaIntent, result: Option<&ActionResult>) -> String {
    let plain = || match what {
        MediaIntent::Pause => "Music paused.".to_string(),
        MediaIntent::Play { resume: true } => "Resuming the music.".into(),
        MediaIntent::Play { resume: false } => "Starting the music.".into(),
        MediaIntent::PlayMedia { query } => format!("Looking for \"{query}\"."),
        MediaIntent::Next => "Next song.".into(),
        MediaIntent::Previous => "Previous song.".into(),
        MediaIntent::NowPlaying => String::new(),
    };
    match result {
        Some(ActionResult::Echo(state)) => match (what, &state.title) {
            (MediaIntent::Pause, _) | (_, None) => plain(),
            (MediaIntent::Play { resume: true }, Some(title)) => format!("Resuming {title}."),
            (MediaIntent::Next | MediaIntent::Previous, Some(title)) => match &state.artist {
                Some(artist) => format!("{title}, {artist}."),
                None => format!("{title}."),
            },
            (_, Some(title)) => format!("Playing {title}."),
        },
        Some(ActionResult::Skipped) => match what {
            MediaIntent::Pause => "The music is already paused.".into(),
            _ => "The music is already playing.".into(),
        },
        Some(ActionResult::Offline) => "The player isn't responding.".into(),
        Some(ActionResult::Failed(why)) if no_answer(why) => "The player didn't answer.".into(),
        Some(ActionResult::Failed(why)) => format!("The player didn't answer: {why}."),
        Some(ActionResult::Ok) | None => plain(),
    }
}

/// A room's speakers: `done` when one of them did it, `already` when all were already there,
/// else what went wrong.
fn speakers(area: &str, done: &str, already: &str, results: &[ActionResult], lang: Lang) -> String {
    let place = place(area, lang);
    let ok = |r: &ActionResult| matches!(r, ActionResult::Ok | ActionResult::Echo(_));
    if results.iter().any(ok) {
        return format!("{done} {place}.");
    }
    if !results.is_empty() && results.iter().all(|r| *r == ActionResult::Skipped) {
        return format!("{already} {place}.");
    }
    let why = results.iter().find_map(|r| match r {
        ActionResult::Failed(why) => Some(why.as_str()),
        _ => None,
    });
    match (why, lang) {
        (Some(why), Lang::Fr) => format!("L'enceinte n'a pas répondu {place} : {why}."),
        (None, Lang::Fr) => format!("L'enceinte ne répond pas {place}."),
        (Some(why), Lang::En) => format!("The speaker didn't answer {place}: {why}."),
        (None, Lang::En) => format!("The speaker isn't responding {place}."),
    }
}

/// One device, labelled by its kind word and its name: « Volet « Baie » », without repeating
/// the kind word when the name starts with one (« « Volet tv » ») and as the bare name when the
/// name is only that word (« Volet »). `same` tells a folded word that stands for the kind word
/// and has its gender, since the reply's verb agrees with the kind word.
fn device_label(kind: &str, same: impl Fn(&str) -> bool, name: &str) -> String {
    let words = name_words(name);
    if words.first().is_some_and(|w| same(w)) {
        if words.len() == 1 {
            return capitalised(name.trim());
        }
        return format!("« {name} »");
    }
    format!("{kind} « {name} »")
}

/// The English label of one device: the name alone when one of its words says the kind of
/// device ("Desk lamp", "Volet"), else the kind word and the quoted name (`Shutter "Baie"`).
fn device_label_en(kind: &str, kinds: &[Class], name: &str) -> String {
    let says_kind = |w: &String| {
        [Lang::En, Lang::Fr]
            .iter()
            .any(|l| class_in(w, *l).is_some_and(|c| kinds.contains(&c)))
    };
    if name_words(name).iter().any(says_kind) {
        capitalised(name.trim())
    } else {
        format!("{kind} \"{name}\"")
    }
}

/// A light word that is feminine like « Lumière » (« allumée » agrees with it): « Lampe »,
/// not « Plafonnier ».
fn feminine_light(word: &str) -> bool {
    matches!(
        word,
        "lumiere"
            | "lumieres"
            | "lampe"
            | "lampes"
            | "led"
            | "leds"
            | "ampoule"
            | "ampoules"
            | "guirlande"
            | "guirlandes"
            | "applique"
            | "appliques"
            | "veilleuse"
    )
}

/// The devices that failed, with why: the hub's reason, or offline.
fn failures<'a>(
    names: &'a [String],
    results: &'a [ActionResult],
    lang: Lang,
) -> Vec<(&'a str, &'a str)> {
    let offline = match lang {
        Lang::Fr => "hors ligne",
        Lang::En => "offline",
    };
    names
        .iter()
        .zip(results)
        .filter_map(|(name, r)| match r {
            ActionResult::Failed(why) => Some((name.as_str(), why.as_str())),
            ActionResult::Offline => Some((name.as_str(), offline)),
            _ => None,
        })
        .collect()
}

/// " 1 sur 3 n'a pas répondu : Led." / " 1 of 3 didn't answer: Led.", empty when none failed.
fn some_failed(failed: &[(&str, &str)], n: usize, lang: Lang) -> String {
    if failed.is_empty() {
        return String::new();
    }
    let list: Vec<&str> = failed.iter().map(|(name, _)| *name).collect();
    match lang {
        Lang::Fr => {
            let verb = if failed.len() == 1 { "n'a" } else { "n'ont" };
            format!(
                " {} sur {n} {verb} pas répondu : {}.",
                failed.len(),
                list.join(", ")
            )
        }
        Lang::En => format!(
            " {} of {n} didn't answer: {}.",
            failed.len(),
            list.join(", ")
        ),
    }
}

/// "Rien n'a répondu dans le Salon : Baie, Fenêtre." when every device failed; `place` starts
/// with a space or is empty.
fn none_answered(failed: &[(&str, &str)], place: &str, lang: Lang) -> String {
    let list: Vec<&str> = failed.iter().map(|(name, _)| *name).collect();
    match lang {
        Lang::Fr => format!("Rien n'a répondu{place} : {}.", list.join(", ")),
        Lang::En => format!("Nothing answered{place}: {}.", list.join(", ")),
    }
}

/// "« Baie » n'a pas répondu dans le Salon : refusé."; `place` starts with a space or is empty.
fn one_failed(name: &str, why: &str, place: &str, lang: Lang) -> String {
    match lang {
        Lang::Fr => format!("« {name} » n'a pas répondu{place} : {why}."),
        Lang::En => format!("\"{name}\" didn't answer{place}: {why}."),
    }
}

fn lights(
    area: &str,
    on: bool,
    colour: Option<&str>,
    names: &[String],
    results: &[ActionResult],
    lang: Lang,
) -> String {
    if lang == Lang::En {
        return lights_en(area, on, colour, names, results);
    }
    let n = names.len();
    let (verb_m, verb_f) = match (on, colour) {
        (true, Some(c)) => (format!("allumé en {c}"), format!("allumée en {c}")),
        (true, None) => ("allumé".into(), "allumée".into()),
        (false, _) => ("éteint".into(), "éteinte".into()),
    };
    let (verb_m, verb_f) = (verb_m.as_str(), verb_f.as_str());
    let failed = failures(names, results, lang);
    let skipped = results
        .iter()
        .filter(|r| **r == ActionResult::Skipped)
        .count();
    let count = if n == 1 {
        "1 lumière".to_string()
    } else {
        format!("{n} lumières")
    };
    if n == 1 {
        let name = &names[0];
        let label = device_label("Lumière", feminine_light, name);
        return match failed.first() {
            Some((_, why)) => one_failed(name, why, &format!(" {}", dans(area)), lang),
            None if skipped == 1 => format!("{label} déjà {verb_f} {}.", dans(area)),
            None => format!("{label} {verb_f} {}.", dans(area)),
        };
    }
    if !failed.is_empty() && failed.len() == n {
        return none_answered(&failed, &format!(" {}", dans(area)), lang);
    }
    let verb = agree(area, verb_m, verb_f);
    let mut text = if skipped == n {
        format!("{} déjà {verb} ({count}).", room(area))
    } else {
        format!("{} {verb} ({count}).", room(area))
    };
    text.push_str(&some_failed(&failed, n, lang));
    text
}

/// [`lights`] in English: "Desk lamp on in the Office.", "Living room lights on (3 lights)."
fn lights_en(
    area: &str,
    on: bool,
    colour: Option<&str>,
    names: &[String],
    results: &[ActionResult],
) -> String {
    let lang = Lang::En;
    let n = names.len();
    let verb = match (on, colour) {
        (true, Some(c)) => format!("on in {c}"),
        (true, None) => "on".into(),
        (false, _) => "off".into(),
    };
    let failed = failures(names, results, lang);
    let skipped = results
        .iter()
        .filter(|r| **r == ActionResult::Skipped)
        .count();
    let place = format!(" {}", in_the(area));
    if n == 1 {
        let name = &names[0];
        let label = device_label_en("Light", &[Class::Light, Class::LightNamed], name);
        return match failed.first() {
            Some((_, why)) => one_failed(name, why, &place, lang),
            None if skipped == 1 => format!("{label} already {verb}{place}."),
            None => format!("{label} {verb}{place}."),
        };
    }
    if !failed.is_empty() && failed.len() == n {
        return none_answered(&failed, &place, lang);
    }
    let already = if skipped == n { "already " } else { "" };
    let mut text = format!(
        "{} lights {already}{verb} ({n} lights).",
        room_in(area, lang)
    );
    text.push_str(&some_failed(&failed, n, lang));
    text
}

/// Shutters of a room (`area`) or of the whole house (`None`), shaped like the lights' reply.
fn covers(
    area: Option<&str>,
    open: bool,
    position: Option<u8>,
    names: &[String],
    unable: &[String],
    results: &[ActionResult],
    lang: Lang,
) -> String {
    let en = lang == Lang::En;
    let n = names.len();
    let (one, many) = match (open, position, en) {
        (true, Some(p), false) => (format!("ouvert à {p} %"), format!("ouverts à {p} %")),
        (true, None, false) => ("ouvert".to_string(), "ouverts".to_string()),
        (false, _, false) => ("fermé".to_string(), "fermés".to_string()),
        (true, Some(p), true) => (format!("open at {p} %"), format!("open at {p} %")),
        (true, None, true) => ("open".to_string(), "open".to_string()),
        (false, _, true) => ("closed".to_string(), "closed".to_string()),
    };
    // " dans le Salon" after a room, nothing for the whole house.
    let place = area
        .map(|a| format!(" {}", self::place(a, lang)))
        .unwrap_or_default();
    let failed = failures(names, results, lang);
    let skipped = results
        .iter()
        .filter(|r| **r == ActionResult::Skipped)
        .count();
    let mut text = if n == 1 {
        let name = &names[0];
        let (label, already) = if en {
            (device_label_en("Shutter", &[Class::Cover], name), "already")
        } else {
            (
                device_label("Volet", |w| class(w) == Some(Class::Cover), name),
                "déjà",
            )
        };
        match failed.first() {
            Some((_, why)) => one_failed(name, why, &place, lang),
            None if skipped == 1 => format!("{label} {already} {one}{place}."),
            None => format!("{label} {one}{place}."),
        }
    } else if !failed.is_empty() && failed.len() == n {
        none_answered(&failed, &place, lang)
    } else {
        let mut text = match (area, en) {
            (Some(a), false) => {
                let already = if skipped == n { "déjà " } else { "" };
                format!("{a} : volets {already}{many} ({n} volets).")
            }
            (None, false) => {
                let already = if skipped == n { "déjà " } else { "" };
                format!("Tous les volets {already}{many} ({n} volets).")
            }
            (Some(a), true) => {
                let already = if skipped == n { "already " } else { "" };
                format!(
                    "{}: shutters {already}{many} ({n} shutters).",
                    room_in(a, lang)
                )
            }
            (None, true) => {
                let already = if skipped == n { "already " } else { "" };
                format!("All shutters {already}{many} ({n} shutters).")
            }
        };
        text.push_str(&some_failed(&failed, n, lang));
        text
    };
    if !unable.is_empty() {
        text.push(' ');
        text.push_str(&cannot_in(unable, open, position, lang));
    }
    text
}

/// TVs or plugs of a room (`area`) or of the whole house (`None`), shaped like the shutters'
/// reply. Both French words are feminine.
fn switches(
    what: SwitchType,
    area: Option<&str>,
    on: bool,
    names: &[String],
    results: &[ActionResult],
    lang: Lang,
) -> String {
    let en = lang == Lang::En;
    let n = names.len();
    let (kind, kind_class, plural) = match (what, en) {
        (SwitchType::Tv, false) => ("Télé", Class::Tv, "télés"),
        (_, false) => ("Prise", Class::Plug, "prises"),
        (SwitchType::Tv, true) => ("TV", Class::Tv, "TVs"),
        (_, true) => ("Plug", Class::Plug, "plugs"),
    };
    let (one, many) = match (on, en) {
        (true, false) => ("allumée", "allumées"),
        (false, false) => ("éteinte", "éteintes"),
        (true, true) => ("on", "on"),
        (false, true) => ("off", "off"),
    };
    let place = area
        .map(|a| format!(" {}", self::place(a, lang)))
        .unwrap_or_default();
    let failed = failures(names, results, lang);
    let skipped = results
        .iter()
        .filter(|r| **r == ActionResult::Skipped)
        .count();
    if n == 1 {
        let name = &names[0];
        let (label, already) = if en {
            (device_label_en(kind, &[kind_class], name), "already")
        } else {
            (
                device_label(kind, |w| class(w) == Some(kind_class), name),
                "déjà",
            )
        };
        return match failed.first() {
            Some((_, why)) => one_failed(name, why, &place, lang),
            None if skipped == 1 => format!("{label} {already} {one}{place}."),
            None => format!("{label} {one}{place}."),
        };
    }
    if !failed.is_empty() && failed.len() == n {
        return none_answered(&failed, &place, lang);
    }
    let mut text = match (area, en) {
        (Some(a), false) => {
            let already = if skipped == n { "déjà " } else { "" };
            format!("{a} : {plural} {already}{many} ({n} {plural}).")
        }
        (None, false) => {
            let already = if skipped == n { "déjà " } else { "" };
            format!("Toutes les {plural} {already}{many} ({n} {plural}).")
        }
        (Some(a), true) => {
            let already = if skipped == n { "already " } else { "" };
            format!(
                "{}: {plural} {already}{many} ({n} {plural}).",
                room_in(a, lang)
            )
        }
        (None, true) => {
            let already = if skipped == n { "already " } else { "" };
            format!("All {plural} {already}{many} ({n} {plural}).")
        }
    };
    text.push_str(&some_failed(&failed, n, lang));
    text
}

/// The shutters that cannot do what was asked: « Volet ne sait pas se mettre à une position. »
pub fn cannot(names: &[String], open: bool, position: Option<u8>) -> String {
    cannot_in(names, open, position, Lang::Fr)
}

/// [`cannot`] in `lang`: "Volet can't be set to a position."
pub fn cannot_in(names: &[String], open: bool, position: Option<u8>, lang: Lang) -> String {
    if lang == Lang::En {
        let what = match (position, open) {
            (Some(_), _) => "be set to a position",
            (None, true) => "open",
            (None, false) => "close",
        };
        return format!("{} can't {what}.", names.join(", "));
    }
    let what = match (position, open) {
        (Some(_), _) => "se mettre à une position",
        (None, true) => "s'ouvrir",
        (None, false) => "se fermer",
    };
    let verb = if names.len() == 1 {
        "ne sait"
    } else {
        "ne savent"
    };
    format!("{} {verb} pas {what}.", names.join(", "))
}

fn setpoint(
    area: &str,
    from: Option<f64>,
    to: f64,
    results: &[ActionResult],
    lang: Lang,
) -> String {
    if lang == Lang::En {
        let shown = temp_in(to, lang);
        let at = in_the(area);
        return match results.first() {
            Some(ActionResult::Failed(why)) => {
                format!("The heating {at} wasn't set to {shown}: {why}.")
            }
            Some(ActionResult::Skipped) => format!("Heating {at} already at {shown}."),
            _ => match from {
                Some(from) if from != to => {
                    format!("Heating {at} set to {shown} (was {}).", temp_in(from, lang))
                }
                _ => format!("Heating {at} set to {shown}."),
            },
        };
    }
    match results.first() {
        Some(ActionResult::Failed(why)) => format!(
            "Le chauffage {} n'a pas été réglé à {} : {why}.",
            de(area),
            temp(to)
        ),
        Some(ActionResult::Skipped) => format!("{} déjà à {}.", room(area), temp(to)),
        _ => {
            let verb = agree(area, "réglé", "réglée");
            match from {
                Some(from) if from != to => {
                    format!(
                        "{} {verb} à {} (avant {}).",
                        room(area),
                        temp(to),
                        temp(from)
                    )
                }
                _ => format!("{} {verb} à {}.", room(area), temp(to)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;
    use crate::plan::Done;

    fn lights_done(on: bool, names: &[&str]) -> Done {
        Done::Lights {
            area: "salon".into(),
            on,
            names: names.iter().map(|n| n.to_string()).collect(),
            colour: None,
            unable: vec![],
        }
    }

    #[test]
    fn colour_reply_shapes() {
        use ActionResult::{Ok, Skipped};
        let coloured = |names: &[&str], unable: &[&str]| Done::Lights {
            area: "salon".into(),
            on: true,
            names: names.iter().map(|n| n.to_string()).collect(),
            colour: Some("rouge".into()),
            unable: unable.iter().map(|n| n.to_string()).collect(),
        };
        assert_eq!(
            said(&coloured(&["Led"], &[]), &[Ok]),
            "Led allumée en rouge dans le Salon."
        );
        assert_eq!(
            said(&coloured(&["Led"], &[]), &[Skipped]),
            "Led déjà allumée en rouge dans le Salon."
        );
        assert_eq!(
            said(
                &coloured(&["Led", "Lampe table"], &["Plafonnier"]),
                &[Ok, Ok]
            ),
            "Salon allumé en rouge (2 lumières). « Plafonnier » ne prend pas de couleur."
        );
        assert_eq!(
            said(&coloured(&["Led"], &["Plafonnier", "Lampe"]), &[Ok]),
            "Led allumée en rouge dans le Salon. Ne prennent pas de couleur : Plafonnier, Lampe."
        );
    }

    fn covers_done(area: Option<&str>, open: bool, position: Option<u8>, names: &[&str]) -> Done {
        Done::Covers {
            area: area.map(str::to_string),
            open,
            position,
            names: names.iter().map(|n| n.to_string()).collect(),
            unable: vec![],
        }
    }

    fn said(d: &Done, results: &[ActionResult]) -> String {
        let f = fixture::house();
        done(d, results, &f.house(), false)
    }

    #[test]
    fn covers_reply_shapes() {
        use ActionResult::{Ok, Skipped};
        let salon = Some("salon");
        assert_eq!(
            said(&covers_done(salon, false, None, &["Baie"]), &[Ok]),
            "Volet « Baie » fermé dans le Salon."
        );
        assert_eq!(
            said(&covers_done(salon, true, Some(30), &["Baie"]), &[Ok]),
            "Volet « Baie » ouvert à 30 % dans le Salon."
        );
        assert_eq!(
            said(&covers_done(salon, false, None, &["Baie"]), &[Skipped]),
            "Volet « Baie » déjà fermé dans le Salon."
        );
        assert_eq!(
            said(
                &covers_done(salon, false, None, &["Baie", "Fenêtre"]),
                &[Ok, Ok]
            ),
            "Salon : volets fermés (2 volets)."
        );
        assert_eq!(
            said(
                &covers_done(salon, true, Some(30), &["Baie", "Fenêtre"]),
                &[Ok, Ok]
            ),
            "Salon : volets ouverts à 30 % (2 volets)."
        );
        assert_eq!(
            said(
                &covers_done(None, false, None, &["Baie", "Fenêtre", "Volet"]),
                &[Ok, Ok, Ok]
            ),
            "Tous les volets fermés (3 volets)."
        );
        assert_eq!(
            said(
                &covers_done(None, true, None, &["Baie", "Fenêtre"]),
                &[Skipped, Skipped]
            ),
            "Tous les volets déjà ouverts (2 volets)."
        );
    }

    #[test]
    fn whole_house_with_one_offline_names_it() {
        use ActionResult::{Offline, Ok};
        assert_eq!(
            said(
                &covers_done(None, false, None, &["Baie", "Fenêtre", "Volet"]),
                &[Ok, Offline, Ok]
            ),
            "Tous les volets fermés (3 volets). 1 sur 3 n'a pas répondu : Fenêtre."
        );
        assert_eq!(
            said(
                &covers_done(Some("salon"), false, None, &["Baie", "Fenêtre"]),
                &[Offline, Offline]
            ),
            "Rien n'a répondu dans le Salon : Baie, Fenêtre."
        );
    }

    #[test]
    fn covers_reply_names_the_unable() {
        let mut d = covers_done(None, true, Some(30), &["Baie"]);
        if let Done::Covers { unable, .. } = &mut d {
            unable.push("Volet".into());
        }
        assert_eq!(
            said(&d, &[ActionResult::Ok]),
            "Volet « Baie » ouvert à 30 %. Volet ne sait pas se mettre à une position."
        );
    }

    fn switches_done(what: SwitchType, area: Option<&str>, on: bool, names: &[&str]) -> Done {
        Done::Switches {
            what,
            area: area.map(str::to_string),
            on,
            names: names.iter().map(|n| n.to_string()).collect(),
        }
    }

    #[test]
    fn switches_reply_shapes() {
        use ActionResult::{Failed, Offline, Ok, Skipped};
        use SwitchType::{Plug, Tv};
        assert_eq!(
            said(&switches_done(Tv, Some("salon"), true, &["Écran"]), &[Ok]),
            "Télé « Écran » allumée dans le Salon."
        );
        assert_eq!(
            said(
                &switches_done(Plug, Some("bureau"), false, &["Cafetière"]),
                &[Skipped]
            ),
            "Prise « Cafetière » déjà éteinte dans le Bureau."
        );
        assert_eq!(
            said(
                &switches_done(Plug, Some("bureau"), false, &["Cafetière", "Chargeur"]),
                &[Ok, Ok]
            ),
            "Bureau : prises éteintes (2 prises)."
        );
        assert_eq!(
            said(
                &switches_done(Plug, None, false, &["Cafetière", "Chargeur", "Lampe"]),
                &[Ok, Ok, Ok]
            ),
            "Toutes les prises éteintes (3 prises)."
        );
        assert_eq!(
            said(
                &switches_done(Tv, None, true, &["Écran", "Projecteur"]),
                &[Ok, Ok]
            ),
            "Toutes les télés allumées (2 télés)."
        );
        assert_eq!(
            said(
                &switches_done(Tv, None, true, &["Écran", "Projecteur"]),
                &[Skipped, Skipped]
            ),
            "Toutes les télés déjà allumées (2 télés)."
        );
        assert_eq!(
            said(
                &switches_done(Plug, None, false, &["Cafetière", "Chargeur", "Lampe"]),
                &[Ok, Offline, Ok]
            ),
            "Toutes les prises éteintes (3 prises). 1 sur 3 n'a pas répondu : Chargeur."
        );
        assert_eq!(
            said(
                &switches_done(Plug, Some("bureau"), false, &["Cafetière"]),
                &[Failed("refusé".into())]
            ),
            "« Cafetière » n'a pas répondu dans le Bureau : refusé."
        );
        assert_eq!(
            said(
                &switches_done(Plug, Some("bureau"), false, &["Cafetière", "Chargeur"]),
                &[Offline, Offline]
            ),
            "Rien n'a répondu dans le Bureau : Cafetière, Chargeur."
        );
    }

    #[test]
    fn a_name_that_is_only_the_kind_word_is_not_repeated() {
        use ActionResult::Ok;
        assert_eq!(
            said(
                &covers_done(Some("bureau"), true, Some(40), &["Volet"]),
                &[Ok]
            ),
            "Volet ouvert à 40 % dans le Bureau."
        );
        assert_eq!(
            said(&lights_done(true, &["Lampe"]), &[Ok]),
            "Lampe allumée dans le Salon."
        );
        assert_eq!(
            said(
                &switches_done(SwitchType::Plug, Some("bureau"), true, &["prise"]),
                &[Ok]
            ),
            "Prise allumée dans le Bureau."
        );
    }

    #[test]
    fn a_masculine_light_word_keeps_lumiere_for_the_agreement() {
        use ActionResult::Ok;
        assert_eq!(
            said(&lights_done(true, &["Plafonnier"]), &[Ok]),
            "Lumière « Plafonnier » allumée dans le Salon."
        );
        assert_eq!(
            said(&lights_done(false, &["Éclairage salon"]), &[Ok]),
            "Lumière « Éclairage salon » éteinte dans le Salon."
        );
    }

    #[test]
    fn a_name_starting_with_the_kind_word_drops_the_prefix() {
        use ActionResult::Ok;
        assert_eq!(
            said(
                &covers_done(Some("salon"), false, None, &["Volet tv"]),
                &[Ok]
            ),
            "« Volet tv » fermé dans le Salon."
        );
        assert_eq!(
            said(&lights_done(true, &["Lampe table"]), &[Ok]),
            "« Lampe table » allumée dans le Salon."
        );
        assert_eq!(
            said(
                &switches_done(SwitchType::Plug, Some("bureau"), true, &["Prise bureau"]),
                &[Ok]
            ),
            "« Prise bureau » allumée dans le Bureau."
        );
        assert_eq!(
            said(
                &switches_done(SwitchType::Tv, Some("salon"), true, &["TV salon"]),
                &[Ok]
            ),
            "« TV salon » allumée dans le Salon."
        );
    }

    #[test]
    fn any_other_name_keeps_the_kind_word() {
        use ActionResult::Ok;
        assert_eq!(
            said(&covers_done(Some("salon"), false, None, &["Baie"]), &[Ok]),
            "Volet « Baie » fermé dans le Salon."
        );
        assert_eq!(
            said(&lights_done(true, &["Éclat"]), &[Ok]),
            "Lumière « Éclat » allumée dans le Salon."
        );
    }

    #[test]
    fn temp_format() {
        assert_eq!(temp(18.2), "18,2 °C");
        assert_eq!(temp(20.0), "20 °C");
        assert_eq!(temp(-0.5), "-0,5 °C");
        assert_eq!(temp(18.25), "18,3 °C");
    }

    #[test]
    fn articles_follow_the_room_word() {
        assert_eq!(dans("Salon"), "dans le Salon");
        assert_eq!(dans("Chambre Léa"), "dans la Chambre Léa");
        assert_eq!(dans("Entrée"), "dans l'Entrée");
        assert_eq!(de("Salon"), "du Salon");
        assert_eq!(de("Chambre Tom"), "de la Chambre Tom");
        assert_eq!(le("Bureau"), "le Bureau");
    }

    #[test]
    fn a_room_named_after_its_person_is_their_bedroom() {
        assert_eq!(le("Léa"), "la chambre de Léa");
        assert_eq!(de("Léa"), "de la chambre de Léa");
        assert_eq!(dans("Léa"), "dans la chambre de Léa");
        assert_eq!(room("Léa"), "Chambre de Léa");
        assert_eq!(agree("Léa", "réglé", "réglée"), "réglée");
        // Room words, another kind of room and the unassigned area keep their own name.
        assert_eq!(room("Salon"), "Salon");
        assert_eq!(room("Chambre Léa"), "Chambre Léa");
        assert_eq!(le("Toilettes"), "Toilettes");
        assert!(!le("Unassigned").contains("chambre"));
    }

    #[test]
    fn replies_name_a_person_room_as_their_bedroom() {
        let f = fixture::person_rooms();
        let h = f.house();
        let d = Done::Setpoint {
            zone: "lea".into(),
            from: Some(17.5),
            to: 19.0,
        };
        assert_eq!(
            done(&d, &[ActionResult::Ok], &h, false),
            "Chambre de Léa réglée à 19 °C (avant 17,5 °C)."
        );
    }

    #[test]
    fn lights_reply() {
        let f = fixture::house();
        let h = f.house();
        let ok = [ActionResult::Ok, ActionResult::Ok, ActionResult::Ok];
        let d = lights_done(true, &["Plafonnier", "Lampe table", "Led"]);
        assert_eq!(done(&d, &ok, &h, false), "Salon allumé (3 lumières).");
        let failed = [
            ActionResult::Ok,
            ActionResult::Ok,
            ActionResult::Failed("no echo".into()),
        ];
        let r = done(&d, &failed, &h, false);
        assert!(r.contains("n'a pas répondu"), "{r}");
        assert!(r.contains("Led"), "{r}");
        assert!(done(&d, &ok, &h, true).starts_with("Simulation :"));
        let off = lights_done(false, &["Led"]);
        let r = done(&off, &[ActionResult::Ok], &h, false);
        assert!(
            r.contains("Led") && r.contains("Salon") && r.contains("éteinte"),
            "{r}"
        );
        let skipped = [
            ActionResult::Skipped,
            ActionResult::Skipped,
            ActionResult::Skipped,
        ];
        assert_eq!(
            done(&d, &skipped, &h, false),
            "Salon déjà allumé (3 lumières)."
        );
    }

    #[test]
    fn setpoint_reply() {
        let f = fixture::house();
        let h = f.house();
        let d = Done::Setpoint {
            zone: "chambre_lea".into(),
            from: Some(17.5),
            to: 19.0,
        };
        assert_eq!(
            done(&d, &[ActionResult::Ok], &h, false),
            "Chambre Léa réglée à 19 °C (avant 17,5 °C)."
        );
        let r = done(&d, &[ActionResult::Failed("refusé".into())], &h, false);
        assert!(
            r.contains("19 °C") && r.contains("refusé") && r.contains("Chambre Léa"),
            "{r}"
        );
    }

    fn media_done(what: crate::intent::MediaIntent) -> Done {
        Done::Media {
            player: "Musique".into(),
            what,
        }
    }

    fn echo(state: &str, title: Option<&str>, artist: Option<&str>) -> ActionResult {
        ActionResult::Echo(nidoo_core::model::MediaPlayerState {
            state: Some(state.into()),
            title: title.map(Into::into),
            artist: artist.map(Into::into),
            ..Default::default()
        })
    }

    #[test]
    fn media_replies() {
        use crate::intent::MediaIntent as M;
        let lucky = echo("playing", Some("Get Lucky"), Some("Daft Punk"));
        let cases: Vec<(M, ActionResult, &str)> = vec![
            (
                M::Pause,
                echo("paused", Some("Get Lucky"), None),
                "Musique en pause.",
            ),
            (M::Pause, ActionResult::Ok, "Musique en pause."),
            (
                M::Pause,
                ActionResult::Skipped,
                "La musique est déjà en pause.",
            ),
            (
                M::Play { resume: true },
                lucky.clone(),
                "Je reprends Get Lucky.",
            ),
            (
                M::Play { resume: true },
                ActionResult::Ok,
                "Je reprends la musique.",
            ),
            (
                M::Play { resume: true },
                ActionResult::Skipped,
                "La musique joue déjà.",
            ),
            (
                M::Play { resume: false },
                lucky.clone(),
                "Je joue Get Lucky.",
            ),
            (
                M::Play { resume: false },
                ActionResult::Ok,
                "Je lance la musique.",
            ),
            (
                M::Play { resume: false },
                echo("playing", None, None),
                "Je lance la musique.",
            ),
            (
                M::PlayMedia {
                    query: "daft punk".into(),
                },
                lucky.clone(),
                "Je joue Get Lucky.",
            ),
            (
                M::PlayMedia {
                    query: "daft punk".into(),
                },
                ActionResult::Ok,
                "Je cherche « daft punk ».",
            ),
            (M::Next, lucky.clone(), "Get Lucky, Daft Punk."),
            (M::Next, echo("playing", Some("Intro"), None), "Intro."),
            (M::Next, ActionResult::Ok, "Chanson suivante."),
            (M::Previous, ActionResult::Ok, "Chanson précédente."),
            (M::Pause, ActionResult::Offline, "Le lecteur ne répond pas."),
            (
                M::Next,
                ActionResult::Failed("pas de réponse de l'appareil".into()),
                "Le lecteur n'a pas répondu.",
            ),
            (
                M::Next,
                ActionResult::Failed("hors ligne".into()),
                "Le lecteur n'a pas répondu : hors ligne.",
            ),
        ];
        for (what, result, want) in cases {
            assert_eq!(said(&media_done(what.clone()), &[result]), want, "{what:?}");
        }
    }

    #[test]
    fn volume_and_mute_replies() {
        let volume = Done::Volume {
            area: "salon".into(),
            level: Some(60),
        };
        assert_eq!(
            said(&volume, &[ActionResult::Ok]),
            "Volume à 60 % dans le Salon."
        );
        assert_eq!(
            said(&volume, &[ActionResult::Offline]),
            "L'enceinte ne répond pas dans le Salon."
        );
        let mute = |on| Done::Mute {
            area: "salon".into(),
            on,
        };
        assert_eq!(
            said(&mute(true), &[ActionResult::Ok]),
            "Son coupé dans le Salon."
        );
        assert_eq!(
            said(&mute(false), &[ActionResult::Ok]),
            "Son remis dans le Salon."
        );
        assert_eq!(
            said(&mute(true), &[ActionResult::Skipped]),
            "Son déjà coupé dans le Salon."
        );
    }

    fn said_en(d: &Done, results: &[ActionResult]) -> String {
        let f = fixture::house();
        done(d, results, &f.house_en(), false)
    }

    #[test]
    fn english_numbers_take_a_point() {
        assert_eq!(number_in(18.25, Lang::En), "18.3");
        assert_eq!(number_in(20.0, Lang::En), "20");
        assert_eq!(temp_in(19.5, Lang::En), "19.5 °C");
        assert_eq!(temp_in(-0.5, Lang::En), "-0.5 °C");
        assert_eq!(temp_in(19.5, Lang::Fr), "19,5 °C");
        assert_eq!(quoted("Desk", Lang::En), "\"Desk\"");
        assert_eq!(quoted("Baie", Lang::Fr), "« Baie »");
    }

    #[test]
    fn english_room_phrases() {
        assert_eq!(the("Kitchen"), "the Kitchen");
        assert_eq!(in_the("Kitchen"), "in the Kitchen");
        assert_eq!(the_start("Kitchen"), "The Kitchen");
        assert_eq!(the("Tom"), "Tom's room");
        assert_eq!(in_the("Tom"), "in Tom's room");
        assert_eq!(the("The Den"), "The Den");
        assert_eq!(the("Tom's Den"), "Tom's Den");
        assert_eq!(the("living room"), "the living room");
        assert_eq!(the("TV room"), "the TV room");
        assert_eq!(room_in("Tom", Lang::En), "Tom's room");
        assert_eq!(room_in("Tom's Den", Lang::En), "Tom's Den");
        assert_eq!(room_in("Living room", Lang::En), "Living room");
        assert_eq!(room_in("Léa", Lang::Fr), "Chambre de Léa");
        assert_eq!(place("Salon", Lang::Fr), "dans le Salon");
        assert_eq!(place("Salon", Lang::En), "in the Salon");
    }

    #[test]
    fn english_cannot() {
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            cannot_in(&names(&["Volet"]), true, Some(30), Lang::En),
            "Volet can't be set to a position."
        );
        assert_eq!(
            cannot_in(&names(&["Bay", "Door"]), false, None, Lang::En),
            "Bay, Door can't close."
        );
        assert_eq!(
            cannot(&names(&["Volet"]), true, None),
            cannot_in(&names(&["Volet"]), true, None, Lang::Fr)
        );
    }

    #[test]
    fn english_lights_replies() {
        use ActionResult::{Failed, Offline, Ok, Skipped};
        let many = lights_done(true, &["Plafonnier", "Lampe table", "Led"]);
        assert_eq!(said_en(&many, &[Ok, Ok, Ok]), "Salon lights on (3 lights).");
        assert_eq!(
            said_en(&many, &[Skipped, Skipped, Skipped]),
            "Salon lights already on (3 lights)."
        );
        assert_eq!(
            said_en(&many, &[Ok, Offline, Ok]),
            "Salon lights on (3 lights). 1 of 3 didn't answer: Lampe table."
        );
        assert_eq!(
            said_en(&many, &[Offline, Offline, Offline]),
            "Nothing answered in the Salon: Plafonnier, Lampe table, Led."
        );
        assert_eq!(
            said_en(&lights_done(false, &["Desk lamp"]), &[Skipped]),
            "Desk lamp already off in the Salon."
        );
        assert_eq!(
            said_en(&lights_done(false, &["Glow"]), &[Ok]),
            "Light \"Glow\" off in the Salon."
        );
        assert_eq!(
            said_en(&lights_done(true, &["Led"]), &[Failed("refused".into())]),
            "\"Led\" didn't answer in the Salon: refused."
        );
        assert_eq!(
            said_en(&lights_done(true, &["Led"]), &[Offline]),
            "\"Led\" didn't answer in the Salon: offline."
        );
    }

    #[test]
    fn english_colour_replies() {
        let coloured = |names: &[&str], unable: &[&str]| Done::Lights {
            area: "salon".into(),
            on: true,
            names: names.iter().map(|n| n.to_string()).collect(),
            colour: Some("red".into()),
            unable: unable.iter().map(|n| n.to_string()).collect(),
        };
        assert_eq!(
            said_en(&coloured(&["Led"], &[]), &[ActionResult::Ok]),
            "Led on in red in the Salon."
        );
        assert_eq!(
            said_en(
                &coloured(&["Led", "Lampe table"], &["Plafonnier"]),
                &[ActionResult::Ok, ActionResult::Ok]
            ),
            "Salon lights on in red (2 lights). \"Plafonnier\" can't take a colour."
        );
        assert_eq!(
            said_en(
                &coloured(&["Led"], &["Plafonnier", "Lamp"]),
                &[ActionResult::Ok]
            ),
            "Led on in red in the Salon. Can't take a colour: Plafonnier, Lamp."
        );
    }

    #[test]
    fn english_setpoint_replies() {
        let f = fixture::house();
        let h = f.house_en();
        let d = |from| Done::Setpoint {
            zone: "chambre_lea".into(),
            from,
            to: 19.5,
        };
        assert_eq!(
            done(&d(Some(17.0)), &[ActionResult::Ok], &h, false),
            "Heating in the Chambre Léa set to 19.5 °C (was 17 °C)."
        );
        assert_eq!(
            done(&d(None), &[ActionResult::Ok], &h, false),
            "Heating in the Chambre Léa set to 19.5 °C."
        );
        assert_eq!(
            done(&d(Some(19.5)), &[ActionResult::Skipped], &h, false),
            "Heating in the Chambre Léa already at 19.5 °C."
        );
        assert_eq!(
            done(
                &d(Some(17.0)),
                &[ActionResult::Failed("refused".into())],
                &h,
                false
            ),
            "The heating in the Chambre Léa wasn't set to 19.5 °C: refused."
        );
        let f = fixture::person_rooms();
        let h = f.house_en();
        let d = Done::Setpoint {
            zone: "tom".into(),
            from: None,
            to: 20.0,
        };
        assert_eq!(
            done(&d, &[ActionResult::Ok], &h, false),
            "Heating in Tom's room set to 20 °C."
        );
    }

    #[test]
    fn english_covers_and_switches_replies() {
        use ActionResult::{Offline, Ok, Skipped};
        let salon = Some("salon");
        assert_eq!(
            said_en(&covers_done(salon, false, None, &["Bay"]), &[Ok]),
            "Shutter \"Bay\" closed in the Salon."
        );
        assert_eq!(
            said_en(
                &covers_done(salon, true, Some(30), &["Bay shutter"]),
                &[Skipped]
            ),
            "Bay shutter already open at 30 % in the Salon."
        );
        assert_eq!(
            said_en(
                &covers_done(salon, false, None, &["Bay", "Window"]),
                &[Ok, Ok]
            ),
            "Salon: shutters closed (2 shutters)."
        );
        assert_eq!(
            said_en(
                &covers_done(None, false, None, &["Bay", "Window", "Volet"]),
                &[Ok, Offline, Ok]
            ),
            "All shutters closed (3 shutters). 1 of 3 didn't answer: Window."
        );
        let mut d = covers_done(None, true, Some(30), &["Bay"]);
        if let Done::Covers { unable, .. } = &mut d {
            unable.push("Volet".into());
        }
        assert_eq!(
            said_en(&d, &[Ok]),
            "Shutter \"Bay\" open at 30 %. Volet can't be set to a position."
        );
        use SwitchType::{Plug, Tv};
        assert_eq!(
            said_en(&switches_done(Tv, salon, true, &["Screen"]), &[Ok]),
            "TV \"Screen\" on in the Salon."
        );
        assert_eq!(
            said_en(
                &switches_done(Plug, Some("bureau"), false, &["Coffee plug"]),
                &[Skipped]
            ),
            "Coffee plug already off in the Bureau."
        );
        assert_eq!(
            said_en(
                &switches_done(Plug, Some("bureau"), false, &["Coffee", "Charger"]),
                &[Ok, Ok]
            ),
            "Bureau: plugs off (2 plugs)."
        );
        assert_eq!(
            said_en(
                &switches_done(Tv, None, true, &["Screen", "Projector"]),
                &[Skipped, Skipped]
            ),
            "All TVs already on (2 TVs)."
        );
    }

    #[test]
    fn english_media_and_speakers_replies() {
        use crate::intent::MediaIntent as M;
        let lucky = echo("playing", Some("Get Lucky"), Some("Daft Punk"));
        let cases: Vec<(M, ActionResult, &str)> = vec![
            (M::Pause, ActionResult::Ok, "Music paused."),
            (
                M::Pause,
                ActionResult::Skipped,
                "The music is already paused.",
            ),
            (
                M::Play { resume: true },
                lucky.clone(),
                "Resuming Get Lucky.",
            ),
            (
                M::Play { resume: false },
                ActionResult::Ok,
                "Starting the music.",
            ),
            (
                M::Play { resume: false },
                lucky.clone(),
                "Playing Get Lucky.",
            ),
            (
                M::PlayMedia {
                    query: "daft punk".into(),
                },
                ActionResult::Ok,
                "Looking for \"daft punk\".",
            ),
            (M::Next, lucky.clone(), "Get Lucky, Daft Punk."),
            (M::Previous, ActionResult::Ok, "Previous song."),
            (
                M::Pause,
                ActionResult::Offline,
                "The player isn't responding.",
            ),
            (
                M::Next,
                ActionResult::Failed(NO_ANSWER_EN.into()),
                "The player didn't answer.",
            ),
            (
                M::Next,
                ActionResult::Failed("offline".into()),
                "The player didn't answer: offline.",
            ),
        ];
        for (what, result, want) in cases {
            assert_eq!(
                said_en(&media_done(what.clone()), &[result]),
                want,
                "{what:?}"
            );
        }
        let volume = Done::Volume {
            area: "bureau".into(),
            level: Some(30),
        };
        assert_eq!(
            said_en(&volume, &[ActionResult::Ok]),
            "Volume at 30 % in the Bureau."
        );
        assert_eq!(
            said_en(&volume, &[ActionResult::Offline]),
            "The speaker isn't responding in the Bureau."
        );
        assert_eq!(
            said_en(&volume, &[ActionResult::Failed("refused".into())]),
            "The speaker didn't answer in the Bureau: refused."
        );
        let mute = |on| Done::Mute {
            area: "salon".into(),
            on,
        };
        assert_eq!(
            said_en(&mute(true), &[ActionResult::Ok]),
            "Sound muted in the Salon."
        );
        assert_eq!(
            said_en(&mute(false), &[ActionResult::Skipped]),
            "Sound already on in the Salon."
        );
    }

    #[test]
    fn english_simulation_prefix() {
        let f = fixture::house();
        let d = lights_done(true, &["Led"]);
        assert_eq!(
            done(&d, &[ActionResult::Ok], &f.house_en(), true),
            "Simulation: Led on in the Salon."
        );
        assert!(done(&d, &[ActionResult::Ok], &f.house(), true).starts_with("Simulation : "));
    }
}
