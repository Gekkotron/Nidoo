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

fn vacuum_actions(plan: &Plan) -> Vec<(String, Command)> {
    match plan {
        Plan::Act { actions, .. } => actions
            .iter()
            .map(|a| match a {
                Action::Vacuum { entity, command } => (entity.clone(), command.clone()),
                other => panic!("not a vacuum action: {other:?}"),
            })
            .collect(),
        other => panic!("not an act: {other:?}"),
    }
}

fn vacuum_action(action: &str) -> Command {
    Command::VacuumAction {
        action: action.into(),
    }
}

fn en_plan(s: &str) -> Plan {
    let f = fixture::house();
    plan(s, &f.house_en())
}

#[test]
fn vacuum_presses_the_room_button() {
    let want = vec![("vacuum_robot_clean_kitchen".to_string(), Command::Press)];
    assert_eq!(vacuum_actions(&p("nettoie la cuisine")), want);
    assert_eq!(vacuum_actions(&en_plan("vacuum the kitchen")), want);
    let Plan::Act { done, trace, .. } = p("aspire la salle de bain") else {
        panic!("not an act");
    };
    assert_eq!(
        done,
        Done::Vacuum {
            action: VacuumIntent::Clean,
            room: Some("salle de bain".into()),
            name: "Aspirateur".into(),
        }
    );
    assert_eq!(trace.intent.as_deref(), Some("vacuum_clean"));
    assert_eq!(trace.entities, vec!["vacuum_robot_clean_bathroom"]);
    // The room the house knows is found by its area too.
    let Plan::Act { trace, .. } = p("nettoie le bureau") else {
        panic!("not an act");
    };
    assert_eq!(trace.room.as_deref(), Some("bureau"));
    assert_eq!(trace.entities, vec!["vacuum_robot_clean_office"]);
}

#[test]
fn a_full_clean_ignores_the_speaker_room() {
    let f = fixture::house();
    let plan = plan_heard("lance l'aspirateur", &f.house(), Some("bureau"));
    assert_eq!(
        vacuum_actions(&plan),
        vec![("vacuum_robot".to_string(), vacuum_action("start"))]
    );
    assert_eq!(
        vacuum_actions(&p("nettoie la maison")),
        vec![("vacuum_robot".to_string(), vacuum_action("start"))]
    );
    assert_eq!(
        vacuum_actions(&en_plan("start the vacuum")),
        vec![("vacuum_robot".to_string(), vacuum_action("start"))]
    );
}

#[test]
fn vacuum_stop_pause_and_dock() {
    for (s, action) in [
        ("arrête l'aspirateur", "stop"),
        ("mets l'aspirateur en pause", "pause"),
        ("renvoie l'aspirateur à la base", "return_to_base"),
    ] {
        assert_eq!(
            vacuum_actions(&p(s)),
            vec![("vacuum_robot".to_string(), vacuum_action(action))],
            "{s}"
        );
    }
    assert_eq!(
        vacuum_actions(&en_plan("send the vacuum back to the dock")),
        vec![("vacuum_robot".to_string(), vacuum_action("return_to_base"))]
    );
}

#[test]
fn an_unknown_room_lists_the_rooms_the_vacuum_knows() {
    let Plan::Answer { reply, .. } = p("nettoie le garage") else {
        panic!("not an answer");
    };
    // The buttons' names lose the vacuum's name and the verb: "Aspirateur Clean kitchen".
    assert_eq!(
        reply,
        "L'aspirateur ne connaît pas la pièce « garage ». \
             Pièces : kitchen, living room, bathroom, entrance, office."
    );
    let Plan::Answer { reply, .. } = en_plan("clean the garage") else {
        panic!("not an answer");
    };
    assert!(reply.starts_with("The vacuum doesn't know the room \"garage\"."));
}

#[test]
fn the_reply_names_the_room_without_the_vacuum() {
    let Plan::Act { done, .. } = en_plan("vacuum the kitchen") else {
        panic!("not an act");
    };
    assert_eq!(
        done,
        Done::Vacuum {
            action: VacuumIntent::Clean,
            room: Some("kitchen".into()),
            name: "Aspirateur".into(),
        }
    );
    // The vacuum renamed in the app: its buttons keep their discovered names.
    let mut f = fixture::house();
    let vac = f.entity_mut("vacuum_robot");
    vac.ui.name = Some("Robbie".into());
    vac.name = "Robbie".into();
    let Plan::Answer { reply, .. } = plan("nettoie le garage", &f.house()) else {
        panic!("not an answer");
    };
    assert!(
        reply.ends_with("Pièces : kitchen, living room, bathroom, entrance, office."),
        "{reply}"
    );
}

