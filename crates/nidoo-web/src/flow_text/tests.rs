use super::*;

#[test]
fn summary_line_renders_expected_string() {
    let en = |w, o| sun_summary(Lang::En, w, o);
    assert_eq!(en(SunWhich::Sunset, 0), "At sunset");
    assert_eq!(en(SunWhich::Sunrise, 15), "15\u{a0}min after sunrise");
    assert_eq!(en(SunWhich::Dusk, -15), "15\u{a0}min before civil dusk");
    assert_eq!(en(SunWhich::Dawn, 0), "At civil dawn");
    assert_eq!(
        sun_summary(Lang::Fr, SunWhich::Sunrise, 15),
        "15\u{a0}min après le lever du soleil"
    );
}
use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};

fn ent(id: &str, name: &str, device: Option<&str>, kind: Kind, unit: Option<&str>) -> EntityView {
    EntityView {
        id: id.into(),
        device_id: None,
        name: name.into(),
        device_name: device.map(Into::into),
        label: None,
        kind,
        component: kind.component().to_string(),
        protocol_kind: kind,
        device_class: None,
        unit: unit.map(Into::into),
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: "salon".into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState::default(),
    }
}

#[test]
fn a_switch_shown_as_a_light_reads_allumee() {
    let mut lamp = ent("s1", "Lampe", None, Kind::Switch, None);
    lamp.ui.show_as = Some("light".into());
    assert_eq!(bool_word(Lang::Fr, Some(&lamp), true), "allumée");
    assert_eq!(bool_word(Lang::Fr, Some(&lamp), false), "éteinte");
    let plain = ent("s2", "Prise", None, Kind::Switch, None);
    assert_eq!(bool_word(Lang::Fr, Some(&plain), true), "activé");
}

fn house() -> BTreeMap<String, EntityView> {
    [
        ent("l1", "Lumière TV", None, Kind::Light, None),
        ent("lux", "Entrée", None, Kind::Sensor, Some("lx")),
        ent("led", "LED", Some("LED strip"), Kind::Light, None),
        ent("p1", "Power 1", None, Kind::Number, Some("%")),
        ent("mot", "Motion", None, Kind::BinarySensor, None),
        ent("robo", "Roborock", None, Kind::Vacuum, None),
    ]
    .into_iter()
    .map(|e| (e.id.clone(), e))
    .collect()
}

fn areas() -> Vec<AreaView> {
    serde_json::from_str(
        r#"[{"id":"salon","name":"Salon","icon":null,"order":null,"entity_count":0,"primary_count":0}]"#,
    )
    .unwrap()
}

fn node(json: &str) -> Node {
    serde_json::from_str(json).unwrap()
}

fn text(json: &str) -> String {
    text_in(Lang::En, json)
}

fn text_in(lang: Lang, json: &str) -> String {
    let entities = house();
    let areas = areas();
    let flows = vec![(
        "force-heating-loop".to_owned(),
        "Force heating loop".to_owned(),
    )];
    title(
        &node(json),
        &Ctx {
            entities: &entities,
            areas: &areas,
            flows: &flows,
            labels: &crate::flow_text::NO_LABELS,
            lang,
        },
    )
}

#[test]
fn flow_label_sentence() {
    let catalog: nidoo_core::flow::FlowLabelCatalog = serde_json::from_value(serde_json::json!({
        "labels": [
            {"id": "hiver", "name": "Hiver", "choice": "saison"},
            {"id": "ete", "name": "Été", "choice": "saison"},
            {"id": "vacances", "name": "Vacances"}
        ],
        "choices": [{"id": "saison", "name": "Saison"}]
    }))
    .unwrap();
    let entities = house();
    let areas = areas();
    let say = |lang: Lang, json: &str| {
        sentence(
            &node(json),
            &Ctx {
                entities: &entities,
                areas: &areas,
                flows: &[],
                labels: &catalog,
                lang,
            },
        )
    };
    let pick = r#"{"id":"a","type":"flow.label","params":{"label":"ete","action":"pick"}}"#;
    assert_eq!(say(Lang::En, pick), "Pick a label: Saison → Été");
    assert_eq!(
        say(Lang::Fr, pick),
        "Choisir une étiquette\u{a0}: Saison → Été"
    );
    let on = r#"{"id":"a","type":"flow.label","params":{"label":"vacances","action":"on"}}"#;
    assert_eq!(say(Lang::En, on), "Turn on \"Vacances\"");
    assert_eq!(say(Lang::Fr, on), "Activer «\u{a0}Vacances\u{a0}»");
    let off = r#"{"id":"a","type":"flow.label","params":{"label":"vacances","action":"off"}}"#;
    assert_eq!(say(Lang::En, off), "Turn off \"Vacances\"");
    assert_eq!(say(Lang::Fr, off), "Désactiver «\u{a0}Vacances\u{a0}»");
    // A label the catalog lost reads as its id; an unset one as the card's name.
    let gone = r#"{"id":"a","type":"flow.label","params":{"label":"gone","action":"pick"}}"#;
    assert_eq!(say(Lang::En, gone), "Pick a label: gone");
    let unset = r#"{"id":"a","type":"flow.label","params":{"label":"","action":"pick"}}"#;
    assert_eq!(say(Lang::En, unset), "Pick a label");
}

