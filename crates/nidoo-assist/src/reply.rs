//! The French replies, written from templates. Every reply names the room, the action and
//! the value.

use nidoo_api::search::fold;

use nidoo_core::model::{MediaPlayerState, SwitchType};

use crate::house::House;
use crate::intent::MediaIntent;
use crate::lexicon::{Class, class};
use crate::plan::Done;
use crate::room::name_words;

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

/// A number with a decimal comma, one decimal at most, no ",0": `18,2`, `20`.
pub fn number(x: f64) -> String {
    let r = (x * 10.0).round() / 10.0;
    if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        format!("{r:.1}").replace('.', ",")
    }
}

/// A temperature: `18,2 °C`, `20 °C`.
pub fn temp(c: f64) -> String {
    format!("{} °C", number(c))
}

#[derive(Clone, Copy, PartialEq)]
enum Gender {
    Masculine,
    Feminine,
    /// Starts with a vowel: `l'`.
    Elided,
    /// Not a common room word (a person's name): no article.
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
    } else {
        Gender::Bare
    }
}

/// `le Salon`, `la Chambre Léa`, `l'Entrée`, `Léa`.
pub fn le(name: &str) -> String {
    match gender(name) {
        Gender::Masculine => format!("le {name}"),
        Gender::Feminine => format!("la {name}"),
        Gender::Elided => format!("l'{name}"),
        Gender::Bare => name.to_string(),
    }
}

/// `dans le Salon`, `dans la Chambre Léa`.
pub fn dans(name: &str) -> String {
    format!("dans {}", le(name))
}

/// `du Salon`, `de la Chambre Léa`, `de l'Entrée`, `de Léa`.
pub fn de(name: &str) -> String {
    match gender(name) {
        Gender::Masculine => format!("du {name}"),
        Gender::Feminine => format!("de la {name}"),
        Gender::Elided => format!("de l'{name}"),
        Gender::Bare => format!("de {name}"),
    }
}

/// The masculine or feminine form, after the room's gender.
pub fn agree<'a>(name: &str, masculine: &'a str, feminine: &'a str) -> &'a str {
    if gender(name) == Gender::Feminine {
        feminine
    } else {
        masculine
    }
}

/// The reply after acting: what was done, what was already so, what did not answer.
pub fn done(done: &Done, results: &[ActionResult], house: &House, simulate: bool) -> String {
    let text = match done {
        Done::Lights { area, on, names } => lights(house.area_name(area), *on, names, results),
        Done::Setpoint { zone, from, to } => setpoint(house.area_name(zone), *from, *to, results),
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
        ),
        Done::Media { what, .. } => media(what, results.first()),
        Done::Volume { area, level } => speakers(
            house.area_name(area),
            &level.map_or_else(|| "Volume changé".into(), |l| format!("Volume à {l} %")),
            "Volume déjà là",
            results,
        ),
        Done::Mute { area, on } => {
            let (done, already) = if *on {
                ("Son coupé", "Son déjà coupé")
            } else {
                ("Son remis", "Son déjà remis")
            };
            speakers(house.area_name(area), done, already, results)
        }
    };
    if simulate {
        format!("Simulation : {text}")
    } else {
        text
    }
}

/// A media player's reply, from its echo when there is one.
fn media(what: &MediaIntent, result: Option<&ActionResult>) -> String {
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
        Some(ActionResult::Failed(why)) if why == NO_ANSWER => "Le lecteur n'a pas répondu.".into(),
        Some(ActionResult::Failed(why)) => format!("Le lecteur n'a pas répondu : {why}."),
        Some(ActionResult::Ok) | None => plain(),
    }
}

