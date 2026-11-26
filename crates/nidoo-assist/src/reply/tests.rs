use super::*;
use crate::fixture;
use crate::plan::Done;

fn lights_done(on: bool, names: &[&str]) -> Done {
    Done::Lights {
        area: "salon".into(),
        on,
        names: names.iter().map(|n| n.to_string()).collect(),
        colour: None,
        brightness: None,
        unable: vec![],
    }
}

#[test]
fn brightness_reply_shapes() {
    use ActionResult::{Ok, Skipped};
    let dimmed = |names: &[&str], colour: Option<&str>, unable: &[&str]| Done::Lights {
        area: "salon".into(),
        on: true,
        names: names.iter().map(|n| n.to_string()).collect(),
        colour: colour.map(Into::into),
        brightness: Some(100),
        unable: unable.iter().map(|n| n.to_string()).collect(),
    };
    assert_eq!(
        said(&dimmed(&["Led"], Some("bleu"), &[]), &[Ok]),
        "Led allumée en bleu à 100 % dans le Salon."
    );
    assert_eq!(
        said(&dimmed(&["Led"], None, &[]), &[Skipped]),
        "Led déjà allumée à 100 % dans le Salon."
    );
    assert_eq!(
        said(
            &dimmed(&["Led", "Lampe table"], None, &["Plafonnier"]),
            &[Ok, Ok]
        ),
        "Salon allumé à 100 % (2 lumières). « Plafonnier » n'a pas de luminosité réglable."
    );
    assert_eq!(
        said(&dimmed(&["Led"], None, &["Plafonnier", "Lampe"]), &[Ok]),
        "Led allumée à 100 % dans le Salon. N'ont pas de luminosité réglable : Plafonnier, Lampe."
    );
    assert_eq!(
        said(&dimmed(&["Led"], Some("bleu"), &["Plafonnier"]), &[Ok]),
        "Led allumée en bleu à 100 % dans le Salon. « Plafonnier » ne prend pas ce réglage."
    );
}

#[test]
fn colour_reply_shapes() {
    use ActionResult::{Ok, Skipped};
    let coloured = |names: &[&str], unable: &[&str]| Done::Lights {
        area: "salon".into(),
        on: true,
        names: names.iter().map(|n| n.to_string()).collect(),
        colour: Some("rouge".into()),
        brightness: None,
        unable: unable.iter().map(|n| n.to_string()).collect(),
    };
    assert_eq!(
        said(&coloured(&["Led"], &[]), &[Ok]),
        "Led allumée en rouge dans le Salon."
    );
    assert_eq!(
        said(&coloured(&["Led"], &[]), &[Skipped]),
        "Led déjà allumée en rouge dans le Salon."
    );
    assert_eq!(
        said(
            &coloured(&["Led", "Lampe table"], &["Plafonnier"]),
            &[Ok, Ok]
        ),
        "Salon allumé en rouge (2 lumières). « Plafonnier » ne prend pas de couleur."
    );
    assert_eq!(
        said(&coloured(&["Led"], &["Plafonnier", "Lampe"]), &[Ok]),
        "Led allumée en rouge dans le Salon. Ne prennent pas de couleur : Plafonnier, Lampe."
    );
}

fn covers_done(area: Option<&str>, open: bool, position: Option<u8>, names: &[&str]) -> Done {
    Done::Covers {
        area: area.map(str::to_string),
        open,
        position,
        names: names.iter().map(|n| n.to_string()).collect(),
        unable: vec![],
    }
}

fn said(d: &Done, results: &[ActionResult]) -> String {
    let f = fixture::house();
    done(d, results, &f.house(), false)
}

