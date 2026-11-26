//! From a sentence and the house to a plan: act, answer, ask or confirm.

use nidoo_api::EntityView;
use nidoo_api::search::fold;
use nidoo_api::{Category, CategoryAction, Target, category_members, commandable};
use nidoo_climate::doc::{Setpoint, ZoneState};
use nidoo_climate::view::ZoneView;
use nidoo_core::flow::Lang;
use nidoo_core::model::{Availability, Command, EntityCategory, Kind, StateValue, SwitchType};

use crate::house::House;
use crate::intent::{Change, Intent, LITTLE, MediaIntent, VacuumIntent, parse_in, play_by_name_in};
use crate::lexicon::{Class, Colour, class, room_noun};
use crate::reply::{
    Lack, cannot_in, dans, de, lack, le, number, number_in, place, room_in, temp, temp_in, the,
};
use crate::room::{RoomMatch, name_words, resolve_room, score};
use crate::text::{Word, words};
use crate::view::Trace;

/// The most choices a question offers.
pub const MAX_CHOICES: usize = 6;

/// An understood intent with what is already resolved: the area, lights and shutters picked
/// from a question, and whether the safety question was answered yes.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub intent: Intent,
    pub area: Option<String>,
    pub lights: Option<Vec<String>>,
    pub covers: Option<Vec<String>>,
    pub switches: Option<Vec<String>>,
    /// The media player chosen in a question.
    pub player: Option<String>,
    /// The vacuum and room button chosen in questions; boxed to keep a pending question small.
    pub vacuum: Option<Box<VacuumPick>>,
    /// The room of the satellite that heard the sentence; `None` from the web.
    pub speaker_room: Option<String>,
    pub confirmed: bool,
}

/// What the questions of the vacuum have settled.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VacuumPick {
    /// The vacuum chosen.
    pub vacuum: Option<String>,
    /// The vacuum's room button chosen.
    pub room: Option<String>,
}

impl Request {
    pub fn new(intent: Intent) -> Self {
        Self {
            intent,
            area: None,
            lights: None,
            covers: None,
            switches: None,
            player: None,
            vacuum: None,
            speaker_room: None,
            confirmed: false,
        }
    }
}

/// One thing to do to the house.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Light {
        entity: String,
        command: Command,
    },
    /// A zone-wide setpoint, in °C.
    Setpoint {
        zone: String,
        setpoint: f64,
    },
    /// A shutter: open, close or a position.
    Cover {
        entity: String,
        command: Command,
    },
    /// A TV or plug: turn on or off.
    Switch {
        entity: String,
        command: Command,
    },
    /// A media player command.
    Media {
        entity: String,
        command: Command,
    },
    /// A speaker's music volume (`SetNumber`) or mute switch (`TurnOn` / `TurnOff`).
    Speaker {
        entity: String,
        command: Command,
    },
    /// The vacuum: a `VacuumAction`, or the `Press` of one of its room buttons.
    Vacuum {
        entity: String,
        command: Command,
    },
}

/// What the actions do, for the reply. `names` are in the actions' order.
#[derive(Clone, Debug, PartialEq)]
pub enum Done {
    /// `colour`: the colour's name in the house's language ("rouge", "red"); `brightness`: the
    /// percent asked; `unable`: the room's lights left as they were because they take no colour
    /// or no brightness.
    Lights {
        area: String,
        on: bool,
        names: Vec<String>,
        colour: Option<String>,
        brightness: Option<u8>,
        unable: Vec<String>,
    },
    Setpoint {
        zone: String,
        from: Option<f64>,
        to: f64,
    },
    /// Shutters of a room, or of the whole house (`area: None`). `unable`: the chosen shutters
    /// that cannot do what was asked (no position, say), sent nothing.
    Covers {
        area: Option<String>,
        open: bool,
        position: Option<u8>,
        names: Vec<String>,
        unable: Vec<String>,
    },
    /// TVs or plugs (`what`) of a room, or of the whole house (`area: None`).
    Switches {
        what: SwitchType,
        area: Option<String>,
        on: bool,
        names: Vec<String>,
    },
    /// A media player command; `player` is its name.
    Media { player: String, what: MediaIntent },
    /// The music volume of a room's speakers; `level`: the first speaker's new level.
    Volume { area: String, level: Option<u8> },
    /// A room's speakers muted (`on`) or not.
    Mute { area: String, on: bool },
    /// The vacuum: `room` is the room cleaned as the reply says it, `None` for a full clean or
    /// any other action; `name` is the vacuum's name.
    Vacuum {
        action: VacuumIntent,
        room: Option<String>,
        name: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    Act {
        actions: Vec<Action>,
        done: Done,
        trace: Trace,
    },
    Answer {
        reply: String,
        trace: Trace,
    },
    /// Each choice is a label and the request it resolves to.
    Ask {
        question: String,
        choices: Vec<(String, Request)>,
        trace: Trace,
    },
    /// The safety gate: the request, confirmed, if the answer is yes.
    Confirm {
        question: String,
        request: Request,
        trace: Trace,
    },
    NotUnderstood {
        reply: String,
        trace: Trace,
    },
}

impl Plan {
    pub fn trace(&self) -> &Trace {
        match self {
            Plan::Act { trace, .. }
            | Plan::Answer { trace, .. }
            | Plan::Ask { trace, .. }
            | Plan::Confirm { trace, .. }
            | Plan::NotUnderstood { trace, .. } => trace,
        }
    }

    fn with_trace(mut self, trace: Trace) -> Self {
        *self.trace_mut() = trace;
        self
    }

    pub fn trace_mut(&mut self) -> &mut Trace {
        match self {
            Plan::Act { trace, .. }
            | Plan::Answer { trace, .. }
            | Plan::Ask { trace, .. }
            | Plan::Confirm { trace, .. }
            | Plan::NotUnderstood { trace, .. } => trace,
        }
    }
}

/// How a word is shown in the trace.
fn shown(word: &Word) -> String {
    match word {
        Word::Text(t) => t.clone(),
        Word::Number(n) => number(*n),
        Word::Signed(n) if *n >= 0.0 => format!("+{}", number(*n)),
        Word::Signed(n) => number(*n),
    }
}

/// Plans a typed sentence.
pub fn plan(sentence: &str, house: &House) -> Plan {
    plan_heard(sentence, house, None)
}

/// Plans a sentence heard by a satellite in `speaker_room` (`None`: typed on the web): the
/// music and volume intents use that room.
pub fn plan_heard(sentence: &str, house: &House, speaker_room: Option<&str>) -> Plan {
    let mut ws = words(sentence);
    // Speech-to-text often hears the opening "mets" as "mais", which never starts an order.
    if house.lang == Lang::Fr && ws.first().is_some_and(|w| w.is("mais")) {
        ws[0] = Word::Text("mets".into());
    }
    let mut plan = match play_by_name_in(sentence, house.lang).or_else(|| parse_in(&ws, house.lang))
    {
        Some(intent) => {
            let mut req = Request::new(intent);
            req.speaker_room = speaker_room.map(str::to_owned);
            plan_request(&req, house)
        }
        None => not_understood(house.lang),
    };
    plan.trace_mut().words = ws.iter().map(shown).collect();
    plan
}

/// Plans an understood request, fresh or resolved by an answer.
pub fn plan_request(req: &Request, house: &House) -> Plan {
    match &req.intent {
        Intent::Lights {
            on,
            names,
            room,
            colour,
            brightness,
            ..
        } => lights(req, *on, *colour, *brightness, names, room, house),
        Intent::ReadTemp { room, outdoor } => read_temp(req, room, *outdoor, house),
        Intent::SetTemp { change, room } => set_temp(req, *change, room, house),
        Intent::Feel { warmer, room } => feel(req, *warmer, room, house),
        Intent::HeatingNoValue { room } => heating_no_value(req, room, house),
        Intent::Covers {
            open,
            position,
            all,
            names,
            room,
        } => covers(req, *open, *position, *all, names, room, house),
        Intent::Switches {
            what,
            on,
            all,
            names,
            room,
        } => switches(req, *what, *on, *all, names, room, house),
        Intent::CoverPositionHint => answer(
            say(
                house,
                "Pour une position, dites par exemple « ouvre les volets à 30 % ».",
                "For a position, say for example \"open the shutters to 30 %\".",
            ),
            Trace {
                intent: Some("covers_hint".into()),
                ..Default::default()
            },
        ),
        Intent::Media(what) => media(req, what, house),
        Intent::Volume { change, room } => volume(req, *change, room, house),
        Intent::Mute { on, room } => mute(req, *on, room, house),
        Intent::Vacuum { action, room } => vacuum(req, *action, room, house),
        Intent::MediaHint => answer(
            say(
                house,
                "Pour un titre ou un artiste, dites « joue » suivi du nom.",
                "For a title or an artist, say \"play\" followed by the name.",
            ),
            Trace {
                intent: Some("media_hint".into()),
                ..Default::default()
            },
        ),
        Intent::BrightnessRange => answer(
            say(
                house,
                "La luminosité va de 1 à 100 %.",
                "The brightness goes from 1 to 100 %.",
            ),
            Trace {
                intent: Some("lights_range".into()),
                ..Default::default()
            },
        ),
        Intent::VolumeRange => answer(
            say(
                house,
                "Le volume va de 0 à 100 %.",
                "The volume goes from 0 to 100 %.",
            ),
            Trace {
                intent: Some("volume_range".into()),
                ..Default::default()
            },
        ),
        Intent::CoverPositionRange => answer(
            say(
                house,
                "La position va de 0 à 100 %.",
                "The position goes from 0 to 100 %.",
            ),
            Trace {
                intent: Some("covers_range".into()),
                ..Default::default()
            },
        ),
    }
}

/// The French or the English text, after the house's language.
fn say(house: &House, fr: &str, en: &str) -> String {
    match house.lang {
        Lang::Fr => fr.into(),
        Lang::En => en.into(),
    }
}

fn not_understood(lang: Lang) -> Plan {
    Plan::NotUnderstood {
        reply: match lang {
            Lang::Fr => "Je n'ai pas compris.",
            Lang::En => "I didn't understand.",
        }
        .into(),
        trace: Trace::default(),
    }
}

fn answer(reply: String, trace: Trace) -> Plan {
    Plan::Answer { reply, trace }
}

/// Asks for a room among `areas`, each choice the same request with that area.
fn ask_room(req: &Request, areas: &[String], house: &House, trace: Trace) -> Plan {
    let choices = areas
        .iter()
        .take(MAX_CHOICES)
        .map(|id| {
            let mut next = req.clone();
            next.area = Some(id.clone());
            (room_in(house.area_name(id), house.lang), next)
        })
        .collect();
    Plan::Ask {
        question: say(house, "Dans quelle pièce ?", "Which room?"),
        choices,
        trace,
    }
}

fn light_members<'a>(on: bool, area: Option<&str>, house: &House<'a>) -> Vec<&'a EntityView> {
    let action = CategoryAction {
        what: Category::Lights,
        to: if on { Target::On } else { Target::Off },
        brightness: None,
        area: area.map(str::to_string),
        except: vec![],
    };
    category_members(&action, house.entities)
}