#[test]
fn the_vacuum_name_in_the_sentence_is_not_a_room() {
    let mut f = fixture::house();
    for e in &mut f.entities {
        if e.device_id.as_deref() == Some("vacuum_robot") {
            e.name = e.name.replace("Aspirateur", "Roborock");
            e.device_name = Some("Roborock".into());
        }
    }
    // Every button's name holds "Roborock": the name alone matches none of them.
    assert_eq!(
        vacuum_actions(&plan("Roborock, nettoie la cuisine", &f.house())),
        vec![("vacuum_robot_clean_kitchen".to_string(), Command::Press)]
    );
    assert_eq!(
        vacuum_actions(&plan("Roborock, nettoie", &f.house())),
        vec![("vacuum_robot".to_string(), vacuum_action("start"))]
    );
}

#[test]
fn a_renamed_button_is_found_by_its_name() {
    let mut f = fixture::house();
    let button = f.entity_mut("vacuum_robot_clean_kitchen");
    button.ui.name = Some("Cuisine".into());
    button.name = "Cuisine".into();
    let plan = plan("nettoie la cuisine", &f.house());
    assert_eq!(
        vacuum_actions(&plan),
        vec![("vacuum_robot_clean_kitchen".to_string(), Command::Press)]
    );
}

#[test]
fn two_matching_buttons_are_asked() {
    let mut f = fixture::house();
    f.entities.push(fixture::vacuum_button(
        "vacuum_robot_clean_kitchen_2",
        "Aspirateur",
        "Clean kitchen island",
        "salon",
        "vacuum_robot",
    ));
    assert_eq!(
        labels(&plan("nettoie la cuisine", &f.house())),
        vec![
            "Aspirateur Clean kitchen",
            "Aspirateur Clean kitchen island"
        ]
    );
}

#[test]
fn several_vacuums_are_asked_and_none_is_answered() {
    let mut f = fixture::house();
    f.entities.push(fixture::vacuum(
        "vacuum_two",
        "Étage",
        "bureau",
        "vacuum_two",
    ));
    let asked = plan("lance l'aspirateur", &f.house());
    assert_eq!(labels(&asked), vec!["Aspirateur", "Étage"]);
    let Plan::Ask { choices, .. } = asked else {
        panic!("not a question");
    };
    assert_eq!(
        vacuum_actions(&plan_request(&choices[1].1, &f.house())),
        vec![("vacuum_two".to_string(), vacuum_action("start"))]
    );
    f.entities.retain(|e| e.kind != Kind::Vacuum);
    let Plan::Answer { reply, .. } = plan("lance l'aspirateur", &f.house()) else {
        panic!("not an answer");
    };
    assert_eq!(reply, "Pas d'aspirateur dans la maison.");
}

#[test]
fn a_vacuum_without_room_buttons_cannot_clean_a_room() {
    let mut f = fixture::house();
    f.entities.retain(|e| e.kind != Kind::Button);
    let Plan::Answer { reply, .. } = plan("nettoie la cuisine", &f.house()) else {
        panic!("not an answer");
    };
    assert_eq!(reply, "L'aspirateur ne sait pas nettoyer une seule pièce.");
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
            names: vec!["Led".into()],
            colour: None,
            brightness: None,
            unable: vec![],
        }
    );
}

#[test]
fn colour_goes_to_the_lights_that_take_one() {
    let red = Command::TurnOn {
        brightness: None,
        color: Some(nidoo_core::model::Hs { h: 0.0, s: 100.0 }),
        effect: None,
    };
    let plan = p("allume le salon en rouge");
    assert_eq!(
        lights_of(&plan),
        vec![
            ("light_lampe_table".to_string(), red.clone()),
            ("light_led".to_string(), red.clone()),
        ]
    );
    let Plan::Act { done, trace, .. } = &plan else {
        unreachable!()
    };
    assert_eq!(trace.intent.as_deref(), Some("lights_colour"));
    assert_eq!(
        *done,
        Done::Lights {
            area: "salon".into(),
            on: true,
            names: vec!["Lampe table".into(), "Led".into()],
            colour: Some("rouge".into()),
            brightness: None,
            unable: vec!["Plafonnier".into(), "Ambiance TV".into()],
        }
    );
    assert_eq!(
        lights_of(&p("mets la led en rouge")),
        vec![("light_led".to_string(), red)]
    );
}