#[test]
fn covers_reply_shapes() {
    use ActionResult::{Ok, Skipped};
    let salon = Some("salon");
    assert_eq!(
        said(&covers_done(salon, false, None, &["Baie"]), &[Ok]),
        "Volet « Baie » fermé dans le Salon."
    );
    assert_eq!(
        said(&covers_done(salon, true, Some(30), &["Baie"]), &[Ok]),
        "Volet « Baie » ouvert à 30 % dans le Salon."
    );
    assert_eq!(
        said(&covers_done(salon, false, None, &["Baie"]), &[Skipped]),
        "Volet « Baie » déjà fermé dans le Salon."
    );
    assert_eq!(
        said(
            &covers_done(salon, false, None, &["Baie", "Fenêtre"]),
            &[Ok, Ok]
        ),
        "Salon : volets fermés (2 volets)."
    );
    assert_eq!(
        said(
            &covers_done(salon, true, Some(30), &["Baie", "Fenêtre"]),
            &[Ok, Ok]
        ),
        "Salon : volets ouverts à 30 % (2 volets)."
    );
    assert_eq!(
        said(
            &covers_done(None, false, None, &["Baie", "Fenêtre", "Volet"]),
            &[Ok, Ok, Ok]
        ),
        "Tous les volets fermés (3 volets)."
    );
    assert_eq!(
        said(
            &covers_done(None, true, None, &["Baie", "Fenêtre"]),
            &[Skipped, Skipped]
        ),
        "Tous les volets déjà ouverts (2 volets)."
    );
}

#[test]
fn whole_house_with_one_offline_names_it() {
    use ActionResult::{Offline, Ok};
    assert_eq!(
        said(
            &covers_done(None, false, None, &["Baie", "Fenêtre", "Volet"]),
            &[Ok, Offline, Ok]
        ),
        "Tous les volets fermés (3 volets). 1 sur 3 n'a pas répondu : Fenêtre."
    );
    assert_eq!(
        said(
            &covers_done(Some("salon"), false, None, &["Baie", "Fenêtre"]),
            &[Offline, Offline]
        ),
        "Rien n'a répondu dans le Salon : Baie, Fenêtre."
    );
}

#[test]
fn covers_reply_names_the_unable() {
    let mut d = covers_done(None, true, Some(30), &["Baie"]);
    if let Done::Covers { unable, .. } = &mut d {
        unable.push("Volet".into());
    }
    assert_eq!(
        said(&d, &[ActionResult::Ok]),
        "Volet « Baie » ouvert à 30 %. Volet ne sait pas se mettre à une position."
    );
}

fn switches_done(what: SwitchType, area: Option<&str>, on: bool, names: &[&str]) -> Done {
    Done::Switches {
        what,
        area: area.map(str::to_string),
        on,
        names: names.iter().map(|n| n.to_string()).collect(),
    }
}

#[test]
fn switches_reply_shapes() {
    use ActionResult::{Failed, Offline, Ok, Skipped};
    use SwitchType::{Plug, Tv};
    assert_eq!(
        said(&switches_done(Tv, Some("salon"), true, &["Écran"]), &[Ok]),
        "Télé « Écran » allumée dans le Salon."
    );
    assert_eq!(
        said(
            &switches_done(Plug, Some("bureau"), false, &["Cafetière"]),
            &[Skipped]
        ),
        "Prise « Cafetière » déjà éteinte dans le Bureau."
    );
    assert_eq!(
        said(
            &switches_done(Plug, Some("bureau"), false, &["Cafetière", "Chargeur"]),
            &[Ok, Ok]
        ),
        "Bureau : prises éteintes (2 prises)."
    );
    assert_eq!(
        said(
            &switches_done(Plug, None, false, &["Cafetière", "Chargeur", "Lampe"]),
            &[Ok, Ok, Ok]
        ),
        "Toutes les prises éteintes (3 prises)."
    );
    assert_eq!(
        said(
            &switches_done(Tv, None, true, &["Écran", "Projecteur"]),
            &[Ok, Ok]
        ),
        "Toutes les télés allumées (2 télés)."
    );
    assert_eq!(
        said(
            &switches_done(Tv, None, true, &["Écran", "Projecteur"]),
            &[Skipped, Skipped]
        ),
        "Toutes les télés déjà allumées (2 télés)."
    );
    assert_eq!(
        said(
            &switches_done(Plug, None, false, &["Cafetière", "Chargeur", "Lampe"]),
            &[Ok, Offline, Ok]
        ),
        "Toutes les prises éteintes (3 prises). 1 sur 3 n'a pas répondu : Chargeur."
    );
    assert_eq!(
        said(
            &switches_done(Plug, Some("bureau"), false, &["Cafetière"]),
            &[Failed("refusé".into())]
        ),
        "« Cafetière » n'a pas répondu dans le Bureau : refusé."
    );
    assert_eq!(
        said(
            &switches_done(Plug, Some("bureau"), false, &["Cafetière", "Chargeur"]),
            &[Offline, Offline]
        ),
        "Rien n'a répondu dans le Bureau : Cafetière, Chargeur."
    );
}

