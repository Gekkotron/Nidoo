//! A question left pending, and reading a typed answer to it.

use crate::lexicon::{Class, class_in};
use crate::plan::{Plan, Request};
use crate::room::{name_words, score};
use crate::text::{Word, number_text, words};
use nidoo_core::flow::Lang;

/// How long a question waits for its answer, in seconds (the hub drops it after that).
pub const PENDING_TTL_S: u64 = 60;

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
/// The words one of which makes a French yes.
const YES_CORE: &[&str] = &[
    "oui", "ouais", "ok", "okay", "vas", "accord", "daccord", "confirme", "valide", "go", "parfait",
];
/// The words one of which makes a French no.
const NO_CORE: &[&str] = &["non", "nan", "annule", "annuler", "laisse"];
const NO: &[&str] = &[
    "non", "nan", "annule", "annuler", "laisse", "tomber", "rien", "merci", "stp", "svp",
];

/// The English yes words: one of [`YES_EN_CORE`] and nothing but these.
const YES_EN: &[&str] = &[
    "yes",
    "yeah",
    "yep",
    "yup",
    "sure",
    "ok",
    "okay",
    "go",
    "ahead",
    "do",
    "it",
    "confirm",
    "fine",
    "right",
    "please",
    "thanks",
    "thank",
    "you",
    "of",
    "course",
    "absolutely",
];
const YES_EN_CORE: &[&str] = &[
    "yes",
    "yeah",
    "yep",
    "yup",
    "sure",
    "ok",
    "okay",
    "go",
    "confirm",
    "absolutely",
];
/// The English no words: one of [`NO_EN_CORE`] and nothing but these.
const NO_EN: &[&str] = &[
    "no", "nope", "nah", "cancel", "never", "mind", "forget", "it", "nothing", "thanks", "thank",
    "you", "please", "stop",
];
const NO_EN_CORE: &[&str] = &["no", "nope", "nah", "cancel", "never", "forget"];

/// Reads a typed sentence as the answer to `pending`: a no to either kind of question, a yes
/// to a confirmation, or the one choice a choice question names. `None` means the sentence is a new request.
pub fn answer_text(sentence: &str, pending: &Pending) -> Option<PendingAnswer> {
    answer_text_in(sentence, pending, Lang::Fr)
}

