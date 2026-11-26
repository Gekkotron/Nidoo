//! The replies, written from templates, in French or in English (the house's language). Every
//! reply names the room, the action and the value.

use nidoo_api::search::fold;

use nidoo_core::flow::Lang;
use nidoo_core::model::{MediaPlayerState, SwitchType};

use crate::house::House;
use crate::intent::{MediaIntent, VacuumIntent};
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
    Echo(Box<MediaPlayerState>),
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
            brightness,
            unable,
        } => {
            let mut text = lights(
                house.area_name(area),
                *on,
                (colour.as_deref(), *brightness),
                names,
                results,
                lang,
            );
            let lack = lack(colour.is_some(), brightness.is_some(), lang);
            match (unable.as_slice(), en) {
                ([], _) => {}
                ([one], false) => text.push_str(&format!(" « {one} » {}.", lack.one)),
                ([one], true) => text.push_str(&format!(" \"{one}\" {}.", lack.one)),
                (many, false) => text.push_str(&format!(" {} : {}.", lack.many, many.join(", "))),
                (many, true) => text.push_str(&format!(" {}: {}.", lack.many, many.join(", "))),
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
        Done::Vacuum { action, room, .. } => {
            vacuum(*action, room.as_deref(), results.first(), lang)
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

/// The vacuum's reply; `room` is the room cleaned, `None` for a full clean.
fn vacuum(
    action: VacuumIntent,
    room: Option<&str>,
    result: Option<&ActionResult>,
    lang: Lang,
) -> String {
    let en = lang == Lang::En;
    match result {
        Some(ActionResult::Skipped) => {
            return match (action, en) {
                (VacuumIntent::Clean, false) => "L'aspirateur est déjà en marche.",
                (VacuumIntent::Clean, true) => "The vacuum is already running.",
                (VacuumIntent::Stop, false) => "L'aspirateur est déjà arrêté.",
                (VacuumIntent::Stop, true) => "The vacuum is already stopped.",
                (VacuumIntent::Pause, false) => "L'aspirateur est déjà en pause.",
                (VacuumIntent::Pause, true) => "The vacuum is already paused.",
                (VacuumIntent::Dock, false) => "L'aspirateur est déjà à sa base.",
                (VacuumIntent::Dock, true) => "The vacuum is already at its dock.",
            }
            .into();
        }
        Some(ActionResult::Offline) => {
            return if en {
                "The vacuum isn't responding.".into()
            } else {
                "L'aspirateur ne répond pas.".into()
            };
        }
        Some(ActionResult::Failed(why)) if no_answer(why) => {
            return if en {
                "The vacuum didn't answer.".into()
            } else {
                "L'aspirateur n'a pas répondu.".into()
            };
        }
        Some(ActionResult::Failed(why)) => {
            return if en {
                format!("The vacuum didn't answer: {why}.")
            } else {
                format!("L'aspirateur n'a pas répondu : {why}.")
            };
        }
        Some(ActionResult::Ok | ActionResult::Echo(_)) | None => {}
    }
    match (action, room, en) {
        (VacuumIntent::Clean, Some(room), false) => format!("L'aspirateur nettoie {}.", le(room)),
        (VacuumIntent::Clean, Some(room), true) => {
            format!("The vacuum is cleaning {}.", the(room))
        }
        (VacuumIntent::Clean, None, false) => "L'aspirateur est lancé.".into(),
        (VacuumIntent::Clean, None, true) => "The vacuum has started.".into(),
        (VacuumIntent::Stop, _, false) => "L'aspirateur est arrêté.".into(),
        (VacuumIntent::Stop, _, true) => "The vacuum has stopped.".into(),
        (VacuumIntent::Pause, _, false) => "L'aspirateur est en pause.".into(),
        (VacuumIntent::Pause, _, true) => "The vacuum is paused.".into(),
        (VacuumIntent::Dock, _, false) => "L'aspirateur rentre à sa base.".into(),
        (VacuumIntent::Dock, _, true) => "The vacuum is going back to its dock.".into(),
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
    (colour, brightness): (Option<&str>, Option<u8>),
    names: &[String],
    results: &[ActionResult],
    lang: Lang,
) -> String {
    if lang == Lang::En {
        return lights_en(area, on, (colour, brightness), names, results);
    }
    let n = names.len();
    let how = format!(
        "{}{}",
        colour.map(|c| format!(" en {c}")).unwrap_or_default(),
        brightness.map(|b| format!(" à {b} %")).unwrap_or_default()
    );
    let (verb_m, verb_f) = if on {
        (format!("allumé{how}"), format!("allumée{how}"))
    } else {
        ("éteint".into(), "éteinte".into())
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

/// How lights that lack what was asked are told: after one name (`one`), after "Aucune lumière"
/// / "No light" (`none`), and at the head of a list of names (`many`).
pub struct Lack {
    pub one: &'static str,
    pub none: &'static str,
    pub many: &'static str,
}

/// What lights lack when a colour, a brightness or both were asked.
pub fn lack(colour: bool, brightness: bool, lang: Lang) -> Lack {
    let (one, none, many) = match (lang, colour, brightness) {
        (Lang::Fr, true, false) => (
            "ne prend pas de couleur",
            "ne prend de couleur",
            "Ne prennent pas de couleur",
        ),
        (Lang::Fr, false, true) => (
            "n'a pas de luminosité réglable",
            "n'a de luminosité réglable",
            "N'ont pas de luminosité réglable",
        ),
        (Lang::Fr, ..) => (
            "ne prend pas ce réglage",
            "ne prend ce réglage",
            "Ne prennent pas ce réglage",
        ),
        (Lang::En, true, false) => (
            "can't take a colour",
            "takes a colour",
            "Can't take a colour",
        ),
        (Lang::En, false, true) => ("can't be dimmed", "can be dimmed", "Can't be dimmed"),
        (Lang::En, ..) => (
            "can't take that setting",
            "takes that setting",
            "Can't take that setting",
        ),
    };
    Lack { one, none, many }
}

/// [`lights`] in English: "Desk lamp on in the Office.", "Living room lights on (3 lights)."
fn lights_en(
    area: &str,
    on: bool,
    (colour, brightness): (Option<&str>, Option<u8>),
    names: &[String],
    results: &[ActionResult],
) -> String {
    let lang = Lang::En;
    let n = names.len();
    let verb = match (on, colour, brightness) {
        (true, Some(c), None) => format!("on in {c}"),
        (true, Some(c), Some(b)) => format!("on in {c} at {b} %"),
        (true, None, Some(b)) => format!("on at {b} %"),
        (true, None, None) => "on".into(),
        (false, ..) => "off".into(),
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
mod tests;
