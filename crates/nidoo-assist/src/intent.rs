//! What a sentence asks for, before looking at the house.

use crate::lexicon::{Class, class};
use crate::text::{Word, number_text};

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
    /// Shutters: `position` is the % open asked ("à 30 %"), only with an open verb; `all`:
    /// "tous" / "toutes" was said; `names` and `room` split like lights.
    Covers {
        open: bool,
        position: Option<u8>,
        all: bool,
        names: Vec<String>,
        room: Vec<String>,
    },
    /// "ferme les volets à 30 %": a position with a close verb is ambiguous; answered with a hint.
    CoverPositionHint,
    /// A shutter position outside 0–100 %.
    CoverPositionRange,
}

/// A setpoint change: a value, or a step from the current setpoint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    Absolute(f64),
    Relative(f64),
}

/// The step of "un peu" / "légèrement".
pub const LITTLE: f64 = 0.5;

/// The step of a bare "monte" / "baisse".
pub const STEP: f64 = 1.0;

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

/// What each number of the sentence is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Role {
    /// After "à": "mets le salon à 19".
    AfterA,
    /// Before "degré(s)" / "°", or after "de" / "d'": "d'un degré", "de 2".
    Amount,
    /// Right before a light word: "les 3 lumières".
    Count,
    /// Anything else is part of a name: "la chambre 2".
    Name,
}

fn roles(toks: &[Tok]) -> Vec<Option<Role>> {
    toks.iter()
        .enumerate()
        .map(|(i, t)| {
            if !matches!(t, Tok::Number(_)) {
                return None;
            }
            let before = i.checked_sub(1).and_then(|j| toks.get(j));
            let after = toks.get(i + 1);
            let linker = |t: Option<&Tok>, words: &[&str]| {
                matches!(t, Some(Tok::Known(Class::Linker, w)) if words.contains(&w.as_str()))
            };
            Some(if linker(before, &["a"]) {
                Role::AfterA
            } else if matches!(after, Some(Tok::Known(Class::Degree, _)))
                || linker(before, &["de", "d"])
            {
                Role::Amount
            } else if matches!(
                after,
                Some(Tok::Known(Class::Light | Class::LightNamed | Class::Cover, _))
            ) {
                Role::Count
            } else {
                Role::Name
            })
        })
        .collect()
}

