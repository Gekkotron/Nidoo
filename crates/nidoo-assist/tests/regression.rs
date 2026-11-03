//! The sentences the assistant must keep getting right, on the fictional house.

use nidoo_assist::fixture;
use nidoo_assist::{Action, Plan, plan};
use nidoo_core::model::Command;

enum Expect {
    Lights(bool, &'static [&'static str]),
    Setpoint(&'static str, f64),
    Confirm(&'static str, f64),
    Answer(&'static str),
    Ask(&'static [&'static str]),
    NotUnderstood,
}

use Expect::*;

const ROWS: &[(&str, Expect)] = &[
    (
        "allume la lumière du salon",
        Lights(
            true,
            &[
                "light_plafonnier",
                "light_lampe_table",
                "light_led",
                "light_ambiance_tv",
            ],
        ),
    ),
    (
        "éteins le salon",
        Lights(
            false,
            &[
                "light_plafonnier",
                "light_lampe_table",
                "light_led",
                "light_ambiance_tv",
            ],
        ),
    ),
    (
        "éteins tout dans le salon",
        Lights(
            false,
            &[
                "light_plafonnier",
                "light_lampe_table",
                "light_led",
                "light_ambiance_tv",
            ],
        ),
    ),
    ("éteins la led du salon", Lights(false, &["light_led"])),
    (
        "allumme la lampe de la chambre de Léa",
        Lights(true, &["light_lampe_lea"]),
    ),
    (
        "etein la lumière chez Léa",
        Lights(false, &["light_lampe_lea"]),
    ),
    ("allume la chambre", Ask(&["Chambre Léa", "Chambre Tom"])),
    (
        "quelle température fait-il dans le salon ?",
        Answer("Il fait 18,2 °C dans le Salon."),
    ),
    (
        "il fait combien dans le bureau ?",
        Answer("Il fait 20,1 °C dans le Bureau."),
    ),
    ("il fait combien dehors ?", Answer("Il fait 7 °C dehors.")),
    ("mets le salon à 20 degrés", Setpoint("salon", 20.0)),
    (
        "règle le chauffage du salon à 19,5",
        Setpoint("salon", 19.5),
    ),
    (
        "baisse un peu le chauffage du salon",
        Setpoint("salon", 17.5),
    ),
    (
        "monte le chauffage d'un degré chez Léa",
        Setpoint("chambre_lea", 17.5),
    ),
    (
        "mets la chambre de Léa à dix-huit",
        Setpoint("chambre_lea", 18.0),
    ),
    ("mets le salon à 25", Confirm("salon", 25.0)),
    (
        "mets le salon à 40",
        Answer("Le chauffage du Salon accepte de 10 à 30 °C : 40 °C est hors limites."),
    ),
    (
        "mets le bureau à 19",
        Answer("Pas de chauffage piloté dans le Bureau."),
    ),
    (
        "allume le chauffage du salon",
        Answer("Dis-moi une température, par exemple « mets le salon à 19 »."),
    ),
    (
        "allume les 3 lumières du salon",
        Lights(
            true,
            &[
                "light_plafonnier",
                "light_lampe_table",
                "light_led",
                "light_ambiance_tv",
            ],
        ),
    ),
    ("mets le salon à 20 %", NotUnderstood),
    ("fais-moi un café", NotUnderstood),
    ("allume et éteins le salon", NotUnderstood),
    ("n'éteins pas le salon", NotUnderstood),
];

#[test]
fn regression_table() {
    let f = fixture::house();
    let house = f.house();
    let mut failures = Vec::new();
    for (sentence, expect) in ROWS {
        let got = plan(sentence, &house);
        let ok = match (expect, &got) {
            (Lights(on, ids), Plan::Act { actions, .. }) => {
                fn command(a: &Action) -> Option<(&str, Command)> {
                    match a {
                        Action::Light { entity, command } => Some((entity, command.clone())),
                        Action::Setpoint { .. } => None,
                    }
                }
                let want = if *on {
                    Command::TurnOn {
                        brightness: None,
                        color: None,
                        effect: None,
                    }
                } else {
                    Command::TurnOff
                };
                let got: Vec<_> = actions.iter().map(command).collect();
                got.len() == ids.len()
                    && got
                        .iter()
                        .zip(ids.iter())
                        .all(|(g, id)| g.as_ref().is_some_and(|(e, c)| e == id && *c == want))
            }
            (Setpoint(zone, value), Plan::Act { actions, .. }) => matches!(
                actions.as_slice(),
                [Action::Setpoint { zone: z, setpoint }] if z == zone && setpoint == value
            ),
            (Confirm(zone, value), Plan::Confirm { request, .. }) => {
                let mut yes = request.clone();
                yes.confirmed = true;
                matches!(
                    nidoo_assist::plan_request(&yes, &house),
                    Plan::Act { actions, .. } if matches!(
                        actions.as_slice(),
                        [Action::Setpoint { zone: z, setpoint }] if z == zone && setpoint == value
                    )
                )
            }
            (Answer(text), Plan::Answer { reply, .. }) => reply == text,
            (Ask(labels), Plan::Ask { choices, .. }) => {
                choices.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>() == *labels
            }
            (NotUnderstood, Plan::NotUnderstood { examples, .. }) => {
                !examples.is_empty() && examples.len() <= 3
            }
            _ => false,
        };
        if !ok {
            failures.push(format!("{sentence:?} → {got:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
