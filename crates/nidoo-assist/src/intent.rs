//! What a sentence asks for, before looking at the house.

use crate::lexicon::{Class, class};
use crate::text::Word;

/// What the sentence asks, with the words left that may name a room or a light.
#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    /// `names`: words that may name lights; `room`: words that may name the room.
    Lights {
        on: bool,
        all: bool,
        names: Vec<String>,
        room: Vec<String>,
    },
    SetTemp {
        change: Change,
        room: Vec<String>,
    },
    /// "allume le chauffage": heating named, no temperature given.
    HeatingNoValue {
        room: Vec<String>,
    },
    ReadTemp {
        room: Vec<String>,
        outdoor: bool,
    },
}

/// A setpoint change: a value, or a step from the current setpoint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    Absolute(f64),
    Relative(f64),
}

/// The step of a bare "monte" / "baisse" or of "un peu".
pub const LITTLE: f64 = 0.5;

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Known(Class, String),
    Number(f64),
    Signed(f64),
    Free(String),
}

fn tag(words: &[Word]) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let next = words.get(i + 1);
        match &words[i] {
            Word::Number(n) if *n == 1.0 && next.is_some_and(|w| w.is("peu")) => {
                out.push(Tok::Known(Class::Little, "un peu".into()));
                i += 2;
                continue;
            }
            Word::Number(n) => out.push(Tok::Number(*n)),
            Word::Signed(n) => out.push(Tok::Signed(*n)),
            Word::Text(t) if t == "pour" && next.is_some_and(|w| w.is("cent")) => {
                out.push(Tok::Known(Class::Percent, "%".into()));
                i += 2;
                continue;
            }
            Word::Text(t) if t == "en" && next.is_some_and(|w| w.is("marche")) => {
                out.push(Tok::Known(Class::On, "en marche".into()));
                i += 2;
                continue;
            }
            Word::Text(t) => out.push(match class(t) {
                Some(c) => Tok::Known(c, t.clone()),
                None => Tok::Free(t.clone()),
            }),
        }
        i += 1;
    }
    out
}

/// Parses a normalised sentence; `None` when it is not understood. Never guesses: two
/// opposite verbs, a negation, a percentage or a device this version does not handle all give
/// `None`.
pub fn parse(words: &[Word]) -> Option<Intent> {
    let toks = tag(words);
    let has = |c: Class| {
        toks.iter()
            .any(|t| matches!(t, Tok::Known(k, _) if *k == c))
    };
    if has(Class::Percent) || has(Class::OtherDevice) || has(Class::Negation) {
        return None;
    }
    let (on, off, set) = (has(Class::On), has(Class::Off), has(Class::Set));
    let (raise, lower) = (has(Class::Raise), has(Class::Lower));
    if (on && off) || (raise && lower) {
        return None;
    }
    let light = has(Class::Light) || has(Class::LightNamed);
    let heating = has(Class::Heating);
    let numbers: Vec<f64> = toks
        .iter()
        .filter_map(|t| match t {
            Tok::Number(n) => Some(*n),
            _ => None,
        })
        .collect();
    let signed = toks.iter().find_map(|t| match t {
        Tok::Signed(n) => Some(*n),
        _ => None,
    });
    // The number right after "à": "mets le salon à 19".
    let after_a = toks.windows(2).find_map(|w| match (&w[0], &w[1]) {
        (Tok::Known(Class::Linker, a), Tok::Number(n)) if a == "a" => Some(*n),
        _ => None,
    });
    let free: Vec<String> = toks
        .iter()
        .filter_map(|t| match t {
            Tok::Free(w) => Some(w.clone()),
            _ => None,
        })
        .collect();
    let action = on || off || set || raise || lower;

    if !action && numbers.is_empty() && signed.is_none() && !heating && !light {
        let asks = has(Class::Temperature) || (has(Class::Fait) && has(Class::Question));
        if asks {
            return Some(Intent::ReadTemp {
                room: free,
                outdoor: has(Class::Outdoor),
            });
        }
        return None;
    }

    let heating_ctx =
        heating || has(Class::Degree) || (has(Class::Temperature) && !numbers.is_empty());
    if heating_ctx {
        if light || (off && !numbers.is_empty()) {
            return None;
        }
        let room = free;
        if raise || lower {
            let change = match after_a {
                Some(n) => Change::Absolute(n),
                None => {
                    let step = if has(Class::Little) {
                        LITTLE
                    } else {
                        signed.or(numbers.first().copied()).map_or(LITTLE, f64::abs)
                    };
                    Change::Relative(if lower { -step } else { step })
                }
            };
            return Some(Intent::SetTemp { change, room });
        }
        if let Some(s) = signed {
            return Some(Intent::SetTemp {
                change: Change::Relative(s),
                room,
            });
        }
        if let Some(n) = after_a.or(numbers.last().copied()) {
            return Some(Intent::SetTemp {
                change: Change::Absolute(n),
                room,
            });
        }
        if heating && (on || off) {
            return Some(Intent::HeatingNoValue { room });
        }
        return None;
    }

    if set && !light && !on {
        if let Some(n) = after_a {
            return Some(Intent::SetTemp {
                change: Change::Absolute(n),
                room: free,
            });
        }
        return None;
    }

    if on || off {
        return Some(lights(&toks, on, has(Class::All)));
    }
    None
}