/// [`answer_text`] for a sentence in `lang`.
pub fn answer_text_in(sentence: &str, pending: &Pending, lang: Lang) -> Option<PendingAnswer> {
    let (yes, yes_core, no, no_core): (&[&str], &[&str], &[&str], &[&str]) = match lang {
        Lang::Fr => (YES, YES_CORE, NO, NO_CORE),
        Lang::En => (YES_EN, YES_EN_CORE, NO_EN, NO_EN_CORE),
    };
    let ws = words(sentence);
    let texts: Vec<&str> = ws.iter().filter_map(Word::text).collect();
    if texts.is_empty() && ws.len() != 1 {
        return None;
    }
    let all_in = |set: &[&str]| ws.len() == texts.len() && texts.iter().all(|w| set.contains(w));
    // A refusal cancels either kind of question; a bare command is not one.
    if texts.iter().any(|w| no_core.contains(w)) && all_in(no) {
        return Some(PendingAnswer::No);
    }
    match pending {
        Pending::Confirm(_) => {
            if texts.iter().any(|w| yes_core.contains(w)) && all_in(yes) {
                Some(PendingAnswer::Yes)
            } else {
                None
            }
        }
        Pending::Choose(choices) => {
            // The said words, numbers kept: "chambre 2", or a bare "2".
            let said: Vec<String> = ws
                .iter()
                .filter_map(|w| match w {
                    Word::Text(t) => Some(t.clone()),
                    Word::Number(n) => Some(number_text(*n)),
                    Word::Signed(_) => None,
                })
                .filter(|w| !matches!(class_in(w, lang), Some(Class::Linker | Class::Noise)))
                .collect();
            if said.is_empty() || ws.iter().any(|w| matches!(w, Word::Signed(_))) {
                return None;
            }
            // The best-named choice, exact before partial ("salon" picks Salon over Salon TV).
            let scores: Vec<u8> = choices
                .iter()
                .map(|(label, _)| score(&said, &name_words(label)))
                .collect();
            let best = scores.iter().copied().max().unwrap_or(0);
            let winners: Vec<usize> = (0..choices.len()).filter(|i| scores[*i] == best).collect();
            match (best, winners.as_slice()) {
                (1.., [one]) => Some(PendingAnswer::Choice(*one)),
                // A bare number no label holds is a position: "2" is the second choice.
                (0, _) => match ws.as_slice() {
                    [Word::Number(n)] if n.fract() == 0.0 && *n >= 1.0 => {
                        let i = *n as usize;
                        (i <= choices.len()).then(|| PendingAnswer::Choice(i - 1))
                    }
                    _ => None,
                },
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn typed_yes_no_in_english() {
        let en = |s: &str, p: &Pending| answer_text_in(s, p, Lang::En);
        for yes in [
            "yes",
            "Yes!",
            "go ahead",
            "ok",
            "sure",
            "yes please",
            "yeah thanks",
        ] {
            assert_eq!(en(yes, &confirm()), Some(PendingAnswer::Yes), "{yes}");
        }
        for no in [
            "no",
            "No thanks",
            "cancel",
            "never mind",
            "forget it",
            "nope",
        ] {
            assert_eq!(en(no, &confirm()), Some(PendingAnswer::No), "{no}");
            assert_eq!(en(no, &rooms()), Some(PendingAnswer::No), "{no}");
        }
        assert_eq!(en("oui", &confirm()), None);
        assert_eq!(en("yes set the kitchen to 25", &confirm()), None);
        assert_eq!(en("2", &rooms()), Some(PendingAnswer::Choice(1)));
        assert_eq!(en("Tom", &rooms()), Some(PendingAnswer::Choice(1)));
        assert_eq!(
            en("in Léa's room", &rooms()),
            Some(PendingAnswer::Choice(0))
        );
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
        assert_eq!(answer_text("oui mets le salon à 25", &confirm()), None);
    }

    #[test]
    fn a_refusal_cancels_a_choice_question_too() {
        for no in ["non", "Non merci", "annule", "laisse tomber"] {
            assert_eq!(answer_text(no, &rooms()), Some(PendingAnswer::No), "{no}");
        }
        // A command, even a short one, is still a new sentence.
        assert_eq!(answer_text("allume la cuisine", &rooms()), None);
        assert_eq!(answer_text("non allume la cuisine", &rooms()), None);
        assert_eq!(answer_text("oui", &rooms()), None);
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
    fn typed_choice_prefers_the_exact_name() {
        let choices = Pending::Choose(vec![
            ("Salon".into(), req("salon")),
            ("Salon TV".into(), req("salon_tv")),
            ("Chambre Léa".into(), req("chambre_lea")),
        ]);
        assert_eq!(
            answer_text("salon", &choices),
            Some(PendingAnswer::Choice(0))
        );
        assert_eq!(
            answer_text("le salon", &choices),
            Some(PendingAnswer::Choice(0))
        );
        assert_eq!(
            answer_text("salon tv", &choices),
            Some(PendingAnswer::Choice(1))
        );
    }

    #[test]
    fn typed_choice_may_say_chambre_before_a_person_named_room() {
        let choices = Pending::Choose(vec![
            ("Salon".into(), req("salon")),
            ("Léa".into(), req("chambre_lea")),
        ]);
        assert_eq!(
            answer_text("la chambre de léa", &choices),
            Some(PendingAnswer::Choice(1))
        );
        assert_eq!(answer_text("la chambre du salon", &choices), None);
    }

    #[test]
    fn a_bare_number_names_a_numbered_room_before_a_position() {
        let choices = Pending::Choose(vec![
            ("Chambre 2".into(), req("chambre_2")),
            ("Chambre 1".into(), req("chambre_1")),
        ]);
        assert_eq!(answer_text("2", &choices), Some(PendingAnswer::Choice(0)));
        assert_eq!(
            answer_text("chambre 1", &choices),
            Some(PendingAnswer::Choice(1))
        );
    }
}