/// A light or shutter whose name holds every name word (a plural word also matches its
/// singular).
fn named(light: &EntityView, names: &[String]) -> bool {
    let words = name_words(&light.name);
    names.iter().all(|n| {
        let n = fold(n);
        words.contains(&n)
            || n.strip_suffix('s')
                .is_some_and(|s| words.iter().any(|w| w == s))
    })
}

/// Areas in the house's order that hold one of `lights`.
fn areas_of(lights: &[&EntityView], house: &House) -> Vec<String> {
    house
        .areas
        .iter()
        .filter(|a| lights.iter().any(|l| l.area == a.id))
        .map(|a| a.id.clone())
        .collect()
}

/// Whether a light takes a colour. Zigbee2MQTT lights advertise `xy` rather than `hs`; the hub
/// sends the hue and saturation whatever the mode.
fn takes_colour(light: &EntityView) -> bool {
    ["hs", "xy", "rgb", "rgbw", "rgbww"]
        .iter()
        .any(|m| light.capabilities.color_modes.contains(*m))
}

fn lights(
    req: &Request,
    on: bool,
    colour: Option<Colour>,
    brightness: Option<u8>,
    names: &[String],
    room: &[String],
    house: &House,
) -> Plan {
    let mut trace = Trace {
        intent: Some(
            match (on, colour, brightness) {
                (true, Some(_), _) => "lights_colour",
                (true, None, Some(_)) => "lights_brightness",
                (true, None, None) => "lights_on",
                (false, ..) => "lights_off",
            }
            .into(),
        ),
        ..Default::default()
    };
    let en = house.lang == Lang::En;
    // A light must take everything asked: the colour and the brightness.
    let able = |l: &&EntityView| {
        (colour.is_none() || takes_colour(l)) && (brightness.is_none() || l.capabilities.brightness)
    };
    let lack = lack(colour.is_some(), brightness.is_some(), house.lang);
    // In English the words before the light word may be part of its name, not a room ("turn on
    // the desk lamp": names `lamp`, room `desk`).
    let together: Vec<String> = names.iter().chain(room).cloned().collect();
    // Words before a linker that name no light but do name a room are the room
    // ("allume le salon" said with a linker after it).
    let (names, room) = if en
        && !names.is_empty()
        && !room.is_empty()
        && matches!(resolve_room(house, room), RoomMatch::Unknown)
        && light_members(on, None, house)
            .iter()
            .any(|l| named(l, &together))
    {
        (&together[..], &[][..])
    } else if room.is_empty()
        && !names.is_empty()
        && !light_members(on, None, house)
            .iter()
            .any(|l| named(l, names))
        && matches!(
            resolve_room(house, names),
            RoomMatch::One(_) | RoomMatch::Many(_)
        )
    {
        (&[][..], names)
    } else {
        (names, room)
    };
    let all_named: Vec<&EntityView> = light_members(on, None, house)
        .into_iter()
        .filter(|l| names.is_empty() || named(l, names))
        .collect();
    let candidates: Vec<&EntityView> = all_named.iter().copied().filter(able).collect();
    let none = if (colour.is_some() || brightness.is_some()) && !all_named.is_empty() {
        match (all_named.as_slice(), en) {
            _ if names.is_empty() && en => format!("No light {}.", lack.none),
            _ if names.is_empty() => format!("Aucune lumière {}.", lack.none),
            ([one], false) => format!("« {} » {}.", one.name, lack.one),
            ([one], true) => format!("\"{}\" {}.", one.name, lack.one),
            (_, false) => format!("Aucune lumière « {} » {}.", names.join(" "), lack.none),
            (_, true) => format!("No \"{}\" light {}.", names.join(" "), lack.none),
        }
    } else if names.is_empty() {
        say(
            house,
            "Pas de lumière dans la maison.",
            "No lights in the house.",
        )
    } else if en {
        format!("I can't find a light called \"{}\".", names.join(" "))
    } else {
        format!("Je ne trouve pas de lumière « {} ».", names.join(" "))
    };
    let (area, how) = match pick_room(req, room, &areas_of(&candidates, house), none, house) {
        Ok(found) => found,
        Err(plan) => return (*plan).with_trace(trace),
    };
    let area_name = house.area_name(&area).to_string();
    trace.room = Some(area.clone());
    trace.room_how = Some(how);
    let members = light_members(on, Some(&area), house);
    if members.is_empty() {
        let reply = if en {
            format!("No lights {}.", place(&area_name, house.lang))
        } else {
            format!("Pas de lumière {}.", dans(&area_name))
        };
        return answer(reply, trace);
    }
    // Words of the room's own name were part of the room phrase ("la chambre de Léa").
    let mut area_words = name_words(&area_name);
    area_words.extend(name_words(&area));
    let names: Vec<String> = names
        .iter()
        .filter(|n| !area_words.contains(&fold(n)))
        .cloned()
        .collect();
    let chosen: Vec<&EntityView> = match &req.lights {
        Some(ids) => members
            .iter()
            .copied()
            .filter(|l| ids.contains(&l.id))
            .collect(),
        None if names.is_empty() => members.clone(),
        None => members
            .iter()
            .copied()
            .filter(|l| named(l, &names))
            .collect(),
    };
    if chosen.is_empty() {
        let choices = members
            .iter()
            .filter(|l| able(l))
            .take(MAX_CHOICES)
            .map(|l| {
                let mut next = req.clone();
                next.area = Some(area.clone());
                next.lights = Some(vec![l.id.clone()]);
                (l.name.clone(), next)
            })
            .collect::<Vec<_>>();
        if choices.is_empty() {
            return answer(no_light_in(&area_name, &lack, house), trace);
        }
        return Plan::Ask {
            question: if en {
                format!("Which light {}?", place(&area_name, house.lang))
            } else {
                format!("Quelle lumière {} ?", de(&area_name))
            },
            choices,
            trace,
        };
    }
    // A colour or a brightness goes to the lights that take it; the others are left as they are.
    let (chosen, unable): (Vec<&EntityView>, Vec<&EntityView>) = chosen.into_iter().partition(able);
    if chosen.is_empty() {
        let reply = match (unable.as_slice(), en) {
            ([one], false) => format!("« {} » {}.", one.name, lack.one),
            ([one], true) => format!("\"{}\" {}.", one.name, lack.one),
            _ => no_light_in(&area_name, &lack, house),
        };
        return answer(reply, trace);
    }
    let command = if on {
        Command::TurnOn {
            brightness: brightness.map(|p| ((u16::from(p) * 255 + 50) / 100) as u8),
            color: colour.map(Colour::hs),
            effect: None,
        }
    } else {
        Command::TurnOff
    };
    trace.entities = chosen.iter().map(|l| l.id.clone()).collect();
    Plan::Act {
        actions: chosen
            .iter()
            .map(|l| Action::Light {
                entity: l.id.clone(),
                command: command.clone(),
            })
            .collect(),
        done: Done::Lights {
            area,
            on,
            names: chosen.iter().map(|l| l.name.clone()).collect(),
            colour: colour.map(|c| c.name_in(house.lang).to_string()),
            brightness,
            unable: unable.iter().map(|l| l.name.clone()).collect(),
        },
        trace,
    }
}