/// Lights: named-light words are names; other free words before the first linker are names
/// too ("la led du salon"), the ones after it name the room. Without a linker every free word
/// names the room.
fn lights(toks: &[Tok], on: bool, all: bool) -> Intent {
    let linker = toks
        .iter()
        .position(|t| matches!(t, Tok::Known(Class::Linker, _)));
    let mut names = Vec::new();
    let mut room = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        match t {
            Tok::Known(Class::LightNamed, w) => names.push(w.clone()),
            Tok::Free(w) => match linker {
                Some(l) if i < l => names.push(w.clone()),
                _ => room.push(w.clone()),
            },
            _ => {}
        }
    }
    Intent::Lights {
        on,
        all,
        names,
        room,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::words;

    fn p(s: &str) -> Option<Intent> {
        parse(&words(s))
    }

    fn v(ws: &[&str]) -> Vec<String> {
        ws.iter().map(|w| w.to_string()).collect()
    }

    fn lights(on: bool, all: bool, names: &[&str], room: &[&str]) -> Option<Intent> {
        Some(Intent::Lights {
            on,
            all,
            names: v(names),
            room: v(room),
        })
    }

    fn set(change: Change, room: &[&str]) -> Option<Intent> {
        Some(Intent::SetTemp {
            change,
            room: v(room),
        })
    }

    #[test]
    fn on_and_off_verbs() {
        assert_eq!(
            p("allume la lumière du salon"),
            lights(true, false, &[], &["salon"])
        );
        assert_eq!(
            p("éteins la lumière du salon"),
            lights(false, false, &[], &["salon"])
        );
        assert_eq!(
            p("coupe la lumière du salon"),
            lights(false, false, &[], &["salon"])
        );
        assert_eq!(p("allume le salon"), lights(true, false, &[], &["salon"]));
        assert_eq!(
            p("allume la chambre de Léa s'il te plaît"),
            lights(true, false, &["chambre"], &["lea"])
        );
    }

    #[test]
    fn typo_never_flips_on_off() {
        assert_eq!(p("allumme le salon"), lights(true, false, &[], &["salon"]));
        assert_eq!(p("etein le salon"), lights(false, false, &[], &["salon"]));
        assert_eq!(p("eteint le salon"), lights(false, false, &[], &["salon"]));
        assert_eq!(p("allume et éteins le salon"), None);
        assert_eq!(p("allumme et etein le salon"), None);
    }

    #[test]
    fn all_word() {
        assert_eq!(
            p("éteins tout dans le salon"),
            lights(false, true, &[], &["salon"])
        );
    }

    #[test]
    fn light_names() {
        assert_eq!(
            p("éteins la led du salon"),
            lights(false, false, &["led"], &["salon"])
        );
        assert_eq!(
            p("éteins le lustre du salon"),
            lights(false, false, &["lustre"], &["salon"])
        );
        assert_eq!(p("allume la led"), lights(true, false, &["led"], &[]));
    }

    #[test]
    fn absolute_setpoints() {
        assert_eq!(
            p("mets le salon à 20 degrés"),
            set(Change::Absolute(20.0), &["salon"])
        );
        assert_eq!(
            p("règle chez Léa à 18,5"),
            set(Change::Absolute(18.5), &["lea"])
        );
        assert_eq!(
            p("mets la chambre de Tom à dix-neuf"),
            set(Change::Absolute(19.0), &["chambre", "tom"])
        );
        assert_eq!(
            p("mets le chauffage à 20"),
            set(Change::Absolute(20.0), &[])
        );
    }

    #[test]
    fn relative_setpoints() {
        assert_eq!(
            p("monte le chauffage d'un degré chez Léa"),
            set(Change::Relative(1.0), &["lea"])
        );
        assert_eq!(
            p("baisse un peu le chauffage"),
            set(Change::Relative(-0.5), &[])
        );
        assert_eq!(p("monte le chauffage"), set(Change::Relative(0.5), &[]));
        assert_eq!(
            p("chauffage +2 au bureau"),
            set(Change::Relative(2.0), &["bureau"])
        );
        assert_eq!(
            p("baisse le chauffage de deux degrés dans le salon"),
            set(Change::Relative(-2.0), &["salon"])
        );
    }

    #[test]
    fn raise_without_heating_word_is_none() {
        // "monte le salon" could be a shutter: never guess heating.
        assert_eq!(p("monte le salon"), None);
    }

    #[test]
    fn heating_without_value() {
        assert_eq!(
            p("allume le chauffage du salon"),
            Some(Intent::HeatingNoValue {
                room: v(&["salon"])
            })
        );
    }

    #[test]
    fn temperature_questions() {
        assert_eq!(
            p("quelle température fait-il dans le salon ?"),
            Some(Intent::ReadTemp {
                room: v(&["salon"]),
                outdoor: false
            })
        );
        assert_eq!(
            p("il fait combien dans le bureau ?"),
            Some(Intent::ReadTemp {
                room: v(&["bureau"]),
                outdoor: false
            })
        );
        assert_eq!(
            p("il fait combien dehors ?"),
            Some(Intent::ReadTemp {
                room: vec![],
                outdoor: true
            })
        );
    }

    #[test]
    fn percent_is_not_a_setpoint() {
        assert_eq!(p("mets le salon à 20 %"), None);
        assert_eq!(p("mets le salon à 20 pour cent"), None);
    }

    #[test]
    fn light_count_is_not_a_setpoint() {
        assert_eq!(
            p("allume les 3 lumières du salon"),
            lights(true, false, &[], &["salon"])
        );
    }

    #[test]
    fn other_devices_are_none() {
        assert_eq!(p("ferme les volets du salon"), None);
        assert_eq!(p("éteins la prise du bureau"), None);
    }

    #[test]
    fn negation_is_none() {
        assert_eq!(p("n'allume pas la lumière du salon"), None);
        assert_eq!(p("ne coupe pas le salon"), None);
    }

    #[test]
    fn unknown_is_none() {
        assert_eq!(p("fais-moi un café"), None);
        assert_eq!(p(""), None);
    }
}