#[test]
fn a_name_that_is_only_the_kind_word_is_not_repeated() {
    use ActionResult::Ok;
    assert_eq!(
        said(
            &covers_done(Some("bureau"), true, Some(40), &["Volet"]),
            &[Ok]
        ),
        "Volet ouvert à 40 % dans le Bureau."
    );
    assert_eq!(
        said(&lights_done(true, &["Lampe"]), &[Ok]),
        "Lampe allumée dans le Salon."
    );
    assert_eq!(
        said(
            &switches_done(SwitchType::Plug, Some("bureau"), true, &["prise"]),
            &[Ok]
        ),
        "Prise allumée dans le Bureau."
    );
}

#[test]
fn a_masculine_light_word_keeps_lumiere_for_the_agreement() {
    use ActionResult::Ok;
    assert_eq!(
        said(&lights_done(true, &["Plafonnier"]), &[Ok]),
        "Lumière « Plafonnier » allumée dans le Salon."
    );
    assert_eq!(
        said(&lights_done(false, &["Éclairage salon"]), &[Ok]),
        "Lumière « Éclairage salon » éteinte dans le Salon."
    );
}

#[test]
fn a_name_starting_with_the_kind_word_drops_the_prefix() {
    use ActionResult::Ok;
    assert_eq!(
        said(
            &covers_done(Some("salon"), false, None, &["Volet tv"]),
            &[Ok]
        ),
        "« Volet tv » fermé dans le Salon."
    );
    assert_eq!(
        said(&lights_done(true, &["Lampe table"]), &[Ok]),
        "« Lampe table » allumée dans le Salon."
    );
    assert_eq!(
        said(
            &switches_done(SwitchType::Plug, Some("bureau"), true, &["Prise bureau"]),
            &[Ok]
        ),
        "« Prise bureau » allumée dans le Bureau."
    );
    assert_eq!(
        said(
            &switches_done(SwitchType::Tv, Some("salon"), true, &["TV salon"]),
            &[Ok]
        ),
        "« TV salon » allumée dans le Salon."
    );
}

#[test]
fn any_other_name_keeps_the_kind_word() {
    use ActionResult::Ok;
    assert_eq!(
        said(&covers_done(Some("salon"), false, None, &["Baie"]), &[Ok]),
        "Volet « Baie » fermé dans le Salon."
    );
    assert_eq!(
        said(&lights_done(true, &["Éclat"]), &[Ok]),
        "Lumière « Éclat » allumée dans le Salon."
    );
}

#[test]
fn temp_format() {
    assert_eq!(temp(18.2), "18,2 °C");
    assert_eq!(temp(20.0), "20 °C");
    assert_eq!(temp(-0.5), "-0,5 °C");
    assert_eq!(temp(18.25), "18,3 °C");
}

#[test]
fn articles_follow_the_room_word() {
    assert_eq!(dans("Salon"), "dans le Salon");
    assert_eq!(dans("Chambre Léa"), "dans la Chambre Léa");
    assert_eq!(dans("Entrée"), "dans l'Entrée");
    assert_eq!(de("Salon"), "du Salon");
    assert_eq!(de("Chambre Tom"), "de la Chambre Tom");
    assert_eq!(le("Bureau"), "le Bureau");
}

#[test]
fn a_room_named_after_its_person_is_their_bedroom() {
    assert_eq!(le("Léa"), "la chambre de Léa");
    assert_eq!(de("Léa"), "de la chambre de Léa");
    assert_eq!(dans("Léa"), "dans la chambre de Léa");
    assert_eq!(room("Léa"), "Chambre de Léa");
    assert_eq!(agree("Léa", "réglé", "réglée"), "réglée");
    // Room words, another kind of room and the unassigned area keep their own name.
    assert_eq!(room("Salon"), "Salon");
    assert_eq!(room("Chambre Léa"), "Chambre Léa");
    assert_eq!(le("Toilettes"), "Toilettes");
    assert!(!le("Unassigned").contains("chambre"));
}