/// "No light in the Salon takes a colour."
fn no_light_in(area_name: &str, lack: &Lack, house: &House) -> String {
    match house.lang {
        Lang::Fr => format!("Aucune lumière {} {}.", lack.none, dans(area_name)),
        Lang::En => format!("No light {} {}.", place(area_name, house.lang), lack.none),
    }
}

fn cover_members<'a>(area: Option<&str>, house: &House<'a>) -> Vec<&'a EntityView> {
    let action = CategoryAction {
        what: Category::Covers,
        to: Target::Close,
        brightness: None,
        area: area.map(str::to_string),
        except: vec![],
    };
    category_members(&action, house.entities)
}

/// What a shutter is sent: a position when one was asked (and it takes one), else open/close,
/// or the position 100 / 0 when it only takes a position. `None`: it cannot do it.
fn cover_command(cover: &EntityView, open: bool, position: Option<u8>) -> Option<Command> {
    let caps = &cover.capabilities;
    match position {
        Some(position) => caps.position.then_some(Command::SetPosition { position }),
        None if caps.open_close => Some(if open { Command::Open } else { Command::Close }),
        None if caps.position => Some(Command::SetPosition {
            position: if open { 100 } else { 0 },
        }),
        None => None,
    }
}

fn covers(
    req: &Request,
    open: bool,
    position: Option<u8>,
    all: bool,
    names: &[String],
    room: &[String],
    house: &House,
) -> Plan {
    let mut trace = Trace {
        intent: Some(if open { "covers_open" } else { "covers_close" }.into()),
        ..Default::default()
    };
    let en = house.lang == Lang::En;
    let together: Vec<String> = names.iter().chain(room).cloned().collect();
    // Words that name no shutter but do name a room are the room ("ferme le salon" form).
    let (names, room) = if en
        && !names.is_empty()
        && !room.is_empty()
        && matches!(resolve_room(house, room), RoomMatch::Unknown)
        && cover_members(None, house)
            .iter()
            .any(|c| named(c, &together))
    {
        // In English the words before the shutter word may be its name ("close the bay
        // shutter").
        (&together[..], &[][..])
    } else if room.is_empty()
        && !names.is_empty()
        && !cover_members(None, house).iter().any(|c| named(c, names))
        && matches!(
            resolve_room(house, names),
            RoomMatch::One(_) | RoomMatch::Many(_)
        )
    {
        (&[][..], names)
    } else if names.is_empty()
        && !room.is_empty()
        && matches!(resolve_room(house, room), RoomMatch::Unknown)
        && cover_members(None, house).iter().any(|c| named(c, room))
    {
        // And the other way: "ferme le volet de la baie" names a shutter, not a room.
        (room, &[][..])
    } else {
        (names, room)
    };
    // "de la maison", "partout": the whole house, unless a room is called that.
    let house_said = !room.is_empty()
        && room
            .iter()
            .all(|w| matches!(fold(w).as_str(), "maison" | "partout"))
        && matches!(resolve_room(house, room), RoomMatch::Unknown);
    let whole_house =
        ((all && room.is_empty()) || house_said) && names.is_empty() && req.area.is_none();
    let (area, members) = if whole_house {
        trace.room_how = Some("toute la maison".into());
        let members = cover_members(None, house);
        if members.is_empty() {
            return answer(
                say(
                    house,
                    "Pas de volet dans la maison.",
                    "No shutters in the house.",
                ),
                trace,
            );
        }
        (None, members)
    } else {
        let candidates: Vec<&EntityView> = cover_members(None, house)
            .into_iter()
            .filter(|c| names.is_empty() || named(c, names))
            .collect();
        let none = if names.is_empty() {
            say(
                house,
                "Pas de volet dans la maison.",
                "No shutters in the house.",
            )
        } else if en {
            format!("I can't find a shutter called \"{}\".", names.join(" "))
        } else {
            format!("Je ne trouve pas de volet « {} ».", names.join(" "))
        };
        let (area, how) = match pick_room(req, room, &areas_of(&candidates, house), none, house) {
            Ok(found) => found,
            Err(plan) => return (*plan).with_trace(trace),
        };
        let area_name = house.area_name(&area).to_string();
        trace.room = Some(area.clone());
        trace.room_how = Some(how);
        let members = cover_members(Some(&area), house);
        if members.is_empty() {
            let reply = if en {
                format!("No shutters {}.", place(&area_name, house.lang))
            } else {
                format!("Pas de volet {}.", dans(&area_name))
            };
            return answer(reply, trace);
        }
        // Words of the room's own name were part of the room phrase.
        let mut area_words = name_words(&area_name);
        area_words.extend(name_words(&area));
        let names: Vec<String> = names
            .iter()
            .filter(|n| !area_words.contains(&fold(n)))
            .cloned()
            .collect();
        let chosen: Vec<&EntityView> = match &req.covers {
            Some(ids) => members
                .iter()
                .copied()
                .filter(|c| ids.contains(&c.id))
                .collect(),
            None if names.is_empty() => members.clone(),
            None => members
                .iter()
                .copied()
                .filter(|c| named(c, &names))
                .collect(),
        };
        if chosen.is_empty() {
            let choices = members
                .iter()
                .take(MAX_CHOICES)
                .map(|c| {
                    let mut next = req.clone();
                    next.area = Some(area.clone());
                    next.covers = Some(vec![c.id.clone()]);
                    (c.name.clone(), next)
                })
                .collect();
            return Plan::Ask {
                question: if en {
                    format!("Which shutter {}?", place(&area_name, house.lang))
                } else {
                    format!("Quel volet {} ?", de(&area_name))
                },
                choices,
                trace,
            };
        }
        (Some(area), chosen)
    };
    let mut actions = Vec::new();
    let mut acted = Vec::new();
    let mut unable = Vec::new();
    for c in &members {
        match cover_command(c, open, position) {
            Some(command) => {
                trace.entities.push(c.id.clone());
                actions.push(Action::Cover {
                    entity: c.id.clone(),
                    command,
                });
                acted.push(c.name.clone());
            }
            None => unable.push(c.name.clone()),
        }
    }
    if actions.is_empty() {
        return answer(cannot_in(&unable, open, position, house.lang), trace);
    }
    Plan::Act {
        actions,
        done: Done::Covers {
            area,
            open,
            position,
            names: acted,
            unable,
        },
        trace,
    }
}

/// The TVs or plugs (by their "Show as" type) that may be commanded; nothing else is ever one.
fn switch_members<'a>(
    what: SwitchType,
    area: Option<&str>,
    house: &House<'a>,
) -> Vec<&'a EntityView> {
    house
        .entities
        .iter()
        .filter(|e| {
            commandable(e)
                && SwitchType::of(e.protocol_kind, &e.ui) == Some(what)
                && area.is_none_or(|a| e.area == a)
        })
        .collect()
}