/// Parses a normalised sentence; `None` when it is not understood. Never guesses: two
/// opposite verbs, a negation, a percentage, a device this version does not handle, or a
/// statement or question that is not a polite request ("il fait 18 degrés", "qui a éteint")
/// all give `None` (a temperature question is read, never acted on).
pub fn parse(words: &[Word]) -> Option<Intent> {
    let toks = tag(words);
    let has = |c: Class| {
        toks.iter()
            .any(|t| matches!(t, Tok::Known(k, _) if *k == c))
    };
    let cover = has(Class::Cover);
    // A percentage only ever means a shutter position; "ouvre" / "ferme" only ever a shutter.
    if (has(Class::Percent) && !cover)
        || has(Class::OtherDevice)
        || has(Class::Negation)
        || ((has(Class::Open) || has(Class::Close)) && !cover)
    {
        return None;
    }
    // "à -2": a signed value after "à" is neither a setpoint nor a change.
    if toks.windows(2).any(
        |w| matches!((&w[0], &w[1]), (Tok::Known(Class::Linker, a), Tok::Signed(_)) if a == "a"),
    ) {
        return None;
    }
    let question_word = toks
        .iter()
        .any(|t| matches!(t, Tok::Known(Class::Question, w) if w != "?"));
    let describes = has(Class::Statement) || has(Class::Fait) || question_word;
    let blocked = describes && !has(Class::Request);
    if cover {
        // "les volets du salon fermés ?": a question mark keeps a past participle a question.
        let asked = has(Class::Question) && !has(Class::Request);
        return if blocked || asked {
            None
        } else {
            covers(&toks, &roles(&toks), &has)
        };
    }
    let (on, off, set) = (has(Class::On), has(Class::Off), has(Class::Set));
    let (raise, lower) = (has(Class::Raise), has(Class::Lower));
    if (on && off) || (raise && lower) {
        return None;
    }
    let light = has(Class::Light) || has(Class::LightNamed);
    let heating = has(Class::Heating);
    let roles = roles(&toks);
    let number_with = |role: Role| {
        toks.iter().zip(&roles).find_map(|(t, r)| match (t, r) {
            (Tok::Number(n), Some(r)) if *r == role => Some(*n),
            _ => None,
        })
    };
    let after_a = number_with(Role::AfterA);
    let amount = number_with(Role::Amount);
    let signed = toks.iter().find_map(|t| match t {
        Tok::Signed(n) => Some(*n),
        _ => None,
    });
    // Room words: the free words and the numbers that are part of a name.
    let free: Vec<String> = toks
        .iter()
        .zip(&roles)
        .filter_map(|(t, r)| match (t, r) {
            (Tok::Free(w), _) => Some(w.clone()),
            (Tok::Number(n), Some(Role::Name)) => Some(number_text(*n)),
            _ => None,
        })
        .collect();
    let action = on || off || set || raise || lower;
    let valued = after_a.is_some() || amount.is_some() || signed.is_some();

    let asks = has(Class::Temperature) || (has(Class::Fait) && question_word);
    if (blocked || !action) && !heating && !light && asks && !(has(Class::Fait) && valued) {
        return Some(Intent::ReadTemp {
            room: free,
            outdoor: has(Class::Outdoor),
        });
    }
    if blocked || (!action && !valued && !heating) {
        return None;
    }

    let heating_ctx = heating || has(Class::Degree) || (has(Class::Temperature) && valued);
    if heating_ctx {
        if light || (off && valued) {
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
                        signed.or(amount).map_or(STEP, f64::abs)
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
        if let Some(n) = after_a.or(amount) {
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
        // "allume le salon à 20": a light verb with a value is neither lights nor heating;
        // switching lights while ignoring the number would be a guess.
        if after_a.is_some() {
            return None;
        }
        return Some(lights(&toks, &roles, on, has(Class::All)));
    }
    None
}

/// Shutters: an open verb ("ouvre", "monte") or a close verb ("ferme", "baisse"), never both
/// or neither, and never with a light, heating or switch word. A position is "à N %" right after
/// the open verb's object; any other number or percentage is not understood. Names and room words
/// split like lights.
fn covers(toks: &[Tok], roles: &[Option<Role>], has: &dyn Fn(Class) -> bool) -> Option<Intent> {
    let other = [
        Class::Light,
        Class::LightNamed,
        Class::Heating,
        Class::Temperature,
        Class::Degree,
        Class::On,
        Class::Off,
        Class::Set,
    ];
    if other.into_iter().any(has) || toks.iter().any(|t| matches!(t, Tok::Signed(_))) {
        return None;
    }
    let open = has(Class::Open) || has(Class::Raise);
    let close = has(Class::Close) || has(Class::Lower);
    if open == close {
        return None;
    }
    // "à N %": the number after "à" with the percent sign right after it.
    let mut position = None;
    for (i, t) in toks.iter().enumerate() {
        if let (Tok::Number(n), Some(Role::AfterA)) = (t, roles[i]) {
            if !matches!(toks.get(i + 1), Some(Tok::Known(Class::Percent, _))) {
                return None;
            }
            position = Some(*n);
        }
    }
    if has(Class::Percent) && position.is_none() {
        return None;
    }
    let position = match position {
        Some(_) if close => return Some(Intent::CoverPositionHint),
        Some(n) if !(0.0..=100.0).contains(&n) => return Some(Intent::CoverPositionRange),
        Some(n) => Some(n.round() as u8),
        None => None,
    };
    let (names, room) = split_names(toks, roles);
    Some(Intent::Covers {
        open,
        position,
        all: has(Class::All),
        names,
        room,
    })
}

/// Lights: named-light words are names; other free words (and name numbers) before the first
/// linker are names too ("la led du salon"), the ones after it name the room. Without a linker
/// every free word names the room. A count ("les 3 lumières") is dropped.
fn lights(toks: &[Tok], roles: &[Option<Role>], on: bool, all: bool) -> Intent {
    let (names, room) = split_names(toks, roles);
    Intent::Lights {
        on,
        all,
        names,
        room,
    }
}

/// Named-light words and the free words (and name numbers) before the first linker are names;
/// the ones after it name the room. Without a linker every free word names the room.
fn split_names(toks: &[Tok], roles: &[Option<Role>]) -> (Vec<String>, Vec<String>) {
    let linker = toks
        .iter()
        .position(|t| matches!(t, Tok::Known(Class::Linker, _)));
    let mut names = Vec::new();
    let mut room = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        let word = match (t, roles[i]) {
            (Tok::Known(Class::LightNamed, w), _) => {
                names.push(w.clone());
                continue;
            }
            (Tok::Free(w), _) => w.clone(),
            (Tok::Number(n), Some(Role::Name)) => number_text(*n),
            _ => continue,
        };
        match linker {
            Some(l) if i < l => names.push(word),
            _ => room.push(word),
        }
    }
    (names, room)
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
        assert_eq!(p("monte le chauffage"), set(Change::Relative(1.0), &[]));
        assert_eq!(
            p("monte un peu le chauffage"),
            set(Change::Relative(0.5), &[])
        );
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
        assert_eq!(p("ferme les rideaux du salon"), None);
        assert_eq!(p("éteins la prise du bureau"), None);
    }

    fn covers(
        open: bool,
        position: Option<u8>,
        all: bool,
        names: &[&str],
        room: &[&str],
    ) -> Option<Intent> {
        Some(Intent::Covers {
            open,
            position,
            all,
            names: v(names),
            room: v(room),
        })
    }

    #[test]
    fn shutter_open_close() {
        assert_eq!(
            p("ouvre les volets du salon"),
            covers(true, None, false, &[], &["salon"])
        );
        assert_eq!(
            p("ferme les volets du salon"),
            covers(false, None, false, &[], &["salon"])
        );
        assert_eq!(
            p("baisse les stores du bureau"),
            covers(false, None, false, &[], &["bureau"])
        );
        assert_eq!(
            p("remonte les volets roulants du salon"),
            covers(true, None, false, &[], &["salon"])
        );
        assert_eq!(
            p("peux-tu fermer les volets du salon"),
            covers(false, None, false, &[], &["salon"])
        );
    }

    #[test]
    fn shutter_position() {
        assert_eq!(
            p("ouvre les volets du salon à 30 %"),
            covers(true, Some(30), false, &[], &["salon"])
        );
        assert_eq!(
            p("ouvre les volets à 30 pour cent"),
            covers(true, Some(30), false, &[], &[])
        );
        assert_eq!(
            p("ferme les volets à 30 %"),
            Some(Intent::CoverPositionHint)
        );
        assert_eq!(
            p("ouvre les volets à 130 %"),
            Some(Intent::CoverPositionRange)
        );
        assert_eq!(p("ouvre les volets à 30"), None);
        assert_eq!(p("ouvre les volets 30 %"), None);
    }

    #[test]
    fn shutter_all_and_names() {
        assert_eq!(
            p("ferme tous les volets"),
            covers(false, None, true, &[], &[])
        );
        assert_eq!(
            p("ouvre tous les volets du salon"),
            covers(true, None, true, &[], &["salon"])
        );
        assert_eq!(
            p("ferme le volet baie du salon"),
            covers(false, None, false, &["baie"], &["salon"])
        );
        assert_eq!(
            p("ferme les 2 volets du salon"),
            covers(false, None, false, &[], &["salon"])
        );
    }

    #[test]
    fn shutter_never_guessed() {
        assert_eq!(p("ne ferme pas les volets"), None);
        assert_eq!(p("les volets sont fermés"), None);
        assert_eq!(p("ferme les rideaux"), None);
        assert_eq!(p("ouvre et ferme les volets"), None);
        assert_eq!(p("les volets du salon"), None);
    }

    #[test]
    fn a_shutter_question_is_not_an_order() {
        // "fermés" folds to "fermes", a close verb: the question mark keeps it a question.
        assert_eq!(p("les volets du salon fermés ?"), None);
        assert_eq!(
            p("peux-tu fermer les volets du salon ?"),
            covers(false, None, false, &[], &["salon"])
        );
    }

    #[test]
    fn remonter_opens() {
        assert_eq!(
            p("peux-tu remonter les volets du salon"),
            covers(true, None, false, &[], &["salon"])
        );
    }

    #[test]
    fn raise_without_shutter_noun_stays_not_understood() {
        assert_eq!(p("monte le salon"), None);
        assert_eq!(p("baisse le salon"), None);
    }

    #[test]
    fn close_verb_on_a_light_is_not_understood() {
        assert_eq!(p("ferme la lumière du salon"), None);
        assert_eq!(p("ouvre le chauffage"), None);
        assert_eq!(p("ferme le salon"), None);
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