#[test]
fn replies_name_a_person_room_as_their_bedroom() {
    let f = fixture::person_rooms();
    let h = f.house();
    let d = Done::Setpoint {
        zone: "lea".into(),
        from: Some(17.5),
        to: 19.0,
    };
    assert_eq!(
        done(&d, &[ActionResult::Ok], &h, false),
        "Chambre de Léa réglée à 19 °C (avant 17,5 °C)."
    );
}

#[test]
fn lights_reply() {
    let f = fixture::house();
    let h = f.house();
    let ok = [ActionResult::Ok, ActionResult::Ok, ActionResult::Ok];
    let d = lights_done(true, &["Plafonnier", "Lampe table", "Led"]);
    assert_eq!(done(&d, &ok, &h, false), "Salon allumé (3 lumières).");
    let failed = [
        ActionResult::Ok,
        ActionResult::Ok,
        ActionResult::Failed("no echo".into()),
    ];
    let r = done(&d, &failed, &h, false);
    assert!(r.contains("n'a pas répondu"), "{r}");
    assert!(r.contains("Led"), "{r}");
    assert!(done(&d, &ok, &h, true).starts_with("Simulation :"));
    let off = lights_done(false, &["Led"]);
    let r = done(&off, &[ActionResult::Ok], &h, false);
    assert!(
        r.contains("Led") && r.contains("Salon") && r.contains("éteinte"),
        "{r}"
    );
    let skipped = [
        ActionResult::Skipped,
        ActionResult::Skipped,
        ActionResult::Skipped,
    ];
    assert_eq!(
        done(&d, &skipped, &h, false),
        "Salon déjà allumé (3 lumières)."
    );
}

#[test]
fn setpoint_reply() {
    let f = fixture::house();
    let h = f.house();
    let d = Done::Setpoint {
        zone: "chambre_lea".into(),
        from: Some(17.5),
        to: 19.0,
    };
    assert_eq!(
        done(&d, &[ActionResult::Ok], &h, false),
        "Chambre Léa réglée à 19 °C (avant 17,5 °C)."
    );
    let r = done(&d, &[ActionResult::Failed("refusé".into())], &h, false);
    assert!(
        r.contains("19 °C") && r.contains("refusé") && r.contains("Chambre Léa"),
        "{r}"
    );
}

fn media_done(what: crate::intent::MediaIntent) -> Done {
    Done::Media {
        player: "Musique".into(),
        what,
    }
}

fn echo(state: &str, title: Option<&str>, artist: Option<&str>) -> ActionResult {
    ActionResult::Echo(Box::new(nidoo_core::model::MediaPlayerState {
        state: Some(state.into()),
        title: title.map(Into::into),
        artist: artist.map(Into::into),
        ..Default::default()
    }))
}