/// TVs or plugs: planned like shutters, on or off.
fn switches(
    req: &Request,
    what: SwitchType,
    on: bool,
    all: bool,
    names: &[String],
    room: &[String],
    house: &House,
) -> Plan {
    let en = house.lang == Lang::En;
    let (noun, intent) = match (what, en) {
        (SwitchType::Tv, false) => ("télé", "tv"),
        (_, false) => ("prise", "plugs"),
        (SwitchType::Tv, true) => ("TV", "tv"),
        (_, true) => ("plug", "plugs"),
    };
    // "No TVs", "No plugs".
    let plural = if what == SwitchType::Tv {
        "TVs"
    } else {
        "plugs"
    };
    let mut trace = Trace {
        intent: Some(format!("{intent}_{}", if on { "on" } else { "off" })),
        ..Default::default()
    };
    let together: Vec<String> = names.iter().chain(room).cloned().collect();
    // Words that name no device but do name a room are the room.
    let (names, room) = if en
        && !names.is_empty()
        && !room.is_empty()
        && matches!(resolve_room(house, room), RoomMatch::Unknown)
        && switch_members(what, None, house)
            .iter()
            .any(|s| named(s, &together))
    {
        // In English the words before the device word may be its name ("the coffee plug").
        (&together[..], &[][..])
    } else if room.is_empty()
        && !names.is_empty()
        && !switch_members(what, None, house)
            .iter()
            .any(|s| named(s, names))
        && matches!(
            resolve_room(house, names),
            RoomMatch::One(_) | RoomMatch::Many(_)
        )
    {
        (&[][..], names)
    } else if names.is_empty()
        && !room.is_empty()
        && matches!(resolve_room(house, room), RoomMatch::Unknown)
        && switch_members(what, None, house)
            .iter()
            .any(|s| named(s, room))
    {
        // And the other way: "éteins la prise cafetière" names a device, not a room.
        (room, &[][..])
    } else {
        (names, room)
    };
    // "de la maison", "partout": the whole house, unless a room is called that.
    let house_said = !room.is_empty()
        && room
            .iter()
            .all(|w| matches!(fold(w).as_str(), "maison" | "partout"))
        && matches!(resolve_room(house, room), RoomMatch::Unknown);
    let whole_house =
        ((all && room.is_empty()) || house_said) && names.is_empty() && req.area.is_none();
    let (area, chosen) = if whole_house {
        trace.room_how = Some("toute la maison".into());
        let members = switch_members(what, None, house);
        if members.is_empty() {
            let reply = if en {
                format!("No {plural} in the house.")
            } else {
                format!("Pas de {noun} dans la maison.")
            };
            return answer(reply, trace);
        }
        (None, members)
    } else {
        let candidates: Vec<&EntityView> = switch_members(what, None, house)
            .into_iter()
            .filter(|s| names.is_empty() || named(s, names))
            .collect();
        let none = match (names.is_empty(), en) {
            (true, false) => format!("Pas de {noun} dans la maison."),
            (false, false) => format!("Je ne trouve pas de {noun} « {} ».", names.join(" ")),
            (true, true) => format!("No {plural} in the house."),
            (false, true) => format!("I can't find a {noun} called \"{}\".", names.join(" ")),
        };
        let (area, how) = match pick_room(req, room, &areas_of(&candidates, house), none, house) {
            Ok(found) => found,
            Err(plan) => return (*plan).with_trace(trace),
        };
        let area_name = house.area_name(&area).to_string();
        trace.room = Some(area.clone());
        trace.room_how = Some(how);
        let members = switch_members(what, Some(&area), house);
        if members.is_empty() {
            let reply = if en {
                format!("No {plural} {}.", place(&area_name, house.lang))
            } else {
                format!("Pas de {noun} {}.", dans(&area_name))
            };
            return answer(reply, trace);
        }
        // Words of the room's own name were part of the room phrase.
        let mut area_words = name_words(&area_name);
        area_words.extend(name_words(&area));
        let names: Vec<String> = names
            .iter()
            .filter(|n| !area_words.contains(&fold(n)))
            .cloned()
            .collect();
        let chosen: Vec<&EntityView> = match &req.switches {
            Some(ids) => members
                .iter()
                .copied()
                .filter(|s| ids.contains(&s.id))
                .collect(),
            None if names.is_empty() => members.clone(),
            None => members
                .iter()
                .copied()
                .filter(|s| named(s, &names))
                .collect(),
        };
        if chosen.is_empty() {
            let choices = members
                .iter()
                .take(MAX_CHOICES)
                .map(|s| {
                    let mut next = req.clone();
                    next.area = Some(area.clone());
                    next.switches = Some(vec![s.id.clone()]);
                    (s.name.clone(), next)
                })
                .collect();
            return Plan::Ask {
                question: if en {
                    format!("Which {noun} {}?", place(&area_name, house.lang))
                } else {
                    format!("Quelle {noun} {} ?", de(&area_name))
                },
                choices,
                trace,
            };
        }
        (Some(area), chosen)
    };
    let command = if on {
        Command::TurnOn {
            brightness: None,
            color: None,
            effect: None,
        }
    } else {
        Command::TurnOff
    };
    trace.entities = chosen.iter().map(|s| s.id.clone()).collect();
    Plan::Act {
        actions: chosen
            .iter()
            .map(|s| Action::Switch {
                entity: s.id.clone(),
                command: command.clone(),
            })
            .collect(),
        done: Done::Switches {
            what,
            area,
            on,
            names: chosen.iter().map(|s| s.name.clone()).collect(),
        },
        trace,
    }
}

/// The media players a sentence can reach.
fn players<'a>(house: &House<'a>) -> Vec<&'a EntityView> {
    house
        .entities
        .iter()
        .filter(|e| e.kind == Kind::MediaPlayer && commandable(e))
        .collect()
}

/// The player a media request is for: the chosen one, else the only online one, else the only
/// online one in the speaker's room; otherwise a question. `Err` is the plan to return instead.
fn pick_player<'a>(
    req: &Request,
    house: &House<'a>,
    trace: &Trace,
) -> Result<&'a EntityView, Box<Plan>> {
    let all = players(house);
    let none = || {
        Box::new(answer(
            say(
                house,
                "Il n'y a pas de lecteur de musique.",
                "There's no music player.",
            ),
            trace.clone(),
        ))
    };
    if let Some(id) = &req.player {
        return all.into_iter().find(|p| &p.id == id).ok_or_else(none);
    }
    if all.is_empty() {
        return Err(none());
    }
    let online: Vec<&EntityView> = all
        .iter()
        .copied()
        .filter(|p| p.state.availability != Availability::Offline)
        .collect();
    match online.as_slice() {
        [] => Err(Box::new(answer(
            say(
                house,
                "Le lecteur ne répond pas.",
                "The player isn't responding.",
            ),
            trace.clone(),
        ))),
        [one] => Ok(one),
        _ => {
            let here: Vec<&EntityView> = match &req.speaker_room {
                Some(room) => online.iter().copied().filter(|p| &p.area == room).collect(),
                None => Vec::new(),
            };
            if let [one] = here.as_slice() {
                return Ok(one);
            }
            let choices = online
                .iter()
                .take(MAX_CHOICES)
                .map(|p| {
                    let mut next = req.clone();
                    next.player = Some(p.id.clone());
                    (p.name.clone(), next)
                })
                .collect();
            Err(Box::new(Plan::Ask {
                question: say(house, "Quel lecteur ?", "Which player?"),
                choices,
                trace: trace.clone(),
            }))
        }
    }
}

/// "Get Lucky, de Daft Punk." / "Get Lucky, by Daft Punk." / "Get Lucky."
fn song(title: &str, artist: Option<&str>, lang: Lang) -> String {
    match (artist, lang) {
        (Some(artist), Lang::Fr) => format!("{title}, de {artist}."),
        (Some(artist), Lang::En) => format!("{title}, by {artist}."),
        (None, _) => format!("{title}."),
    }
}

fn nothing_playing(house: &House) -> String {
    say(house, "Rien ne joue.", "Nothing is playing.")
}

fn media(req: &Request, what: &MediaIntent, house: &House) -> Plan {
    let intent = match what {
        MediaIntent::Pause => "media_pause",
        MediaIntent::Play { resume: true } => "media_resume",
        MediaIntent::Play { resume: false } => "media_play",
        MediaIntent::Next => "media_next",
        MediaIntent::Previous => "media_previous",
        MediaIntent::NowPlaying => "media_now_playing",
        MediaIntent::PlayMedia { .. } => "media_play_by_name",
    };
    let mut trace = Trace {
        intent: Some(intent.into()),
        ..Default::default()
    };
    let player = match pick_player(req, house, &trace) {
        Ok(player) => player,
        Err(plan) => return *plan,
    };
    trace.entities = vec![player.id.clone()];
    let (feature, command) = match what {
        MediaIntent::NowPlaying => {
            let reply = match &player.state.value {
                StateValue::MediaPlayer(state) => match (state.state.as_deref(), &state.title) {
                    (Some("playing"), Some(title)) => {
                        song(title, state.artist.as_deref(), house.lang)
                    }
                    (Some("paused"), Some(title)) => format!(
                        "{} {}",
                        say(house, "En pause :", "Paused:"),
                        song(title, state.artist.as_deref(), house.lang)
                    ),
                    _ => nothing_playing(house),
                },
                _ => nothing_playing(house),
            };
            return answer(reply, trace);
        }
        MediaIntent::PlayMedia { query } => (
            "play_media",
            Command::PlayMedia {
                query: query.clone(),
            },
        ),
        MediaIntent::Pause => (
            "pause",
            Command::MediaAction {
                action: "pause".into(),
            },
        ),
        MediaIntent::Play { .. } => (
            "play",
            Command::MediaAction {
                action: "play".into(),
            },
        ),
        MediaIntent::Next => (
            "next",
            Command::MediaAction {
                action: "next".into(),
            },
        ),
        MediaIntent::Previous => (
            "previous",
            Command::MediaAction {
                action: "previous".into(),
            },
        ),
    };
    // An idle player has no song to pause or skip, and would never echo the command.
    let idle = matches!(&player.state.value,
        StateValue::MediaPlayer(state) if state.state.as_deref() == Some("idle"));
    if idle
        && matches!(
            what,
            MediaIntent::Pause | MediaIntent::Next | MediaIntent::Previous
        )
    {
        return answer(nothing_playing(house), trace);
    }
    if !player
        .capabilities
        .media_features
        .iter()
        .any(|f| f == feature)
    {
        let reply = if feature == "play_media" {
            say(
                house,
                "Ce lecteur ne sait pas chercher un titre.",
                "This player can't search for a title.",
            )
        } else {
            say(
                house,
                "Ce lecteur ne sait pas faire ça.",
                "This player can't do that.",
            )
        };
        return answer(reply, trace);
    }
    Plan::Act {
        actions: vec![Action::Media {
            entity: player.id.clone(),
            command,
        }],
        done: Done::Media {
            player: player.name.clone(),
            what: what.clone(),
        },
        trace,
    }
}