#[test]
fn colour_without_a_colour_light_answers() {
    assert_eq!(
        reply_of(&p("allume le plafonnier en bleu")),
        "« Plafonnier » ne prend pas de couleur."
    );
    assert_eq!(
        reply_of(&p("allume la chambre de Léa en bleu")),
        "« Lampe » ne prend pas de couleur."
    );
    // No room said: only rooms with a colour light are possible.
    assert!(matches!(
        p("allume en vert"),
        Plan::Act { done: Done::Lights { area, .. }, .. } if area == "salon"
    ));
}

fn dimmed(brightness: u8, colour: Option<nidoo_core::model::Hs>) -> Command {
    Command::TurnOn {
        brightness: Some(brightness),
        color: colour,
        effect: None,
    }
}

#[test]
fn brightness_goes_to_the_lights_that_dim() {
    let half = dimmed(128, None);
    let plan = p("allume le salon à 50 %");
    assert_eq!(
        lights_of(&plan),
        vec![
            ("light_lampe_table".to_string(), half.clone()),
            ("light_led".to_string(), half.clone()),
        ]
    );
    let Plan::Act { done, trace, .. } = &plan else {
        unreachable!()
    };
    assert_eq!(trace.intent.as_deref(), Some("lights_brightness"));
    assert_eq!(
        *done,
        Done::Lights {
            area: "salon".into(),
            on: true,
            names: vec!["Lampe table".into(), "Led".into()],
            colour: None,
            brightness: Some(50),
            unable: vec!["Plafonnier".into(), "Ambiance TV".into()],
        }
    );
    assert_eq!(
        lights_of(&p("allume la led à 50 %")),
        vec![("light_led".to_string(), half)]
    );
    let blue = Some(nidoo_core::model::Hs { h: 225.0, s: 100.0 });
    assert_eq!(
        lights_of(&p("allume les leds du salon en bleu à 100 %")),
        vec![("light_led".to_string(), dimmed(255, blue))]
    );
    let Plan::Act { trace, .. } = p("allume la led en bleu à 100 %") else {
        unreachable!()
    };
    assert_eq!(trace.intent.as_deref(), Some("lights_colour"));
}

#[test]
fn brightness_without_a_dimmable_light_answers() {
    assert_eq!(
        reply_of(&p("allume le plafonnier à 50 %")),
        "« Plafonnier » n'a pas de luminosité réglable."
    );
    assert_eq!(
        reply_of(&p("allume la chambre de Léa à 50 %")),
        "« Lampe » n'a pas de luminosité réglable."
    );
}

