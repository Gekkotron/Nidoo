//! The French replies, written from templates. Every reply names the room, the action and
//! the value.

use nidoo_api::search::fold;

use crate::house::House;
use crate::plan::Done;

/// What happened to one action, in the plan's order.
#[derive(Clone, Debug, PartialEq)]
pub enum ActionResult {
    Ok,
    /// Nothing to do: already in the wanted state.
    Skipped,
    /// The device is offline: nothing was sent, and the reply says it did not answer.
    Offline,
    Failed(String),
}

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
    };
    if simulate {
        format!("Simulation : {text}")
    } else {
        text
    }
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
        return match failed.first() {
            Some((_, why)) => format!("« {name} » n'a pas répondu {} : {why}.", dans(area)),
            None if skipped == 1 => format!("Lumière « {name} » déjà {verb_f} {}.", dans(area)),
            None => format!("Lumière « {name} » {verb_f} {}.", dans(area)),
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
        match failed.first() {
            Some((_, why)) => format!("« {name} » n'a pas répondu{place} : {why}."),
            None if skipped == 1 => format!("Volet « {name} » déjà {one}{place}."),
            None => format!("Volet « {name} » {one}{place}."),
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
}