/// The vacuums that may be commanded.
fn vacuums<'a>(house: &House<'a>) -> Vec<&'a EntityView> {
    house
        .entities
        .iter()
        .filter(|e| e.kind == Kind::Vacuum && commandable(e))
        .collect()
}

/// Words that say the whole house: "nettoie la maison", "aspire partout".
const WHOLE_HOUSE: &[&str] = &["maison", "partout", "tout", "house", "everywhere"];

/// The words of the vacuum's own names, its device's and its own: "Roborock, nettoie" names
/// the vacuum, not a room.
fn vacuum_words(vac: &EntityView) -> Vec<String> {
    [vac.device_name.as_deref(), Some(vac.name.as_str())]
        .into_iter()
        .flatten()
        .flat_map(name_words)
        .collect()
}

/// A room button's own name: the user's name for it when they gave one, else the name it was
/// discovered with, without its device's ("Clean kitchen" of "Roborock Clean kitchen").
fn button_name(button: &EntityView) -> &str {
    let renamed = button
        .ui
        .name
        .as_deref()
        .is_some_and(|n| !n.trim().is_empty());
    match &button.label {
        Some(label) if !renamed => label,
        _ => &button.name,
    }
}

/// A room button's name without its leading verb: "Clean kitchen" is "kitchen".
fn room_label(button: &EntityView) -> String {
    let lower = button_name(button).trim().to_lowercase();
    for verb in ["clean ", "nettoyer ", "nettoie ", "aspirer ", "aspire "] {
        if let Some(rest) = lower.strip_prefix(verb) {
            return rest.trim().to_owned();
        }
    }
    lower
}

/// The words of a button's name that name its room: no linkers, no verb.
fn button_words(button: &EntityView) -> Vec<String> {
    name_words(&room_label(button))
        .into_iter()
        .filter(|w| !matches!(class(w), Some(Class::Clean)) && w != "nettoyer")
        .collect()
}

/// The room buttons (of the vacuum's device) the said words name: by the button's own name
/// ("Cuisine"), or through the generic room nouns ("cuisine" is "Clean kitchen"). A room the
/// house calls the same way is tried too, by its name and id.
fn room_buttons<'a>(
    said: &[String],
    buttons: &[&'a EntityView],
    house: &House,
) -> Vec<&'a EntityView> {
    let mut candidates = vec![said.to_vec()];
    if let RoomMatch::One(id) = resolve_room(house, said) {
        candidates.push(name_words(house.area_name(&id)));
        candidates.push(name_words(&id));
    }
    buttons
        .iter()
        .copied()
        .filter(|b| {
            let words = button_words(b);
            candidates.iter().any(|cand| {
                !cand.is_empty()
                    && (score(cand, &words) > 0
                        || room_noun(cand).is_some_and(|noun| {
                            noun.names
                                .iter()
                                .any(|alt| alt.split(' ').all(|w| words.iter().any(|bw| bw == w)))
                        }))
            })
        })
        .collect()
}

/// The vacuum: start, stop, pause, go back to its base, or clean one room by pressing the room's
/// button. A full clean never depends on the room the satellite is in.
fn vacuum(req: &Request, action: VacuumIntent, room: &[String], house: &House) -> Plan {
    let intent = match action {
        VacuumIntent::Clean => "vacuum_clean",
        VacuumIntent::Stop => "vacuum_stop",
        VacuumIntent::Pause => "vacuum_pause",
        VacuumIntent::Dock => "vacuum_dock",
    };
    let mut trace = Trace {
        intent: Some(intent.into()),
        ..Default::default()
    };
    let all = vacuums(house);
    let chosen = req.vacuum.as_deref().cloned().unwrap_or_default();
    let picked = match (&chosen.vacuum, all.as_slice()) {
        (Some(id), _) => all.iter().copied().find(|v| &v.id == id),
        (None, [one]) => Some(*one),
        _ => None,
    };
    let vac = match picked {
        Some(vac) => vac,
        None if all.is_empty() || chosen.vacuum.is_some() => {
            return answer(
                say(
                    house,
                    "Pas d'aspirateur dans la maison.",
                    "No vacuum in the house.",
                ),
                trace,
            );
        }
        None => {
            let choices = all
                .iter()
                .take(MAX_CHOICES)
                .map(|v| {
                    let mut next = req.clone();
                    next.vacuum = Some(Box::new(VacuumPick {
                        vacuum: Some(v.id.clone()),
                        room: None,
                    }));
                    (v.name.clone(), next)
                })
                .collect();
            return Plan::Ask {
                question: say(house, "Quel aspirateur ?", "Which vacuum?"),
                choices,
                trace,
            };
        }
    };
    let feature = match action {
        VacuumIntent::Stop => Some("stop"),
        VacuumIntent::Pause => Some("pause"),
        VacuumIntent::Dock => Some("return_to_base"),
        VacuumIntent::Clean => None,
    };
    let mut said: Vec<String> = room
        .iter()
        .map(|w| fold(w))
        .filter(|w| !matches!(class(w), Some(Class::Linker | Class::Noise)))
        .collect();
    let buttons: Vec<&EntityView> = house
        .entities
        .iter()
        .filter(|e| {
            e.kind == Kind::Button
                && commandable(e)
                && e.device_id.is_some()
                && e.device_id == vac.device_id
        })
        .collect();
    // "Roborock, nettoie la cuisine": the vacuum's own name is not a room, unless a room button
    // is called that way.
    let own = vacuum_words(vac);
    if said.iter().any(|w| own.contains(w)) && room_buttons(&said, &buttons, house).is_empty() {
        said.retain(|w| !own.contains(w));
    }
    let whole = action == VacuumIntent::Clean
        && (said.is_empty()
            || (said.iter().all(|w| WHOLE_HOUSE.contains(&w.as_str()))
                && matches!(resolve_room(house, &said), RoomMatch::Unknown)));
    if feature.is_some() || whole {
        let wanted = feature.unwrap_or("start");
        if !vac.capabilities.vacuum_actions.iter().any(|a| a == wanted) {
            return answer(
                say(
                    house,
                    "Cet aspirateur ne sait pas faire ça.",
                    "This vacuum can't do that.",
                ),
                trace,
            );
        }
        let command = Command::VacuumAction {
            action: wanted.into(),
        };
        if whole {
            trace.room_how = Some(say(house, "toute la maison", "the whole house"));
        }
        trace.entities = vec![vac.id.clone()];
        return Plan::Act {
            actions: vec![Action::Vacuum {
                entity: vac.id.clone(),
                command,
            }],
            done: Done::Vacuum {
                action,
                room: None,
                name: vac.name.clone(),
            },
            trace,
        };
    }
    if buttons.is_empty() {
        return answer(
            say(
                house,
                "L'aspirateur ne sait pas nettoyer une seule pièce.",
                "The vacuum can't clean a single room.",
            ),
            trace,
        );
    }
    let found: Vec<&EntityView> = match &chosen.room {
        Some(id) => buttons.iter().copied().filter(|b| &b.id == id).collect(),
        None => room_buttons(&said, &buttons, house),
    };
    trace.room_how = Some(format!(
        "{} : {}",
        say(house, "mots", "words"),
        said.join(", ")
    ));
    if let RoomMatch::One(id) = resolve_room(house, &said) {
        trace.room = Some(id);
    }
    match found.as_slice() {
        [] => {
            let rooms = buttons
                .iter()
                .map(|b| room_label(b))
                .collect::<Vec<_>>()
                .join(", ");
            let reply = match house.lang {
                Lang::Fr => format!(
                    "L'aspirateur ne connaît pas la pièce « {} ». Pièces : {rooms}.",
                    said.join(" ")
                ),
                Lang::En => format!(
                    "The vacuum doesn't know the room \"{}\". Rooms: {rooms}.",
                    said.join(" ")
                ),
            };
            answer(reply, trace)
        }
        [button] => {
            trace.entities = vec![button.id.clone()];
            let noun = room_noun(&said);
            let label = match (house.lang, noun) {
                (Lang::Fr, Some(noun)) => noun.fr.to_owned(),
                _ => room_label(button),
            };
            Plan::Act {
                actions: vec![Action::Vacuum {
                    entity: button.id.clone(),
                    command: Command::Press,
                }],
                done: Done::Vacuum {
                    action,
                    room: Some(label),
                    name: vac.name.clone(),
                },
                trace,
            }
        }
        several => {
            let choices = several
                .iter()
                .take(MAX_CHOICES)
                .map(|b| {
                    let mut next = req.clone();
                    next.vacuum = Some(Box::new(VacuumPick {
                        vacuum: Some(vac.id.clone()),
                        room: Some(b.id.clone()),
                    }));
                    (b.name.clone(), next)
                })
                .collect();
            Plan::Ask {
                question: say(house, "Quelle pièce ?", "Which room?"),
                choices,
                trace,
            }
        }
    }
}