#[test]
fn backup_run_sentence_and_ports() {
    let json =
        r#"{"id":"b","type":"backup.run","params":{"destination":"ftp","timeout_minutes":30}}"#;
    assert_eq!(text_in(Lang::En, json), "Back up to the FTP server");
    assert_eq!(text_in(Lang::Fr, json), "Sauvegarder sur le serveur FTP");
    assert_eq!(label(Lang::En, json, "ok"), "Backed up");
    assert_eq!(label(Lang::En, json, "failed"), "Failed");
    assert_eq!(label(Lang::Fr, json, "ok"), "Sauvegardé");
    assert_eq!(label(Lang::Fr, json, "failed"), "Échec");
    assert_eq!(label(Lang::En, json, "partial"), "Partial");
    assert_eq!(label(Lang::Fr, json, "partial"), "Partielle");
}

#[test]
fn french_twins_of_the_english_sentences() {
    let fr = |json: &str| text_in(Lang::Fr, json);
    let off =
        r#"{"id":"a","type":"command","params":{"entity":"led","command":{"type":"turn_off"}}}"#;
    assert_eq!(fr(off), "Éteindre LED");
    let on = r#"{"id":"a","type":"command","params":{"entity":"l1","command":{"type":"turn_on"}}}"#;
    assert_eq!(fr(on), "Allumer Lumière TV");
    let changes = r#"{"id":"a","type":"trigger.state","params":{"entity":"l1"}}"#;
    assert_eq!(fr(changes), "Quand Lumière TV change");
    let is_on = r#"{"id":"a","type":"if","params":{"entity":"l1","op":"eq","value":true}}"#;
    assert_eq!(fr(is_on), "Lumière TV est allumée\u{a0}?");
    let turns = r#"{"id":"a","type":"trigger.state","params":{"entity":"l1","to":true}}"#;
    assert_eq!(fr(turns), "Quand Lumière TV s'allume");
    let gt = r#"{"id":"a","type":"if","params":{"entity":"lux","op":"gt","value":16}}"#;
    assert_eq!(fr(gt), "Entrée > 16\u{a0}lx\u{a0}?");
    let wait = r#"{"id":"a","type":"delay","params":{"for":"1h30m","up_to":"2h"}}"#;
    assert_eq!(fr(wait), "Attendre de 1\u{a0}h 30\u{a0}min à 2\u{a0}h");
    let notify = r#"{"id":"a","type":"notify","params":{"title":"Porte"}}"#;
    assert_eq!(fr(notify), "Notifier «\u{a0}Porte\u{a0}»");
    let dark = r#"{"id":"d","type":"if","params":{"op":"is_dark"}}"#;
    assert_eq!(fr(dark), "Fait-il nuit\u{a0}? (soleil sous \u{2212}6°)");
    let daily = r#"{"id":"s","type":"trigger.schedule","params":{"cron":"0 7 * * *"}}"#;
    assert_eq!(text(daily), "Every day at 07:00");
    assert_eq!(fr(daily), "Tous les jours à 07:00");
    let weekdays = r#"{"id":"s","type":"trigger.schedule","params":{"cron":"0 7 * * 1-5"}}"#;
    assert_eq!(fr(weekdays), "Du lundi au vendredi à 07:00");
    let enable = r#"{"id":"a","type":"flow.enable","params":{"enabled":false}}"#;
    assert_eq!(fr(enable), "Désactiver cette automatisation");
}

