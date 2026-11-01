//! FR/EN text: Moods: examples, apply reports and action sentences.

use super::*;

/// The name and subtitle of the `i`-th example ambiance (0 to 3).
pub fn ambiance_example(lang: Lang, i: usize) -> (&'static str, &'static str) {
    let (en, fr) = match i {
        0 => (
            ("Good night", "The house goes to sleep"),
            ("Bonne nuit", "La maison s'endort"),
        ),
        1 => (
            ("I'm heading out", "Peace of mind"),
            ("Je m'absente", "L'esprit tranquille"),
        ),
        2 => (
            ("Quiet evening", "Dimmed lights"),
            ("Soirée tranquille", "Lumières tamisées"),
        ),
        _ => (
            ("A little reading", "Your favourite spot"),
            ("Un peu de lecture", "Votre coin préféré"),
        ),
    };
    match lang {
        Lang::En => en,
        Lang::Fr => fr,
    }
}

/// The toast after an ambiance ran with no failure: "Good night: done".
pub fn ambiance_done(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("{name}: done"),
        Lang::Fr => format!("{name} : c'est fait"),
    }
}

/// The toast after an ambiance ran and `n` devices failed.
pub fn ambiance_partial(lang: Lang, name: &str, n: usize) -> String {
    match (lang, n == 1) {
        (Lang::En, true) => format!("{name}: 1 device did not respond"),
        (Lang::En, false) => format!("{name}: {n} devices did not respond"),
        (Lang::Fr, true) => format!("{name} : 1 appareil n'a pas répondu"),
        (Lang::Fr, false) => format!("{name} : {n} appareils n'ont pas répondu"),
    }
}

/// The question before deleting an ambiance.
pub fn ambiance_delete_question(lang: Lang, name: &str) -> String {
    match lang {
        Lang::En => format!("Delete {name}?"),
        Lang::Fr => format!("Supprimer {name} ?"),
    }
}

/// The live count under a category action: "1 device" / "3 devices".
pub fn category_count(lang: Lang, n: usize) -> String {
    match (lang, n) {
        (Lang::En, 1) => "1 device".to_owned(),
        (Lang::En, _) => format!("{n} devices"),
        (Lang::Fr, 0 | 1) => format!("{n} appareil"),
        (Lang::Fr, _) => format!("{n} appareils"),
    }
}

/// A category action: "Every light in the house: off" / "Tous les éclairages de la maison :
/// éteindre". `what` is `lights`, `plugs` or `covers`, `to` is `on`, `off`, `open` or `close`;
/// `area` is an area name, `None` for the whole house.
pub fn category_sentence(
    lang: Lang,
    what: &str,
    to: &str,
    brightness: Option<u8>,
    area: Option<&str>,
) -> String {
    match lang {
        Lang::En => {
            let noun = match what {
                "lights" => "Every light",
                "plugs" => "Every plug",
                _ => "Every shutter",
            };
            let place = area.map_or_else(|| "the house".to_owned(), str::to_owned);
            let verb = match (to, brightness) {
                ("on", Some(n)) => format!("on at {n} %"),
                _ => to.to_owned(),
            };
            format!("{noun} in {place}: {verb}")
        }
        Lang::Fr => {
            let noun = match what {
                "lights" => "Tous les éclairages",
                "plugs" => "Toutes les prises",
                _ => "Tous les volets",
            };
            let place = area.map_or_else(|| " de la maison".to_owned(), |a| format!(" — {a}"));
            let verb = match (to, brightness) {
                ("on", Some(n)) => format!("allumer à {n} %"),
                ("on", None) => "allumer".to_owned(),
                ("off", _) => "éteindre".to_owned(),
                ("open", _) => "ouvrir".to_owned(),
                _ => "fermer".to_owned(),
            };
            format!("{noun}{place} : {verb}")
        }
    }
}

/// A climate action. `zone` is a zone name, `None` for every zone; `setpoint` is the text to
/// show (a temperature or "off"), `None` when the action sets none.
pub fn climate_sentence(
    lang: Lang,
    zone: Option<&str>,
    clear: bool,
    setpoint: Option<&str>,
    hours: Option<u32>,
) -> String {
    let fr = lang == Lang::Fr;
    let head = match (zone, fr) {
        (None, false) => "Heating, every zone".to_owned(),
        (None, true) => "Chauffage, toutes les zones".to_owned(),
        (Some(z), false) => format!("Heating, {z}"),
        (Some(z), true) => format!("Chauffage, {z}"),
    };
    let colon = if fr { " : " } else { ": " };
    if clear {
        let tail = if fr {
            "retour au programme"
        } else {
            "back to the schedule"
        };
        return format!("{head}{colon}{tail}");
    }
    let mut parts = Vec::new();
    if let Some(sp) = setpoint {
        parts.push(match (sp, fr) {
            ("off", true) => "arrêt".to_owned(),
            ("off", false) => "off".to_owned(),
            (t, _) => format!("{t} °C"),
        });
    }
    if let Some(h) = hours {
        let for_ = if fr { "pendant" } else { "for" };
        parts.push(format!("{for_} {h} h"));
    }
    if parts.is_empty() {
        head
    } else {
        format!("{head}{colon}{}", parts.join(" "))
    }
}

/// A flow action: "Automation Away lights: switch on". `do_` is `enable`, `disable` or `run`.
pub fn flow_sentence(lang: Lang, name: &str, do_: &str) -> String {
    match lang {
        Lang::En => {
            let verb = match do_ {
                "enable" => "switch on",
                "disable" => "switch off",
                _ => "run",
            };
            format!("Automation {name}: {verb}")
        }
        Lang::Fr => {
            let verb = match do_ {
                "enable" => "activer",
                "disable" => "désactiver",
                _ => "lancer",
            };
            format!("Automatisation {name} : {verb}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn category_count_is_singular_for_one() {
        assert_eq!(category_count(Lang::En, 1), "1 device");
        assert_eq!(category_count(Lang::En, 3), "3 devices");
        assert_eq!(category_count(Lang::Fr, 1), "1 appareil");
        assert_eq!(category_count(Lang::Fr, 0), "0 appareil");
        assert_eq!(category_count(Lang::Fr, 2), "2 appareils");
    }
}
