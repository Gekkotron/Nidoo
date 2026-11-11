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
    /// "en rouge", only ever with `on`.
    Lights {
        on: bool,
        all: bool,
        names: Vec<String>,
        room: Vec<String>,
        colour: Option<Colour>,
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
    // "turn the lamp red": with a colour and no other verb, "turn" or "switch" sets it.
    let verb = |t: &Tok| matches!(t, Tok::Known(Class::On | Class::Off | Class::Set, _));
    if lang == Lang::En && colour.is_some() && !toks.iter().any(verb) {
        for t in &mut toks {
            if matches!(t, Tok::Known(Class::Noise, w) if w == "turn" || w == "switch") {
                *t = Tok::Known(Class::Set, "turn".into());
            }
        }
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
            },
            Some(c),
        ) => Some(Intent::Lights {
            on: true,
            all,
            names,
            room,
            colour: Some(c),
        }),
        // A colour for heating, shutters, music or an off order would be ignored: not a guess.
        _ => None,
    }
}

/// [`parse`] without the colour words; `coloured`: a colour was said, so "mets" alone switches
/// lights on ("mets le salon en bleu").
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
    use crate::lexicon::colour_of;
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
            colour: None,
        })
    }

    fn coloured(names: &[&str], room: &[&str], colour: &str) -> Option<Intent> {
        Some(Intent::Lights {
            on: true,
            all: false,
            names: v(names),
            room: v(room),
            colour: colour_of(colour),
        })
    }

    #[test]
    fn colour_after_en() {
        assert_eq!(
            p("allume la lumière du salon en rouge"),
            coloured(&[], &["salon"], "rouge")
        );
        assert_eq!(
            p("allume la led en bleu dans le salon"),
            coloured(&["led"], &["salon"], "bleu")
        );
        assert_eq!(
            p("mets la lampe de la chambre en vert"),
            coloured(&["lampe"], &["chambre"], "vert")
        );
        assert_eq!(
            p("mets le salon en rose"),
            coloured(&[], &["salon"], "rose")
        );
        assert_eq!(
            p("passe les lumières du salon en blanc"),
            coloured(&[], &["salon"], "blanc")
        );
        assert_eq!(p("Allume en Violette"), coloured(&[], &[], "violet"));
    }

    #[test]
    fn colour_never_guessed() {
        // Off, heating, shutters, music or two colours: not understood.
        assert_eq!(p("éteins le salon en rouge"), None);
        assert_eq!(p("mets le chauffage en rouge"), None);
        assert_eq!(p("ouvre les volets en bleu"), None);
        assert_eq!(p("mets la musique en bleu"), None);
        assert_eq!(p("allume le salon en rouge et en bleu"), None);
        assert_eq!(p("ne mets pas le salon en rouge"), None);
        assert_eq!(p("mets le salon en rouge à 20"), None);
        // The same colour twice is one colour.
        assert_eq!(
            p("allume le salon en rouge, en rouge"),
            coloured(&[], &["salon"], "rouge")
        );
        // Without "en" a colour word is part of a name, as before.
        assert_eq!(
            p("allume la lampe rose"),
            lights(true, false, &["lampe"], &["rose"])
        );
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

    fn switches(
        what: SwitchType,
        on: bool,
        all: bool,
        names: &[&str],
        room: &[&str],
    ) -> Option<Intent> {
        Some(Intent::Switches {
            what,
            on,
            all,
            names: v(names),
            room: v(room),
        })
    }

    #[test]
    fn tv_on_off() {
        use SwitchType::Tv;
        assert_eq!(p("allume la télé"), switches(Tv, true, false, &[], &[]));
        assert_eq!(
            p("éteins la télé du salon"),
            switches(Tv, false, false, &[], &["salon"])
        );
        assert_eq!(p("éteins les télés"), switches(Tv, false, false, &[], &[]));
        assert_eq!(
            p("peux-tu allumer la télévision du salon"),
            switches(Tv, true, false, &[], &["salon"])
        );
        assert_eq!(
            p("coupe la tv du salon"),
            switches(Tv, false, false, &[], &["salon"])
        );
    }

    #[test]
    fn plug_on_off_names_and_all() {
        use SwitchType::Plug;
        assert_eq!(
            p("éteins la prise du bureau"),
            switches(Plug, false, false, &[], &["bureau"])
        );
        assert_eq!(
            p("allume la prise bureau"),
            switches(Plug, true, false, &[], &["bureau"])
        );
        assert_eq!(
            p("éteins toutes les prises"),
            switches(Plug, false, true, &[], &[])
        );
        assert_eq!(
            p("allume les 2 prises du bureau"),
            switches(Plug, true, false, &[], &["bureau"])
        );
        assert_eq!(
            p("allume la prise cafetière du bureau"),
            switches(Plug, true, false, &["cafetiere"], &["bureau"])
        );
    }

    #[test]
    fn switches_never_guessed() {
        assert_eq!(p("allume la télé et la prise"), None);
        assert_eq!(p("allume et éteins la prise du bureau"), None);
        assert_eq!(p("la prise du bureau"), None);
        assert_eq!(p("ouvre la prise du bureau"), None);
        assert_eq!(p("monte la télé"), None);
        assert_eq!(p("mets la télé à 20"), None);
        assert_eq!(p("allume la prise du bureau à 50 %"), None);
        assert_eq!(p("allume la prise à 3"), None);
        assert_eq!(p("allume la prise de 2"), None);
        assert_eq!(p("n'éteins pas la télé"), None);
        assert_eq!(p("la télé est allumée"), None);
        assert_eq!(p("la télé du salon allumée ?"), None);
        assert_eq!(p("qui a éteint la prise du bureau"), None);
        assert_eq!(p("éteins le rideau de la télé"), None);
    }

    #[test]
    fn a_light_cover_or_heating_word_keeps_tv_and_plug_as_names() {
        assert_eq!(
            p("allume la lumière du salon tv"),
            lights(true, false, &[], &["salon", "tv"])
        );
        assert_eq!(
            p("ferme le volet tv"),
            covers(false, None, false, &[], &["tv"])
        );
        assert_eq!(
            p("allume la lampe tv du salon"),
            lights(true, false, &["lampe", "tv"], &["salon"])
        );
        assert_eq!(
            p("allume la lumière de la prise"),
            lights(true, false, &[], &["prise"])
        );
        assert_eq!(
            p("allume le chauffage du salon tv"),
            Some(Intent::HeatingNoValue {
                room: v(&["salon", "tv"])
            })
        );
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

    fn media(m: MediaIntent) -> Option<Intent> {
        Some(Intent::Media(m))
    }

    fn volume(change: Change, room: &[&str]) -> Option<Intent> {
        Some(Intent::Volume {
            change,
            room: v(room),
        })
    }

    #[test]
    fn music_sentences() {
        assert_eq!(p("mets pause"), media(MediaIntent::Pause));
        assert_eq!(p("pause"), media(MediaIntent::Pause));
        assert_eq!(p("pause la musique"), media(MediaIntent::Pause));
        assert_eq!(p("stop la musique"), media(MediaIntent::Pause));
        assert_eq!(p("arrête la musique"), media(MediaIntent::Pause));
        assert_eq!(
            p("reprends la musique"),
            media(MediaIntent::Play { resume: true })
        );
        assert_eq!(
            p("relance la musique"),
            media(MediaIntent::Play { resume: true })
        );
        assert_eq!(
            p("mets de la musique"),
            media(MediaIntent::Play { resume: false })
        );
        assert_eq!(
            p("lance de la musique"),
            media(MediaIntent::Play { resume: false })
        );
        assert_eq!(
            p("peux-tu mettre de la musique"),
            media(MediaIntent::Play { resume: false })
        );
        assert_eq!(p("mets de la musique classique"), Some(Intent::MediaHint));
        assert_eq!(p("chanson suivante"), media(MediaIntent::Next));
        assert_eq!(p("musique suivante"), media(MediaIntent::Next));
        assert_eq!(p("passe à la suivante"), media(MediaIntent::Next));
        assert_eq!(p("chanson précédente"), media(MediaIntent::Previous));
        assert_eq!(p("reviens en arrière"), media(MediaIntent::Previous));
        assert_eq!(p("chanson suivante et précédente"), None);
        assert_eq!(
            p("c'est quoi cette chanson ?"),
            media(MediaIntent::NowPlaying)
        );
        assert_eq!(
            p("qu'est-ce qu'on écoute ?"),
            media(MediaIntent::NowPlaying)
        );
        assert_eq!(p("quel est le titre ?"), media(MediaIntent::NowPlaying));
        assert_eq!(p("j'ai mis pause"), None);
        assert_eq!(p("ne coupe pas le son"), None);
        assert_eq!(p("ne mets pas pause"), None);
    }

    #[test]
    fn volume_sentences() {
        assert_eq!(p("monte le son"), volume(Change::Relative(10.0), &[]));
        assert_eq!(p("baisse le son"), volume(Change::Relative(-10.0), &[]));
        assert_eq!(
            p("baisse un peu le son"),
            volume(Change::Relative(-5.0), &[])
        );
        assert_eq!(p("monte le son de 20"), volume(Change::Relative(20.0), &[]));
        assert_eq!(p("monte la musique"), volume(Change::Relative(10.0), &[]));
        assert_eq!(p("mets le son à 60 %"), volume(Change::Absolute(60.0), &[]));
        assert_eq!(
            p("mets le volume à 60"),
            volume(Change::Absolute(60.0), &[])
        );
        assert_eq!(p("mets le son à 140"), Some(Intent::VolumeRange));
        assert_eq!(
            p("baisse le son du salon"),
            volume(Change::Relative(-10.0), &["salon"])
        );
        assert_eq!(
            p("coupe le son"),
            Some(Intent::Mute {
                on: true,
                room: vec![]
            })
        );
        assert_eq!(
            p("remets le son dans le salon"),
            Some(Intent::Mute {
                on: false,
                room: v(&["salon"])
            })
        );
    }

    /// Sentences that meant something before the music words keep meaning it.
    #[test]
    fn house_sentences_are_unchanged() {
        assert_eq!(
            p("mets le chauffage à 20"),
            set(Change::Absolute(20.0), &[])
        );
        assert_eq!(
            p("passe le chauffage à 20"),
            set(Change::Absolute(20.0), &[])
        );
        assert_eq!(p("coupe la lumière"), lights(false, false, &[], &[]));
        assert!(matches!(
            p("monte le volet"),
            Some(Intent::Covers { open: true, .. })
        ));
        assert_eq!(p("passe"), None);
        assert_eq!(
            p("mets son bureau à 20"),
            set(Change::Absolute(20.0), &["son", "bureau"])
        );
        assert_eq!(
            p("allume son bureau"),
            lights(true, false, &[], &["son", "bureau"])
        );
    }

    fn m(s: &str) -> Option<Intent> {
        play_by_name(s)
    }

    fn query(q: &str) -> Option<Intent> {
        media(MediaIntent::PlayMedia { query: q.into() })
    }

    #[test]
    fn play_by_name_reads_the_words_as_typed() {
        assert_eq!(m("joue du Daft Punk"), query("du Daft Punk"));
        assert_eq!(m("Joue Beyoncé !"), query("Beyoncé"));
        assert_eq!(m("peux-tu jouer la vie en rose"), query("la vie en rose"));
        assert_eq!(m("joue 99 Luftballons"), query("99 Luftballons"));
        assert_eq!(m("joue Daft Punk s'il te plaît"), query("Daft Punk"));
        assert_eq!(
            m("joue de la musique"),
            media(MediaIntent::Play { resume: false })
        );
        assert_eq!(
            m("joue une chanson"),
            media(MediaIntent::Play { resume: false })
        );
        assert_eq!(m("joue"), media(MediaIntent::Play { resume: false }));
        assert_eq!(m("qu'est-ce qui joue ?"), media(MediaIntent::NowPlaying));
        assert_eq!(m("ne joue pas Daft Punk"), None);
        assert_eq!(m("mets Daft Punk"), None);
        assert_eq!(m("allume la lumière"), None);
    }

    #[test]
    fn joue_is_an_order_only_at_the_start() {
        assert_eq!(m("on a joué au foot"), None);
        assert_eq!(m("baisse le son, ça joue trop fort"), None);
        assert_eq!(m("ça joue trop fort"), None);
        assert_eq!(m("mets pause, la musique joue trop fort"), None);
        assert_eq!(m("peux-tu me jouer Daft Punk"), query("Daft Punk"));
        assert_eq!(m("tu peux jouer Daft Punk"), query("Daft Punk"));
    }

    /// An English title can tip speech recognition into English: "joue" comes out as "Play" or
    /// "Ju", and a filler like "hum" lands in front of the title.
    #[test]
    fn english_titles_survive_the_transcription() {
        assert_eq!(m("Play Lost on You."), query("Lost on You"));
        assert_eq!(m("Ju Shape of You."), query("Shape of You"));
        assert_eq!(
            m("Joue Hum Merci de Cho Mendes."),
            query("Merci de Cho Mendes")
        );
        assert_eq!(m("joue euh Daft Punk"), query("Daft Punk"));
        assert_eq!(m("play"), media(MediaIntent::Play { resume: false }));
        assert_eq!(m("baisse le son, play"), None);
    }

    /// "merci" is politeness after a name, but a title on its own ("Mercy" heard as "merci").
    #[test]
    fn a_lone_merci_is_the_title() {
        assert_eq!(m("Joue merci."), query("merci"));
        assert_eq!(m("joue Daft Punk merci"), query("Daft Punk"));
        assert_eq!(m("joue Daft Punk, merci"), query("Daft Punk"));
    }

    #[test]
    fn on_verbs_and_rooms_with_music() {
        assert_eq!(
            p("allume la musique"),
            media(MediaIntent::Play { resume: false })
        );
        assert_eq!(
            p("allume le son"),
            Some(Intent::Mute {
                on: false,
                room: vec![]
            })
        );
        assert_eq!(
            p("mets de la musique dans le salon"),
            media(MediaIntent::Play { resume: false })
        );
        // A music word the music intents do not use is not understood, never lights.
        assert_eq!(p("chanson"), None);
        assert_eq!(
            p("allume la chanson du salon"),
            media(MediaIntent::Play { resume: false })
        );
    }

    /// With a house word the music words are plain words again: the old parse, unchanged.
    #[test]
    fn house_words_keep_music_words_as_names() {
        assert_eq!(
            p("allume la lumière de son bureau"),
            lights(true, false, &[], &["son", "bureau"])
        );
        assert_eq!(
            p("allume la lumière suivante"),
            lights(true, false, &[], &["suivante"])
        );
        assert_eq!(
            p("remets le chauffage à 20"),
            set(Change::Absolute(20.0), &["remets"])
        );
    }
}

#[cfg(test)]
mod english {
    use super::*;
    use crate::text::words;

    fn p(s: &str) -> Option<Intent> {
        parse_in(&words(s), Lang::En)
    }

    fn m(s: &str) -> Option<Intent> {
        play_by_name_in(s, Lang::En).or_else(|| p(s))
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
            colour: None,
        })
    }

    fn coloured(names: &[&str], room: &[&str], colour: Colour) -> Option<Intent> {
        Some(Intent::Lights {
            on: true,
            all: false,
            names: v(names),
            room: v(room),
            colour: Some(colour),
        })
    }

    fn set(change: Change, room: &[&str]) -> Option<Intent> {
        Some(Intent::SetTemp {
            change,
            room: v(room),
        })
    }

    fn media(m: MediaIntent) -> Option<Intent> {
        Some(Intent::Media(m))
    }

    #[test]
    fn lights_on_and_off() {
        let kitchen = lights(true, false, &[], &["kitchen"]);
        assert_eq!(p("turn on the kitchen lights"), kitchen);
        assert_eq!(p("Turn on the lights in the kitchen"), kitchen);
        assert_eq!(p("turn the kitchen lights on please"), kitchen);
        assert_eq!(p("switch on the kitchen light"), kitchen);
        assert_eq!(p("can you turn on the kitchen?"), kitchen);
        assert_eq!(
            p("turn off the lights in the living room"),
            lights(false, false, &[], &["living"])
        );
        assert_eq!(
            p("turn off the lamp in Tom's bedroom"),
            lights(false, false, &["lamp"], &["tom", "bedroom"])
        );
        assert_eq!(
            p("turn off the desk lamp"),
            lights(false, false, &["lamp"], &["desk"])
        );
        assert_eq!(p("turn off all the lights"), lights(false, true, &[], &[]));
        assert_eq!(p("turn everything off"), lights(false, true, &[], &[]));
        assert_eq!(p("turn the lights back on"), lights(true, false, &[], &[]));
    }

    #[test]
    fn light_colours() {
        assert_eq!(
            p("turn on the kitchen lights in red"),
            coloured(&[], &["kitchen"], Colour::Red)
        );
        assert_eq!(
            p("turn the lamp in the office blue"),
            coloured(&["lamp"], &["office"], Colour::Blue)
        );
        assert_eq!(
            p("set the living room to green please"),
            coloured(&[], &["living"], Colour::Green)
        );
        assert_eq!(
            p("make the bedroom lights pink"),
            coloured(&[], &["bedroom"], Colour::Pink)
        );
        // Before the light word, a colour is a plain word, as in French ("la lampe rose").
        assert_eq!(
            p("turn on the red lamp"),
            lights(true, false, &["lamp"], &["red"])
        );
        assert_eq!(p("turn off the kitchen in red"), None);
        assert_eq!(p("set the heating to red"), None);
    }

    #[test]
    fn setpoints() {
        assert_eq!(
            p("set the heating to 20 in the kitchen"),
            set(Change::Absolute(20.0), &["kitchen"])
        );
        assert_eq!(
            p("set the bedroom to nineteen point five degrees"),
            set(Change::Absolute(19.5), &["bedroom"])
        );
        assert_eq!(
            p("turn up the heat in the office"),
            set(Change::Relative(STEP), &["office"])
        );
        assert_eq!(
            p("turn the heating down a bit in the kitchen"),
            set(Change::Relative(-LITTLE), &["kitchen"])
        );
        assert_eq!(
            p("raise the temperature by 2 degrees in the office"),
            set(Change::Relative(2.0), &["office"])
        );
        assert_eq!(
            p("turn on the heating in the kitchen"),
            Some(Intent::HeatingNoValue {
                room: v(&["kitchen"])
            })
        );
        assert_eq!(p("turn up the office"), None);
    }

    #[test]
    fn temperature_questions() {
        let read = |room: &[&str], outdoor| {
            Some(Intent::ReadTemp {
                room: v(room),
                outdoor,
            })
        };
        assert_eq!(
            p("what's the temperature in the kitchen?"),
            read(&["kitchen"], false)
        );
        assert_eq!(p("how warm is it in the office"), read(&["office"], false));
        assert_eq!(p("how cold is it outside?"), read(&[], true));
        assert_eq!(p("what is the temperature"), read(&[], false));
        // A statement is not a question nor an order.
        assert_eq!(p("it is cold in the kitchen"), None);
        assert_eq!(p("the kitchen is 18 degrees"), None);
    }

    #[test]
    fn covers_and_switches() {
        assert_eq!(
            p("open the shutters in the living room"),
            Some(Intent::Covers {
                open: true,
                position: None,
                all: false,
                names: vec![],
                room: v(&["living"]),
            })
        );
        assert_eq!(
            p("open the blinds to 30%"),
            Some(Intent::Covers {
                open: true,
                position: Some(30),
                all: false,
                names: vec![],
                room: vec![],
            })
        );
        assert_eq!(
            p("close all the shutters"),
            Some(Intent::Covers {
                open: false,
                position: None,
                all: true,
                names: vec![],
                room: vec![],
            })
        );
        assert_eq!(
            p("close the blinds to 30 percent"),
            Some(Intent::CoverPositionHint)
        );
        assert_eq!(p("are the shutters closed?"), None);
        assert_eq!(p("close the kitchen lights"), None);
        assert_eq!(
            p("turn off the tv in the living room"),
            Some(Intent::Switches {
                what: SwitchType::Tv,
                on: false,
                all: false,
                names: vec![],
                room: v(&["living"]),
            })
        );
        assert_eq!(
            p("switch on the coffee plug"),
            Some(Intent::Switches {
                what: SwitchType::Plug,
                on: true,
                all: false,
                names: vec![],
                room: v(&["coffee"]),
            })
        );
    }

    #[test]
    fn never_guessed() {
        assert_eq!(p("don't turn on the kitchen"), None);
        assert_eq!(p("do not turn off the lights"), None);
        assert_eq!(p("turn on and off the lights"), None);
        assert_eq!(p("who turned off the lights"), None);
        assert_eq!(p("open the curtains"), None);
        assert_eq!(p("hello there"), None);
    }

    #[test]
    fn music_and_volume() {
        assert_eq!(m("pause the music"), media(MediaIntent::Pause));
        assert_eq!(m("pause"), media(MediaIntent::Pause));
        assert_eq!(m("next song"), media(MediaIntent::Next));
        assert_eq!(m("skip this track"), media(MediaIntent::Next));
        assert_eq!(m("previous song"), media(MediaIntent::Previous));
        assert_eq!(m("go back"), media(MediaIntent::Previous));
        assert_eq!(
            m("resume the music"),
            media(MediaIntent::Play { resume: true })
        );
        assert_eq!(m("stop the music"), media(MediaIntent::Pause));
        assert_eq!(m("what's playing?"), media(MediaIntent::NowPlaying));
        assert_eq!(m("what song is this?"), media(MediaIntent::NowPlaying));
        assert_eq!(
            m("turn the volume up"),
            Some(Intent::Volume {
                change: Change::Relative(VOLUME_STEP),
                room: vec![],
            })
        );
        assert_eq!(
            m("set the volume to 30 in the kitchen"),
            Some(Intent::Volume {
                change: Change::Absolute(30.0),
                room: v(&["kitchen"]),
            })
        );
        assert_eq!(
            m("turn the sound down a little"),
            Some(Intent::Volume {
                change: Change::Relative(-VOLUME_LITTLE),
                room: vec![],
            })
        );
        assert_eq!(m("set the volume to 150"), Some(Intent::VolumeRange));
        assert_eq!(
            m("mute the kitchen"),
            Some(Intent::Mute {
                on: true,
                room: v(&["kitchen"]),
            })
        );
        assert_eq!(
            m("unmute"),
            Some(Intent::Mute {
                on: false,
                room: vec![],
            })
        );
    }

    #[test]
    fn play_by_name() {
        let query = |q: &str| media(MediaIntent::PlayMedia { query: q.into() });
        assert_eq!(m("play Shape of You"), query("Shape of You"));
        assert_eq!(m("can you play Lost on You please"), query("Lost on You"));
        assert_eq!(
            m("Play some music"),
            media(MediaIntent::Play { resume: false })
        );
        assert_eq!(m("play"), media(MediaIntent::Play { resume: false }));
        assert_eq!(m("play music"), media(MediaIntent::Play { resume: false }));
        assert_eq!(m("play Please"), query("Please"));
        assert_eq!(m("don't play anything"), None);
        assert_eq!(m("the kids play upstairs"), None);
    }
}
