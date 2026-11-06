//! From a sentence and the house to a plan: act, answer, ask or confirm.

use nidoo_api::EntityView;
use nidoo_api::search::fold;
use nidoo_api::{Category, CategoryAction, Target, category_members, commandable};
use nidoo_climate::doc::{Setpoint, ZoneState};
use nidoo_climate::view::ZoneView;
use nidoo_core::model::{Command, EntityCategory, Kind, StateValue, SwitchType};

use crate::house::House;
use crate::intent::{Change, Intent, parse};
use crate::reply::{cannot, dans, de, le, number, temp};
use crate::room::{RoomMatch, name_words, resolve_room};
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
    pub confirmed: bool,
}

impl Request {
    pub fn new(intent: Intent) -> Self {
        Self {
            intent,
            area: None,
            lights: None,
            covers: None,
            switches: None,
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
}

/// What the actions do, for the reply. `names` are in the actions' order.
#[derive(Clone, Debug, PartialEq)]
pub enum Done {
    Lights {
        area: String,
        on: bool,
        names: Vec<String>,
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
        examples: Vec<String>,
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
    let ws = words(sentence);
    let mut plan = match parse(&ws) {
        Some(intent) => plan_request(&Request::new(intent), house),
        None => not_understood(house),
    };
    plan.trace_mut().words = ws.iter().map(shown).collect();
    plan
}

/// Plans an understood request, fresh or resolved by an answer.
pub fn plan_request(req: &Request, house: &House) -> Plan {
    match &req.intent {
        Intent::Lights {
            on, names, room, ..
        } => lights(req, *on, names, room, house),
        Intent::ReadTemp { room, outdoor } => read_temp(req, room, *outdoor, house),
        Intent::SetTemp { change, room } => set_temp(req, *change, room, house),
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
            "Pour une position, dites par exemple « ouvre les volets à 30 % ».".into(),
            Trace {
                intent: Some("covers_hint".into()),
                ..Default::default()
            },
        ),
        // Planned in the next commit.
        Intent::Media(_)
        | Intent::Volume { .. }
        | Intent::Mute { .. }
        | Intent::MediaHint
        | Intent::VolumeRange => not_understood(house),
        Intent::CoverPositionRange => answer(
            "La position va de 0 à 100 %.".into(),
            Trace {
                intent: Some("covers_range".into()),
                ..Default::default()
            },
        ),
    }
}

fn not_understood(house: &House) -> Plan {
    Plan::NotUnderstood {
        reply: "Je n'ai pas compris.".into(),
        examples: crate::pending::examples(house),
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
            (house.area_name(id).to_string(), next)
        })
        .collect();
    Plan::Ask {
        question: "Dans quelle pièce ?".into(),
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

fn lights(req: &Request, on: bool, names: &[String], room: &[String], house: &House) -> Plan {
    let mut trace = Trace {
        intent: Some(if on { "lights_on" } else { "lights_off" }.into()),
        ..Default::default()
    };
    // Words before a linker that name no light but do name a room are the room
    // ("allume le salon" said with a linker after it).
    let (names, room) = if room.is_empty()
        && !names.is_empty()
        && !light_members(on, None, house)
            .iter()
            .any(|l| named(l, names))
        && matches!(
            resolve_room(house, names),
            RoomMatch::One(_) | RoomMatch::Many(_)
        ) {
        (&[][..], names)
    } else {
        (names, room)
    };
    let candidates: Vec<&EntityView> = light_members(on, None, house)
        .into_iter()
        .filter(|l| names.is_empty() || named(l, names))
        .collect();
    let none = if names.is_empty() {
        "Pas de lumière dans la maison.".to_string()
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
        return answer(format!("Pas de lumière {}.", dans(&area_name)), trace);
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
            .take(MAX_CHOICES)
            .map(|l| {
                let mut next = req.clone();
                next.area = Some(area.clone());
                next.lights = Some(vec![l.id.clone()]);
                (l.name.clone(), next)
            })
            .collect();
        return Plan::Ask {
            question: format!("Quelle lumière {} ?", de(&area_name)),
            choices,
            trace,
        };
    }
    let command = if on {
        Command::TurnOn {
            brightness: None,
            color: None,
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
        },
        trace,
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
    // Words that name no shutter but do name a room are the room ("ferme le salon" form).
    let (names, room) = if room.is_empty()
        && !names.is_empty()
        && !cover_members(None, house).iter().any(|c| named(c, names))
        && matches!(
            resolve_room(house, names),
            RoomMatch::One(_) | RoomMatch::Many(_)
        ) {
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
            return answer("Pas de volet dans la maison.".into(), trace);
        }
        (None, members)
    } else {
        let candidates: Vec<&EntityView> = cover_members(None, house)
            .into_iter()
            .filter(|c| names.is_empty() || named(c, names))
            .collect();
        let none = if names.is_empty() {
            "Pas de volet dans la maison.".to_string()
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
            return answer(format!("Pas de volet {}.", dans(&area_name)), trace);
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
                question: format!("Quel volet {} ?", de(&area_name)),
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
        return answer(cannot(&unable, open, position), trace);
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
    let (noun, intent) = match what {
        SwitchType::Tv => ("télé", "tv"),
        _ => ("prise", "plugs"),
    };
    let mut trace = Trace {
        intent: Some(format!("{intent}_{}", if on { "on" } else { "off" })),
        ..Default::default()
    };
    // Words that name no device but do name a room are the room.
    let (names, room) = if room.is_empty()
        && !names.is_empty()
        && !switch_members(what, None, house)
            .iter()
            .any(|s| named(s, names))
        && matches!(
            resolve_room(house, names),
            RoomMatch::One(_) | RoomMatch::Many(_)
        ) {
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
            return answer(format!("Pas de {noun} dans la maison."), trace);
        }
        (None, members)
    } else {
        let candidates: Vec<&EntityView> = switch_members(what, None, house)
            .into_iter()
            .filter(|s| names.is_empty() || named(s, names))
            .collect();
        let none = if names.is_empty() {
            format!("Pas de {noun} dans la maison.")
        } else {
            format!("Je ne trouve pas de {noun} « {} ».", names.join(" "))
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
            return answer(format!("Pas de {noun} {}.", dans(&area_name)), trace);
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
                question: format!("Quelle {noun} {} ?", de(&area_name)),
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
            format!("Je ne connais pas de pièce « {} ».", room.join(" ")),
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
                format!("Il fait {} dehors{}.", temp(value), age_note(age, house)),
                trace,
            ),
            None => answer("Pas de température extérieure.".into(), trace),
        };
    }
    let candidates: Vec<String> = house
        .areas
        .iter()
        .filter(|a| room_reading(house, &a.id).is_some())
        .map(|a| a.id.clone())
        .collect();
    let none = "Pas de thermomètre dans la maison.".to_string();
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
                    "Il fait {} {}{}.",
                    temp(value),
                    dans(&name),
                    age_note(age, house)
                ),
                trace,
            )
        }
        None => answer(format!("Pas de thermomètre {}.", dans(&name)), trace),
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

/// An example sentence for a room: "mets le salon à 19".
fn example_for(name: &str) -> String {
    format!("mets {} à 19", le(name).to_lowercase())
}

fn ask_value(name: &str) -> String {
    format!(
        "Dis-moi une température, par exemple « {} ».",
        example_for(name)
    )
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
    let none = "Aucun chauffage piloté n'est actif.".to_string();
    let (area, how) = match pick_room(req, room, &active_zones(house), none, house) {
        Ok(found) => found,
        Err(plan) => return Err(Box::new((*plan).with_trace(trace.clone()))),
    };
    let name = house.area_name(&area).to_string();
    trace.room = Some(area.clone());
    trace.room_how = Some(how);
    let Some(zone) = house.zones.iter().find(|z| z.zone == area && !z.orphan) else {
        return Err(Box::new(answer(
            format!("Pas de chauffage piloté {}.", dans(&name)),
            trace.clone(),
        )));
    };
    if zone.settings.state != ZoneState::Active {
        return Err(Box::new(answer(
            format!("Le chauffage {} n'est pas encore actif.", de(&name)),
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
            .map_or("la pièce".into(), |a| house.area_name(&a).to_string()),
    };
    trace.room = Some(name.clone());
    answer(ask_value(&name), trace)
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
                    let shown = match values.split_last() {
                        Some((last, rest)) if !rest.is_empty() => format!(
                            "{} et {}",
                            rest.iter()
                                .map(|v| number(*v))
                                .collect::<Vec<_>>()
                                .join(", "),
                            temp(*last)
                        ),
                        _ => String::new(),
                    };
                    return answer(
                        format!(
                            "Les appareils {} ont des consignes différentes ({shown}) : dis-moi une température, par exemple « {} ».",
                            de(&name),
                            example_for(&name)
                        ),
                        trace,
                    );
                }
                RealSetpoint::Off | RealSetpoint::Unknown => {
                    let why = if real == RealSetpoint::Off {
                        format!("Le chauffage {} est sur arrêt", de(&name))
                    } else {
                        format!("Je ne connais pas la consigne {}", de(&name))
                    };
                    return answer(
                        format!(
                            "{why} : dis-moi une température, par exemple « {} ».",
                            example_for(&name)
                        ),
                        trace,
                    );
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
        return answer(
            format!(
                "Le chauffage {} accepte {range} : {} est hors limites.",
                de(&name),
                temp(target)
            ),
            trace,
        );
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
            question: format!("Passer {} à {} ?", le(&name), temp(target)),
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
mod tests {
    use super::*;
    use crate::fixture;
    use nidoo_core::model::Command;

    fn p(s: &str) -> Plan {
        let f = fixture::house();
        plan(s, &f.house())
    }

    fn on() -> Command {
        Command::TurnOn {
            brightness: None,
            color: None,
            effect: None,
        }
    }

    fn lights_of(plan: &Plan) -> Vec<(String, Command)> {
        match plan {
            Plan::Act { actions, .. } => actions
                .iter()
                .map(|a| match a {
                    Action::Light { entity, command } => (entity.clone(), command.clone()),
                    other => panic!("not a light: {other:?}"),
                })
                .collect(),
            other => panic!("not an act: {other:?}"),
        }
    }

    fn labels(plan: &Plan) -> Vec<String> {
        match plan {
            Plan::Ask { choices, .. } => choices.iter().map(|(l, _)| l.clone()).collect(),
            other => panic!("not a question: {other:?}"),
        }
    }

    #[test]
    fn room_lights_are_the_ambiance_set() {
        let got = lights_of(&p("allume la lumière du salon"));
        let ids: Vec<&str> = got.iter().map(|(e, _)| e.as_str()).collect();
        assert_eq!(
            ids,
            [
                "light_plafonnier",
                "light_lampe_table",
                "light_led",
                "light_ambiance_tv"
            ]
        );
        assert!(got.iter().all(|(_, c)| *c == on()));
    }

    #[test]
    fn named_light() {
        assert_eq!(
            lights_of(&p("éteins la led du salon")),
            vec![("light_led".to_string(), Command::TurnOff)]
        );
        assert_eq!(
            lights_of(&p("éteins les lampes du salon")),
            vec![("light_lampe_table".to_string(), Command::TurnOff)]
        );
    }

    #[test]
    fn named_light_not_found_asks() {
        let plan = p("éteins le lustre du salon");
        assert_eq!(
            labels(&plan),
            ["Plafonnier", "Lampe table", "Led", "Ambiance TV"]
        );
        let Plan::Ask { choices, .. } = plan else {
            unreachable!()
        };
        let f = fixture::house();
        assert_eq!(
            lights_of(&plan_request(&choices[2].1, &f.house())),
            vec![("light_led".to_string(), Command::TurnOff)]
        );
    }

    #[test]
    fn no_room_asks_rooms_with_lights() {
        let plan = p("allume la lumière");
        assert_eq!(labels(&plan), ["Salon", "Chambre Léa"]);
        let Plan::Ask { choices, .. } = plan else {
            unreachable!()
        };
        let f = fixture::house();
        assert_eq!(
            lights_of(&plan_request(&choices[1].1, &f.house())),
            vec![("light_lampe_lea".to_string(), on())]
        );
    }

    #[test]
    fn named_light_unique_in_house_needs_no_room() {
        assert_eq!(
            lights_of(&p("éteins la led")),
            vec![("light_led".to_string(), Command::TurnOff)]
        );
    }

    #[test]
    fn room_named_with_a_person() {
        assert_eq!(
            lights_of(&p("allume la chambre de Léa")),
            vec![("light_lampe_lea".to_string(), on())]
        );
    }

    #[test]
    fn room_without_lights_answers() {
        assert!(matches!(
            p("allume le garage"),
            Plan::Answer { reply, .. } if reply == "Pas de lumière dans le Garage."
        ));
    }

    #[test]
    fn ambiguous_room_asks() {
        assert_eq!(
            labels(&p("allume la chambre")),
            ["Chambre Léa", "Chambre Tom"]
        );
    }

    #[test]
    fn unknown_room_answers() {
        assert!(matches!(
            p("allume la cuisine"),
            Plan::Answer { reply, .. } if reply.contains("cuisine")
        ));
    }

    #[test]
    fn not_understood_does_nothing() {
        assert!(matches!(
            p("fais-moi un café"),
            Plan::NotUnderstood { reply, .. } if reply.starts_with("Je n'ai pas compris")
        ));
    }

    #[test]
    fn done_names_the_lights_in_action_order() {
        let Plan::Act { done, .. } = p("éteins la led du salon") else {
            panic!()
        };
        assert_eq!(
            done,
            Done::Lights {
                area: "salon".into(),
                on: false,
                names: vec!["Led".into()]
            }
        );
    }

    fn reply_of(plan: &Plan) -> String {
        match plan {
            Plan::Answer { reply, .. } => reply.clone(),
            other => panic!("not an answer: {other:?}"),
        }
    }

    fn covers_of(plan: &Plan) -> Vec<(String, Command)> {
        match plan {
            Plan::Act { actions, .. } => actions
                .iter()
                .map(|a| match a {
                    Action::Cover { entity, command } => (entity.clone(), command.clone()),
                    other => panic!("not a cover: {other:?}"),
                })
                .collect(),
            other => panic!("not an act: {other:?}"),
        }
    }

    fn ids(got: &[(String, Command)]) -> Vec<&str> {
        got.iter().map(|(e, _)| e.as_str()).collect()
    }

    #[test]
    fn room_shutters_close() {
        let plan = p("ferme les volets du salon");
        let got = covers_of(&plan);
        assert_eq!(ids(&got), ["cover_baie", "cover_fenetre"]);
        assert!(got.iter().all(|(_, c)| *c == Command::Close));
        match plan {
            Plan::Act { done, .. } => assert_eq!(
                done,
                Done::Covers {
                    area: Some("salon".into()),
                    open: false,
                    position: None,
                    names: vec!["Baie".into(), "Fenêtre".into()],
                    unable: vec![],
                }
            ),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn whole_house_needs_no_room() {
        let plan = p("ferme tous les volets");
        let got = covers_of(&plan);
        assert_eq!(ids(&got), ["cover_baie", "cover_fenetre", "cover_lea"]);
        assert!(matches!(
            plan,
            Plan::Act {
                done: Done::Covers { area: None, .. },
                ..
            }
        ));
        let open = covers_of(&p("ouvre tous les volets"));
        assert!(open.iter().all(|(_, c)| *c == Command::Open));
    }

    #[test]
    fn the_house_said_is_the_whole_house() {
        for sentence in [
            "ferme tous les volets de la maison",
            "ferme les volets de toute la maison",
            "ferme les volets partout",
        ] {
            let plan = p(sentence);
            assert_eq!(
                ids(&covers_of(&plan)),
                ["cover_baie", "cover_fenetre", "cover_lea"],
                "{sentence}"
            );
            assert!(matches!(
                plan,
                Plan::Act {
                    done: Done::Covers { area: None, .. },
                    ..
                }
            ));
        }
    }

    #[test]
    fn no_room_asks_rooms_with_shutters() {
        assert_eq!(labels(&p("ferme les volets")), ["Salon", "Chambre Léa"]);
    }

    #[test]
    fn a_choice_resolves_the_room() {
        let choice = match p("ferme les volets") {
            Plan::Ask { choices, .. } => choices[1].1.clone(),
            other => panic!("{other:?}"),
        };
        let f = fixture::house();
        let got = covers_of(&plan_request(&choice, &f.house()));
        assert_eq!(ids(&got), ["cover_lea"]);
    }

    #[test]
    fn named_shutter() {
        let got = covers_of(&p("ferme le volet baie du salon"));
        assert_eq!(ids(&got), ["cover_baie"]);
    }

    #[test]
    fn shutter_named_after_a_linker() {
        // "de la baie" names no room but does name a shutter.
        let got = covers_of(&p("ferme le volet de la baie"));
        assert_eq!(ids(&got), ["cover_baie"]);
        assert_eq!(
            reply_of(&p("ferme le volet de la cave")),
            "Je ne connais pas de pièce « cave »."
        );
    }

    #[test]
    fn unknown_named_shutter_asks_which() {
        let plan = p("ferme le volet porte du salon");
        match &plan {
            Plan::Ask { question, .. } => assert_eq!(question, "Quel volet du Salon ?"),
            other => panic!("{other:?}"),
        }
        assert_eq!(labels(&plan), ["Baie", "Fenêtre"]);
    }

    #[test]
    fn position_sets_position() {
        let got = covers_of(&p("ouvre les volets du salon à 30 %"));
        assert_eq!(ids(&got), ["cover_baie", "cover_fenetre"]);
        assert!(
            got.iter()
                .all(|(_, c)| *c == Command::SetPosition { position: 30 })
        );
    }

    #[test]
    fn position_on_open_close_only_room_answers() {
        let reply = reply_of(&p("ouvre les volets de la chambre de léa à 30 %"));
        assert!(
            reply.contains("ne sait pas se mettre à une position"),
            "{reply}"
        );
    }

    #[test]
    fn position_with_some_unable_acts_on_the_others() {
        let plan = p("ouvre tous les volets à 30 %");
        assert_eq!(ids(&covers_of(&plan)), ["cover_baie", "cover_fenetre"]);
        match plan {
            Plan::Act {
                done: Done::Covers { unable, .. },
                ..
            } => {
                assert_eq!(unable, ["Volet"])
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn close_with_position_gives_the_hint() {
        assert!(reply_of(&p("ferme les volets à 30 %")).starts_with("Pour une position"));
        assert_eq!(
            reply_of(&p("ouvre les volets à 130 %")),
            "La position va de 0 à 100 %."
        );
    }

    #[test]
    fn room_without_shutters_answers() {
        assert_eq!(
            reply_of(&p("ferme les volets du bureau")),
            "Pas de volet dans le Bureau."
        );
    }

    #[test]
    fn zone_reading() {
        assert_eq!(
            reply_of(&p("quelle température fait-il dans le salon ?")),
            "Il fait 18,2 °C dans le Salon."
        );
    }

    #[test]
    fn thermometer_without_zone() {
        let r = reply_of(&p("il fait combien dans le bureau ?"));
        assert_eq!(r, "Il fait 20,1 °C dans le Bureau.");
    }

    #[test]
    fn outdoor() {
        assert_eq!(
            reply_of(&p("il fait combien dehors ?")),
            "Il fait 7 °C dehors."
        );
        let mut f = fixture::house();
        let mut garden = fixture::thermometer("sensor_outdoor", "garage", 5.5);
        garden.name = "Extérieur".into();
        f.entities.push(garden);
        f.outdoor_entity = Some("sensor_outdoor".into());
        assert_eq!(
            reply_of(&plan("il fait combien dehors ?", &f.house())),
            "Il fait 5,5 °C dehors."
        );
    }

    #[test]
    fn stale_reading_says_age() {
        let mut f = fixture::house();
        f.zone_mut("salon").reading_age_s = Some(3 * 3600);
        f.entity_mut("sensor_salon_temperature").state.last_update = Some(fixture::NOW - 3 * 3600);
        let r = reply_of(&plan("quelle température dans le salon ?", &f.house()));
        assert!(r.contains("18,2 °C") && r.contains("il y a 3 h"), "{r}");
        f.entity_mut("sensor_bureau_temperature").state.last_update = Some(fixture::NOW - 90 * 60);
        let r = reply_of(&plan("il fait combien dans le bureau ?", &f.house()));
        assert!(r.contains("20,1 °C") && r.contains("il y a 1 h 30"), "{r}");
    }

    #[test]
    fn zone_without_reading_uses_the_thermometer() {
        let mut f = fixture::house();
        f.zone_mut("salon").reading = None;
        f.entity_mut("sensor_salon_temperature").state.value =
            nidoo_core::model::StateValue::Number(17.9);
        let r = reply_of(&plan("quelle température dans le salon ?", &f.house()));
        assert_eq!(r, "Il fait 17,9 °C dans le Salon.");
    }

    #[test]
    fn no_thermometer() {
        assert_eq!(
            reply_of(&p("il fait combien au garage ?")),
            "Pas de thermomètre dans le Garage."
        );
    }

    #[test]
    fn reading_without_room_asks() {
        assert_eq!(
            labels(&p("quelle température fait-il ?")),
            ["Salon", "Bureau", "Chambre Léa"]
        );
    }

    fn setpoint_of(plan: &Plan) -> (String, f64) {
        match plan {
            Plan::Act { actions, .. } => match actions.as_slice() {
                [Action::Setpoint { zone, setpoint }] => (zone.clone(), *setpoint),
                other => panic!("not one setpoint: {other:?}"),
            },
            other => panic!("not an act: {other:?}"),
        }
    }

    fn confirm_of(plan: &Plan) -> (String, Request) {
        match plan {
            Plan::Confirm {
                question, request, ..
            } => (question.clone(), request.clone()),
            other => panic!("not a confirmation: {other:?}"),
        }
    }

    #[test]
    fn absolute_in_range_acts() {
        let plan = p("mets le salon à 20 degrés");
        assert_eq!(setpoint_of(&plan), ("salon".into(), 20.0));
        let Plan::Act { done, .. } = plan else {
            unreachable!()
        };
        assert_eq!(
            done,
            Done::Setpoint {
                zone: "salon".into(),
                from: Some(19.0),
                to: 20.0
            }
        );
    }

    #[test]
    fn relative_from_decided_setpoint() {
        assert_eq!(
            setpoint_of(&p("baisse un peu le chauffage du salon")),
            ("salon".into(), 17.5)
        );
        // A stale reading is ignored: the step starts from the setpoint.
        let mut f = fixture::house();
        f.zone_mut("chambre_lea").reading_age_s = Some(3 * 3600);
        let plan = plan("monte le chauffage d'un degré chez Léa", &f.house());
        let (question, request) = confirm_of(&plan);
        assert_eq!(question, "Passer la Chambre Léa à 15 °C ?");
        let f = fixture::house();
        let mut yes = request;
        yes.confirmed = true;
        assert_eq!(
            setpoint_of(&plan_request(&yes, &f.house())),
            ("chambre_lea".into(), 15.0)
        );
        assert_eq!(plan.trace().value.as_deref(), Some("14 + 1 = 15 °C"));
    }

    #[test]
    fn outside_range_confirms() {
        let (question, request) = confirm_of(&p("mets le salon à 25"));
        assert_eq!(question, "Passer le Salon à 25 °C ?");
        let f = fixture::house();
        let mut yes = request;
        yes.confirmed = true;
        assert_eq!(
            setpoint_of(&plan_request(&yes, &f.house())),
            ("salon".into(), 25.0)
        );
    }

    #[test]
    fn far_from_reading_confirms() {
        // Within 16–23 °C, but 3,8 °C above the reading of 18,2 °C.
        confirm_of(&p("mets le salon à 22"));
        setpoint_of(&p("mets le salon à 21"));
    }

    #[test]
    fn range_comes_from_the_house() {
        let mut f = fixture::house();
        f.range.max = 25.0;
        f.zone_mut("salon").reading = Some(22.0);
        assert_eq!(
            setpoint_of(&plan("mets le salon à 24", &f.house())),
            ("salon".into(), 24.0)
        );
    }

    #[test]
    fn rounds_to_device_step() {
        assert_eq!(
            setpoint_of(&p("mets le salon à 19,3")),
            ("salon".into(), 19.5)
        );
        let mut f = fixture::house();
        f.entity_mut("climate_salon").capabilities.temp_step = Some(1.0);
        assert_eq!(
            setpoint_of(&plan("mets le salon à 19,4", &f.house())),
            ("salon".into(), 19.0)
        );
    }

    #[test]
    fn outside_device_range_refused() {
        let r = reply_of(&p("mets le salon à 40"));
        assert!(
            r.contains("10") && r.contains("30") && r.contains("40"),
            "{r}"
        );
        let r = reply_of(&p("mets le salon à 40 degrés"));
        assert!(r.contains("30"), "{r}");
    }

    #[test]
    fn no_zone() {
        assert_eq!(
            reply_of(&p("mets le bureau à 19")),
            "Pas de chauffage piloté dans le Bureau."
        );
    }

    #[test]
    fn zone_not_active() {
        assert_eq!(
            reply_of(&p("mets la chambre de Tom à 19")),
            "Le chauffage de la Chambre Tom n'est pas encore actif."
        );
    }

    #[test]
    fn off_setpoint_needs_absolute() {
        let mut f = fixture::house();
        f.zone_mut("salon").decision.as_mut().unwrap().setpoint =
            Some(nidoo_climate::doc::Setpoint::Off);
        let r = reply_of(&plan("monte le chauffage du salon", &f.house()));
        assert!(r.contains("température"), "{r}");
        f.zone_mut("salon").decision = None;
        let r = reply_of(&plan("monte le chauffage du salon", &f.house()));
        assert!(r.contains("température"), "{r}");
        // An absolute value still works.
        setpoint_of(&plan("mets le salon à 19", &f.house()));
    }

    #[test]
    fn heating_without_value() {
        assert_eq!(
            reply_of(&p("allume le chauffage du salon")),
            "Dis-moi une température, par exemple « mets le salon à 19 »."
        );
    }

    #[test]
    fn single_zone_house_needs_no_room() {
        let mut f = fixture::house();
        f.zone_mut("chambre_lea").settings.state = nidoo_climate::doc::ZoneState::Pending;
        assert_eq!(
            setpoint_of(&plan("mets le chauffage à 20", &f.house())),
            ("salon".into(), 20.0)
        );
    }

    #[test]
    fn no_room_asks_active_zones() {
        assert_eq!(
            labels(&p("mets le chauffage à 20")),
            ["Salon", "Chambre Léa"]
        );
    }

    #[test]
    fn light_count_is_not_a_setpoint() {
        let got = lights_of(&p("allume les 3 lumières du salon"));
        assert_eq!(got.len(), 4);
    }

    #[test]
    fn confirmed_value_is_the_one_asked() {
        let mut f = fixture::house();
        f.zone_mut("chambre_lea").reading_age_s = Some(3 * 3600);
        let (_, request) = confirm_of(&plan("monte le chauffage d'un degré chez Léa", &f.house()));
        // The schedule moved on before the answer came.
        f.zone_mut("chambre_lea")
            .decision
            .as_mut()
            .unwrap()
            .setpoint = Some(nidoo_climate::doc::Setpoint::Temp(17.0));
        let mut yes = request;
        yes.confirmed = true;
        assert_eq!(
            setpoint_of(&plan_request(&yes, &f.house())),
            ("chambre_lea".into(), 15.0)
        );
    }

    #[test]
    fn heating_in_the_chambre_of_a_person_named_room() {
        let f = fixture::person_rooms();
        let p = |s: &str| plan(s, &f.house());
        let got = p("augmente le chauffage dans la chambre de léa");
        assert!(acts(&got), "{got:?}");
        assert_eq!(got.trace().room.as_deref(), Some("lea"));
        // A bare "chambre" asks among the rooms that may be bedrooms: never the Salon.
        assert_eq!(
            labels(&p("augmente le chauffage dans la chambre")),
            ["Léa", "Tom"]
        );
        assert_eq!(
            reply_of(&p("augmente le chauffage dans la chambre du salon")),
            "Je ne connais pas de pièce « chambre salon »."
        );
    }

    #[test]
    fn zone_choices_follow_the_room_order() {
        let mut f = fixture::house();
        f.zones.reverse();
        assert_eq!(
            labels(&plan("mets le chauffage à 20", &f.house())),
            ["Salon", "Chambre Léa"]
        );
    }

    fn acts(plan: &Plan) -> bool {
        matches!(plan, Plan::Act { .. } | Plan::Confirm { .. })
    }

    #[test]
    fn statements_and_questions_never_act() {
        for sentence in [
            // C1: a temperature in a statement or a question is not an order.
            "il fait 18 degrés au salon",
            "est-ce que la température du salon est à 20 ?",
            "le chauffage du salon est à 20 ?",
            "le salon est à 19 degrés",
            "quelle est la consigne du salon à 19 ?",
            // C2: real words one edit from a verb.
            "laisse le chauffage allumé dans le salon",
            "le chauffage du salon, laisse-le",
            "laissez le chauffage du salon",
            "peux-tu me montrer la consigne du salon ?",
            // I1: questions and past tense about the lights.
            "est-ce que la lumière du salon est allumée ?",
            "la lumière du salon est éteinte ?",
            "qui a éteint le salon ?",
            "j'ai éteint la lumière du salon",
            // M1: "à" then a signed number.
            "mets le chauffage du salon à -2",
            // I2: a stray number.
            "mets le salon à 19 , 5",
        ] {
            let plan = p(sentence);
            assert!(!acts(&plan), "{sentence:?} → {plan:?}");
            assert!(!matches!(plan, Plan::Ask { .. }), "{sentence:?} → {plan:?}");
        }
    }

    #[test]
    fn polite_requests_still_act() {
        assert_eq!(
            lights_of(&p("peux-tu allumer la lumière du salon ?")).len(),
            4
        );
        assert_eq!(
            setpoint_of(&p("est-ce que tu peux mettre le salon à 20 ?")),
            ("salon".into(), 20.0)
        );
        assert_eq!(lights_of(&p("allume le salon ?")).len(), 4);
        assert_eq!(
            reply_of(&p("est-ce que tu peux me dire la température du salon ?")),
            "Il fait 18,2 °C dans le Salon."
        );
    }

    fn numbered_house() -> fixture::Fixture {
        use nidoo_climate::doc::{Setpoint, ZoneState};
        let mut f = fixture::house();
        f.areas.push(fixture::area("chambre_1", "Chambre 1"));
        f.areas.push(fixture::area("chambre_2", "Chambre 2"));
        f.entities.push(fixture::climate("climate_c1", "chambre_1"));
        f.entities.push(fixture::climate("climate_c2", "chambre_2"));
        f.entities
            .push(fixture::light("light_c2", "Plafonnier", "chambre_2", false));
        for (id, name) in [("chambre_1", "Chambre 1"), ("chambre_2", "Chambre 2")] {
            f.zones.push(fixture::zone(
                id,
                name,
                ZoneState::Active,
                Some(Setpoint::Temp(19.0)),
                Some(19.0),
                &[if id == "chambre_1" {
                    "climate_c1"
                } else {
                    "climate_c2"
                }],
            ));
        }
        f
    }

    #[test]
    fn digits_in_a_room_name_are_the_room() {
        let f = numbered_house();
        let h = f.house();
        assert_eq!(
            setpoint_of(&plan("monte le chauffage de la chambre 2", &h)),
            ("chambre_2".into(), 20.0)
        );
        assert_eq!(
            setpoint_of(&plan("monte la chambre 2 d'un degré", &h)),
            ("chambre_2".into(), 20.0)
        );
        assert_eq!(
            setpoint_of(&plan("mets la chambre 1 à 20", &h)),
            ("chambre_1".into(), 20.0)
        );
        assert_eq!(
            lights_of(&plan("allume la chambre 2", &h)),
            vec![("light_c2".to_string(), on())]
        );
    }

    #[test]
    fn ordinal_indicator_typo_is_degrees() {
        let f = fixture::house();
        let h = f.house();
        assert_eq!(
            setpoint_of(&plan("mets le salon à 20ºC", &h)),
            setpoint_of(&plan("mets le salon à 20°C", &h))
        );
    }

    #[test]
    fn room_right_after_the_verb_with_a_number_is_not_a_light_name() {
        let f = fixture::house();
        let h = f.house();
        for s in ["allume le salon à 20", "éteins le salon à 20"] {
            let plan = plan(s, &h);
            assert!(matches!(plan, Plan::NotUnderstood { .. }), "{s}: {plan:?}");
        }
    }

    #[test]
    fn plain_room_after_the_verb_lights_the_room() {
        let f = fixture::house();
        let h = f.house();
        assert_eq!(
            lights_of(&plan("allume le salon", &h)),
            lights_of(&plan("allume la lumière du salon", &h))
        );
        assert!(!lights_of(&plan("allume le salon", &h)).is_empty());
    }

    #[test]
    fn room_before_a_linker_is_the_room_when_no_light_has_that_name() {
        let f = fixture::house();
        let h = f.house();
        let req = Request::new(Intent::Lights {
            on: true,
            all: false,
            names: vec!["salon".into()],
            room: vec![],
        });
        let plan = plan_request(&req, &h);
        assert_eq!(
            lights_of(&plan),
            lights_of(&p("allume la lumière du salon"))
        );
    }

    #[test]
    fn relative_changes_round_towards_the_change() {
        let mut f = fixture::house();
        f.entity_mut("climate_salon").capabilities.temp_step = Some(1.0);
        f.zone_mut("salon").reading = Some(19.0);
        let h = f.house();
        assert_eq!(
            setpoint_of(&plan("baisse le chauffage du salon", &h)),
            ("salon".into(), 18.0)
        );
        assert_eq!(
            setpoint_of(&plan("monte un peu le chauffage du salon", &h)),
            ("salon".into(), 20.0)
        );
    }

    fn target(setpoint: f64, held: bool) -> nidoo_climate::doc::DeviceTarget {
        nidoo_climate::doc::DeviceTarget {
            setpoint: Some(setpoint),
            mode: "heat".into(),
            held,
        }
    }

    fn salon_with(setpoint: f64, reading: f64, units: &[(&str, f64, bool)]) -> fixture::Fixture {
        let mut f = fixture::house();
        let zone = f.zone_mut("salon");
        zone.reading = Some(reading);
        let d = zone.decision.as_mut().unwrap();
        d.setpoint = Some(Setpoint::Temp(setpoint));
        for (id, t, held) in units {
            d.entities.insert((*id).into(), target(*t, *held));
        }
        f
    }

    #[test]
    fn raise_starts_from_what_a_held_unit_is_set_to() {
        let f = salon_with(15.5, 14.7, &[("climate_salon", 19.0, true)]);
        let got = plan("augmente le chauffage du salon", &f.house());
        let (question, _) = confirm_of(&got);
        assert_eq!(question, "Passer le Salon à 20 °C ?");
        assert_eq!(got.trace().value.as_deref(), Some("19 + 1 = 20 °C"));
    }

    #[test]
    fn done_carries_the_real_setpoint() {
        let f = salon_with(15.5, 19.0, &[("climate_salon", 19.0, true)]);
        let Plan::Act { done, .. } = plan("augmente le chauffage du salon", &f.house()) else {
            panic!("not an act")
        };
        assert_eq!(
            done,
            Done::Setpoint {
                zone: "salon".into(),
                from: Some(19.0),
                to: 20.0
            }
        );
    }

    #[test]
    fn raise_starts_from_a_fresh_reading_past_the_setpoint() {
        let f = salon_with(15.5, 18.0, &[]);
        let got = plan("augmente le chauffage du salon", &f.house());
        assert_eq!(setpoint_of(&got), ("salon".into(), 19.0));
        assert_eq!(
            got.trace().value.as_deref(),
            Some("mesure 18 > consigne 15,5 : 18 + 1 = 19 °C")
        );
    }

    #[test]
    fn lower_starts_from_a_fresh_reading_below_the_setpoint() {
        let f = salon_with(19.0, 17.5, &[]);
        let got = plan("baisse le chauffage du salon", &f.house());
        assert_eq!(setpoint_of(&got), ("salon".into(), 16.5));
        let f = salon_with(17.0, 18.0, &[]);
        let got = plan("baisse le chauffage du salon", &f.house());
        assert_eq!(setpoint_of(&got), ("salon".into(), 16.0));
        assert_eq!(got.trace().value.as_deref(), Some("17 − 1 = 16 °C"));
    }

    #[test]
    fn lower_below_a_reading_below_the_setpoint_says_why() {
        let f = salon_with(16.5, 14.0, &[]);
        let got = plan("baisse le chauffage du salon", &f.house());
        assert_eq!(
            got.trace().value.as_deref(),
            Some("mesure 14 < consigne 16,5 : 14 − 1 = 13 °C")
        );
    }

    #[test]
    fn a_stale_reading_is_ignored() {
        let mut f = salon_with(15.5, 18.0, &[]);
        f.zone_mut("salon").reading_age_s = Some(3 * 3600);
        let got = plan("augmente le chauffage du salon", &f.house());
        assert_eq!(setpoint_of(&got), ("salon".into(), 16.5));
        assert_eq!(got.trace().value.as_deref(), Some("15,5 + 1 = 16,5 °C"));
    }

    #[test]
    fn units_with_different_setpoints_are_not_guessed() {
        let f = salon_with(
            15.5,
            14.7,
            &[
                ("climate_salon", 19.0, true),
                ("climate_salon_2", 15.5, false),
            ],
        );
        let r = reply_of(&plan("augmente le chauffage du salon", &f.house()));
        assert!(
            r.contains("consignes différentes (15,5 et 19 °C)") && r.contains("température"),
            "{r}"
        );
        // An absolute value is unaffected.
        setpoint_of(&plan("mets le salon à 16", &f.house()));
    }

    #[test]
    fn a_unit_off_by_hand_needs_a_value() {
        let mut f = salon_with(15.5, 14.7, &[("climate_salon", 19.0, true)]);
        f.zone_mut("salon")
            .decision
            .as_mut()
            .unwrap()
            .entities
            .get_mut("climate_salon")
            .unwrap()
            .mode = "off".into();
        let r = reply_of(&plan("augmente le chauffage du salon", &f.house()));
        assert!(r.contains("arrêt"), "{r}");
    }

    #[test]
    fn a_little_step_stays_half_a_degree() {
        let f = salon_with(19.0, 19.0, &[]);
        let got = plan("monte un peu le chauffage du salon", &f.house());
        assert_eq!(setpoint_of(&got), ("salon".into(), 19.5));
    }
    fn switches_of(plan: &Plan) -> Vec<(String, Command)> {
        match plan {
            Plan::Act { actions, .. } => actions
                .iter()
                .map(|a| match a {
                    Action::Switch { entity, command } => (entity.clone(), command.clone()),
                    other => panic!("not a switch: {other:?}"),
                })
                .collect(),
            other => panic!("not an act: {other:?}"),
        }
    }

    #[test]
    fn one_tv_turns_on() {
        let plan = p("allume la télé");
        assert_eq!(switches_of(&plan), [("switch_tv".to_string(), on())]);
        assert!(matches!(
            plan,
            Plan::Act {
                done: Done::Switches { what: SwitchType::Tv, area: Some(a), on: true, .. },
                ..
            } if a == "salon"
        ));
        let off = switches_of(&p("éteins la télé du salon"));
        assert_eq!(off, [("switch_tv".to_string(), Command::TurnOff)]);
    }

    #[test]
    fn a_plug_by_room_and_by_name() {
        let by_room = switches_of(&p("éteins la prise du bureau"));
        assert_eq!(
            by_room,
            [("switch_prise_bureau".to_string(), Command::TurnOff)]
        );
        assert_eq!(
            switches_of(&p("allume la prise bureau")),
            [("switch_prise_bureau".to_string(), on())]
        );
        let by_name = p("éteins la prise chargeur");
        assert_eq!(
            switches_of(&by_name),
            [("switch_prise_tom".to_string(), Command::TurnOff)]
        );
    }

    #[test]
    fn all_the_plugs_of_the_house() {
        let plan = p("éteins toutes les prises");
        assert_eq!(
            ids(&switches_of(&plan)),
            ["switch_prise_bureau", "switch_prise_tom"]
        );
        assert!(matches!(
            plan,
            Plan::Act {
                done: Done::Switches {
                    area: None,
                    on: false,
                    ..
                },
                ..
            }
        ));
        assert_eq!(
            ids(&switches_of(&p("allume les 2 prises du bureau"))),
            ["switch_prise_bureau"]
        );
    }

    #[test]
    fn which_plug_and_which_room() {
        assert_eq!(labels(&p("éteins la prise")), ["Bureau", "Chambre Tom"]);
        let mut f = fixture::house();
        f.entities.push(fixture::switch(
            "switch_prise_bureau_2",
            "Chauffage appoint",
            "bureau",
            Some("plug"),
        ));
        let h = f.house();
        let asked = plan("éteins la prise grille du bureau", &h);
        let Plan::Ask {
            question, choices, ..
        } = &asked
        else {
            panic!("{asked:?}")
        };
        assert_eq!(question, "Quelle prise du Bureau ?");
        assert_eq!(
            choices.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(),
            ["Cafetière", "Chauffage appoint"]
        );
        let picked = plan_request(&choices[1].1, &h);
        assert_eq!(
            switches_of(&picked),
            [("switch_prise_bureau_2".to_string(), Command::TurnOff)]
        );
    }

    #[test]
    fn tv_or_plug_not_found() {
        assert_eq!(
            reply_of(&p("éteins la prise du garage")),
            "Pas de prise dans le Garage."
        );
        let by_name = Request::new(Intent::Switches {
            what: SwitchType::Plug,
            on: false,
            all: false,
            names: vec!["grille".into()],
            room: vec![],
        });
        assert_eq!(
            reply_of(&plan_request(&by_name, &fixture::house().house())),
            "Je ne trouve pas de prise « grille »."
        );
        let mut f = fixture::house();
        f.entities.retain(|e| e.id != "switch_tv");
        assert_eq!(
            reply_of(&plan("allume la télé", &f.house())),
            "Pas de télé dans la maison."
        );
        assert_eq!(
            reply_of(&plan("éteins toutes les télés", &f.house())),
            "Pas de télé dans la maison."
        );
    }

    #[test]
    fn untyped_and_siren_switches_are_never_chosen() {
        for sentence in [
            "éteins toutes les prises",
            "éteins les prises du bureau",
            "éteins la prise interrupteur",
            "éteins tout",
            "éteins le salon",
        ] {
            let plan = p(sentence);
            if let Plan::Act { actions, .. } = &plan {
                assert!(
                    actions
                        .iter()
                        .all(|a| !matches!(a, Action::Switch { entity, .. }
                            if entity == "switch_interrupteur" || entity == "switch_sirene")),
                    "{sentence}"
                );
            }
        }
        assert_eq!(
            reply_of(&p("éteins la prise interrupteur")),
            "Je ne connais pas de pièce « interrupteur »."
        );
        assert!(matches!(
            p("éteins la prise sirène"),
            Plan::NotUnderstood { .. }
        ));
        assert_eq!(
            reply_of(&p("éteins la prise du garage")),
            "Pas de prise dans le Garage."
        );
    }

    #[test]
    fn groups_and_virtual_switches_are_never_chosen() {
        let mut f = fixture::house();
        f.entities.push(fixture::switch(
            "group_prises",
            "Prises",
            "bureau",
            Some("plug"),
        ));
        f.entities.push(fixture::switch(
            "virtual_prise",
            "Virtuelle",
            "bureau",
            Some("plug"),
        ));
        let got = switches_of(&plan("éteins toutes les prises", &f.house()));
        assert_eq!(ids(&got), ["switch_prise_bureau", "switch_prise_tom"]);
    }

    #[test]
    fn turning_off_a_room_stays_lights_only() {
        for sentence in [
            "éteins le salon",
            "éteins tout dans le salon",
            "éteins le bureau",
        ] {
            if let Plan::Act { actions, .. } = p(sentence) {
                assert!(
                    actions.iter().all(|a| matches!(a, Action::Light { .. })),
                    "{sentence}"
                );
            }
        }
    }
}