#[test]
fn media_replies() {
    use crate::intent::MediaIntent as M;
    let lucky = echo("playing", Some("Get Lucky"), Some("Daft Punk"));
    let cases: Vec<(M, ActionResult, &str)> = vec![
        (
            M::Pause,
            echo("paused", Some("Get Lucky"), None),
            "Musique en pause.",
        ),
        (M::Pause, ActionResult::Ok, "Musique en pause."),
        (
            M::Pause,
            ActionResult::Skipped,
            "La musique est déjà en pause.",
        ),
        (
            M::Play { resume: true },
            lucky.clone(),
            "Je reprends Get Lucky.",
        ),
        (
            M::Play { resume: true },
            ActionResult::Ok,
            "Je reprends la musique.",
        ),
        (
            M::Play { resume: true },
            ActionResult::Skipped,
            "La musique joue déjà.",
        ),
        (
            M::Play { resume: false },
            lucky.clone(),
            "Je joue Get Lucky.",
        ),
        (
            M::Play { resume: false },
            ActionResult::Ok,
            "Je lance la musique.",
        ),
        (
            M::Play { resume: false },
            echo("playing", None, None),
            "Je lance la musique.",
        ),
        (
            M::PlayMedia {
                query: "daft punk".into(),
            },
            lucky.clone(),
            "Je joue Get Lucky.",
        ),
        (
            M::PlayMedia {
                query: "daft punk".into(),
            },
            ActionResult::Ok,
            "Je cherche « daft punk ».",
        ),
        (M::Next, lucky.clone(), "Get Lucky, Daft Punk."),
        (M::Next, echo("playing", Some("Intro"), None), "Intro."),
        (M::Next, ActionResult::Ok, "Chanson suivante."),
        (M::Previous, ActionResult::Ok, "Chanson précédente."),
        (M::Pause, ActionResult::Offline, "Le lecteur ne répond pas."),
        (
            M::Next,
            ActionResult::Failed("pas de réponse de l'appareil".into()),
            "Le lecteur n'a pas répondu.",
        ),
        (
            M::Next,
            ActionResult::Failed("hors ligne".into()),
            "Le lecteur n'a pas répondu : hors ligne.",
        ),
    ];
    for (what, result, want) in cases {
        assert_eq!(said(&media_done(what.clone()), &[result]), want, "{what:?}");
    }
}

#[test]
fn volume_and_mute_replies() {
    let volume = Done::Volume {
        area: "salon".into(),
        level: Some(60),
    };
    assert_eq!(
        said(&volume, &[ActionResult::Ok]),
        "Volume à 60 % dans le Salon."
    );
    assert_eq!(
        said(&volume, &[ActionResult::Offline]),
        "L'enceinte ne répond pas dans le Salon."
    );
    let mute = |on| Done::Mute {
        area: "salon".into(),
        on,
    };
    assert_eq!(
        said(&mute(true), &[ActionResult::Ok]),
        "Son coupé dans le Salon."
    );
    assert_eq!(
        said(&mute(false), &[ActionResult::Ok]),
        "Son remis dans le Salon."
    );
    assert_eq!(
        said(&mute(true), &[ActionResult::Skipped]),
        "Son déjà coupé dans le Salon."
    );
}

fn said_en(d: &Done, results: &[ActionResult]) -> String {
    let f = fixture::house();
    done(d, results, &f.house_en(), false)
}

#[test]
fn english_numbers_take_a_point() {
    assert_eq!(number_in(18.25, Lang::En), "18.3");
    assert_eq!(number_in(20.0, Lang::En), "20");
    assert_eq!(temp_in(19.5, Lang::En), "19.5 °C");
    assert_eq!(temp_in(-0.5, Lang::En), "-0.5 °C");
    assert_eq!(temp_in(19.5, Lang::Fr), "19,5 °C");
    assert_eq!(quoted("Desk", Lang::En), "\"Desk\"");
    assert_eq!(quoted("Baie", Lang::Fr), "« Baie »");
}

#[test]
fn english_room_phrases() {
    assert_eq!(the("Kitchen"), "the Kitchen");
    assert_eq!(in_the("Kitchen"), "in the Kitchen");
    assert_eq!(the_start("Kitchen"), "The Kitchen");
    assert_eq!(the("Tom"), "Tom's room");
    assert_eq!(in_the("Tom"), "in Tom's room");
    assert_eq!(the("The Den"), "The Den");
    assert_eq!(the("Tom's Den"), "Tom's Den");
    assert_eq!(the("living room"), "the living room");
    assert_eq!(the("TV room"), "the TV room");
    assert_eq!(room_in("Tom", Lang::En), "Tom's room");
    assert_eq!(room_in("Tom's Den", Lang::En), "Tom's Den");
    assert_eq!(room_in("Living room", Lang::En), "Living room");
    assert_eq!(room_in("Léa", Lang::Fr), "Chambre de Léa");
    assert_eq!(place("Salon", Lang::Fr), "dans le Salon");
    assert_eq!(place("Salon", Lang::En), "in the Salon");
}

#[test]
fn english_cannot() {
    let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        cannot_in(&names(&["Volet"]), true, Some(30), Lang::En),
        "Volet can't be set to a position."
    );
    assert_eq!(
        cannot_in(&names(&["Bay", "Door"]), false, None, Lang::En),
        "Bay, Door can't close."
    );
    assert_eq!(
        cannot(&names(&["Volet"]), true, None),
        cannot_in(&names(&["Volet"]), true, None, Lang::Fr)
    );
}