/// A room's speaker controls marked `device_class` (`volume` numbers, `mute` switches), every
/// room's when `area` is `None`.
fn speaker_controls<'a>(class: &str, area: Option<&str>, house: &House<'a>) -> Vec<&'a EntityView> {
    // Not `commandable`: a satellite's Mute is folded into its Music volume (`part_of`).
    let usable = |e: &EntityView| {
        e.category == EntityCategory::Primary
            && e.enabled_by_default
            && !e.id.starts_with("group_")
            && !e.id.starts_with("virtual_")
    };
    let kind = if class == "volume" {
        Kind::Number
    } else {
        Kind::Switch
    };
    house
        .entities
        .iter()
        .filter(|e| {
            e.kind == kind
                && e.device_class.as_deref() == Some(class)
                && usable(e)
                && area.is_none_or(|a| speaker_area(e, house) == a)
        })
        .collect()
}

/// A speaker control's room: a folded one (a satellite's Mute) is in the room of the Music
/// volume it is folded into, which is where the satellite's room is set.
fn speaker_area(e: &EntityView, house: &House) -> String {
    e.part_of
        .as_deref()
        .and_then(|parent| house.entity(parent))
        .map_or_else(|| e.area.clone(), |parent| parent.area.clone())
}

/// The room a volume or mute request is for: answered, else named, else the speaker's, else
/// the only room with such a control, else a question.
fn speaker_room(
    req: &Request,
    room: &[String],
    class: &str,
    house: &House,
) -> Result<(String, String), Box<Plan>> {
    if req.area.is_none()
        && room.is_empty()
        && let Some(heard) = &req.speaker_room
    {
        return Ok((heard.clone(), "pièce du satellite".into()));
    }
    let controls = speaker_controls(class, None, house);
    let candidates: Vec<String> = house
        .areas
        .iter()
        .filter(|a| controls.iter().any(|c| speaker_area(c, house) == a.id))
        .map(|a| a.id.clone())
        .collect();
    pick_room(
        req,
        room,
        &candidates,
        say(house, "Il n'y a pas d'enceinte.", "There's no speaker."),
        house,
    )
}

/// "No speakers in the Kitchen."
fn no_speaker_in(area_name: &str, house: &House) -> String {
    match house.lang {
        Lang::Fr => format!("Pas d'enceinte {}.", dans(area_name)),
        Lang::En => format!("No speakers {}.", place(area_name, house.lang)),
    }
}

fn volume(req: &Request, change: Change, room: &[String], house: &House) -> Plan {
    let mut trace = Trace {
        intent: Some("volume".into()),
        ..Default::default()
    };
    let (area, how) = match speaker_room(req, room, "volume", house) {
        Ok(found) => found,
        Err(plan) => return (*plan).with_trace(trace),
    };
    trace.room = Some(area.clone());
    trace.room_how = Some(how);
    let controls = speaker_controls("volume", Some(&area), house);
    if controls.is_empty() {
        return answer(no_speaker_in(house.area_name(&area), house), trace);
    }
    let levels: Vec<(&EntityView, f64)> = controls
        .iter()
        .filter_map(|c| {
            let level = match change {
                Change::Absolute(v) => v,
                Change::Relative(step) => match c.state.value {
                    StateValue::Number(now) => now + step,
                    _ => return None,
                },
            };
            Some((*c, level.round().clamp(0.0, 100.0)))
        })
        .collect();
    if levels.is_empty() {
        return answer(
            say(
                house,
                "Je ne connais pas le volume actuel.",
                "I don't know the current volume.",
            ),
            trace,
        );
    }
    trace.entities = levels.iter().map(|(c, _)| c.id.clone()).collect();
    trace.value = Some(format!("{} %", number(levels[0].1)));
    Plan::Act {
        actions: levels
            .iter()
            .map(|(c, level)| Action::Speaker {
                entity: c.id.clone(),
                command: Command::SetNumber { value: *level },
            })
            .collect(),
        done: Done::Volume {
            area,
            level: Some(levels[0].1 as u8),
        },
        trace,
    }
}

fn mute(req: &Request, on: bool, room: &[String], house: &House) -> Plan {
    let mut trace = Trace {
        intent: Some("mute".into()),
        ..Default::default()
    };
    let (area, how) = match speaker_room(req, room, "mute", house) {
        Ok(found) => found,
        Err(plan) => return (*plan).with_trace(trace),
    };
    trace.room = Some(area.clone());
    trace.room_how = Some(how);
    let controls = speaker_controls("mute", Some(&area), house);
    if controls.is_empty() {
        return answer(no_speaker_in(house.area_name(&area), house), trace);
    }
    trace.entities = controls.iter().map(|c| c.id.clone()).collect();
    let command = if on {
        Command::TurnOn {
            brightness: None,
            color: None,
            effect: None,
        }
    } else {
        Command::TurnOff
    };
    Plan::Act {
        actions: controls
            .iter()
            .map(|c| Action::Speaker {
                entity: c.id.clone(),
                command: command.clone(),
            })
            .collect(),
        done: Done::Mute { area, on },
        trace,
    }
}

/// The room of a request: the answered one, else the one the words name, else the only
/// candidate. `Err` is the plan to return instead: a question or a plain answer (`none` when
/// there is no candidate at all).
fn pick_room(
    req: &Request,
    room: &[String],
    candidates: &[String],
    none: String,
    house: &House,
) -> Result<(String, String), Box<Plan>> {
    if let Some(area) = &req.area {
        return Ok((area.clone(), "choix".into()));
    }
    match resolve_room(house, room) {
        RoomMatch::One(area) => Ok((area, format!("mots : {}", room.join(", ")))),
        RoomMatch::Many(areas) => Err(Box::new(ask_room(req, &areas, house, Trace::default()))),
        RoomMatch::Unknown => Err(Box::new(answer(
            match house.lang {
                Lang::Fr => format!("Je ne connais pas de pièce « {} ».", room.join(" ")),
                Lang::En => format!("I don't know a room called \"{}\".", room.join(" ")),
            },
            Trace::default(),
        ))),
        RoomMatch::NoRoomSaid => match candidates {
            [] => Err(Box::new(answer(none, Trace::default()))),
            [one] => Ok((one.clone(), "seule pièce possible".into())),
            _ => Err(Box::new(ask_room(req, candidates, house, Trace::default()))),
        },
    }
}

/// The value of a numeric sensor.
fn sensor_value(e: &EntityView) -> Option<f64> {
    match e.state.value {
        StateValue::Number(n) => Some(n),
        _ => None,
    }
}

/// The area's thermometer by the climate engine's rule: a primary temperature sensor of the
/// area, not a group, not the outdoor sensor, not part of a climate device; the smallest id.
fn thermometer<'a>(house: &House<'a>, area: &str) -> Option<&'a EntityView> {
    let climate_devices: Vec<&str> = house
        .entities
        .iter()
        .filter(|e| e.kind == Kind::Climate && e.area == area)
        .filter_map(|e| e.device_id.as_deref())
        .collect();
    house
        .entities
        .iter()
        .filter(|e| {
            e.kind == Kind::Sensor
                && e.area == area
                && e.device_class.as_deref() == Some("temperature")
                && e.category == EntityCategory::Primary
                && !e.id.starts_with("group_")
                && house.outdoor_entity != Some(e.id.as_str())
                && e.device_id
                    .as_deref()
                    .is_none_or(|d| !climate_devices.contains(&d))
        })
        .min_by(|a, b| a.id.cmp(&b.id))
}

/// A room's temperature and its age in seconds: the zone's reading, else its thermometer.
fn room_reading(house: &House, area: &str) -> Option<(f64, Option<u64>, String)> {
    if let Some(zone) = house.zone(area)
        && let Some(reading) = zone.reading
    {
        return Some((reading, zone.reading_age_s, format!("zone {area}")));
    }
    let sensor = thermometer(house, area)?;
    let value = sensor_value(sensor)?;
    let age = sensor
        .state
        .last_update
        .map(|t| house.now.saturating_sub(t));
    Some((value, age, sensor.id.clone()))
}

