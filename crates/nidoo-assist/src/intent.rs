//! What a sentence asks for, before looking at the house.

use crate::lexicon::{Class, Colour, class, class_in, colour_in};
use crate::text::{Word, number_text};
use nidoo_api::search::fold;
use nidoo_core::flow::Lang;
use nidoo_core::model::SwitchType;

/// What the sentence asks, with the words left that may name a room or a light.
#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    /// `names`: words that may name lights; `room`: words that may name the room; `colour`:
    /// "en rouge" and `brightness`: "à 50 %" (a percent, 1–100), only ever with `on`.
    Lights {
        on: bool,
        all: bool,
        names: Vec<String>,
        room: Vec<String>,
        colour: Option<Colour>,
        brightness: Option<u8>,
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
    /// TVs or plugs on or off: `what` is `SwitchType::Tv` or `SwitchType::Plug` only; `all`:
    /// "tous" / "toutes" was said; `names` and `room` split like lights.
    Switches {
        what: SwitchType,
        on: bool,
        all: bool,
        names: Vec<String>,
        room: Vec<String>,
    },
    /// The music: pause, play, skip, ask or search.
    Media(MediaIntent),
    /// A speaker's music volume, in points of 0–100; `room`: the words that may name the room.
    Volume {
        change: Change,
        room: Vec<String>,
    },
    /// Mute (`on`) or unmute the speakers; `room` as for the volume.
    Mute {
        on: bool,
        room: Vec<String>,
    },
    /// "mets de la musique classique": a name after "mets"; answered with the "joue" hint.
    MediaHint,
    /// A volume outside 0–100 %.
    VolumeRange,
    /// A light brightness outside 1–100 %.
    BrightnessRange,
    /// "j'ai froid au salon", "il fait chaud": a small heating step, up (`warmer`) or down.
    Feel {
        warmer: bool,
        room: Vec<String>,
    },
    /// The robot vacuum: `room` holds the words that may name the room to clean, only ever with
    /// [`VacuumIntent::Clean`].
    Vacuum {
        action: VacuumIntent,
        room: Vec<String>,
    },
}

/// What is asked of the vacuum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VacuumIntent {
    /// Clean a room, or the whole house when no room is said.
    Clean,
    Stop,
    Pause,
    /// Send it back to its base.
    Dock,
}

/// What is asked of the music player.
#[derive(Clone, Debug, PartialEq)]
pub enum MediaIntent {
    Pause,
    /// `resume`: "reprends la musique", answered as a resume.
    Play {
        resume: bool,
    },
    Next,
    Previous,
    /// "c'est quoi cette chanson ?": read, never acted on.
    NowPlaying,
    /// The words after "joue", as typed.
    PlayMedia {
        query: String,
    },
}

/// The volume step of a bare "monte / baisse le son", in points.
pub const VOLUME_STEP: f64 = 10.0;

/// The volume step of "un peu".
pub const VOLUME_LITTLE: f64 = 5.0;

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
    /// "en rouge": a colour, the "en" included.
    Colour(Colour),
}

