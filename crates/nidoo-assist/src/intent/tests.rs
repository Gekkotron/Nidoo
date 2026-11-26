use super::*;
use crate::lexicon::colour_of;
use crate::text::words;

fn p(s: &str) -> Option<Intent> {
    parse(&words(s))
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

fn coloured(names: &[&str], room: &[&str], colour: &str) -> Option<Intent> {
    Some(Intent::Lights {
        on: true,
        all: false,
        names: v(names),
        room: v(room),
        colour: colour_of(colour),
        brightness: None,
    })
}

#[test]
fn colour_after_en() {
    assert_eq!(
        p("allume la lumière du salon en rouge"),
        coloured(&[], &["salon"], "rouge")
    );
    assert_eq!(
        p("allume la led en bleu dans le salon"),
        coloured(&["led"], &["salon"], "bleu")
    );
    assert_eq!(
        p("mets la lampe de la chambre en vert"),
        coloured(&["lampe"], &["chambre"], "vert")
    );
    assert_eq!(
        p("mets le salon en rose"),
        coloured(&[], &["salon"], "rose")
    );
    assert_eq!(
        p("passe les lumières du salon en blanc"),
        coloured(&[], &["salon"], "blanc")
    );
    assert_eq!(p("Allume en Violette"), coloured(&[], &[], "violet"));
}

fn bright(names: &[&str], room: &[&str], colour: Option<&str>, percent: u8) -> Option<Intent> {
    Some(Intent::Lights {
        on: true,
        all: false,
        names: v(names),
        room: v(room),
        colour: colour.and_then(colour_of),
        brightness: Some(percent),
    })
}

#[test]
fn brightness_after_a() {
    assert_eq!(
        p("allume les leds du salon en bleu à 100 %"),
        bright(&["leds"], &["salon"], Some("bleu"), 100)
    );
    assert_eq!(
        p("allume les leds du salon à 50 % en rouge"),
        bright(&["leds"], &["salon"], Some("rouge"), 50)
    );
    assert_eq!(
        p("allume la led à 50 pour cent"),
        bright(&["led"], &[], None, 50)
    );
    assert_eq!(
        p("allume le salon à 30 %"),
        bright(&[], &["salon"], None, 30)
    );
    assert_eq!(
        p("mets la lumière du salon à 40 %"),
        bright(&[], &["salon"], None, 40)
    );
    assert_eq!(
        p("passe la lampe du bureau à 70 %"),
        bright(&["lampe"], &["bureau"], None, 70)
    );
    assert_eq!(
        p("mets le salon en bleu à 20 %"),
        bright(&[], &["salon"], Some("bleu"), 20)
    );
}

#[test]
fn brightness_never_guessed() {
    // Off, heating without a light word, plugs, TVs, shutters and music are as before.
    assert_eq!(p("éteins le salon à 50 %"), None);
    assert_eq!(p("éteins la led du salon à 50 %"), None);
    assert_eq!(p("mets le salon à 20 %"), None);
    assert_eq!(p("allume la prise du bureau à 50 %"), None);
    assert_eq!(p("allume la télé du salon à 50 %"), None);
    assert_eq!(p("allume le chauffage à 50 %"), None);
    assert!(matches!(
        p("ferme les volets du salon à 50 %"),
        Some(Intent::CoverPositionHint)
    ));
    // Out of range: a hint, not a guess.
    assert_eq!(p("allume la led à 0 %"), Some(Intent::BrightnessRange));
    assert_eq!(p("allume la led à 150 %"), Some(Intent::BrightnessRange));
    // The brightness is not a setpoint.
    assert_eq!(p("allume la led à 50"), None);
}

#[test]
fn colour_never_guessed() {
    // Off, heating, shutters, music or two colours: not understood.
    assert_eq!(p("éteins le salon en rouge"), None);
    assert_eq!(p("mets le chauffage en rouge"), None);
    assert_eq!(p("ouvre les volets en bleu"), None);
    assert_eq!(p("mets la musique en bleu"), None);
    assert_eq!(p("allume le salon en rouge et en bleu"), None);
    assert_eq!(p("ne mets pas le salon en rouge"), None);
    assert_eq!(p("mets le salon en rouge à 20"), None);
    // The same colour twice is one colour.
    assert_eq!(
        p("allume le salon en rouge, en rouge"),
        coloured(&[], &["salon"], "rouge")
    );
    // Without "en" a colour word is part of a name, as before.
    assert_eq!(
        p("allume la lampe rose"),
        lights(true, false, &["lampe"], &["rose"])
    );
}

fn set(change: Change, room: &[&str]) -> Option<Intent> {
    Some(Intent::SetTemp {
        change,
        room: v(room),
    })
}

#[test]
fn on_and_off_verbs() {
    assert_eq!(
        p("allume la lumière du salon"),
        lights(true, false, &[], &["salon"])
    );
    assert_eq!(
        p("éteins la lumière du salon"),
        lights(false, false, &[], &["salon"])
    );
    assert_eq!(
        p("coupe la lumière du salon"),
        lights(false, false, &[], &["salon"])
    );
    assert_eq!(p("allume le salon"), lights(true, false, &[], &["salon"]));
    assert_eq!(
        p("allume la chambre de Léa s'il te plaît"),
        lights(true, false, &["chambre"], &["lea"])
    );
}

#[test]
fn typo_never_flips_on_off() {
    assert_eq!(p("allumme le salon"), lights(true, false, &[], &["salon"]));
    assert_eq!(p("etein le salon"), lights(false, false, &[], &["salon"]));
    assert_eq!(p("eteint le salon"), lights(false, false, &[], &["salon"]));
    assert_eq!(p("allume et éteins le salon"), None);
    assert_eq!(p("allumme et etein le salon"), None);
}

#[test]
fn all_word() {
    assert_eq!(
        p("éteins tout dans le salon"),
        lights(false, true, &[], &["salon"])
    );
}

#[test]
fn light_names() {
    assert_eq!(
        p("éteins la led du salon"),
        lights(false, false, &["led"], &["salon"])
    );
    assert_eq!(
        p("éteins le lustre du salon"),
        lights(false, false, &["lustre"], &["salon"])
    );
    assert_eq!(p("allume la led"), lights(true, false, &["led"], &[]));
}

#[test]
fn absolute_setpoints() {
    assert_eq!(
        p("mets le salon à 20 degrés"),
        set(Change::Absolute(20.0), &["salon"])
    );
    assert_eq!(
        p("règle chez Léa à 18,5"),
        set(Change::Absolute(18.5), &["lea"])
    );
    assert_eq!(
        p("mets la chambre de Tom à dix-neuf"),
        set(Change::Absolute(19.0), &["chambre", "tom"])
    );
    assert_eq!(
        p("mets le chauffage à 20"),
        set(Change::Absolute(20.0), &[])
    );
}

#[test]
fn relative_setpoints() {
    assert_eq!(
        p("monte le chauffage d'un degré chez Léa"),
        set(Change::Relative(1.0), &["lea"])
    );
    assert_eq!(
        p("baisse un peu le chauffage"),
        set(Change::Relative(-0.5), &[])
    );
    assert_eq!(p("monte le chauffage"), set(Change::Relative(1.0), &[]));
    assert_eq!(
        p("monte un peu le chauffage"),
        set(Change::Relative(0.5), &[])
    );
    assert_eq!(
        p("chauffage +2 au bureau"),
        set(Change::Relative(2.0), &["bureau"])
    );
    assert_eq!(
        p("baisse le chauffage de deux degrés dans le salon"),
        set(Change::Relative(-2.0), &["salon"])
    );
}

#[test]
fn raise_without_heating_word_is_none() {
    // "monte le salon" could be a shutter: never guess heating.
    assert_eq!(p("monte le salon"), None);
}

#[test]
fn heating_without_value() {
    assert_eq!(
        p("allume le chauffage du salon"),
        Some(Intent::HeatingNoValue {
            room: v(&["salon"])
        })
    );
}

#[test]
fn feelings_step_the_heating() {
    let feel = |warmer: bool, room: &[&str]| {
        Some(Intent::Feel {
            warmer,
            room: v(room),
        })
    };
    assert_eq!(p("j'ai froid au salon"), feel(true, &["salon"]));
    assert_eq!(p("il fait froid"), feel(true, &[]));
    assert_eq!(
        p("il fait trop chaud dans la chambre de Tom"),
        feel(false, &["chambre", "tom"])
    );
    assert_eq!(
        p("ça caille dans la chambre 2"),
        feel(true, &["chambre", "2"])
    );
    assert_eq!(p("j'ai un peu chaud"), feel(false, &[]));
}

#[test]
fn feelings_never_guessed() {
    assert_eq!(p("je n'ai pas froid"), None);
    assert_eq!(p("j'ai froid et chaud"), None);
    assert_eq!(p("il fait froid à 18"), None);
    assert_eq!(
        p("il fait froid au salon ?"),
        Some(Intent::ReadTemp {
            room: v(&["salon"]),
            outdoor: false
        })
    );
    assert_eq!(
        p("il fait froid dehors"),
        Some(Intent::ReadTemp {
            room: vec![],
            outdoor: true
        })
    );
    // With a device word, "chaude" is a plain word as before.
    assert_eq!(
        p("allume la lumière chaude du salon"),
        lights(true, false, &["chaude"], &["salon"])
    );
}

#[test]
fn temperature_questions() {
    assert_eq!(
        p("quelle température fait-il dans le salon ?"),
        Some(Intent::ReadTemp {
            room: v(&["salon"]),
            outdoor: false
        })
    );
    assert_eq!(
        p("il fait combien dans le bureau ?"),
        Some(Intent::ReadTemp {
            room: v(&["bureau"]),
            outdoor: false
        })
    );
    assert_eq!(
        p("il fait combien dehors ?"),
        Some(Intent::ReadTemp {
            room: vec![],
            outdoor: true
        })
    );
}

#[test]
fn percent_is_not_a_setpoint() {
    assert_eq!(p("mets le salon à 20 %"), None);
    assert_eq!(p("mets le salon à 20 pour cent"), None);
}

#[test]
fn light_count_is_not_a_setpoint() {
    assert_eq!(
        p("allume les 3 lumières du salon"),
        lights(true, false, &[], &["salon"])
    );
}

#[test]
fn other_devices_are_none() {
    assert_eq!(p("ferme les rideaux du salon"), None);
}

fn covers(
    open: bool,
    position: Option<u8>,
    all: bool,
    names: &[&str],
    room: &[&str],
) -> Option<Intent> {
    Some(Intent::Covers {
        open,
        position,
        all,
        names: v(names),
        room: v(room),
    })
}

#[test]
fn shutter_open_close() {
    assert_eq!(
        p("ouvre les volets du salon"),
        covers(true, None, false, &[], &["salon"])
    );
    assert_eq!(
        p("ferme les volets du salon"),
        covers(false, None, false, &[], &["salon"])
    );
    assert_eq!(
        p("baisse les stores du bureau"),
        covers(false, None, false, &[], &["bureau"])
    );
    assert_eq!(
        p("remonte les volets roulants du salon"),
        covers(true, None, false, &[], &["salon"])
    );
    assert_eq!(
        p("peux-tu fermer les volets du salon"),
        covers(false, None, false, &[], &["salon"])
    );
}

#[test]
fn shutter_position() {
    assert_eq!(
        p("ouvre les volets du salon à 30 %"),
        covers(true, Some(30), false, &[], &["salon"])
    );
    assert_eq!(
        p("ouvre les volets à 30 pour cent"),
        covers(true, Some(30), false, &[], &[])
    );
    assert_eq!(
        p("ferme les volets à 30 %"),
        Some(Intent::CoverPositionHint)
    );
    assert_eq!(
        p("ouvre les volets à 130 %"),
        Some(Intent::CoverPositionRange)
    );
    assert_eq!(p("ouvre les volets à 30"), None);
    assert_eq!(p("ouvre les volets 30 %"), None);
}

#[test]
fn shutter_all_and_names() {
    assert_eq!(
        p("ferme tous les volets"),
        covers(false, None, true, &[], &[])
    );
    assert_eq!(
        p("ouvre tous les volets du salon"),
        covers(true, None, true, &[], &["salon"])
    );
    assert_eq!(
        p("ferme le volet baie du salon"),
        covers(false, None, false, &["baie"], &["salon"])
    );
    assert_eq!(
        p("ferme les 2 volets du salon"),
        covers(false, None, false, &[], &["salon"])
    );
}

#[test]
fn shutter_never_guessed() {
    assert_eq!(p("ne ferme pas les volets"), None);
    assert_eq!(p("les volets sont fermés"), None);
    assert_eq!(p("ferme les rideaux"), None);
    assert_eq!(p("ouvre et ferme les volets"), None);
    assert_eq!(p("les volets du salon"), None);
}

#[test]
fn a_shutter_question_is_not_an_order() {
    // "fermés" folds to "fermes", a close verb: the question mark keeps it a question.
    assert_eq!(p("les volets du salon fermés ?"), None);
    assert_eq!(
        p("peux-tu fermer les volets du salon ?"),
        covers(false, None, false, &[], &["salon"])
    );
}

#[test]
fn remonter_opens() {
    assert_eq!(
        p("peux-tu remonter les volets du salon"),
        covers(true, None, false, &[], &["salon"])
    );
}

#[test]
fn raise_without_shutter_noun_stays_not_understood() {
    assert_eq!(p("monte le salon"), None);
    assert_eq!(p("baisse le salon"), None);
}

#[test]
fn close_verb_on_a_light_is_not_understood() {
    assert_eq!(p("ferme la lumière du salon"), None);
    assert_eq!(p("ouvre le chauffage"), None);
    assert_eq!(p("ferme le salon"), None);
}

fn switches(
    what: SwitchType,
    on: bool,
    all: bool,
    names: &[&str],
    room: &[&str],
) -> Option<Intent> {
    Some(Intent::Switches {
        what,
        on,
        all,
        names: v(names),
        room: v(room),
    })
}

#[test]
fn tv_on_off() {
    use SwitchType::Tv;
    assert_eq!(p("allume la télé"), switches(Tv, true, false, &[], &[]));
    assert_eq!(
        p("éteins la télé du salon"),
        switches(Tv, false, false, &[], &["salon"])
    );
    assert_eq!(p("éteins les télés"), switches(Tv, false, false, &[], &[]));
    assert_eq!(
        p("peux-tu allumer la télévision du salon"),
        switches(Tv, true, false, &[], &["salon"])
    );
    assert_eq!(
        p("coupe la tv du salon"),
        switches(Tv, false, false, &[], &["salon"])
    );
}

#[test]
fn plug_on_off_names_and_all() {
    use SwitchType::Plug;
    assert_eq!(
        p("éteins la prise du bureau"),
        switches(Plug, false, false, &[], &["bureau"])
    );
    assert_eq!(
        p("allume la prise bureau"),
        switches(Plug, true, false, &[], &["bureau"])
    );
    assert_eq!(
        p("éteins toutes les prises"),
        switches(Plug, false, true, &[], &[])
    );
    assert_eq!(
        p("allume les 2 prises du bureau"),
        switches(Plug, true, false, &[], &["bureau"])
    );
    assert_eq!(
        p("allume la prise cafetière du bureau"),
        switches(Plug, true, false, &["cafetiere"], &["bureau"])
    );
}

#[test]
fn switches_never_guessed() {
    assert_eq!(p("allume la télé et la prise"), None);
    assert_eq!(p("allume et éteins la prise du bureau"), None);
    assert_eq!(p("la prise du bureau"), None);
    assert_eq!(p("ouvre la prise du bureau"), None);
    assert_eq!(p("monte la télé"), None);
    assert_eq!(p("mets la télé à 20"), None);
    assert_eq!(p("allume la prise du bureau à 50 %"), None);
    assert_eq!(p("allume la prise à 3"), None);
    assert_eq!(p("allume la prise de 2"), None);
    assert_eq!(p("n'éteins pas la télé"), None);
    assert_eq!(p("la télé est allumée"), None);
    assert_eq!(p("la télé du salon allumée ?"), None);
    assert_eq!(p("qui a éteint la prise du bureau"), None);
    assert_eq!(p("éteins le rideau de la télé"), None);
}

#[test]
fn a_light_cover_or_heating_word_keeps_tv_and_plug_as_names() {
    assert_eq!(
        p("allume la lumière du salon tv"),
        lights(true, false, &[], &["salon", "tv"])
    );
    assert_eq!(
        p("ferme le volet tv"),
        covers(false, None, false, &[], &["tv"])
    );
    assert_eq!(
        p("allume la lampe tv du salon"),
        lights(true, false, &["lampe", "tv"], &["salon"])
    );
    assert_eq!(
        p("allume la lumière de la prise"),
        lights(true, false, &[], &["prise"])
    );
    assert_eq!(
        p("allume le chauffage du salon tv"),
        Some(Intent::HeatingNoValue {
            room: v(&["salon", "tv"])
        })
    );
}

#[test]
fn negation_is_none() {
    assert_eq!(p("n'allume pas la lumière du salon"), None);
    assert_eq!(p("ne coupe pas le salon"), None);
}

#[test]
fn unknown_is_none() {
    assert_eq!(p("fais-moi un café"), None);
    assert_eq!(p(""), None);
}

fn media(m: MediaIntent) -> Option<Intent> {
    Some(Intent::Media(m))
}

fn volume(change: Change, room: &[&str]) -> Option<Intent> {
    Some(Intent::Volume {
        change,
        room: v(room),
    })
}

#[test]
fn music_sentences() {
    assert_eq!(p("mets pause"), media(MediaIntent::Pause));
    assert_eq!(p("pause"), media(MediaIntent::Pause));
    assert_eq!(p("pause la musique"), media(MediaIntent::Pause));
    assert_eq!(p("stop la musique"), media(MediaIntent::Pause));
    assert_eq!(p("arrête la musique"), media(MediaIntent::Pause));
    assert_eq!(
        p("reprends la musique"),
        media(MediaIntent::Play { resume: true })
    );
    assert_eq!(
        p("relance la musique"),
        media(MediaIntent::Play { resume: true })
    );
    assert_eq!(
        p("mets de la musique"),
        media(MediaIntent::Play { resume: false })
    );
    assert_eq!(
        p("lance de la musique"),
        media(MediaIntent::Play { resume: false })
    );
    assert_eq!(
        p("peux-tu mettre de la musique"),
        media(MediaIntent::Play { resume: false })
    );
    assert_eq!(p("mets de la musique classique"), Some(Intent::MediaHint));
    assert_eq!(p("chanson suivante"), media(MediaIntent::Next));
    assert_eq!(p("musique suivante"), media(MediaIntent::Next));
    assert_eq!(p("passe à la suivante"), media(MediaIntent::Next));
    assert_eq!(p("chanson précédente"), media(MediaIntent::Previous));
    assert_eq!(p("reviens en arrière"), media(MediaIntent::Previous));
    assert_eq!(p("chanson suivante et précédente"), None);
    assert_eq!(
        p("c'est quoi cette chanson ?"),
        media(MediaIntent::NowPlaying)
    );
    assert_eq!(
        p("qu'est-ce qu'on écoute ?"),
        media(MediaIntent::NowPlaying)
    );
    assert_eq!(p("quel est le titre ?"), media(MediaIntent::NowPlaying));
    assert_eq!(p("j'ai mis pause"), None);
    assert_eq!(p("ne coupe pas le son"), None);
    assert_eq!(p("ne mets pas pause"), None);
}

#[test]
fn volume_sentences() {
    assert_eq!(p("monte le son"), volume(Change::Relative(10.0), &[]));
    assert_eq!(p("baisse le son"), volume(Change::Relative(-10.0), &[]));
    assert_eq!(
        p("baisse un peu le son"),
        volume(Change::Relative(-5.0), &[])
    );
    assert_eq!(p("monte le son de 20"), volume(Change::Relative(20.0), &[]));
    assert_eq!(p("monte la musique"), volume(Change::Relative(10.0), &[]));
    assert_eq!(p("mets le son à 60 %"), volume(Change::Absolute(60.0), &[]));
    assert_eq!(
        p("mets le volume à 60"),
        volume(Change::Absolute(60.0), &[])
    );
    assert_eq!(p("mets le son à 140"), Some(Intent::VolumeRange));
    assert_eq!(
        p("baisse le son du salon"),
        volume(Change::Relative(-10.0), &["salon"])
    );
    assert_eq!(
        p("coupe le son"),
        Some(Intent::Mute {
            on: true,
            room: vec![]
        })
    );
    assert_eq!(
        p("remets le son dans le salon"),
        Some(Intent::Mute {
            on: false,
            room: v(&["salon"])
        })
    );
}

/// Sentences that meant something before the music words keep meaning it.
#[test]
fn house_sentences_are_unchanged() {
    assert_eq!(
        p("mets le chauffage à 20"),
        set(Change::Absolute(20.0), &[])
    );
    assert_eq!(
        p("passe le chauffage à 20"),
        set(Change::Absolute(20.0), &[])
    );
    assert_eq!(p("coupe la lumière"), lights(false, false, &[], &[]));
    assert!(matches!(
        p("monte le volet"),
        Some(Intent::Covers { open: true, .. })
    ));
    assert_eq!(p("passe"), None);
    assert_eq!(
        p("mets son bureau à 20"),
        set(Change::Absolute(20.0), &["son", "bureau"])
    );
    assert_eq!(
        p("allume son bureau"),
        lights(true, false, &[], &["son", "bureau"])
    );
}

fn m(s: &str) -> Option<Intent> {
    play_by_name(s)
}

fn query(q: &str) -> Option<Intent> {
    media(MediaIntent::PlayMedia { query: q.into() })
}

#[test]
fn play_by_name_reads_the_words_as_typed() {
    assert_eq!(m("joue du Daft Punk"), query("du Daft Punk"));
    assert_eq!(m("Joue Beyoncé !"), query("Beyoncé"));
    assert_eq!(m("peux-tu jouer la vie en rose"), query("la vie en rose"));
    assert_eq!(m("joue 99 Luftballons"), query("99 Luftballons"));
    assert_eq!(m("joue Daft Punk s'il te plaît"), query("Daft Punk"));
    assert_eq!(
        m("joue de la musique"),
        media(MediaIntent::Play { resume: false })
    );
    assert_eq!(
        m("joue une chanson"),
        media(MediaIntent::Play { resume: false })
    );
    assert_eq!(m("joue"), media(MediaIntent::Play { resume: false }));
    assert_eq!(m("qu'est-ce qui joue ?"), media(MediaIntent::NowPlaying));
    assert_eq!(m("ne joue pas Daft Punk"), None);
    assert_eq!(m("mets Daft Punk"), None);
    assert_eq!(m("allume la lumière"), None);
}

#[test]
fn joue_is_an_order_only_at_the_start() {
    assert_eq!(m("on a joué au foot"), None);
    assert_eq!(m("baisse le son, ça joue trop fort"), None);
    assert_eq!(m("ça joue trop fort"), None);
    assert_eq!(m("mets pause, la musique joue trop fort"), None);
    assert_eq!(m("peux-tu me jouer Daft Punk"), query("Daft Punk"));
    assert_eq!(m("tu peux jouer Daft Punk"), query("Daft Punk"));
}

/// An English title can tip speech recognition into English: "joue" comes out as "Play" or
/// "Ju", and a filler like "hum" lands in front of the title.
#[test]
fn english_titles_survive_the_transcription() {
    assert_eq!(m("Play Lost on You."), query("Lost on You"));
    assert_eq!(m("Ju Shape of You."), query("Shape of You"));
    assert_eq!(
        m("Joue Hum Merci de Cho Mendes."),
        query("Merci de Cho Mendes")
    );
    assert_eq!(m("joue euh Daft Punk"), query("Daft Punk"));
    assert_eq!(m("play"), media(MediaIntent::Play { resume: false }));
    assert_eq!(m("baisse le son, play"), None);
}

/// "merci" is politeness after a name, but a title on its own ("Mercy" heard as "merci").
#[test]
fn a_lone_merci_is_the_title() {
    assert_eq!(m("Joue merci."), query("merci"));
    assert_eq!(m("joue Daft Punk merci"), query("Daft Punk"));
    assert_eq!(m("joue Daft Punk, merci"), query("Daft Punk"));
}

#[test]
fn on_verbs_and_rooms_with_music() {
    assert_eq!(
        p("allume la musique"),
        media(MediaIntent::Play { resume: false })
    );
    assert_eq!(
        p("allume le son"),
        Some(Intent::Mute {
            on: false,
            room: vec![]
        })
    );
    assert_eq!(
        p("mets de la musique dans le salon"),
        media(MediaIntent::Play { resume: false })
    );
    // A music word the music intents do not use is not understood, never lights.
    assert_eq!(p("chanson"), None);
    assert_eq!(
        p("allume la chanson du salon"),
        media(MediaIntent::Play { resume: false })
    );
}

/// With a house word the music words are plain words again: the old parse, unchanged.
#[test]
fn house_words_keep_music_words_as_names() {
    assert_eq!(
        p("allume la lumière de son bureau"),
        lights(true, false, &[], &["son", "bureau"])
    );
    assert_eq!(
        p("allume la lumière suivante"),
        lights(true, false, &[], &["suivante"])
    );
    assert_eq!(
        p("remets le chauffage à 20"),
        set(Change::Absolute(20.0), &["remets"])
    );
}

fn vacuum(action: VacuumIntent, room: &[&str]) -> Option<Intent> {
    Some(Intent::Vacuum {
        action,
        room: v(room),
    })
}

#[test]
fn vacuum_cleans_one_room() {
    for (s, room) in [
        ("aspire la cuisine", &["cuisine"][..]),
        ("lance l'aspirateur dans la cuisine", &["cuisine"]),
        ("nettoie la cuisine", &["cuisine"]),
        ("nettoie le salon", &["salon"]),
        ("passe l'aspirateur dans le bureau", &["bureau"]),
        ("aspire la salle de bain", &["salle", "bain"]),
        ("nettoie l'entrée", &["entree"]),
    ] {
        assert_eq!(p(s), vacuum(VacuumIntent::Clean, room), "{s}");
    }
}

#[test]
fn vacuum_cleans_everything() {
    for s in [
        "lance l'aspirateur",
        "passe l'aspirateur",
        "aspire",
        "démarre l'aspirateur",
        "mets l'aspirateur en marche",
        "peux-tu lancer l'aspirateur ?",
    ] {
        assert_eq!(p(s), vacuum(VacuumIntent::Clean, &[]), "{s}");
    }
    assert_eq!(
        p("nettoie la maison"),
        vacuum(VacuumIntent::Clean, &["maison"])
    );
    assert_eq!(
        p("aspire partout"),
        vacuum(VacuumIntent::Clean, &["partout"])
    );
}

#[test]
fn vacuum_stop_pause_and_dock() {
    for s in ["arrête l'aspirateur", "stop l'aspirateur"] {
        assert_eq!(p(s), vacuum(VacuumIntent::Stop, &[]), "{s}");
    }
    for s in ["mets l'aspirateur en pause", "pause l'aspirateur"] {
        assert_eq!(p(s), vacuum(VacuumIntent::Pause, &[]), "{s}");
    }
    for s in [
        "renvoie l'aspirateur à la base",
        "ramène l'aspirateur à sa base",
        "l'aspirateur rentre à la base",
    ] {
        assert_eq!(p(s), vacuum(VacuumIntent::Dock, &[]), "{s}");
    }
}

#[test]
fn vacuum_never_guessed() {
    assert_eq!(p("n'aspire pas la cuisine"), None);
    assert_eq!(p("ne lance pas l'aspirateur"), None);
    assert_eq!(p("l'aspirateur"), None);
    assert_eq!(p("l'aspirateur est en marche"), None);
    assert_eq!(p("qui a lancé l'aspirateur"), None);
    assert_eq!(p("l'aspirateur nettoie quoi ?"), None);
    assert_eq!(p("ouvre l'aspirateur"), None);
}

#[test]
fn vacuum_words_stay_plain_words_elsewhere() {
    assert_eq!(
        p("éteins le chargeur"),
        lights(false, false, &[], &["chargeur"])
    );
    assert_eq!(
        p("allume la lumière de la station"),
        lights(true, false, &[], &["station"])
    );
    assert_eq!(p("pause"), Some(Intent::Media(MediaIntent::Pause)));
}
