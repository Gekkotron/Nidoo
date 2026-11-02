//! A question left pending, and reading a typed answer to it.

use crate::house::House;
use crate::intent::parse;
use crate::lexicon::{Class, class};
use crate::plan::{Plan, Request, plan_request};
use crate::reply::{dans, de, le};
use crate::room::name_words;
use crate::text::{Word, words};

/// What the hub keeps while a question waits for its answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Pending {
    /// Labels and the request each one resolves to.
    Choose(Vec<(String, Request)>),
    /// The request to run, confirmed, on a yes.
    Confirm(Request),
}

impl Pending {
    /// The pending question a plan leaves, if any.
    pub fn of(plan: &Plan) -> Option<Pending> {
        match plan {
            Plan::Ask { choices, .. } => Some(Pending::Choose(choices.clone())),
            Plan::Confirm { request, .. } => Some(Pending::Confirm(request.clone())),
            _ => None,
        }
    }

    /// The request an answer leads to; `None` for a no, or a choice out of range.
    pub fn resolve(&self, answer: PendingAnswer) -> Option<Request> {
        match (self, answer) {
            (Pending::Choose(choices), PendingAnswer::Choice(i)) => {
                choices.get(i).map(|(_, r)| r.clone())
            }
            (Pending::Confirm(request), PendingAnswer::Yes) => {
                let mut request = request.clone();
                request.confirmed = true;
                Some(request)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingAnswer {
    Choice(usize),
    Yes,
    No,
}

const YES: &[&str] = &[
    "oui",
    "ouais",
    "ok",
    "okay",
    "vas",
    "y",
    "d",
    "accord",
    "daccord",
    "confirme",
    "confirmer",
    "valide",
    "go",
    "bien",
    "sur",
    "parfait",
    "merci",
    "stp",
    "svp",
];
const NO: &[&str] = &[
    "non", "nan", "annule", "annuler", "laisse", "tomber", "rien", "merci", "stp", "svp",
];

/// Reads a typed sentence as the answer to `pending`: a yes / no to a confirmation, or the
/// one choice it names. `None` means the sentence is a new request.
pub fn answer_text(sentence: &str, pending: &Pending) -> Option<PendingAnswer> {
    let ws = words(sentence);
    let texts: Vec<&str> = ws.iter().filter_map(Word::text).collect();
    if texts.is_empty() && ws.len() != 1 {
        return None;
    }
    match pending {
        Pending::Confirm(_) => {
            let all_in =
                |set: &[&str]| ws.len() == texts.len() && texts.iter().all(|w| set.contains(w));
            if texts.iter().any(|w| {
                matches!(
                    *w,
                    "oui"
                        | "ouais"
                        | "ok"
                        | "okay"
                        | "vas"
                        | "accord"
                        | "daccord"
                        | "confirme"
                        | "valide"
                        | "go"
                        | "parfait"
                )
            }) && all_in(YES)
            {
                Some(PendingAnswer::Yes)
            } else if texts
                .iter()
                .any(|w| matches!(*w, "non" | "nan" | "annule" | "annuler" | "laisse"))
                && all_in(NO)
            {
                Some(PendingAnswer::No)
            } else {
                None
            }
        }
        Pending::Choose(choices) => {
            if let [Word::Number(n)] = ws.as_slice() {
                let i = *n as usize;
                return (n.fract() == 0.0 && i >= 1 && i <= choices.len())
                    .then(|| PendingAnswer::Choice(i - 1));
            }
            let said: Vec<String> = texts
                .iter()
                .filter(|w| !matches!(class(w), Some(Class::Linker | Class::Noise)))
                .map(|w| w.to_string())
                .collect();
            if said.is_empty() || ws.len() != texts.len() {
                return None;
            }
            let matching: Vec<usize> = choices
                .iter()
                .enumerate()
                .filter(|(_, (label, _))| {
                    let label = name_words(label);
                    said.iter().all(|w| label.contains(w))
                })
                .map(|(i, _)| i)
                .collect();
            match matching.as_slice() {
                [one] => Some(PendingAnswer::Choice(*one)),
                _ => None,
            }
        }
    }
}

/// Plans a sentence without ever building examples (which would recurse); `None` when it is
/// not understood.
fn try_plan(sentence: &str, house: &House) -> Option<Plan> {
    parse(&words(sentence)).map(|intent| plan_request(&Request::new(intent), house))
}

/// Up to three sentences the assistant understands, built from this house's own rooms: a
/// room with lights, an active heating zone (another room when there is one) and a room with
/// a temperature reading.
pub fn examples(house: &House) -> Vec<String> {
    let names: Vec<&str> = house.areas.iter().map(|a| a.name.as_str()).collect();
    let lights = |n: &&str| format!("allume la lumière {}", de(n));
    let heat = |n: &&str| format!("mets {} à 19", le(n));
    let read = |n: &&str| format!("quelle température fait-il {} ?", dans(n));
    let mut out = Vec::new();
    let light_room = names
        .iter()
        .find(|n| matches!(try_plan(&lights(n), house), Some(Plan::Act { .. })));
    out.extend(light_room.map(lights));
    let heat_rooms: Vec<&&str> = names
        .iter()
        .filter(|n| {
            matches!(
                try_plan(&heat(n), house),
                Some(Plan::Act { .. } | Plan::Confirm { .. })
            )
        })
        .collect();
    let heat_room = heat_rooms
        .iter()
        .find(|n| Some(**n) != light_room)
        .or(heat_rooms.first());
    out.extend(heat_room.map(|n| heat(n)));
    let read_room = names.iter().find(|n| {
        matches!(
            try_plan(&read(n), house),
            Some(Plan::Answer { ref reply, .. }) if reply.starts_with("Il fait")
        )
    });
    out.extend(read_room.map(read));
    out.truncate(3);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;
    use crate::intent::{Change, Intent};
    use crate::plan::Request;

    fn req(area: &str) -> Request {
        let mut r = Request::new(Intent::SetTemp {
            change: Change::Absolute(19.0),
            room: vec![],
        });
        r.area = Some(area.into());
        r
    }

    fn rooms() -> Pending {
        Pending::Choose(vec![
            ("Chambre Léa".into(), req("chambre_lea")),
            ("Chambre Tom".into(), req("chambre_tom")),
        ])
    }

    fn confirm() -> Pending {
        Pending::Confirm(req("salon"))
    }

    #[test]
    fn typed_yes_no() {
        for yes in ["oui", "Oui !", "vas-y", "d'accord", "ok", "oui merci"] {
            assert_eq!(
                answer_text(yes, &confirm()),
                Some(PendingAnswer::Yes),
                "{yes}"
            );
        }
        for no in ["non", "annule", "Non merci", "laisse tomber"] {
            assert_eq!(answer_text(no, &confirm()), Some(PendingAnswer::No), "{no}");
        }
        assert_eq!(answer_text("oui", &rooms()), None);
        assert_eq!(answer_text("non", &rooms()), None);
        assert_eq!(answer_text("oui mets le salon à 25", &confirm()), None);
    }

    #[test]
    fn typed_choice() {
        assert_eq!(answer_text("Léa", &rooms()), Some(PendingAnswer::Choice(0)));
        assert_eq!(
            answer_text("la chambre de Léa", &rooms()),
            Some(PendingAnswer::Choice(0))
        );
        assert_eq!(answer_text("tom", &rooms()), Some(PendingAnswer::Choice(1)));
        assert_eq!(answer_text("2", &rooms()), Some(PendingAnswer::Choice(1)));
        // Both labels hold "chambre": not an answer.
        assert_eq!(answer_text("la chambre", &rooms()), None);
        assert_eq!(answer_text("3", &rooms()), None);
        assert_eq!(answer_text("Léa", &confirm()), None);
    }

    #[test]
    fn other_sentence_is_new() {
        assert_eq!(answer_text("il fait combien dehors ?", &rooms()), None);
        assert_eq!(answer_text("il fait combien dehors ?", &confirm()), None);
        assert_eq!(answer_text("allume la chambre de Léa", &rooms()), None);
    }

    #[test]
    fn examples_use_real_names() {
        let f = fixture::house();
        let ex = examples(&f.house());
        assert!(!ex.is_empty() && ex.len() <= 3, "{ex:?}");
        for e in &ex {
            assert!(e.contains("Salon") || e.contains("Chambre Léa"), "{e}");
            for absent in ["Cuisine", "Garage", "Chambre Tom"] {
                assert!(!e.contains(absent), "{e}");
            }
            // Every example is understood.
            assert!(
                !matches!(
                    crate::plan::plan(e, &f.house()),
                    crate::plan::Plan::NotUnderstood { .. }
                ),
                "{e}"
            );
        }
    }

    #[test]
    fn empty_house_has_no_examples() {
        let mut f = fixture::house();
        f.areas.clear();
        f.entities.clear();
        f.zones.clear();
        assert!(examples(&f.house()).is_empty());
    }

    #[test]
    fn a_room_name_the_parser_rejects_is_skipped() {
        // "volets" is a device word: a sentence naming this room is not understood.
        let mut f = fixture::house();
        f.areas.insert(0, fixture::area("volets", "Volets"));
        f.entities
            .push(fixture::light("light_volets", "Lampe", "volets", false));
        let ex = examples(&f.house());
        assert!(ex.iter().all(|e| !e.contains("Volets")), "{ex:?}");
    }
}