#[test]
fn english_lights_replies() {
    use ActionResult::{Failed, Offline, Ok, Skipped};
    let many = lights_done(true, &["Plafonnier", "Lampe table", "Led"]);
    assert_eq!(said_en(&many, &[Ok, Ok, Ok]), "Salon lights on (3 lights).");
    assert_eq!(
        said_en(&many, &[Skipped, Skipped, Skipped]),
        "Salon lights already on (3 lights)."
    );
    assert_eq!(
        said_en(&many, &[Ok, Offline, Ok]),
        "Salon lights on (3 lights). 1 of 3 didn't answer: Lampe table."
    );
    assert_eq!(
        said_en(&many, &[Offline, Offline, Offline]),
        "Nothing answered in the Salon: Plafonnier, Lampe table, Led."
    );
    assert_eq!(
        said_en(&lights_done(false, &["Desk lamp"]), &[Skipped]),
        "Desk lamp already off in the Salon."
    );
    assert_eq!(
        said_en(&lights_done(false, &["Glow"]), &[Ok]),
        "Light \"Glow\" off in the Salon."
    );
    assert_eq!(
        said_en(&lights_done(true, &["Led"]), &[Failed("refused".into())]),
        "\"Led\" didn't answer in the Salon: refused."
    );
    assert_eq!(
        said_en(&lights_done(true, &["Led"]), &[Offline]),
        "\"Led\" didn't answer in the Salon: offline."
    );
}

#[test]
fn english_colour_replies() {
    let coloured = |names: &[&str], unable: &[&str]| Done::Lights {
        area: "salon".into(),
        on: true,
        names: names.iter().map(|n| n.to_string()).collect(),
        colour: Some("red".into()),
        brightness: None,
        unable: unable.iter().map(|n| n.to_string()).collect(),
    };
    assert_eq!(
        said_en(&coloured(&["Led"], &[]), &[ActionResult::Ok]),
        "Led on in red in the Salon."
    );
    assert_eq!(
        said_en(
            &coloured(&["Led", "Lampe table"], &["Plafonnier"]),
            &[ActionResult::Ok, ActionResult::Ok]
        ),
        "Salon lights on in red (2 lights). \"Plafonnier\" can't take a colour."
    );
    assert_eq!(
        said_en(
            &coloured(&["Led"], &["Plafonnier", "Lamp"]),
            &[ActionResult::Ok]
        ),
        "Led on in red in the Salon. Can't take a colour: Plafonnier, Lamp."
    );
}

#[test]
fn english_setpoint_replies() {
    let f = fixture::house();
    let h = f.house_en();
    let d = |from| Done::Setpoint {
        zone: "chambre_lea".into(),
        from,
        to: 19.5,
    };
    assert_eq!(
        done(&d(Some(17.0)), &[ActionResult::Ok], &h, false),
        "Heating in the Chambre Léa set to 19.5 °C (was 17 °C)."
    );
    assert_eq!(
        done(&d(None), &[ActionResult::Ok], &h, false),
        "Heating in the Chambre Léa set to 19.5 °C."
    );
    assert_eq!(
        done(&d(Some(19.5)), &[ActionResult::Skipped], &h, false),
        "Heating in the Chambre Léa already at 19.5 °C."
    );
    assert_eq!(
        done(
            &d(Some(17.0)),
            &[ActionResult::Failed("refused".into())],
            &h,
            false
        ),
        "The heating in the Chambre Léa wasn't set to 19.5 °C: refused."
    );
    let f = fixture::person_rooms();
    let h = f.house_en();
    let d = Done::Setpoint {
        zone: "tom".into(),
        from: None,
        to: 20.0,
    };
    assert_eq!(
        done(&d, &[ActionResult::Ok], &h, false),
        "Heating in Tom's room set to 20 °C."
    );
}