/// ", mesuré il y a 3 h" when the reading is older than the house's stale limit.
fn age_note(age_s: Option<u64>, house: &House) -> String {
    match age_s {
        Some(age) if age > u64::from(house.stale_after_min) * 60 => {
            let (h, m) = (age / 3600, (age % 3600) / 60);
            if house.lang == Lang::En {
                let ago = match (h, m) {
                    (0, m) => format!("{m} min"),
                    (h, _) if h >= 48 => format!("{} days", h / 24),
                    (h, 0) => format!("{h} h"),
                    (h, m) => format!("{h} h {m} min"),
                };
                return format!(", measured {ago} ago");
            }
            let ago = match (h, m) {
                (0, m) => format!("{m} min"),
                (h, _) if h >= 48 => format!("{} j", h / 24),
                (h, 0) => format!("{h} h"),
                (h, m) => format!("{h} h {m:02}"),
            };
            format!(", mesuré il y a {ago}")
        }
        _ => String::new(),
    }
}

fn read_temp(req: &Request, room: &[String], outdoor: bool, house: &House) -> Plan {
    let mut trace = Trace {
        intent: Some("read_temp".into()),
        ..Default::default()
    };
    if outdoor {
        let id = house.outdoor_entity.unwrap_or("weather_temperature");
        trace.entities = vec![id.to_string()];
        let reading = house.entity(id).and_then(|e| {
            let age = e.state.last_update.map(|t| house.now.saturating_sub(t));
            sensor_value(e).map(|v| (v, age))
        });
        return match reading {
            Some((value, age)) => answer(
                match house.lang {
                    Lang::Fr => {
                        format!("Il fait {} dehors{}.", temp(value), age_note(age, house))
                    }
                    Lang::En => format!(
                        "It's {} outside{}.",
                        temp_in(value, house.lang),
                        age_note(age, house)
                    ),
                },
                trace,
            ),
            None => answer(
                say(
                    house,
                    "Pas de température extérieure.",
                    "No outdoor temperature.",
                ),
                trace,
            ),
        };
    }
    let candidates: Vec<String> = house
        .areas
        .iter()
        .filter(|a| room_reading(house, &a.id).is_some())
        .map(|a| a.id.clone())
        .collect();
    let none = say(
        house,
        "Pas de thermomètre dans la maison.",
        "No thermometer in the house.",
    );
    let (area, how) = match pick_room(req, room, &candidates, none, house) {
        Ok(found) => found,
        Err(plan) => return (*plan).with_trace(trace),
    };
    let name = house.area_name(&area).to_string();
    trace.room = Some(area.clone());
    trace.room_how = Some(how);
    match room_reading(house, &area) {
        Some((value, age, source)) => {
            trace.entities = vec![source];
            trace.value = Some(temp(value));
            answer(
                format!(
                    "{} {} {}{}.",
                    say(house, "Il fait", "It's"),
                    temp_in(value, house.lang),
                    place(&name, house.lang),
                    age_note(age, house)
                ),
                trace,
            )
        }
        None => answer(
            match house.lang {
                Lang::Fr => format!("Pas de thermomètre {}.", dans(&name)),
                Lang::En => format!("No thermometer {}.", place(&name, house.lang)),
            },
            trace,
        ),
    }
}

/// The most a setpoint may differ from the room's reading before the assistant asks.
pub const MAX_FROM_READING: f64 = 3.0;

/// The step when no device says.
const DEFAULT_STEP: f64 = 0.5;

/// The zone's step and range over its climate devices: the coarsest step and the narrowest
/// range.
fn zone_limits(zone: &ZoneView, house: &House) -> (f64, Option<f64>, Option<f64>) {
    let caps = zone
        .entities
        .iter()
        .filter_map(|id| house.entity(id))
        .map(|e| &e.capabilities);
    let (mut step, mut min, mut max): (Option<f64>, Option<f64>, Option<f64>) = (None, None, None);
    for c in caps {
        if let Some(s) = c.temp_step.filter(|s| *s > 0.0) {
            step = Some(step.map_or(s, |x| x.max(s)));
        }
        if let Some(m) = c.min_temp {
            min = Some(min.map_or(m, |x| x.max(m)));
        }
        if let Some(m) = c.max_temp {
            max = Some(max.map_or(m, |x| x.min(m)));
        }
    }
    (step.unwrap_or(DEFAULT_STEP), min, max)
}

/// Down or up to the step; a value already on it (within float noise) stays.
fn round_down(value: f64, step: f64) -> f64 {
    clean((value / step + 1e-9).floor() * step)
}

fn round_up(value: f64, step: f64) -> f64 {
    clean((value / step - 1e-9).ceil() * step)
}

fn clean(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn round_to(value: f64, step: f64) -> f64 {
    ((value / step).round() * step * 100.0).round() / 100.0
}

/// An example sentence for a room: "mets le salon à 19", "set the Kitchen to 19".
fn example_for(name: &str, lang: Lang) -> String {
    match lang {
        Lang::Fr => format!("mets {} à 19", le(name).to_lowercase()),
        Lang::En => format!("set {} to 19", the(name)),
    }
}

/// `why`, then asks for a temperature with an example: "…: tell me a temperature, for
/// example "set the Kitchen to 19"."
fn ask_value_after(why: &str, name: &str, lang: Lang) -> String {
    match lang {
        Lang::Fr => format!(
            "{why} : dis-moi une température, par exemple « {} ».",
            example_for(name, lang)
        ),
        Lang::En => format!(
            "{why}: tell me a temperature, for example \"{}\".",
            example_for(name, lang)
        ),
    }
}

fn ask_value(name: &str, lang: Lang) -> String {
    match lang {
        Lang::Fr => format!(
            "Dis-moi une température, par exemple « {} ».",
            example_for(name, lang)
        ),
        Lang::En => format!(
            "Tell me a temperature, for example \"{}\".",
            example_for(name, lang)
        ),
    }
}

/// The active zones' ids, in the house's room order.
fn active_zones(house: &House) -> Vec<String> {
    house
        .areas
        .iter()
        .filter(|a| {
            house
                .zones
                .iter()
                .any(|z| z.zone == a.id && z.settings.state == ZoneState::Active && !z.orphan)
        })
        .map(|a| a.id.clone())
        .collect()
}

/// The zone of the heating request, or the answer that says why there is none.
fn pick_zone<'a>(
    req: &Request,
    room: &[String],
    house: &House<'a>,
    trace: &mut Trace,
) -> Result<(&'a ZoneView, String), Box<Plan>> {
    let none = say(
        house,
        "Aucun chauffage piloté n'est actif.",
        "No controlled heating is active.",
    );
    let (area, how) = match pick_room(req, room, &active_zones(house), none, house) {
        Ok(found) => found,
        Err(plan) => return Err(Box::new((*plan).with_trace(trace.clone()))),
    };
    let name = house.area_name(&area).to_string();
    trace.room = Some(area.clone());
    trace.room_how = Some(how);
    let Some(zone) = house.zones.iter().find(|z| z.zone == area && !z.orphan) else {
        return Err(Box::new(answer(
            match house.lang {
                Lang::Fr => format!("Pas de chauffage piloté {}.", dans(&name)),
                Lang::En => format!("No controlled heating {}.", place(&name, house.lang)),
            },
            trace.clone(),
        )));
    };
    if zone.settings.state != ZoneState::Active {
        return Err(Box::new(answer(
            match house.lang {
                Lang::Fr => format!("Le chauffage {} n'est pas encore actif.", de(&name)),
                Lang::En => format!("The heating {} isn't active yet.", place(&name, house.lang)),
            },
            trace.clone(),
        )));
    }
    trace.entities = zone.entities.clone();
    Ok((zone, name))
}

fn heating_no_value(req: &Request, room: &[String], house: &House) -> Plan {
    let mut trace = Trace {
        intent: Some("heating_no_value".into()),
        ..Default::default()
    };
    let name = match resolve_room(house, room) {
        RoomMatch::One(area) => house.area_name(&area).to_string(),
        _ => req
            .area
            .clone()
            .or_else(|| active_zones(house).into_iter().next())
            .map_or(say(house, "la pièce", "room"), |a| {
                house.area_name(&a).to_string()
            }),
    };
    trace.room = Some(name.clone());
    answer(ask_value(&name, house.lang), trace)
}

/// What a zone's units are really set to.
#[derive(Clone, Debug, PartialEq)]
enum RealSetpoint {
    Temp(f64),
    /// The units disagree: the distinct values, ascending.
    Differ(Vec<f64>),
    Off,
    Unknown,
}