/// A room's speakers: `done` when one of them did it, `already` when all were already there,
/// else what went wrong.
fn speakers(area: &str, done: &str, already: &str, results: &[ActionResult]) -> String {
    let place = dans(area);
    let ok = |r: &ActionResult| matches!(r, ActionResult::Ok | ActionResult::Echo(_));
    if results.iter().any(ok) {
        return format!("{done} {place}.");
    }
    if !results.is_empty() && results.iter().all(|r| *r == ActionResult::Skipped) {
        return format!("{already} {place}.");
    }
    match results.iter().find_map(|r| match r {
        ActionResult::Failed(why) => Some(why.as_str()),
        _ => None,
    }) {
        Some(why) => format!("L'enceinte n'a pas répondu {place} : {why}."),
        None => format!("L'enceinte ne répond pas {place}."),
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
            let mut chars = name.trim().chars();
            return match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            };
        }
        return format!("« {name} »");
    }
    format!("{kind} « {name} »")
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

fn lights(area: &str, on: bool, names: &[String], results: &[ActionResult]) -> String {
    let n = names.len();
    let (verb_m, verb_f) = if on {
        ("allumé", "allumée")
    } else {
        ("éteint", "éteinte")
    };
    let failed: Vec<(&str, &str)> = names
        .iter()
        .zip(results)
        .filter_map(|(name, r)| match r {
            ActionResult::Failed(why) => Some((name.as_str(), why.as_str())),
            ActionResult::Offline => Some((name.as_str(), "hors ligne")),
            _ => None,
        })
        .collect();
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
            Some((_, why)) => format!("« {name} » n'a pas répondu {} : {why}.", dans(area)),
            None if skipped == 1 => format!("{label} déjà {verb_f} {}.", dans(area)),
            None => format!("{label} {verb_f} {}.", dans(area)),
        };
    }
    if !failed.is_empty() && failed.len() == n {
        let list: Vec<&str> = failed.iter().map(|(name, _)| *name).collect();
        return format!("Rien n'a répondu {} : {}.", dans(area), list.join(", "));
    }
    let verb = agree(area, verb_m, verb_f);
    let mut text = if skipped == n {
        format!("{area} déjà {verb} ({count}).")
    } else {
        format!("{area} {verb} ({count}).")
    };
    if !failed.is_empty() {
        let list: Vec<&str> = failed.iter().map(|(name, _)| *name).collect();
        let verb = if failed.len() == 1 { "n'a" } else { "n'ont" };
        text.push_str(&format!(
            " {} sur {n} {verb} pas répondu : {}.",
            failed.len(),
            list.join(", ")
        ));
    }
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
) -> String {
    let n = names.len();
    let (one, many) = match (open, position) {
        (true, Some(p)) => (format!("ouvert à {p} %"), format!("ouverts à {p} %")),
        (true, None) => ("ouvert".to_string(), "ouverts".to_string()),
        (false, _) => ("fermé".to_string(), "fermés".to_string()),
    };
    // " dans le Salon" after a room, nothing for the whole house.
    let place = area.map(|a| format!(" {}", dans(a))).unwrap_or_default();
    let failed: Vec<(&str, &str)> = names
        .iter()
        .zip(results)
        .filter_map(|(name, r)| match r {
            ActionResult::Failed(why) => Some((name.as_str(), why.as_str())),
            ActionResult::Offline => Some((name.as_str(), "hors ligne")),
            _ => None,
        })
        .collect();
    let skipped = results
        .iter()
        .filter(|r| **r == ActionResult::Skipped)
        .count();
    let mut text = if n == 1 {
        let name = &names[0];
        let label = device_label("Volet", |w| class(w) == Some(Class::Cover), name);
        match failed.first() {
            Some((_, why)) => format!("« {name} » n'a pas répondu{place} : {why}."),
            None if skipped == 1 => format!("{label} déjà {one}{place}."),
            None => format!("{label} {one}{place}."),
        }
    } else if !failed.is_empty() && failed.len() == n {
        let list: Vec<&str> = failed.iter().map(|(name, _)| *name).collect();
        format!("Rien n'a répondu{place} : {}.", list.join(", "))
    } else {
        let already = if skipped == n { "déjà " } else { "" };
        let mut text = match area {
            Some(a) => format!("{a} : volets {already}{many} ({n} volets)."),
            None => format!("Tous les volets {already}{many} ({n} volets)."),
        };
        if !failed.is_empty() {
            let list: Vec<&str> = failed.iter().map(|(name, _)| *name).collect();
            let verb = if failed.len() == 1 { "n'a" } else { "n'ont" };
            text.push_str(&format!(
                " {} sur {n} {verb} pas répondu : {}.",
                failed.len(),
                list.join(", ")
            ));
        }
        text
    };
    if !unable.is_empty() {
        text.push(' ');
        text.push_str(&cannot(unable, open, position));
    }
    text
}