#[test]
fn english_covers_and_switches_replies() {
    use ActionResult::{Offline, Ok, Skipped};
    let salon = Some("salon");
    assert_eq!(
        said_en(&covers_done(salon, false, None, &["Bay"]), &[Ok]),
        "Shutter \"Bay\" closed in the Salon."
    );
    assert_eq!(
        said_en(
            &covers_done(salon, true, Some(30), &["Bay shutter"]),
            &[Skipped]
        ),
        "Bay shutter already open at 30 % in the Salon."
    );
    assert_eq!(
        said_en(
            &covers_done(salon, false, None, &["Bay", "Window"]),
            &[Ok, Ok]
        ),
        "Salon: shutters closed (2 shutters)."
    );
    assert_eq!(
        said_en(
            &covers_done(None, false, None, &["Bay", "Window", "Volet"]),
            &[Ok, Offline, Ok]
        ),
        "All shutters closed (3 shutters). 1 of 3 didn't answer: Window."
    );
    let mut d = covers_done(None, true, Some(30), &["Bay"]);
    if let Done::Covers { unable, .. } = &mut d {
        unable.push("Volet".into());
    }
    assert_eq!(
        said_en(&d, &[Ok]),
        "Shutter \"Bay\" open at 30 %. Volet can't be set to a position."
    );
    use SwitchType::{Plug, Tv};
    assert_eq!(
        said_en(&switches_done(Tv, salon, true, &["Screen"]), &[Ok]),
        "TV \"Screen\" on in the Salon."
    );
    assert_eq!(
        said_en(
            &switches_done(Plug, Some("bureau"), false, &["Coffee plug"]),
            &[Skipped]
        ),
        "Coffee plug already off in the Bureau."
    );
    assert_eq!(
        said_en(
            &switches_done(Plug, Some("bureau"), false, &["Coffee", "Charger"]),
            &[Ok, Ok]
        ),
        "Bureau: plugs off (2 plugs)."
    );
    assert_eq!(
        said_en(
            &switches_done(Tv, None, true, &["Screen", "Projector"]),
            &[Skipped, Skipped]
        ),
        "All TVs already on (2 TVs)."
    );
}

#[test]
fn english_media_and_speakers_replies() {
    use crate::intent::MediaIntent as M;
    let lucky = echo("playing", Some("Get Lucky"), Some("Daft Punk"));
    let cases: Vec<(M, ActionResult, &str)> = vec![
        (M::Pause, ActionResult::Ok, "Music paused."),
        (
            M::Pause,
            ActionResult::Skipped,
            "The music is already paused.",
        ),
        (
            M::Play { resume: true },
            lucky.clone(),
            "Resuming Get Lucky.",
        ),
        (
            M::Play { resume: false },
            ActionResult::Ok,
            "Starting the music.",
        ),
        (
            M::Play { resume: false },
            lucky.clone(),
            "Playing Get Lucky.",
        ),
        (
            M::PlayMedia {
                query: "daft punk".into(),
            },
            ActionResult::Ok,
            "Looking for \"daft punk\".",
        ),
        (M::Next, lucky.clone(), "Get Lucky, Daft Punk."),
        (M::Previous, ActionResult::Ok, "Previous song."),
        (
            M::Pause,
            ActionResult::Offline,
            "The player isn't responding.",
        ),
        (
            M::Next,
            ActionResult::Failed(NO_ANSWER_EN.into()),
            "The player didn't answer.",
        ),
        (
            M::Next,
            ActionResult::Failed("offline".into()),
            "The player didn't answer: offline.",
        ),
    ];
    for (what, result, want) in cases {
        assert_eq!(
            said_en(&media_done(what.clone()), &[result]),
            want,
            "{what:?}"
        );
    }
    let volume = Done::Volume {
        area: "bureau".into(),
        level: Some(30),
    };
    assert_eq!(
        said_en(&volume, &[ActionResult::Ok]),
        "Volume at 30 % in the Bureau."
    );
    assert_eq!(
        said_en(&volume, &[ActionResult::Offline]),
        "The speaker isn't responding in the Bureau."
    );
    assert_eq!(
        said_en(&volume, &[ActionResult::Failed("refused".into())]),
        "The speaker didn't answer in the Bureau: refused."
    );
    let mute = |on| Done::Mute {
        area: "salon".into(),
        on,
    };
    assert_eq!(
        said_en(&mute(true), &[ActionResult::Ok]),
        "Sound muted in the Salon."
    );
    assert_eq!(
        said_en(&mute(false), &[ActionResult::Skipped]),
        "Sound already on in the Salon."
    );
}