fn tag(words: &[Word], lang: Lang) -> Vec<Tok> {
    if lang == Lang::En {
        return tag_en(words);
    }
    let colour_of = |w: &str| colour_in(w, Lang::Fr);
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
            Word::Text(t)
                if t == "en" && next.and_then(Word::text).and_then(colour_of).is_some() =>
            {
                if let Some(c) = next.and_then(Word::text).and_then(colour_of) {
                    out.push(Tok::Colour(c));
                }
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

/// Words that end an English sentence without carrying meaning: a colour before them is still
/// the last word ("turn the lamp red please").
const EN_TAIL: &[&str] = &["please", "pls", "plz", "now", "thanks", "thank", "you", "?"];

/// Words that may come before an English "vacuum" that is a verb: "can you vacuum the kitchen".
const EN_POLITE: &[&str] = &[
    "please", "pls", "plz", "can", "could", "would", "will", "you", "hey", "ok", "okay", "now",
    "just", "and",
];

/// [`tag`] for English: "per cent", "go back", "mute" and "unmute", and a colour said after "in"
/// or "to" ("in red", "to blue") or as the last word after a word that is not an article ("turn
/// the lamp red"). Any other colour word is part of a name ("the red lamp").
fn tag_en(words: &[Word]) -> Vec<Tok> {
    let text = |i: usize| words.get(i).and_then(Word::text);
    let colour = |i: usize| text(i).and_then(|w| colour_in(w, Lang::En));
    let last = (0..words.len())
        .rev()
        .find(|&i| !text(i).is_some_and(|w| EN_TAIL.contains(&w)));
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        match &words[i] {
            Word::Number(n) => out.push(Tok::Number(*n)),
            Word::Signed(n) => out.push(Tok::Signed(*n)),
            Word::Text(t) => {
                let t = t.as_str();
                if matches!(t, "in" | "to") && colour(i + 1).is_some() {
                    if let Some(c) = colour(i + 1) {
                        out.push(Tok::Colour(c));
                    }
                    i += 2;
                    continue;
                }
                if let Some(c) = colour(i).filter(|_| Some(i) == last && i > 0)
                    && !matches!(
                        class_in(text(i - 1).unwrap_or_default(), Lang::En),
                        Some(Class::Noise | Class::Linker)
                    )
                    && words[i - 1].text().is_some()
                {
                    out.push(Tok::Colour(c));
                    i += 1;
                    continue;
                }
                match (t, text(i + 1)) {
                    ("per", Some("cent")) => {
                        out.push(Tok::Known(Class::Percent, "%".into()));
                        i += 2;
                        continue;
                    }
                    ("go" | "skip" | "jump", Some("back")) => {
                        out.push(Tok::Known(Class::Previous, "back".into()));
                        i += 2;
                        continue;
                    }
                    // "vacuum the kitchen" cleans; "start the vacuum" names the machine.
                    ("vacuum" | "hoover", _)
                        if (0..i).all(|j| text(j).is_some_and(|w| EN_POLITE.contains(&w))) =>
                    {
                        out.push(Tok::Known(Class::Clean, t.to_owned()));
                    }
                    ("last", Some("song" | "track")) => {
                        out.push(Tok::Known(Class::Previous, "last".into()));
                    }
                    ("mute", _) => {
                        out.push(Tok::Known(Class::Off, "mute".into()));
                        out.push(Tok::Known(Class::Sound, "mute".into()));
                    }
                    ("unmute", _) => {
                        out.push(Tok::Known(Class::Resume, "unmute".into()));
                        out.push(Tok::Known(Class::Sound, "unmute".into()));
                    }
                    _ => out.push(match class_in(t, Lang::En) {
                        Some(c) => Tok::Known(c, t.to_owned()),
                        None => Tok::Free(t.to_owned()),
                    }),
                }
            }
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

/// The linkers a value follows: "à 19", "to 19", "at 19".
const AFTER_A: &[&str] = &["a", "to", "at"];

/// The linkers an amount follows: "de 2", "d'un degré", "by 2".
const AMOUNT: &[&str] = &["de", "d", "by"];

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
            Some(if linker(before, AFTER_A) {
                Role::AfterA
            } else if matches!(after, Some(Tok::Known(Class::Degree, _)))
                || linker(before, AMOUNT)
            {
                Role::Amount
            } else if matches!(
                after,
                Some(Tok::Known(
                    Class::Light | Class::LightNamed | Class::Cover | Class::Tv | Class::Plug,
                    _
                ))
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
    parse_in(words, Lang::Fr)
}

/// [`parse`] in `lang`.
pub fn parse_in(words: &[Word], lang: Lang) -> Option<Intent> {
    let mut toks = tag(words, lang);
    // "en rouge": taken out of the sentence, which then parses as before; only lights switched
    // on take it. Two different colours are not understood.
    let mut colour = None;
    for t in &toks {
        if let Tok::Colour(c) = t {
            if colour.is_some_and(|seen: Colour| seen != *c) {
                return None;
            }
            colour = Some(*c);
        }
    }
    toks.retain(|t| !matches!(t, Tok::Colour(_)));
    // "à 50 %": a brightness, only for lights switched on; anything else with a percentage
    // (shutters, an off order, heating, plugs) is read as before.
    if let Some((stripped, percent)) = take_brightness(&toks)
        && let Some(intent) = bright_lights(stripped, percent, colour, lang)
    {
        return Some(intent);
    }
    if colour.is_some() {
        turn_sets(&mut toks, lang);
    }
    let intent = parse_toks(toks, colour.is_some())?;
    match (intent, colour) {
        (intent, None) => Some(intent),
        (
            Intent::Lights {
                on: true,
                all,
                names,
                room,
                colour: None,
                brightness: None,
            },
            Some(c),
        ) => Some(Intent::Lights {
            on: true,
            all,
            names,
            room,
            colour: Some(c),
            brightness: None,
        }),
        // A colour for heating, shutters, music or an off order would be ignored: not a guess.
        _ => None,
    }
}

/// "turn the lamp red": with a colour or a brightness and no other verb, "turn" or "switch"
/// sets it.
fn turn_sets(toks: &mut [Tok], lang: Lang) {
    let verb = |t: &Tok| matches!(t, Tok::Known(Class::On | Class::Off | Class::Set, _));
    if lang == Lang::En && !toks.iter().any(verb) {
        for t in toks {
            if matches!(t, Tok::Known(Class::Noise, w) if w == "turn" || w == "switch") {
                *t = Tok::Known(Class::Set, "turn".into());
            }
        }
    }
}

/// The sentence without its one "à 50 %" ("at" / "to" in English), and the number.
fn take_brightness(toks: &[Tok]) -> Option<(Vec<Tok>, f64)> {
    let i = toks.windows(3).position(|w| {
        matches!(&w[0], Tok::Known(Class::Linker, a) if AFTER_A.contains(&a.as_str()))
            && matches!(w[1], Tok::Number(_))
            && matches!(w[2], Tok::Known(Class::Percent, _))
    })?;
    let Tok::Number(n) = toks[i + 1] else {
        return None;
    };
    let mut stripped = toks.to_vec();
    stripped.drain(i..i + 3);
    Some((stripped, n))
}

/// Lights switched on at a brightness. Needs a light word, a colour or an "allume": "mets le
/// salon à 20 %" could as well be the heating, so it stays not understood. `None` when the
/// sentence is anything but lights switched on.
fn bright_lights(
    mut toks: Vec<Tok>,
    percent: f64,
    colour: Option<Colour>,
    lang: Lang,
) -> Option<Intent> {
    let named = toks.iter().any(|t| {
        matches!(
            t,
            Tok::Known(Class::Light | Class::LightNamed | Class::On, _)
        )
    });
    if colour.is_none() && !named {
        return None;
    }
    turn_sets(&mut toks, lang);
    let Intent::Lights {
        on: true,
        all,
        names,
        room,
        colour: None,
        brightness: None,
    } = parse_toks(toks, true)?
    else {
        return None;
    };
    let percent = percent.round();
    if !(1.0..=100.0).contains(&percent) {
        return Some(Intent::BrightnessRange);
    }
    Some(Intent::Lights {
        on: true,
        all,
        names,
        room,
        colour,
        brightness: Some(percent as u8),
    })
}

/// [`parse`] without the colour words; `coloured`: a colour or a brightness was said, so "mets"
/// alone switches lights on ("mets le salon en bleu").
fn parse_toks(mut toks: Vec<Tok>, coloured: bool) -> Option<Intent> {
    // A light, shutter, heating or temperature word decides the sentence: a TV or plug word is
    // then part of a name ("la lumière du salon tv", "le volet tv").
    let decided = toks.iter().any(|t| {
        matches!(
            t,
            Tok::Known(
                Class::Light
                    | Class::LightNamed
                    | Class::Cover
                    | Class::Heating
                    | Class::Temperature
                    | Class::Degree,
                _
            )
        )
    });
    if decided {
        for t in &mut toks {
            if let Tok::Known(Class::Tv | Class::Plug, w) = t {
                *t = Tok::Free(std::mem::take(w));
            }
        }
    }
    // "le son" is the speakers' sound; "son bureau" is a possessive, a free word as before.
    for i in 0..toks.len() {
        if matches!(&toks[i], Tok::Known(Class::Sound, w) if w == "son") {
            let article = i.checked_sub(1).is_some_and(|j| {
                matches!(&toks[j], Tok::Known(Class::Noise | Class::Linker, w)
                    if matches!(w.as_str(), "le" | "du" | "au" | "de" | "l"))
            });
            if !article {
                toks[i] = Tok::Free("son".into());
            }
        }
    }
    if vacuum_sentence(&toks) {
        return vacuum(&toks);
    }
    // Not about the vacuum: its words are plain words again ("le chargeur", "la station"), as
    // before they were known.
    for t in &mut toks {
        if let Tok::Known(Class::Vacuum | Class::Clean | Class::Dock, w) = t {
            *t = Tok::Free(std::mem::take(w));
        }
    }
    if music_sentence(&toks) {
        // A music word the music intents cannot use is not understood: never lights or heating.
        return media(&toks);
    }
    // Not about music: the music words are plain words again, as before they were known, so
    // "la lumière suivante" or "remets le chauffage à 20" parse exactly as they did.
    for t in &mut toks {
        if let Tok::Known(c, w) = t
            && MEDIA_CLASSES.contains(c)
        {
            *t = Tok::Free(std::mem::take(w));
        }
    }
    if let Some(intent) = feeling(&mut toks) {
        return Some(intent);
    }
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
    if toks.windows(2).any(|w| {
        matches!((&w[0], &w[1]), (Tok::Known(Class::Linker, a), Tok::Signed(_))
            if AFTER_A.contains(&a.as_str()))
    }) {
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
    if has(Class::Tv) || has(Class::Plug) {
        // "les télés du salon allumées ?": a question mark keeps a past participle a question.
        let asked = has(Class::Question) && !has(Class::Request);
        return if blocked || asked {
            None
        } else {
            switches(&toks, &roles(&toks), &has)
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

    if coloured && set && !off && !valued {
        return Some(lights(&toks, &roles, true, has(Class::All)));
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

/// Words that make a feeling stronger, never part of a room's name: "il fait trop chaud".
const INTENSIFIERS: &[&str] = &[
    "trop",
    "tres",
    "vraiment",
    "super",
    "hyper",
    "assez",
    "grave",
    "bien",
    "si",
    "tellement",
];

/// "j'ai froid au salon", "il fait chaud", "ça caille": a feeling about the room's warmth,
/// when nothing else is asked (no verb, no device word, no value, no negation). A question or
/// "dehors" reads the temperature instead. Otherwise the cold and hot words are plain words
/// again ("la lumière chaude"), as before they were known.
fn feeling(toks: &mut [Tok]) -> Option<Intent> {
    let has = |c: Class| {
        toks.iter()
            .any(|t| matches!(t, Tok::Known(k, _) if *k == c))
    };
    let (cold, hot) = (has(Class::Cold), has(Class::Hot));
    if !cold && !hot {
        return None;
    }
    let other = [
        Class::On,
        Class::Off,
        Class::Set,
        Class::Raise,
        Class::Lower,
        Class::Light,
        Class::LightNamed,
        Class::Cover,
        Class::Open,
        Class::Close,
        Class::Tv,
        Class::Plug,
        Class::OtherDevice,
        Class::Percent,
        Class::Negation,
        Class::Degree,
    ];
    let roles = roles(toks);
    let valued = toks.iter().zip(&roles).any(|(t, r)| {
        matches!(t, Tok::Signed(_)) || (matches!(t, Tok::Number(_)) && *r != Some(Role::Name))
    });
    if cold != hot && !valued && !other.iter().any(|c| has(*c)) {
        let room: Vec<String> = toks
            .iter()
            .zip(&roles)
            .filter_map(|(t, r)| match (t, r) {
                (Tok::Free(w), _) if !INTENSIFIERS.contains(&w.as_str()) => Some(w.clone()),
                (Tok::Number(n), Some(Role::Name)) => Some(number_text(*n)),
                _ => None,
            })
            .collect();
        let outdoor = has(Class::Outdoor);
        if has(Class::Question) || outdoor {
            return Some(Intent::ReadTemp { room, outdoor });
        }
        return Some(Intent::Feel { warmer: cold, room });
    }
    for t in toks.iter_mut() {
        if let Tok::Known(Class::Cold | Class::Hot, w) = t {
            *t = Tok::Free(std::mem::take(w));
        }
    }
    None
}

/// A sentence about the vacuum: a vacuum or a clean word, and no light, shutter, heating,
/// temperature, TV or plug word (those sentences are read as before).
fn vacuum_sentence(toks: &[Tok]) -> bool {
    let has = |c: Class| {
        toks.iter()
            .any(|t| matches!(t, Tok::Known(k, _) if *k == c))
    };
    let blockers = [
        Class::Light,
        Class::LightNamed,
        Class::Cover,
        Class::Heating,
        Class::Temperature,
        Class::Tv,
        Class::Plug,
    ];
    (has(Class::Vacuum) || has(Class::Clean)) && !blockers.into_iter().any(has)
}

/// The vacuum: clean (a room, or everything), stop, pause or go back to the base. A negation,
/// a statement or a question that is not a polite request, a value, or a sentence with no verb
/// ("l'aspirateur") is `None`.
fn vacuum(toks: &[Tok]) -> Option<Intent> {
    let has = |c: Class| {
        toks.iter()
            .any(|t| matches!(t, Tok::Known(k, _) if *k == c))
    };
    let question_word = toks
        .iter()
        .any(|t| matches!(t, Tok::Known(Class::Question, w) if w != "?"));
    let describes = has(Class::Statement) || has(Class::Fait) || question_word;
    let asked = has(Class::Question) && !has(Class::Request);
    let roles = roles(toks);
    let valued = toks.iter().zip(&roles).any(|(t, r)| {
        matches!(t, Tok::Signed(_)) || (matches!(t, Tok::Number(_)) && *r != Some(Role::Name))
    });
    let other = [
        Class::Negation,
        Class::Percent,
        Class::Degree,
        Class::Raise,
        Class::Lower,
        Class::Open,
        Class::Close,
        Class::OtherDevice,
    ];
    if (describes && !has(Class::Request)) || asked || valued || other.into_iter().any(has) {
        return None;
    }
    let action = if has(Class::Vacuum) && has(Class::Dock) {
        VacuumIntent::Dock
    } else if has(Class::Pause) {
        VacuumIntent::Pause
    } else if has(Class::Off) || has(Class::Stop) {
        VacuumIntent::Stop
    } else if has(Class::On)
        || has(Class::Launch)
        || has(Class::Set)
        || has(Class::Resume)
        || has(Class::Clean)
    {
        VacuumIntent::Clean
    } else {
        return None;
    };
    let room = match action {
        VacuumIntent::Clean => toks
            .iter()
            .zip(&roles)
            .filter_map(|(t, r)| match (t, r) {
                (Tok::Free(w), _) => Some(w.clone()),
                (Tok::Number(n), Some(Role::Name)) => Some(number_text(*n)),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Some(Intent::Vacuum { action, room })
}

/// The lexicon classes only the music and volume intents read.
const MEDIA_CLASSES: [Class; 10] = [
    Class::Media,
    Class::Sound,
    Class::Pause,
    Class::Next,
    Class::Previous,
    Class::Resume,
    Class::Launch,
    Class::Stop,
    Class::Back,
    Class::Listen,
];

/// Words that name the room after them ("dans le salon"), not the music before them.
const ROOM_LINKERS: &[&str] = &[
    "dans", "du", "au", "aux", "chez", "sur", "in", "at", "from", "inside",
];

/// A sentence about the music or the speakers: a music word, and no light, shutter, heating,
/// TV, plug or other device word, and no negation.
fn music_sentence(toks: &[Tok]) -> bool {
    let has = |c: Class| {
        toks.iter()
            .any(|t| matches!(t, Tok::Known(k, _) if *k == c))
    };
    let back = has(Class::Back)
        && toks
            .iter()
            .any(|t| matches!(t, Tok::Free(w) if w == "arriere"));
    let worded = [
        Class::Media,
        Class::Listen,
        Class::Sound,
        Class::Pause,
        Class::Next,
        Class::Previous,
        Class::Stop,
    ]
    .into_iter()
    .any(has)
        || back;
    let blockers = [
        Class::Light,
        Class::LightNamed,
        Class::Cover,
        Class::Heating,
        Class::Temperature,
        Class::Degree,
        Class::Tv,
        Class::Plug,
        Class::OtherDevice,
        Class::Negation,
    ];
    worded && !blockers.into_iter().any(has)
}

/// Music and volume: only with a music word ("musique", "le son", "pause", "suivante" …) and
/// never with a light, shutter, heating, TV, plug or other device word or a negation, so every
/// house sentence parses as before. `None` hands the sentence back to the house intents.
fn media(toks: &[Tok]) -> Option<Intent> {
    let has = |c: Class| {
        toks.iter()
            .any(|t| matches!(t, Tok::Known(k, _) if *k == c))
    };
    let back = has(Class::Back)
        && toks
            .iter()
            .any(|t| matches!(t, Tok::Free(w) if w == "arriere"));
    let music = has(Class::Media) || has(Class::Listen);
    let sound = has(Class::Sound);
    let worded = music
        || sound
        || has(Class::Pause)
        || has(Class::Next)
        || has(Class::Previous)
        || has(Class::Stop)
        || back;
    let blockers = [
        Class::Light,
        Class::LightNamed,
        Class::Cover,
        Class::Heating,
        Class::Temperature,
        Class::Degree,
        Class::Tv,
        Class::Plug,
        Class::OtherDevice,
        Class::Negation,
    ];
    if !worded || blockers.into_iter().any(has) {
        return None;
    }
    let verbs = [
        Class::On,
        Class::Off,
        Class::Set,
        Class::Raise,
        Class::Lower,
        Class::Resume,
        Class::Launch,
        Class::Stop,
        Class::Pause,
    ];
    let verb = verbs.into_iter().any(has);
    if music && (has(Class::Question) || has(Class::Statement)) && !verb {
        return Some(Intent::Media(MediaIntent::NowPlaying));
    }
    // "j'ai mis pause" describes; only a polite request is an order.
    if has(Class::Statement) && !has(Class::Request) {
        return None;
    }
    let (next, previous) = (has(Class::Next), has(Class::Previous) || back);
    if next || previous {
        let intent = if next {
            MediaIntent::Next
        } else {
            MediaIntent::Previous
        };
        return (next != previous).then_some(Intent::Media(intent));
    }
    let off = has(Class::Off);
    if has(Class::Pause) || ((has(Class::Stop) || off) && has(Class::Media)) {
        return Some(Intent::Media(MediaIntent::Pause));
    }
    let roles = roles(toks);
    let number_with = |role: Role| {
        toks.iter().zip(&roles).find_map(|(t, r)| match (t, r) {
            (Tok::Number(n), Some(r)) if *r == role => Some(*n),
            _ => None,
        })
    };
    let room: Vec<String> = toks
        .iter()
        .zip(&roles)
        .filter_map(|(t, r)| match (t, r) {
            (Tok::Free(w), _) => Some(w.clone()),
            (Tok::Number(n), Some(Role::Name)) => Some(number_text(*n)),
            _ => None,
        })
        .collect();
    let (raise, lower) = (has(Class::Raise), has(Class::Lower));
    let absolute = |n: f64| {
        Some(if (0.0..=100.0).contains(&n) {
            Intent::Volume {
                change: Change::Absolute(n),
                room: room.clone(),
            }
        } else {
            Intent::VolumeRange
        })
    };
    if (raise || lower) && (sound || has(Class::Media)) {
        if raise && lower {
            return None;
        }
        if let Some(n) = number_with(Role::AfterA) {
            return absolute(n);
        }
        let signed = toks.iter().find_map(|t| match t {
            Tok::Signed(n) => Some(n.abs()),
            _ => None,
        });
        let step = if has(Class::Little) {
            VOLUME_LITTLE
        } else {
            signed
                .or(number_with(Role::Amount))
                .map_or(VOLUME_STEP, f64::abs)
        };
        return Some(Intent::Volume {
            change: Change::Relative(if lower { -step } else { step }),
            room,
        });
    }
    if sound {
        if has(Class::Set) {
            return number_with(Role::AfterA).and_then(absolute);
        }
        if off {
            return Some(Intent::Mute { on: true, room });
        }
        if has(Class::Resume) || has(Class::On) {
            return Some(Intent::Mute { on: false, room });
        }
        return None;
    }
    if has(Class::Media) {
        if has(Class::Resume) {
            return Some(Intent::Media(MediaIntent::Play { resume: true }));
        }
        if has(Class::Set) || has(Class::Launch) || has(Class::On) {
            // Words before a room linker name the music; "dans le salon" names a room.
            let mut in_room = false;
            let named = toks.iter().any(|t| match t {
                Tok::Known(Class::Linker, w) if ROOM_LINKERS.contains(&w.as_str()) => {
                    in_room = true;
                    false
                }
                Tok::Free(_) | Tok::Number(_) => !in_room,
                _ => false,
            });
            return Some(if named {
                Intent::MediaHint
            } else {
                Intent::Media(MediaIntent::Play { resume: false })
            });
        }
    }
    None
}

/// The forms of "jouer" that start a search by name, and how speech recognition writes "joue"
/// when an English title tips it into English ("Play Lost on You", "Ju Shape of You").
const PLAY_WORDS: &[&str] = &["joue", "joues", "jouez", "jouer", "play", "ju"];

/// Hesitations speech recognition writes down; never the start of a name.
const FILLERS: &[&str] = &["hum", "hmm", "mm", "euh", "heu"];

/// Words after "joue" that ask for some music rather than a name.
const SOME_MUSIC: &[&str] = &[
    "musique",
    "musiques",
    "quelque chose",
    "morceau",
    "chanson",
    "truc",
];

/// "joue …" anywhere in the raw sentence: the words after it, as typed (case and accents kept),
/// are a search; none, or just "de la musique", plays something. A question ("qu'est-ce qui
/// joue ?") asks what is playing; "ne joue pas" is `None`. Read on the raw sentence, before
/// [`parse`]: "joue" means music whatever follows.
pub fn play_by_name(sentence: &str) -> Option<Intent> {
    let raw: Vec<&str> = sentence.split_whitespace().collect();
    let clean = |w: &str| fold(w.trim_matches(|c: char| !c.is_alphanumeric()));
    // Lower case but not folded: "joué" ("on a joué au foot") is not "joue".
    let at = raw.iter().position(|w| {
        let w = w
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        PLAY_WORDS.contains(&w.as_str())
    })?;
    // Every word before it, split at hyphens and apostrophes too ("peux-tu", "qu'est-ce").
    let before: Vec<String> = raw[..at]
        .iter()
        .flat_map(|w| w.split(['-', '\'', '’']))
        .map(clean)
        .filter(|w| !w.is_empty())
        .collect();
    let mut rest: Vec<&str> = raw[at + 1..].to_vec();
    if rest.first().is_some_and(|w| clean(w) == "pas") {
        return None;
    }
    if before.iter().any(|w| w == "qui" || w == "quoi") {
        return Some(Intent::Media(MediaIntent::NowPlaying));
    }
    // "joue" orders only at the start, after a polite "peux-tu" at most: "baisse le son, ça
    // joue trop fort" or "ne joue pas" is not a search.
    let polite = |w: &String| {
        class(w) == Some(Class::Request) || matches!(w.as_str(), "tu" | "vous" | "me" | "m" | "moi")
    };
    if !before.iter().all(polite) {
        return None;
    }
    if sentence.trim_end().ends_with('?') && rest.len() <= 1 {
        return Some(Intent::Media(MediaIntent::NowPlaying));
    }
    while rest
        .first()
        .is_some_and(|w| FILLERS.contains(&clean(w).as_str()))
    {
        rest.remove(0);
    }
    // Politeness at the end is not part of a name, but a lone "merci" is one ("Mercy").
    loop {
        match rest.last().map(|w| clean(w)).as_deref() {
            Some("merci") if rest.len() == 1 => break,
            Some("stp" | "svp" | "merci" | "") => {
                rest.pop();
            }
            Some("plait") => {
                rest.pop();
                if rest
                    .last()
                    .is_some_and(|w| matches!(clean(w).as_str(), "te" | "vous"))
                {
                    rest.pop();
                }
                if rest
                    .last()
                    .is_some_and(|w| clean(w) == "s'il" || clean(w) == "sil")
                {
                    rest.pop();
                }
            }
            _ => break,
        }
    }
    let query = rest
        .join(" ")
        .trim_end_matches(|c: char| c.is_whitespace() || "?!.,;".contains(c))
        .to_owned();
    let folded: Vec<String> = query.split_whitespace().map(clean).collect();
    let skip = match folded.as_slice() {
        [de, la, ..] if de == "de" && la == "la" => 2,
        [article, ..]
            if matches!(
                article.as_str(),
                "du" | "des" | "le" | "la" | "les" | "un" | "une"
            ) =>
        {
            1
        }
        _ => 0,
    };
    let what = folded.get(skip..).unwrap_or_default().join(" ");
    if what.is_empty() || SOME_MUSIC.contains(&what.as_str()) {
        return Some(Intent::Media(MediaIntent::Play { resume: false }));
    }
    Some(Intent::Media(MediaIntent::PlayMedia { query }))
}

/// [`play_by_name`] in `lang`.
pub fn play_by_name_in(sentence: &str, lang: Lang) -> Option<Intent> {
    match lang {
        Lang::Fr => play_by_name(sentence),
        Lang::En => play_by_name_en(sentence),
    }
}

/// Hesitations written down in English; never the start of a name.
const FILLERS_EN: &[&str] = &["um", "uh", "erm", "er", "hmm", "hum", "mm"];

/// Words before "play" that keep it an order: "can you play", "please play".
const POLITE_EN: &[&str] = &[
    "can", "could", "would", "will", "you", "please", "just", "hey", "ok", "okay", "now", "me",
    "for", "us",
];

/// Words after "play" that ask for some music rather than a name.
const SOME_MUSIC_EN: &[&str] = &[
    "music",
    "something",
    "anything",
    "song",
    "songs",
    "a song",
    "tunes",
    "some tunes",
    "some music",
];

/// [`play_by_name`] for English: "play …" at the start, after "can you" or "please" at most.
/// "what are you playing ?" is not here: "playing" is a music word the parser reads.
fn play_by_name_en(sentence: &str) -> Option<Intent> {
    let raw: Vec<&str> = sentence.split_whitespace().collect();
    let clean = |w: &str| fold(w.trim_matches(|c: char| !c.is_alphanumeric()));
    let at = raw.iter().position(|w| clean(w) == "play")?;
    let before: Vec<String> = raw[..at]
        .iter()
        .flat_map(|w| w.split(['-', '\'', '’']))
        .map(clean)
        .filter(|w| !w.is_empty())
        .collect();
    if before.iter().any(|w| w == "who" || w == "what") {
        return Some(Intent::Media(MediaIntent::NowPlaying));
    }
    if !before.iter().all(|w| POLITE_EN.contains(&w.as_str())) {
        return None;
    }
    let mut rest: Vec<&str> = raw[at + 1..].to_vec();
    if sentence.trim_end().ends_with('?') && rest.len() <= 1 && before.is_empty() {
        return Some(Intent::Media(MediaIntent::NowPlaying));
    }
    while rest
        .first()
        .is_some_and(|w| FILLERS_EN.contains(&clean(w).as_str()))
    {
        rest.remove(0);
    }
    // Politeness at the end is not part of a name, but a lone word is one ("Please").
    while rest.len() > 1 {
        match rest.last().map(|w| clean(w)).as_deref() {
            Some("please" | "thanks" | "") => {
                rest.pop();
            }
            Some("you")
                if rest
                    .get(rest.len().wrapping_sub(2))
                    .is_some_and(|w| clean(w) == "thank") =>
            {
                rest.truncate(rest.len() - 2);
            }
            _ => break,
        }
    }
    let query = rest
        .join(" ")
        .trim_end_matches(|c: char| c.is_whitespace() || "?!.,;".contains(c))
        .to_owned();
    let folded: Vec<String> = query.split_whitespace().map(clean).collect();
    let skip = usize::from(
        folded
            .first()
            .is_some_and(|a| matches!(a.as_str(), "the" | "a" | "an" | "some")),
    );
    let what = folded.get(skip..).unwrap_or_default().join(" ");
    if what.is_empty() || SOME_MUSIC_EN.contains(&what.as_str()) {
        return Some(Intent::Media(MediaIntent::Play { resume: false }));
    }
    Some(Intent::Media(MediaIntent::PlayMedia { query }))
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

/// TVs or plugs: one on verb or one off verb, never both or neither, and never both a TV and a
/// plug word. Any value (a number after "à" or "de", a signed number), "mets" or a raise or
/// lower verb is not understood: switching while ignoring a value would be a guess. The device
/// words are not names; names and room words split like lights.
fn switches(toks: &[Tok], roles: &[Option<Role>], has: &dyn Fn(Class) -> bool) -> Option<Intent> {
    let (tv, plug) = (has(Class::Tv), has(Class::Plug));
    let (on, off) = (has(Class::On), has(Class::Off));
    if tv == plug
        || on == off
        || has(Class::Set)
        || has(Class::Raise)
        || has(Class::Lower)
        || toks.iter().any(|t| matches!(t, Tok::Signed(_)))
        || roles
            .iter()
            .any(|r| matches!(r, Some(Role::AfterA | Role::Amount)))
    {
        return None;
    }
    let (names, room) = split_names(toks, roles);
    Some(Intent::Switches {
        what: if tv { SwitchType::Tv } else { SwitchType::Plug },
        on,
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
        colour: None,
        brightness: None,
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
mod tests;

#[cfg(test)]
mod english;