/// TVs or plugs of a room (`area`) or of the whole house (`None`), shaped like the shutters'
/// reply. Both words are feminine.
fn switches(
    what: SwitchType,
    area: Option<&str>,
    on: bool,
    names: &[String],
    results: &[ActionResult],
) -> String {
    let n = names.len();
    let (kind, kind_class, plural) = match what {
        SwitchType::Tv => ("Télé", Class::Tv, "télés"),
        _ => ("Prise", Class::Plug, "prises"),
    };
    let (one, many) = if on {
        ("allumée", "allumées")
    } else {
        ("éteinte", "éteintes")
    };
    let place = area.map(|a| format!(" {}", dans(a))).unwrap_or_default();
    let failed: Vec<(&str, &str)> = names
        .iter()
        .zip(results)
        .filter_map(|(name, r)| match r {
            ActionResult::Failed(why) => Some((name.as_str(), why.as_str())),
            ActionResult::Offline => Some((name.as_str(), "hors ligne")),
            _ => None,
        })
        .collect();
    let skipped = results
        .iter()
        .filter(|r| **r == ActionResult::Skipped)
        .count();
    if n == 1 {
        let name = &names[0];
        let label = device_label(kind, |w| class(w) == Some(kind_class), name);
        return match failed.first() {
            Some((_, why)) => format!("« {name} » n'a pas répondu{place} : {why}."),
            None if skipped == 1 => format!("{label} déjà {one}{place}."),
            None => format!("{label} {one}{place}."),
        };
    }
    if !failed.is_empty() && failed.len() == n {
        let list: Vec<&str> = failed.iter().map(|(name, _)| *name).collect();
        return format!("Rien n'a répondu{place} : {}.", list.join(", "));
    }
    let already = if skipped == n { "déjà " } else { "" };
    let mut text = match area {
        Some(a) => format!("{a} : {plural} {already}{many} ({n} {plural})."),
        None => format!("Toutes les {plural} {already}{many} ({n} {plural})."),
    };
    if !failed.is_empty() {
        let list: Vec<&str> = failed.iter().map(|(name, _)| *name).collect();
        let verb = if failed.len() == 1 { "n'a" } else { "n'ont" };
        text.push_str(&format!(
            " {} sur {n} {verb} pas répondu : {}.",
            failed.len(),
            list.join(", ")
        ));
    }
    text
}

/// The shutters that cannot do what was asked: « Volet ne sait pas se mettre à une position. »
pub fn cannot(names: &[String], open: bool, position: Option<u8>) -> String {
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

fn setpoint(area: &str, from: Option<f64>, to: f64, results: &[ActionResult]) -> String {
    match results.first() {
        Some(ActionResult::Failed(why)) => format!(
            "Le chauffage {} n'a pas été réglé à {} : {why}.",
            de(area),
            temp(to)
        ),
        Some(ActionResult::Skipped) => format!("{area} déjà à {}.", temp(to)),
        _ => {
            let verb = agree(area, "réglé", "réglée");
            match from {
                Some(from) if from != to => {
                    format!("{area} {verb} à {} (avant {}).", temp(to), temp(from))
                }
                _ => format!("{area} {verb} à {}.", temp(to)),
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
        }
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
}
