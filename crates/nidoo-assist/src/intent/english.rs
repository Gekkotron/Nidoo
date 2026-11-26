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
        brightness: None,
    })
}

fn coloured(names: &[&str], room: &[&str], colour: Colour) -> Option<Intent> {
    Some(Intent::Lights {
        on: true,
        all: false,
        names: v(names),
        room: v(room),
        colour: Some(colour),
        brightness: None,
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

fn bright(names: &[&str], room: &[&str], colour: Option<Colour>, percent: u8) -> Option<Intent> {
    Some(Intent::Lights {
        on: true,
        all: false,
        names: v(names),
        room: v(room),
        colour,
        brightness: Some(percent),
    })
}

#[test]
fn light_brightness() {
    assert_eq!(
        p("turn on the lamp in the office at 50 %"),
        bright(&["lamp"], &["office"], None, 50)
    );
    assert_eq!(
        p("set the kitchen lights to 30 percent"),
        bright(&[], &["kitchen"], None, 30)
    );
    assert_eq!(
        p("turn the lamp to 80 per cent"),
        bright(&["lamp"], &[], None, 80)
    );
    assert_eq!(
        p("turn on the kitchen lights in blue at 100 %"),
        bright(&[], &["kitchen"], Some(Colour::Blue), 100)
    );
    assert_eq!(p("turn off the kitchen lights at 50 %"), None);
    assert_eq!(p("set the kitchen to 20 %"), None);
    assert_eq!(p("turn on the heating at 50 %"), None);
    assert_eq!(p("turn on the lamp at 0 %"), Some(Intent::BrightnessRange));
    assert_eq!(
        p("turn on the lamp at 101 %"),
        Some(Intent::BrightnessRange)
    );
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

fn vacuum(action: VacuumIntent, room: &[&str]) -> Option<Intent> {
    Some(Intent::Vacuum {
        action,
        room: v(room),
    })
}

#[test]
fn vacuum_sentences() {
    assert_eq!(
        p("vacuum the kitchen"),
        vacuum(VacuumIntent::Clean, &["kitchen"])
    );
    assert_eq!(
        p("clean the living room"),
        vacuum(VacuumIntent::Clean, &["living"])
    );
    assert_eq!(
        p("can you vacuum the office please"),
        vacuum(VacuumIntent::Clean, &["office"])
    );
    assert_eq!(p("start the vacuum"), vacuum(VacuumIntent::Clean, &[]));
    assert_eq!(p("stop the vacuum"), vacuum(VacuumIntent::Stop, &[]));
    assert_eq!(p("pause the vacuum"), vacuum(VacuumIntent::Pause, &[]));
    assert_eq!(p("send the vacuum home"), vacuum(VacuumIntent::Dock, &[]));
    assert_eq!(
        p("send the vacuum back to the dock"),
        vacuum(VacuumIntent::Dock, &[])
    );
    assert_eq!(
        p("clean the house"),
        vacuum(VacuumIntent::Clean, &["house"])
    );
}

#[test]
fn vacuum_never_guessed() {
    assert_eq!(p("don't vacuum the kitchen"), None);
    assert_eq!(p("the vacuum"), None);
    assert_eq!(p("is the vacuum running?"), None);
    assert_eq!(p("who started the vacuum"), None);
}

/// The table of docs/users/assistant.md, both columns.
#[test]
fn the_documented_sentences_parse() {
    let en = [
        "turn on the kitchen lights",
        "turn off the lamp in the office",
        "turn everything off",
        "turn the living room lights red",
        "turn on the lamp in blue",
        "set the bedroom to 19.5",
        "turn up the heating in the office",
        "turn the heat down a bit",
        "what's the temperature in the kitchen?",
        "how cold is it outside?",
        "open the shutters to 30%",
        "close all the blinds",
        "turn off the TV",
        "switch on the coffee plug",
        "play Shape of You",
        "pause",
        "next song",
        "turn the volume up",
        "mute the kitchen",
        "what's playing?",
        "vacuum the kitchen",
        "start the vacuum",
        "send the vacuum home",
    ];
    for s in en {
        assert!(m(s).is_some(), "{s}");
    }
    let fr = [
        "allume la lumière de la cuisine",
        "éteins la lampe du bureau",
        "éteins tout",
        "passe le salon en rouge",
        "allume la lampe en bleu",
        "règle la chambre à 19,5",
        "monte le chauffage du bureau",
        "baisse un peu le chauffage",
        "quelle température dans la cuisine ?",
        "il fait combien dehors ?",
        "ouvre les volets à 30 %",
        "ferme tous les volets",
        "éteins la télé",
        "allume la prise café",
        "joue Shape of You",
        "pause",
        "chanson suivante",
        "monte le son",
        "coupe le son de la cuisine",
        "qu'est-ce qu'on écoute ?",
        "aspire la cuisine",
        "lance l'aspirateur",
        "renvoie l'aspirateur à la base",
    ];
    for s in fr {
        assert!(
            super::play_by_name(s)
                .or_else(|| parse(&words(s)))
                .is_some(),
            "{s}"
        );
    }
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