#[test]
fn english_simulation_prefix() {
    let f = fixture::house();
    let d = lights_done(true, &["Led"]);
    assert_eq!(
        done(&d, &[ActionResult::Ok], &f.house_en(), true),
        "Simulation: Led on in the Salon."
    );
    assert!(done(&d, &[ActionResult::Ok], &f.house(), true).starts_with("Simulation : "));
}

#[test]
fn vacuum_replies() {
    let d = |action, room: Option<&str>| Done::Vacuum {
        action,
        room: room.map(Into::into),
        name: "Aspirateur".into(),
    };
    let clean = |room| d(VacuumIntent::Clean, room);
    let ok = [ActionResult::Ok];
    assert_eq!(
        said(&clean(Some("cuisine")), &ok),
        "L'aspirateur nettoie la cuisine."
    );
    assert_eq!(
        said(&clean(Some("salon")), &ok),
        "L'aspirateur nettoie le salon."
    );
    assert_eq!(
        said(&clean(Some("entrée")), &ok),
        "L'aspirateur nettoie l'entrée."
    );
    assert_eq!(
        said(&clean(Some("salle de bain")), &ok),
        "L'aspirateur nettoie la salle de bain."
    );
    assert_eq!(said(&clean(None), &ok), "L'aspirateur est lancé.");
    assert_eq!(
        said(&d(VacuumIntent::Stop, None), &ok),
        "L'aspirateur est arrêté."
    );
    assert_eq!(
        said(&d(VacuumIntent::Pause, None), &ok),
        "L'aspirateur est en pause."
    );
    assert_eq!(
        said(&d(VacuumIntent::Dock, None), &ok),
        "L'aspirateur rentre à sa base."
    );
    assert_eq!(
        said(&clean(None), &[ActionResult::Skipped]),
        "L'aspirateur est déjà en marche."
    );
    assert_eq!(
        said(&d(VacuumIntent::Dock, None), &[ActionResult::Skipped]),
        "L'aspirateur est déjà à sa base."
    );
    assert_eq!(
        said(&clean(None), &[ActionResult::Offline]),
        "L'aspirateur ne répond pas."
    );
    assert_eq!(
        said(&clean(None), &[ActionResult::Failed(NO_ANSWER.into())]),
        "L'aspirateur n'a pas répondu."
    );
    assert_eq!(
        said(&clean(None), &[ActionResult::Failed("refusé".into())]),
        "L'aspirateur n'a pas répondu : refusé."
    );
    let f = fixture::house();
    assert_eq!(
        done(&clean(None), &ok, &f.house(), true),
        "Simulation : L'aspirateur est lancé."
    );
}

#[test]
fn english_vacuum_replies() {
    let d = |action, room: Option<&str>| Done::Vacuum {
        action,
        room: room.map(Into::into),
        name: "Vacuum".into(),
    };
    let ok = [ActionResult::Ok];
    assert_eq!(
        said_en(&d(VacuumIntent::Clean, Some("kitchen")), &ok),
        "The vacuum is cleaning the kitchen."
    );
    assert_eq!(
        said_en(&d(VacuumIntent::Clean, None), &ok),
        "The vacuum has started."
    );
    assert_eq!(
        said_en(&d(VacuumIntent::Stop, None), &ok),
        "The vacuum has stopped."
    );
    assert_eq!(
        said_en(&d(VacuumIntent::Pause, None), &ok),
        "The vacuum is paused."
    );
    assert_eq!(
        said_en(&d(VacuumIntent::Dock, None), &ok),
        "The vacuum is going back to its dock."
    );
    assert_eq!(
        said_en(&d(VacuumIntent::Stop, None), &[ActionResult::Skipped]),
        "The vacuum is already stopped."
    );
    assert_eq!(
        said_en(&d(VacuumIntent::Stop, None), &[ActionResult::Offline]),
        "The vacuum isn't responding."
    );
}
