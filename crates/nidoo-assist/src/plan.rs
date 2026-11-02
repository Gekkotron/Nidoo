//! From a sentence and the house to a plan: act, answer, ask or confirm.

use nidoo_api::EntityView;
use nidoo_api::search::fold;
use nidoo_api::{Category, CategoryAction, Target, category_members};
use nidoo_core::model::{Command, EntityCategory, Kind, StateValue};

use crate::house::House;
use crate::intent::{Intent, parse};
use crate::reply::{dans, de, number, temp};
use crate::room::{RoomMatch, name_words, resolve_room};
use crate::text::{Word, words};
use crate::view::Trace;

/// The most choices a question offers.
pub const MAX_CHOICES: usize = 6;

/// An understood intent with what is already resolved: the area and lights picked from a
/// question, and whether the safety question was answered yes.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub intent: Intent,
    pub area: Option<String>,
    pub lights: Option<Vec<String>>,
    pub confirmed: bool,
}

impl Request {
    pub fn new(intent: Intent) -> Self {
        Self {
            intent,
            area: None,
            lights: None,
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
        _ => not_understood(house),
    }
}

fn not_understood(_house: &House) -> Plan {
    Plan::NotUnderstood {
        reply: "Je n'ai pas compris.".into(),
        examples: Vec::new(),
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

/// A light whose name holds every name word (a plural word also matches its singular).
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
        Err(plan) => return plan.with_trace(trace),
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

/// The room of a request: the answered one, else the one the words name, else the only
/// candidate. `Err` is the plan to return instead: a question or a plain answer (`none` when
/// there is no candidate at all).
fn pick_room(
    req: &Request,
    room: &[String],
    candidates: &[String],
    none: String,
    house: &House,
) -> Result<(String, String), Plan> {
    if let Some(area) = &req.area {
        return Ok((area.clone(), "choix".into()));
    }
    match resolve_room(house, room) {
        RoomMatch::One(area) => Ok((area, format!("mots : {}", room.join(", ")))),
        RoomMatch::Many(areas) => Err(ask_room(req, &areas, house, Trace::default())),
        RoomMatch::Unknown => Err(answer(
            format!("Je ne connais pas de pièce « {} ».", room.join(" ")),
            Trace::default(),
        )),
        RoomMatch::NoRoomSaid => match candidates {
            [] => Err(answer(none, Trace::default())),
            [one] => Ok((one.clone(), "seule pièce possible".into())),
            _ => Err(ask_room(req, candidates, house, Trace::default())),
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
        Err(plan) => return plan.with_trace(trace),
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
}