#[test]
fn brightness_out_of_range_and_refused_cases() {
    assert_eq!(
        reply_of(&p("allume la led à 0 %")),
        "La luminosité va de 1 à 100 %."
    );
    assert_eq!(
        reply_of(&p("allume la led à 150 %")),
        "La luminosité va de 1 à 100 %."
    );
    for s in [
        "éteins le salon à 50 %",
        "mets le salon à 20 %",
        "allume la prise du bureau à 50 %",
    ] {
        assert!(
            !matches!(p(s), Plan::Act { actions, .. } if actions.iter().any(
                |a| matches!(a, Action::Light { command: Command::TurnOn { brightness: Some(_), .. }, .. })
            )),
            "{s}"
        );
    }
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
fn an_unclassed_cover_without_the_mark_is_not_opened_as_a_shutter() {
    let mut f = fixture::house();
    f.entities.push(fixture::unmarked_cover(
        "cover_garage",
        "Garage",
        "salon",
        false,
    ));
    let got = covers_of(&plan("ouvre tous les volets", &f.house()));
    assert_eq!(ids(&got), ["cover_baie", "cover_fenetre", "cover_lea"]);
    assert!(got.iter().all(|(id, _)| id != "cover_garage"));
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
fn misheard_mets_is_not_a_room_word() {
    // Speech-to-text often hears "mets" as "mais".
    assert_eq!(
        setpoint_of(&p("mais le chauffage au salon à 18 degrés")),
        ("salon".into(), 18.0)
    );
}

#[test]
fn feeling_steps_the_named_room() {
    assert_eq!(
        setpoint_of(&p("j'ai froid au salon")),
        ("salon".into(), 19.5)
    );
    // The step starts from the reading when it is already past the setpoint.
    assert_eq!(
        setpoint_of(&p("il fait froid dans la chambre de Léa")),
        ("chambre_lea".into(), 17.0)
    );
    assert_eq!(
        setpoint_of(&p("j'ai chaud au salon")),
        ("salon".into(), 17.5)
    );
}

#[test]
fn feeling_uses_the_satellite_room() {
    let f = fixture::house();
    assert_eq!(
        setpoint_of(&heard(&f, "j'ai froid", Some("salon"))),
        ("salon".into(), 19.5)
    );
    // A named room beats the satellite's.
    assert_eq!(
        setpoint_of(&heard(&f, "j'ai froid au salon", Some("garage"))),
        ("salon".into(), 19.5)
    );
}

#[test]
fn feeling_from_a_room_without_heating_asks_which() {
    let f = fixture::house();
    for room in [Some("garage"), Some("chambre_tom"), None] {
        let asked = heard(&f, "il fait trop froid", room);
        assert_eq!(labels(&asked), ["Salon", "Chambre Léa"], "{room:?}");
        let pending = crate::Pending::of(&asked).expect("a question");
        let request = pending
            .resolve(crate::PendingAnswer::Choice(0))
            .expect("choice 1");
        assert_eq!(
            setpoint_of(&plan_request(&request, &f.house())),
            ("salon".into(), 19.5)
        );
    }
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
        ["Chambre de Léa", "Chambre de Tom"]
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
        colour: None,
        brightness: None,
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

fn heard(f: &fixture::Fixture, s: &str, room: Option<&str>) -> Plan {
    plan_heard(s, &f.house(), room)
}

fn media_of(plan: &Plan) -> (String, Command) {
    match plan {
        Plan::Act { actions, .. } => match actions.as_slice() {
            [Action::Media { entity, command }] => (entity.clone(), command.clone()),
            other => panic!("not one media action: {other:?}"),
        },
        other => panic!("not an act: {other:?}"),
    }
}

fn reply_text(plan: &Plan) -> &str {
    match plan {
        Plan::Answer { reply, .. } => reply,
        other => panic!("not an answer: {other:?}"),
    }
}

fn pause() -> Command {
    Command::MediaAction {
        action: "pause".into(),
    }
}

#[test]
fn two_players_pick_the_speaker_room() {
    let f = fixture::two_players();
    assert_eq!(
        media_of(&heard(&f, "mets pause", Some("bureau"))),
        ("player_bureau".to_string(), pause())
    );
}

#[test]
fn two_players_without_speaker_room_ask_which() {
    let f = fixture::two_players();
    let asked = heard(&f, "mets pause", None);
    assert_eq!(labels(&asked), vec!["Musique", "Musique bureau"]);
    let pending = crate::Pending::of(&asked).expect("a question");
    let request = pending
        .resolve(crate::PendingAnswer::Choice(1))
        .expect("choice 2");
    assert_eq!(
        media_of(&plan_request(&request, &f.house())),
        ("player_bureau".to_string(), pause())
    );
}

#[test]
fn speaker_room_survives_the_question() {
    let f = fixture::house();
    let asked = heard(&f, "monte le son", Some("garage"));
    let Plan::Answer { reply, .. } = &asked else {
        panic!("no speaker in the garage: {asked:?}");
    };
    assert_eq!(reply, "Pas d'enceinte dans le Garage.");
    let f = fixture::two_players();
    let asked = heard(&f, "chanson suivante", Some("garage"));
    let Plan::Ask { choices, .. } = &asked else {
        panic!("a question: {asked:?}");
    };
    assert!(
        choices
            .iter()
            .all(|(_, r)| r.speaker_room.as_deref() == Some("garage"))
    );
}

#[test]
fn volume_uses_the_speaker_room() {
    let f = fixture::house();
    let plan = heard(&f, "monte le son", Some("salon"));
    let Plan::Act { actions, .. } = &plan else {
        panic!("an act: {plan:?}");
    };
    assert_eq!(
        actions,
        &vec![Action::Speaker {
            entity: "number_salon_volume".into(),
            command: Command::SetNumber { value: 50.0 }
        }]
    );
}

#[test]
fn volume_is_clamped_and_absolute() {
    let f = fixture::house();
    let plan = heard(&f, "mets le son à 100 %", Some("bureau"));
    let Plan::Act { actions, .. } = &plan else {
        panic!("an act: {plan:?}");
    };
    assert_eq!(
        actions,
        &vec![Action::Speaker {
            entity: "number_bureau_volume".into(),
            command: Command::SetNumber { value: 100.0 }
        }]
    );
    let mut f = fixture::house();
    f.entity_mut("number_salon_volume").state.value = StateValue::Number(96.0);
    let plan = heard(&f, "monte le son", Some("salon"));
    let Plan::Act { actions, .. } = &plan else {
        panic!("an act: {plan:?}");
    };
    assert_eq!(
        actions,
        &vec![Action::Speaker {
            entity: "number_salon_volume".into(),
            command: Command::SetNumber { value: 100.0 }
        }]
    );
}

#[test]
fn unknown_volume_cannot_step() {
    let mut f = fixture::house();
    f.entity_mut("number_salon_volume").state.value = StateValue::Unknown;
    assert_eq!(
        reply_text(&heard(&f, "monte le son", Some("salon"))),
        "Je ne connais pas le volume actuel."
    );
}

#[test]
fn no_player() {
    let mut f = fixture::house();
    f.entities.retain(|e| e.kind != Kind::MediaPlayer);
    assert_eq!(
        reply_text(&heard(&f, "mets pause", None)),
        "Il n'y a pas de lecteur de musique."
    );
}

#[test]
fn offline_player_answers_it_does_not_answer() {
    let mut f = fixture::house();
    f.entity_mut("player_music").state.availability = nidoo_core::model::Availability::Offline;
    assert_eq!(
        reply_text(&heard(&f, "mets pause", None)),
        "Le lecteur ne répond pas."
    );
}

#[test]
fn player_without_play_media_cannot_search() {
    let mut f = fixture::house();
    f.entity_mut("player_music")
        .capabilities
        .media_features
        .retain(|feature| feature != "play_media" && feature != "next");
    assert_eq!(
        reply_text(&heard(&f, "joue Daft Punk", None)),
        "Ce lecteur ne sait pas chercher un titre."
    );
    assert_eq!(
        reply_text(&heard(&f, "chanson suivante", None)),
        "Ce lecteur ne sait pas faire ça."
    );
}

#[test]
fn now_playing_reads_the_state() {
    let mut f = fixture::house();
    assert_eq!(
        reply_text(&heard(&f, "qu'est-ce qu'on écoute ?", None)),
        "Get Lucky, de Daft Punk."
    );
    f.entity_mut("player_music").state.value =
        StateValue::MediaPlayer(nidoo_core::model::MediaPlayerState {
            state: Some("paused".into()),
            title: Some("Get Lucky".into()),
            ..Default::default()
        });
    assert_eq!(
        reply_text(&heard(&f, "c'est quoi cette chanson ?", None)),
        "En pause : Get Lucky."
    );
    f.entity_mut("player_music").state.value =
        StateValue::MediaPlayer(nidoo_core::model::MediaPlayerState {
            state: Some("idle".into()),
            ..Default::default()
        });
    assert_eq!(
        reply_text(&heard(&f, "c'est quoi cette chanson ?", None)),
        "Rien ne joue."
    );
}

#[test]
fn media_hints() {
    let f = fixture::house();
    assert_eq!(
        reply_text(&heard(&f, "mets de la musique classique", None)),
        "Pour un titre ou un artiste, dites « joue » suivi du nom."
    );
    assert_eq!(
        reply_text(&heard(&f, "mets le son à 140", Some("salon"))),
        "Le volume va de 0 à 100 %."
    );
}

/// A satellite's Mute is folded into its Music volume (`part_of`): still a speaker control.
#[test]
fn a_folded_mute_is_still_the_room_mute() {
    let mut f = fixture::house();
    let mute = f.entity_mut("switch_salon_mute");
    mute.part_of = Some("number_salon_volume".into());
    // Its own room is unset: the satellite's room lives on its Music volume.
    mute.area = String::new();
    let plan = heard(&f, "coupe le son", Some("salon"));
    let Plan::Act { actions, .. } = &plan else {
        panic!("an act: {plan:?}");
    };
    assert!(matches!(
        actions.as_slice(),
        [Action::Speaker { entity, command: Command::TurnOn { .. } }] if entity == "switch_salon_mute"
    ));
}

#[test]
fn an_idle_player_has_nothing_to_pause_or_skip() {
    let mut f = fixture::house();
    f.entity_mut("player_music").state.value =
        StateValue::MediaPlayer(nidoo_core::model::MediaPlayerState {
            state: Some("idle".into()),
            ..Default::default()
        });
    for s in ["mets pause", "chanson suivante", "chanson précédente"] {
        assert_eq!(reply_text(&heard(&f, s, None)), "Rien ne joue.", "{s}");
    }
    assert_eq!(
        media_of(&heard(&f, "mets de la musique", None)).1,
        Command::MediaAction {
            action: "play".into()
        }
    );
}

/// [`fixture::house`] with English names: Living room, TV room, Office, Kitchen (the
/// heated room with the open/close shutter), Tom (a person's room) and Garage; a desk
/// lamp in the Office and a night light in Tom's room.
fn english() -> fixture::Fixture {
    let mut f = fixture::house();
    for a in &mut f.areas {
        a.name = match a.id.as_str() {
            "salon" => "Living room",
            "salon_tv" => "TV room",
            "bureau" => "Office",
            "chambre_lea" => "Kitchen",
            "chambre_tom" => "Tom",
            _ => "Garage",
        }
        .into();
    }
    for (id, name) in [
        ("light_plafonnier", "Ceiling"),
        ("light_lampe_table", "Table lamp"),
        ("light_led", "Led"),
        ("light_lampe_lea", "Lamp"),
        ("cover_baie", "Bay"),
        ("cover_fenetre", "Window"),
        ("cover_lea", "Shutter"),
        ("switch_tv", "Screen"),
        ("switch_prise_bureau", "Coffee maker"),
        ("switch_prise_tom", "Charger"),
        ("player_music", "Music"),
    ] {
        f.entity_mut(id).name = name.into();
    }
    f.entities
        .push(fixture::light("light_desk", "Desk lamp", "bureau", true));
    f.entities.push(fixture::light(
        "light_night",
        "Night light",
        "chambre_tom",
        false,
    ));
    f
}

fn words(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

/// The plan of `intent` in the English house, and its reply when it acts and every
/// action succeeds.
fn en(intent: Intent) -> (Plan, String) {
    let f = english();
    let house = f.house_en();
    let plan = plan_request(&Request::new(intent), &house);
    let text = match &plan {
        Plan::Act { actions, done, .. } => crate::reply::done(
            done,
            &vec![crate::reply::ActionResult::Ok; actions.len()],
            &house,
            false,
        ),
        Plan::Answer { reply, .. } | Plan::NotUnderstood { reply, .. } => reply.clone(),
        Plan::Ask { question, .. } | Plan::Confirm { question, .. } => question.clone(),
    };
    (plan, text)
}

fn lights_intent(on: bool, names: &str, room: &str) -> Intent {
    Intent::Lights {
        on,
        all: false,
        names: words(names),
        room: words(room),
        colour: None,
        brightness: None,
    }
}

#[test]
fn english_lights() {
    assert_eq!(
        en(lights_intent(true, "", "living room")).1,
        "Living room lights on (4 lights)."
    );
    assert_eq!(
        en(lights_intent(false, "led", "living room")).1,
        "Led off in the Living room."
    );
    assert_eq!(
        en(lights_intent(true, "", "tom")).1,
        "Night light on in Tom's room."
    );
    // "turn on the desk lamp": the room words are part of the light's name.
    let (plan, text) = en(lights_intent(true, "lamp", "desk"));
    assert_eq!(lights_of(&plan), vec![("light_desk".to_string(), on())]);
    assert_eq!(text, "Desk lamp on in the Office.");
    assert_eq!(
        en(lights_intent(true, "", "attic")).1,
        "I don't know a room called \"attic\"."
    );
    assert_eq!(
        en(lights_intent(true, "", "garage")).1,
        "No lights in the Garage."
    );
    assert_eq!(
        en(lights_intent(true, "chandelier", "")).1,
        "I can't find a light called \"chandelier\"."
    );
    let (plan, text) = en(lights_intent(true, "chandelier", "living room"));
    assert_eq!(text, "Which light in the Living room?");
    assert_eq!(
        labels(&plan),
        ["Ceiling", "Table lamp", "Led", "Ambiance TV"]
    );
}

#[test]
fn english_room_question() {
    let (plan, text) = en(lights_intent(true, "", ""));
    assert_eq!(text, "Which room?");
    assert_eq!(
        labels(&plan),
        ["Living room", "Office", "Kitchen", "Tom's room"]
    );
}

#[test]
fn english_colour() {
    let (plan, text) = en(Intent::Lights {
        on: true,
        all: false,
        names: words("led"),
        room: vec![],
        colour: Some(Colour::Red),
        brightness: None,
    });
    let Plan::Act { done, .. } = plan else {
        panic!("{plan:?}")
    };
    assert!(matches!(done, Done::Lights { colour: Some(c), .. } if c == "red"));
    assert_eq!(text, "Led on in red in the Living room.");
    assert_eq!(
        en(Intent::Lights {
            on: true,
            all: false,
            names: words("ceiling"),
            room: vec![],
            colour: Some(Colour::Blue),
            brightness: None,
        })
        .1,
        "\"Ceiling\" can't take a colour."
    );
}

#[test]
fn english_brightness() {
    let intent = |names: &str, colour, brightness| Intent::Lights {
        on: true,
        all: false,
        names: words(names),
        room: vec![],
        colour,
        brightness,
    };
    let (plan, text) = en(intent("led", None, Some(50)));
    assert_eq!(text, "Led on at 50 % in the Living room.");
    assert_eq!(
        lights_of(&plan),
        vec![("light_led".to_string(), dimmed(128, None))]
    );
    assert_eq!(
        en(intent("led", Some(Colour::Blue), Some(100))).1,
        "Led on in blue at 100 % in the Living room."
    );
    assert_eq!(
        en(intent("ceiling", None, Some(50))).1,
        "\"Ceiling\" can't be dimmed."
    );
    assert_eq!(
        en(Intent::BrightnessRange).1,
        "The brightness goes from 1 to 100 %."
    );
}

#[test]
fn english_temperatures() {
    let set = |t: f64, room: &str| Intent::SetTemp {
        change: Change::Absolute(t),
        room: words(room),
    };
    assert_eq!(
        en(set(20.0, "living room")).1,
        "Heating in the Living room set to 20 °C (was 19 °C)."
    );
    assert_eq!(
        en(set(25.0, "living room")).1,
        "Set the heating in the Living room to 25 °C?"
    );
    assert_eq!(
        en(set(20.0, "tom")).1,
        "The heating in Tom's room isn't active yet."
    );
    assert_eq!(
        en(set(20.0, "office")).1,
        "No controlled heating in the Office."
    );
    assert_eq!(
        en(Intent::HeatingNoValue {
            room: words("kitchen")
        })
        .1,
        "Tell me a temperature, for example \"set the Kitchen to 19\"."
    );
    assert_eq!(
        en(Intent::ReadTemp {
            room: words("office"),
            outdoor: false
        })
        .1,
        "It's 20.1 °C in the Office."
    );
    assert_eq!(
        en(Intent::ReadTemp {
            room: vec![],
            outdoor: true
        })
        .1,
        "It's 7 °C outside."
    );
    // A setpoint without a known setpoint before it.
    let mut f = english();
    f.zone_mut("salon").decision = None;
    let house = f.house_en();
    let plan = plan_request(&Request::new(set(20.5, "living room")), &house);
    let Plan::Act { done, .. } = plan else {
        panic!("{plan:?}")
    };
    assert_eq!(
        crate::reply::done(&done, &[crate::reply::ActionResult::Ok], &house, false),
        "Heating in the Living room set to 20.5 °C."
    );
    let plan = plan_request(
        &Request::new(Intent::SetTemp {
            change: Change::Relative(1.0),
            room: words("living room"),
        }),
        &house,
    );
    assert!(matches!(
        plan,
        Plan::Answer { reply, .. } if reply == "I don't know the setpoint in the Living room: tell me a temperature, for example \"set the Living room to 19\"."
    ));
}

#[test]
fn english_covers() {
    let covers = |open: bool, position: Option<u8>, all: bool, room: &str| Intent::Covers {
        open,
        position,
        all,
        names: vec![],
        room: words(room),
    };
    assert_eq!(
        en(covers(false, None, true, "")).1,
        "All shutters closed (3 shutters)."
    );
    assert_eq!(
        en(covers(true, Some(30), false, "living room")).1,
        "Living room: shutters open at 30 % (2 shutters)."
    );
    assert_eq!(
        en(covers(true, Some(30), false, "kitchen")).1,
        "Shutter can't be set to a position."
    );
    assert_eq!(
        en(covers(false, None, false, "kitchen")).1,
        "Shutter closed in the Kitchen."
    );
    // "close the bay shutter" arrives as names `shutter`, room `bay`.
    assert_eq!(
        en(Intent::Covers {
            open: false,
            position: None,
            all: false,
            names: words("bay"),
            room: vec![],
        })
        .1,
        "Shutter \"Bay\" closed in the Living room."
    );
    assert_eq!(
        en(Intent::CoverPositionHint).1,
        "For a position, say for example \"open the shutters to 30 %\"."
    );
}

#[test]
fn english_switches() {
    let switch = |what, on, names: &str, room: &str| Intent::Switches {
        what,
        on,
        all: false,
        names: words(names),
        room: words(room),
    };
    assert_eq!(
        en(switch(SwitchType::Tv, true, "", "")).1,
        "TV \"Screen\" on in the Living room."
    );
    // "turn off the coffee maker plug": names `maker`, room `coffee`.
    assert_eq!(
        en(switch(SwitchType::Plug, false, "maker", "coffee")).1,
        "Plug \"Coffee maker\" off in the Office."
    );
    let (plan, text) = en(switch(SwitchType::Plug, false, "", ""));
    assert_eq!(text, "Which room?");
    assert_eq!(labels(&plan), ["Office", "Tom's room"]);
    assert_eq!(
        en(Intent::Switches {
            what: SwitchType::Plug,
            on: false,
            all: true,
            names: vec![],
            room: vec![],
        })
        .1,
        "All plugs off (2 plugs)."
    );
    assert_eq!(
        en(switch(SwitchType::Tv, true, "", "garage")).1,
        "No TVs in the Garage."
    );
}

#[test]
fn english_media_and_speakers() {
    assert_eq!(en(Intent::Media(MediaIntent::Pause)).1, "Music paused.");
    assert_eq!(
        en(Intent::Media(MediaIntent::NowPlaying)).1,
        "Get Lucky, by Daft Punk."
    );
    assert_eq!(
        en(Intent::Volume {
            change: Change::Absolute(30.0),
            room: words("office"),
        })
        .1,
        "Volume at 30 % in the Office."
    );
    assert_eq!(
        en(Intent::Volume {
            change: Change::Absolute(30.0),
            room: vec![],
        })
        .1,
        "Which room?"
    );
    assert_eq!(
        en(Intent::Mute {
            on: true,
            room: words("living room"),
        })
        .1,
        "Sound muted in the Living room."
    );
    assert_eq!(
        en(Intent::Mute {
            on: true,
            room: words("office"),
        })
        .1,
        "No speakers in the Office."
    );
    assert_eq!(
        en(Intent::VolumeRange).1,
        "The volume goes from 0 to 100 %."
    );
    assert_eq!(
        en(Intent::MediaHint).1,
        "For a title or an artist, say \"play\" followed by the name."
    );
    let mut f = english();
    f.entity_mut("player_music").state.availability = Availability::Offline;
    let plan = plan_request(
        &Request::new(Intent::Media(MediaIntent::Pause)),
        &f.house_en(),
    );
    assert!(matches!(
        plan,
        Plan::Answer { reply, .. } if reply == "The player isn't responding."
    ));
}

#[test]
fn english_not_understood_and_failures() {
    let f = english();
    assert!(matches!(
        plan("xyzzy plugh", &f.house_en()),
        Plan::NotUnderstood { reply, .. } if reply == "I didn't understand."
    ));
    let house = f.house_en();
    let Plan::Act { done, .. } = en(lights_intent(true, "", "living room")).0 else {
        panic!()
    };
    use crate::reply::ActionResult::{Failed, Ok};
    assert_eq!(
        crate::reply::done(
            &done,
            &[Ok, Ok, Failed(crate::reply::NO_ANSWER_EN.into()), Ok],
            &house,
            true
        ),
        "Simulation: Living room lights on (4 lights). 1 of 4 didn't answer: Led."
    );
}