#[test]
fn state_trigger_crossings_and_holds() {
    let at = |extra: &str| {
        format!(r#"{{"id":"a","type":"trigger.state","params":{{"entity":"lux"{extra}}}}}"#)
    };
    let above = at(r#","above":25"#);
    assert_eq!(text(&above), "When Entrée goes above 25\u{a0}lx");
    assert_eq!(text_in(Lang::Fr, &above), "Quand Entrée dépasse 25\u{a0}lx");
    let below = at(r#","below":18"#);
    assert_eq!(text(&below), "When Entrée goes below 18\u{a0}lx");
    assert_eq!(
        text_in(Lang::Fr, &below),
        "Quand Entrée descend sous 18\u{a0}lx"
    );
    let both = at(r#","above":18,"below":22"#);
    assert_eq!(text(&both), "When Entrée goes between 18 and 22\u{a0}lx");
    assert_eq!(
        text_in(Lang::Fr, &both),
        "Quand Entrée passe entre 18 et 22\u{a0}lx"
    );
    let held = at(r#","above":25,"for":"5m""#);
    assert_eq!(
        text(&held),
        "When Entrée goes above 25\u{a0}lx for 5\u{a0}min"
    );
    assert_eq!(
        text_in(Lang::Fr, &held),
        "Quand Entrée dépasse 25\u{a0}lx pendant 5\u{a0}min"
    );
    let still = at(r#","to":true,"for":"5m""#);
    assert!(
        text(&still).ends_with(" for 5\u{a0}min"),
        "{}",
        text(&still)
    );
}

#[test]
fn switch_and_time_sentences_en_fr() {
    let switch =
        r#"{"id":"s","type":"switch","params":{"entity":"lux","rules":[{"op":"lt","value":18}]}}"#;
    assert_eq!(text(switch), "Switch on Entrée");
    assert_eq!(text_in(Lang::Fr, switch), "Commuter selon Entrée");
    let bare = r#"{"id":"s","type":"switch","params":{"rules":[{"op":"lt","value":18}]}}"#;
    assert_eq!(text(bare), "Switch on Trigger value");
    let time = r#"{"id":"t","type":"time.between","params":{"from":"07:00","to":"22:30"}}"#;
    assert_eq!(text(time), "Between 07:00 and 22:30");
    assert_eq!(text_in(Lang::Fr, time), "Entre 07:00 et 22:30");
    assert_eq!(role(&node(switch).kind), Role::Check);
    assert_eq!(role(&node(time).kind), Role::Check);
}

#[test]
fn ask_sentence_en_fr() {
    let ask = r#"{"id":"a","type":"ask","params":{"text":"Heat up?","room":"salon","answers":[{"label":"Oui"},{"label":"Non"}]}}"#;
    assert_eq!(
        text(ask),
        "Ask \"Heat up?\" in Salon: Oui / Non / no answer"
    );
    assert_eq!(
        text_in(Lang::Fr, ask),
        "Demande «\u{a0}Heat up?\u{a0}» dans Salon\u{a0}: Oui / Non / pas de réponse"
    );
    let anywhere = r#"{"id":"a","type":"ask","params":{"text":"Q","answers":[{"label":"Yes"}]}}"#;
    assert_eq!(text(anywhere), "Ask \"Q\" — All rooms: Yes / no answer");
    assert_eq!(role(&node(ask).kind), Role::Action);
}

#[test]
fn ask_exit_labels() {
    let ask = r#"{"id":"a","type":"ask","params":{"text":"Q","answers":[{"label":"Oui"},{"label":"Non"}]}}"#;
    assert_eq!(label(Lang::En, ask, "1"), "Oui");
    assert_eq!(label(Lang::En, ask, "2"), "Non");
    assert_eq!(label(Lang::En, ask, "none"), "No answer");
    assert_eq!(label(Lang::Fr, ask, "none"), "Pas de réponse");
    assert_eq!(label(Lang::En, ask, "3"), "");
}

fn label(lang: Lang, json: &str, port: &str) -> String {
    let entities = house();
    let areas = areas();
    let ctx = Ctx {
        entities: &entities,
        areas: &areas,
        flows: &[],
        labels: &crate::flow_text::NO_LABELS,
        lang,
    };
    exit_label(lang, &node(json).kind, port, &ctx)
}

#[test]
fn exit_labels() {
    let switch = r#"{"id":"s","type":"switch","params":{"entity":"lux","otherwise":true,"rules":[
            {"op":"lt","value":18},{"op":"between","value":18,"to":22},{"op":"eq","value":true}]}}"#;
    assert_eq!(label(Lang::En, switch, "1"), "< 18\u{a0}lx");
    assert_eq!(label(Lang::En, switch, "2"), "18–22\u{a0}lx");
    assert_eq!(label(Lang::En, switch, "3"), "= on");
    assert_eq!(label(Lang::En, switch, "otherwise"), "otherwise");
    assert_eq!(label(Lang::Fr, switch, "otherwise"), "sinon");
    assert_eq!(label(Lang::Fr, switch, "2"), "18–22\u{a0}lx");
    assert_eq!(label(Lang::En, switch, "9"), "");
    let light =
        r#"{"id":"s","type":"switch","params":{"entity":"l1","rules":[{"op":"eq","value":true}]}}"#;
    assert_eq!(label(Lang::Fr, light, "1"), "= allumée");
    let time = r#"{"id":"t","type":"time.between","params":{"from":"07:00","to":"22:30"}}"#;
    assert_eq!(label(Lang::En, time, "in"), "07:00–22:30");
    assert_eq!(label(Lang::En, time, "out"), "outside");
    assert_eq!(label(Lang::Fr, time, "out"), "en dehors");
    let ifnode = r#"{"id":"a","type":"if","params":{"entity":"l1","op":"eq","value":true}}"#;
    assert_eq!(label(Lang::En, ifnode, "true"), "");
}

#[test]
fn french_colours_and_numbers() {
    let fr = |json: &str| text_in(Lang::Fr, json);
    let on = |cmd: &str| {
        fr(&format!(
            r#"{{"id":"a","type":"command","params":{{"entity":"led","command":{cmd}}}}}"#
        ))
    };
    assert_eq!(
        on(r#"{"type":"turn_on","brightness":100,"color":{"h":295.0,"s":100.0}}"#),
        "Allumer LED à 39\u{a0}% en magenta"
    );
    assert_eq!(
        on(r#"{"type":"turn_on","color":{"h":30.0,"s":4.0},"effect":"breathe"}"#),
        "Allumer LED en blanc avec breathe"
    );
    assert_eq!(
        on(r#"{"type":"set_temperature","temperature":21.5}"#),
        "Régler LED sur 21,5\u{a0}°C"
    );
}

#[test]
fn manual_card_reads_with_its_value() {
    let card = |value: &str| format!(r#"{{"id":"m","type":"trigger.manual","params":{value}}}"#);
    let none = card("{}");
    let on = card(r#"{"value":true}"#);
    let num = card(r#"{"value":21.5}"#);
    assert_eq!(text(&none), "Manual");
    assert_eq!(text(&on), "Manual · on");
    assert_eq!(text(&num), "Manual · 21.5");
    assert_eq!(text_in(Lang::Fr, &none), "Manuel");
    assert_eq!(text_in(Lang::Fr, &on), "Manuel · activé");
    assert_eq!(text_in(Lang::Fr, &num), "Manuel · 21,5");
}

#[test]
fn sun_and_is_dark_cards_read_as_sentences() {
    let sun = r#"{"id":"s","type":"trigger.sun","params":{"which":"dusk","offset_minutes":-15}}"#;
    assert_eq!(text(sun), "15\u{a0}min before civil dusk");
    let dark = r#"{"id":"d","type":"if","params":{"op":"is_dark"}}"#;
    assert_eq!(text(dark), "Is it dark now? (sun below \u{2212}6°)");
    let early = r#"{"id":"d","type":"if","params":{"op":"is_dark","offset_minutes":-30}}"#;
    assert_eq!(
        text(early),
        "Is it dark? (from 30\u{a0}min before civil dusk)"
    );
}

#[test]
fn off_peak_cards_read_as_sentences() {
    let start = r#"{"id":"o","type":"trigger.off_peak","params":{"edge":"start"}}"#;
    assert_eq!(text(start), "Off-peak starts");
    let end = r#"{"id":"o","type":"trigger.off_peak","params":{"edge":"end"}}"#;
    assert_eq!(text(end), "Off-peak ends");
    assert_eq!(text_in(Lang::Fr, end), "Les heures creuses finissent");
    let check = r#"{"id":"c","type":"if","params":{"op":"is_off_peak"}}"#;
    assert_eq!(text(check), "Is it off-peak?");
    assert_eq!(text_in(Lang::Fr, check), "Est-ce les heures creuses\u{a0}?");
}

#[test]
fn alarm_cards_read_as_sentences() {
    let card = |params: &str| format!(r#"{{"id":"a","type":"trigger.alarm","params":{params}}}"#);
    let any = card("{}");
    assert_eq!(text(&any), "When the alarm changes mode");
    assert_eq!(text_in(Lang::Fr, &any), "Quand l'alarme change de mode");
    let to = card(r#"{"to":"armed_away"}"#);
    assert_eq!(text(&to), "When the alarm becomes Armed away");
    assert_eq!(
        text_in(Lang::Fr, &to),
        "Quand l'alarme passe en Activée (absent)"
    );
    let from = card(r#"{"from":"disarmed"}"#);
    assert_eq!(text(&from), "When the alarm leaves Disarmed");
    assert_eq!(text_in(Lang::Fr, &from), "Quand l'alarme quitte Désactivée");
    let both = card(r#"{"from":"disarmed","to":"armed"}"#);
    assert_eq!(
        text(&both),
        "When the alarm goes from Disarmed to Armed (any mode)"
    );
    assert_eq!(
        text_in(Lang::Fr, &both),
        "Quand l'alarme passe de Désactivée à Activée (tous modes)"
    );
}

#[test]
fn silent_and_limit_cards_read_as_sentences() {
    let silent = r#"{"id":"s","type":"trigger.silent","params":{"entity":"l1","for":"50m"}}"#;
    assert_eq!(text(silent), "Lumière TV silent for 50\u{a0}min");
    assert_eq!(
        text_in(Lang::Fr, silent),
        "Lumière TV silencieux depuis 50\u{a0}min"
    );
    let unset = r#"{"id":"s","type":"trigger.silent","params":{"entity":"","for":"50m"}}"#;
    assert_eq!(text(unset), "Silent for 50\u{a0}min");
    let limit = r#"{"id":"l","type":"limit","params":{"max":4,"per":"24h"}}"#;
    assert_eq!(text(limit), "At most 4 every 24h");
    assert_eq!(text_in(Lang::Fr, limit), "Au plus 4 fois par 24h");
}

#[test]
fn if_eq_bool_reads_on() {
    let n = r#"{"id":"a","type":"if","name":"  ","params":{"entity":"l1","op":"eq","value":true}}"#;
    assert_eq!(text(n), "Lumière TV is on?");
}

#[test]
fn if_sentence_names_the_field() {
    let n = r#"{"id":"a","type":"if","params":{"entity":"robo","field":"battery","op":"lte","value":55}}"#;
    assert_eq!(text(n), "Roborock · Battery ≤ 55\u{a0}%?");
    assert_eq!(
        text_in(Lang::Fr, n),
        "Roborock · Batterie ≤ 55\u{a0}%\u{a0}?"
    );
    let when = r#"{"id":"a","type":"trigger.state","params":{"entity":"robo","field":"battery"}}"#;
    assert_eq!(text(when), "When Roborock · Battery changes");
    let switch = r#"{"id":"a","type":"switch","params":{"entity":"robo","field":"battery","rules":[{"op":"lte","value":20}]}}"#;
    assert_eq!(text(switch), switch_on_text("Roborock · Battery"));
}

fn switch_on_text(e: &str) -> String {
    crate::i18n::flows::switch_on(Lang::En, e)
}

#[test]
fn if_gt_uses_unit() {
    let n = r#"{"id":"a","type":"if","params":{"entity":"lux","op":"gt","value":16}}"#;
    assert_eq!(text(n), "Entrée > 16\u{a0}lx?");
}

#[test]
fn command_turn_off() {
    let n =
        r#"{"id":"a","type":"command","params":{"entity":"led","command":{"type":"turn_off"}}}"#;
    assert_eq!(text(n), "Turn off LED");
}

#[test]
fn command_turn_on_names_colour_and_effect() {
    let on = |cmd: &str| {
        text(&format!(
            r#"{{"id":"a","type":"command","params":{{"entity":"led","command":{cmd}}}}}"#
        ))
    };
    assert_eq!(on(r#"{"type":"turn_on"}"#), "Turn on LED");
    assert_eq!(
        on(r#"{"type":"turn_on","brightness":100,"color":{"h":295.0,"s":100.0}}"#),
        "Turn on LED at 39\u{a0}% in magenta"
    );
    assert_eq!(
        on(r#"{"type":"turn_on","color":{"h":30.0,"s":4.0}}"#),
        "Turn on LED in white"
    );
    assert_eq!(
        on(r#"{"type":"turn_on","color":{"h":355.0,"s":90.0},"effect":"breathe"}"#),
        "Turn on LED in red with breathe"
    );
}

#[test]
fn command_set_number() {
    let n = r#"{"id":"a","type":"command","params":{"entity":"p1","command":{"type":"set_number","value":100}}}"#;
    assert_eq!(text(n), "Set Power 1 to 100\u{a0}%");
}

#[test]
fn trigger_to_true() {
    let n = r#"{"id":"a","type":"trigger.state","params":{"entity":"mot","to":true}}"#;
    assert_eq!(text(n), "When Motion turns on");
}

#[test]
fn delay_words() {
    let d = |s: &str| {
        text(&format!(
            r#"{{"id":"a","type":"delay","params":{{"for":"{s}"}}}}"#
        ))
    };
    assert_eq!(d("5m"), "Wait 5\u{a0}min");
    assert_eq!(d("1h30m"), "Wait 1\u{a0}h 30\u{a0}min");
    assert_eq!(duration_words("45s"), "45\u{a0}s");
    assert_eq!(duration_words("junk"), "junk");
    let random = text(r#"{"id":"a","type":"delay","params":{"for":"1m","up_to":"2h"}}"#);
    assert_eq!(random, "Wait 1\u{a0}min to 2\u{a0}h");
}

#[test]
fn notify_text() {
    let n = r#"{"id":"a","type":"notify","params":{"title":"Door open"}}"#;
    assert_eq!(text(n), "Notify \"Door open\"");
    let empty = r#"{"id":"a","type":"notify","params":{"title":""}}"#;
    assert_eq!(text(empty), "Notify");
}

#[test]
fn notify_title_names_placeholder_entities() {
    let n = r#"{"id":"a","type":"notify","params":{"title":"Light {lux:0} {gone} {trigger}"}}"#;
    assert_eq!(text(n), "Notify \"Light {Entrée:0} {gone} {trigger}\"");
    let broken = r#"{"id":"a","type":"notify","params":{"title":"Light {lux"}}"#;
    assert_eq!(text(broken), "Notify \"Light {lux\"");
}

#[test]
fn flow_enable_names() {
    let named =
        r#"{"id":"a","type":"flow.enable","params":{"flow":"force-heating-loop","enabled":false}}"#;
    assert_eq!(text(named), "Disable Force heating loop");
    let own = r#"{"id":"a","type":"flow.enable","params":{"enabled":false}}"#;
    assert_eq!(text(own), "Disable this flow");
}

#[test]
fn name_wins_and_subline() {
    let entities = house();
    let areas = areas();
    let ctx = Ctx {
        entities: &entities,
        areas: &areas,
        flows: &[],
        labels: &crate::flow_text::NO_LABELS,
        lang: crate::i18n::Lang::En,
    };
    let n = node(
        r#"{"id":"a","name":"LED is on","type":"if","params":{"entity":"led","op":"eq","value":true}}"#,
    );
    assert_eq!(title(&n, &ctx), "LED is on");
    assert_eq!(subline(&n, &ctx), Some("Salon · LED strip".to_owned()));
}

#[test]
fn unnamed_delay_has_no_subline() {
    let entities = house();
    let ctx = Ctx {
        entities: &entities,
        areas: &[],
        flows: &[],
        labels: &crate::flow_text::NO_LABELS,
        lang: crate::i18n::Lang::En,
    };
    let n = node(r#"{"id":"a","type":"delay","params":{"for":"5m"}}"#);
    assert_eq!(subline(&n, &ctx), None);
    assert_eq!(role(&n.kind), Role::Wait);
}

#[test]
fn cards_that_read_a_math_result_name_it() {
    let set = r#"{"id":"a","type":"command","params":{"entity":"led","command":{"type":"set_number","value":0},"value_from":"half"}}"#;
    assert_eq!(text(set), "Set LED to half");
    assert_eq!(text_in(Lang::Fr, set), "Régler LED sur half");
    let cmp = r#"{"id":"a","type":"if","params":{"result":"half","op":"gt","value":300}}"#;
    assert_eq!(text(cmp), "half > 300?");
    assert_eq!(text_in(Lang::Fr, cmp), "half > 300\u{a0}?");
    let switch =
        r#"{"id":"s","type":"switch","params":{"result":"half","rules":[{"op":"lt","value":18}]}}"#;
    assert_eq!(text(switch), "Switch on half");
    assert_eq!(text_in(Lang::Fr, switch), "Commuter selon half");
    // A command with no number ignores a stale link.
    let off = r#"{"id":"a","type":"command","params":{"entity":"led","command":{"type":"turn_off"},"value_from":"half"}}"#;
    assert_eq!(text(off), "Turn off LED");
}
