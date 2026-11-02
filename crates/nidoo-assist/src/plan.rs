//! From a sentence and the house to a plan: act, answer, ask or confirm.

use nidoo_api::EntityView;
use nidoo_api::search::fold;
use nidoo_api::{Category, CategoryAction, Target, category_members};
use nidoo_core::model::Command;

use crate::house::House;
use crate::intent::{Intent, parse};
use crate::reply::{dans, de, number};
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
    let (area, how) = match &req.area {
        Some(area) => (area.clone(), "choix".to_string()),
        None => match resolve_room(house, room) {
            RoomMatch::One(area) => (area, format!("mots : {}", room.join(", "))),
            RoomMatch::Many(areas) => return ask_room(req, &areas, house, trace),
            RoomMatch::Unknown => {
                return answer(
                    format!("Je ne connais pas de pièce « {} ».", room.join(" ")),
                    trace,
                );
            }
            RoomMatch::NoRoomSaid => {
                let all = light_members(on, None, house);
                let candidates: Vec<&EntityView> = if names.is_empty() {
                    all
                } else {
                    all.into_iter().filter(|l| named(l, names)).collect()
                };
                let areas = areas_of(&candidates, house);
                match areas.as_slice() {
                    [] if names.is_empty() => {
                        return answer("Pas de lumière dans la maison.".into(), trace);
                    }
                    [] => {
                        return answer(
                            format!("Je ne trouve pas de lumière « {} ».", names.join(" ")),
                            trace,
                        );
                    }
                    [one] => (one.clone(), "seule pièce possible".to_string()),
                    _ => return ask_room(req, &areas, house, trace),
                }
            }
        },
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
}