/// A held unit counts at its own setpoint, any other at the zone's decided setpoint (not the
/// unit's target, which carries the thermometer correction). No unit listed: the zone's.
fn real_setpoint(zone: &ZoneView) -> RealSetpoint {
    let Some(decision) = zone.decision.as_ref() else {
        return RealSetpoint::Unknown;
    };
    let zone_temp = match decision.setpoint {
        Some(Setpoint::Temp(t)) => Some(t),
        _ => None,
    };
    let zone_value = match zone_temp {
        Some(t) => RealSetpoint::Temp(t),
        None if decision.setpoint == Some(Setpoint::Off) => RealSetpoint::Off,
        None => RealSetpoint::Unknown,
    };
    if decision.entities.is_empty() {
        return zone_value;
    }
    let mut values: Vec<f64> = Vec::new();
    for target in decision.entities.values() {
        let value = if target.held {
            if target.mode == "off" {
                return RealSetpoint::Off;
            }
            match target.setpoint {
                Some(t) => t,
                None => return RealSetpoint::Off,
            }
        } else {
            match zone_value {
                RealSetpoint::Temp(t) => t,
                ref other => return other.clone(),
            }
        };
        if !values.iter().any(|v| (v - value).abs() < 0.01) {
            values.push(value);
        }
    }
    values.sort_by(f64::total_cmp);
    match values.as_slice() {
        [only] => RealSetpoint::Temp(*only),
        _ => RealSetpoint::Differ(values),
    }
}

/// "j'ai froid": a small step for the named room, else for the satellite's room when it has
/// active heating; a satellite in a room without asks which room. Typed with no room: as
/// "monte un peu le chauffage" (the only heated room, else a question).
fn feel(req: &Request, warmer: bool, room: &[String], house: &House) -> Plan {
    let change = Change::Relative(if warmer { LITTLE } else { -LITTLE });
    let mut next = req.clone();
    next.intent = Intent::SetTemp {
        change,
        room: room.to_vec(),
    };
    if next.area.is_none()
        && room.is_empty()
        && let Some(heard) = &req.speaker_room
    {
        let zones = active_zones(house);
        if zones.contains(heard) {
            next.area = Some(heard.clone());
        } else if !zones.is_empty() {
            let trace = Trace {
                intent: Some("set_temp".into()),
                ..Default::default()
            };
            return ask_room(&next, &zones, house, trace);
        }
    }
    set_temp(&next, change, room, house)
}

fn set_temp(req: &Request, change: Change, room: &[String], house: &House) -> Plan {
    let mut trace = Trace {
        intent: Some("set_temp".into()),
        ..Default::default()
    };
    let (zone, name) = match pick_zone(req, room, house, &mut trace) {
        Ok(found) => found,
        Err(plan) => return *plan,
    };
    let real = real_setpoint(zone);
    let from = match real {
        RealSetpoint::Temp(t) => Some(t),
        _ => None,
    };
    let wanted = match change {
        Change::Absolute(t) => {
            trace.value = Some(temp(t));
            t
        }
        Change::Relative(delta) => {
            let base = match real {
                RealSetpoint::Temp(t) => t,
                RealSetpoint::Differ(values) => {
                    let lang = house.lang;
                    let shown = match values.split_last() {
                        Some((last, rest)) if !rest.is_empty() => format!(
                            "{} {} {}",
                            rest.iter()
                                .map(|v| number_in(*v, lang))
                                .collect::<Vec<_>>()
                                .join(", "),
                            say(house, "et", "and"),
                            temp_in(*last, lang)
                        ),
                        _ => String::new(),
                    };
                    let why = match lang {
                        Lang::Fr => format!(
                            "Les appareils {} ont des consignes différentes ({shown})",
                            de(&name)
                        ),
                        Lang::En => format!(
                            "The devices {} have different setpoints ({shown})",
                            place(&name, lang)
                        ),
                    };
                    return answer(ask_value_after(&why, &name, lang), trace);
                }
                RealSetpoint::Off | RealSetpoint::Unknown => {
                    let why = match (real == RealSetpoint::Off, house.lang) {
                        (true, Lang::Fr) => format!("Le chauffage {} est sur arrêt", de(&name)),
                        (false, Lang::Fr) => {
                            format!("Je ne connais pas la consigne {}", de(&name))
                        }
                        (true, Lang::En) => {
                            format!("The heating {} is off", place(&name, house.lang))
                        }
                        (false, Lang::En) => {
                            format!("I don't know the setpoint {}", place(&name, house.lang))
                        }
                    };
                    return answer(ask_value_after(&why, &name, house.lang), trace);
                }
            };
            let sign = if delta < 0.0 { "−" } else { "+" };
            // A step starts from the room's reading when that is already past the setpoint.
            let fresh_reading = zone.reading.filter(|_| {
                zone.reading_age_s
                    .is_none_or(|a| a <= u64::from(house.stale_after_min) * 60)
            });
            let past = fresh_reading.filter(|r| if delta < 0.0 { *r < base } else { *r > base });
            let (start, lead) = match past {
                Some(r) => {
                    let cmp = if delta < 0.0 { "<" } else { ">" };
                    (
                        r,
                        format!("mesure {} {cmp} consigne {} : ", number(r), number(base)),
                    )
                }
                None => (base, String::new()),
            };
            let t = start + delta;
            trace.value = Some(format!(
                "{lead}{} {sign} {} = {}",
                number(start),
                number(delta.abs()),
                temp(t)
            ));
            t
        }
    };
    let (step, min, max) = zone_limits(zone, house);
    // A change rounds towards itself, so a step never rounds back to the setpoint it left.
    let target = match change {
        Change::Relative(delta) if delta < 0.0 => round_down(wanted, step),
        Change::Relative(_) => round_up(wanted, step),
        Change::Absolute(_) => round_to(wanted, step),
    };
    if target != round_to(wanted, 0.01) {
        let shown = trace.value.take().unwrap_or_default();
        trace.value = Some(format!(
            "{shown} → {} (pas de {})",
            temp(target),
            number(step)
        ));
    }
    if min.is_some_and(|m| target < m) || max.is_some_and(|m| target > m) {
        let range = match (min, max) {
            (Some(lo), Some(hi)) => format!("de {} à {}", number(lo), temp(hi)),
            (Some(lo), None) => format!("à partir de {}", temp(lo)),
            (None, Some(hi)) => format!("jusqu'à {}", temp(hi)),
            (None, None) => unreachable!(),
        };
        trace.gate = Some(format!("hors des limites de l'appareil ({range})"));
        let reply = match house.lang {
            Lang::Fr => format!(
                "Le chauffage {} accepte {range} : {} est hors limites.",
                de(&name),
                temp(target)
            ),
            Lang::En => {
                let lang = house.lang;
                let range = match (min, max) {
                    (Some(lo), Some(hi)) => {
                        format!("{} to {}", number_in(lo, lang), temp_in(hi, lang))
                    }
                    (Some(lo), None) => format!("from {}", temp_in(lo, lang)),
                    (None, Some(hi)) => format!("up to {}", temp_in(hi, lang)),
                    (None, None) => unreachable!(),
                };
                format!(
                    "The heating {} accepts {range}: {} is out of range.",
                    place(&name, lang),
                    temp_in(target, lang)
                )
            }
        };
        return answer(reply, trace);
    }
    let in_range = (house.range.min..=house.range.max).contains(&target);
    let near_reading = zone
        .reading
        .is_none_or(|r| (target - r).abs() <= MAX_FROM_READING);
    let range = format!("{}–{}", number(house.range.min), temp(house.range.max));
    if !(in_range && near_reading) && !req.confirmed {
        trace.gate = Some(if in_range {
            format!(
                "à plus de {} de la mesure ({}) : demander",
                temp(MAX_FROM_READING),
                temp(zone.reading.unwrap_or_default())
            )
        } else {
            format!("hors de {range} : demander")
        });
        // A yes applies exactly the value asked about, even if the schedule moves meanwhile.
        let mut request = req.clone();
        request.area = Some(zone.zone.clone());
        request.intent = Intent::SetTemp {
            change: Change::Absolute(target),
            room: Vec::new(),
        };
        return Plan::Confirm {
            question: match house.lang {
                Lang::Fr => format!("Passer {} à {} ?", le(&name), temp(target)),
                Lang::En => format!(
                    "Set the heating {} to {}?",
                    place(&name, house.lang),
                    temp_in(target, house.lang)
                ),
            },
            request,
            trace,
        };
    }
    trace.gate = Some(if req.confirmed {
        "confirmé".into()
    } else {
        format!("dans {range}, près de la mesure : ok")
    });
    Plan::Act {
        actions: vec![Action::Setpoint {
            zone: zone.zone.clone(),
            setpoint: target,
        }],
        done: Done::Setpoint {
            zone: zone.zone.clone(),
            from,
            to: target,
        },
        trace,
    }
}

#[cfg(test)]
mod tests;
