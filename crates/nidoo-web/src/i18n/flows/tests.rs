use super::*;

/// Same word in both languages.
const SAME: &[FlowText] = &[
    FlowText::Action,
    FlowText::ParamPosition,
    FlowText::ParamCode,
    FlowText::ColourOrange,
    FlowText::ColourCyan,
    FlowText::ColourMagenta,
    FlowText::DefaultNotifyTitle,
    FlowText::AskTextLabel,
    FlowText::Auto,
    FlowText::NodeSignal,
    FlowText::Minutes,
    FlowText::Zoom,
    FlowText::Mode,
    FlowText::StepOk,
    FlowText::SevInfo,
    FlowText::NodeAction,
];

#[test]
fn ask_sentences() {
    assert_eq!(
        ask_card(Lang::En, "Heat up?", Some("Kitchen"), &["Yes", "No"]),
        "Ask \"Heat up?\" in Kitchen: Yes / No / no answer"
    );
    assert_eq!(
        ask_card(Lang::Fr, "Chauffer ?", Some("Cuisine"), &["Oui", "Non"]),
        "Demande «\u{a0}Chauffer ?\u{a0}» dans Cuisine\u{a0}: Oui / Non / pas de réponse"
    );
    assert_eq!(
        ask_card(Lang::En, "Heat up?", None, &["Yes"]),
        "Ask \"Heat up?\" — All rooms: Yes / no answer"
    );
    assert_eq!(
        ask_card(Lang::Fr, "Q", None, &[]),
        "Demande «\u{a0}Q\u{a0}» — Toutes les pièces\u{a0}: pas de réponse"
    );
}

#[test]
fn speak_sentences() {
    assert_eq!(
        speak(Lang::En, "Dinner", Some("Kitchen")),
        "Speak \"Dinner\" — Kitchen"
    );
    assert_eq!(
        speak(Lang::En, "Dinner", None),
        "Speak \"Dinner\" — All rooms"
    );
    assert_eq!(
        speak(Lang::Fr, "Dîner", None),
        "Annoncer «\u{a0}Dîner\u{a0}» — Toutes les pièces"
    );
    assert_eq!(speak(Lang::En, " ", None), "Speak — All rooms");
}

#[test]
fn removed_wires_counts() {
    assert_eq!(removed_wires(Lang::En, 1), "Removed 1 wire");
    assert_eq!(removed_wires(Lang::En, 3), "Removed 3 wires");
    assert_eq!(removed_wires(Lang::Fr, 1), "1 fil supprimé");
    assert_eq!(removed_wires(Lang::Fr, 2), "2 fils supprimés");
}

#[test]
fn switch_and_time_sentences() {
    assert_eq!(switch_on(Lang::En, "Lamp"), "Switch on Lamp");
    assert_eq!(switch_on(Lang::Fr, "Lampe"), "Commuter selon Lampe");
    assert_eq!(
        time_between(Lang::En, "07:00", "22:30"),
        "Between 07:00 and 22:30"
    );
    assert_eq!(
        time_between(Lang::Fr, "07:00", "22:30"),
        "Entre 07:00 et 22:30"
    );
}

fn switched(
    action: nidoo_core::flow::LabelAction,
    label: &str,
    on: usize,
    off: usize,
    skipped: &[(&str, &str)],
) -> nidoo_api::LabelSwitchResult {
    nidoo_api::LabelSwitchResult {
        label: label.into(),
        action,
        enabled: (0..on).map(|i| format!("on{i}")).collect(),
        disabled: (0..off).map(|i| format!("off{i}")).collect(),
        skipped: skipped
            .iter()
            .map(|(name, reason)| nidoo_api::SkippedFlow {
                id: name.to_lowercase(),
                name: (*name).into(),
                reason: (*reason).into(),
            })
            .collect(),
    }
}

#[test]
fn label_switch_result_sentences() {
    use nidoo_core::flow::{FlowLabelCatalog, LabelAction::*};
    let catalog: FlowLabelCatalog = serde_json::from_value(serde_json::json!({
        "labels": [
            {"id": "ete", "name": "Été", "choice": "saison"},
            {"id": "vacances", "name": "Vacances"}
        ],
        "choices": [{"id": "saison", "name": "Saison"}]
    }))
    .unwrap();
    let say = |lang, r| label_switch_result(lang, &r, &catalog);
    assert_eq!(
        say(Lang::Fr, switched(Pick, "ete", 5, 12, &[])),
        "Été choisi\u{a0}: 5 activées, 12 désactivées."
    );
    assert_eq!(
        say(Lang::En, switched(Pick, "ete", 5, 12, &[])),
        "Été picked: 5 on, 12 off."
    );
    assert_eq!(
        say(Lang::Fr, switched(Pick, "ete", 2, 0, &[])),
        "Été choisi\u{a0}: 2 activées, 0 désactivées."
    );
    assert_eq!(
        say(Lang::Fr, switched(Pick, "ete", 1, 1, &[])),
        "Été choisi\u{a0}: 1 activée, 1 désactivée."
    );
    assert_eq!(
        say(Lang::Fr, switched(Off, "vacances", 0, 3, &[])),
        "Vacances\u{a0}: 3 désactivées."
    );
    assert_eq!(
        say(Lang::En, switched(Off, "vacances", 0, 3, &[])),
        "Vacances: 3 off."
    );
    assert_eq!(
        say(Lang::Fr, switched(On, "vacances", 3, 0, &[])),
        "Vacances\u{a0}: 3 activées."
    );
    assert_eq!(
        say(Lang::Fr, switched(Pick, "ete", 0, 0, &[])),
        "Été choisi\u{a0}: rien à changer."
    );
    assert_eq!(
        say(Lang::En, switched(Pick, "ete", 0, 0, &[])),
        "Été picked: nothing to change."
    );
    assert_eq!(
        say(
            Lang::Fr,
            switched(Pick, "ete", 5, 12, &[("Nom", "invalid")])
        ),
        "Été choisi\u{a0}: 5 activées, 12 désactivées. 1 non modifiée\u{a0}: Nom (invalide)."
    );
    assert_eq!(
        say(
            Lang::En,
            switched(
                Off,
                "vacances",
                0,
                1,
                &[("A", "invalid"), ("B", "publish: broker offline")]
            )
        ),
        "Vacances: 1 off. 2 not changed: A (invalid), B (publish: broker offline)."
    );
    // A label gone from the catalog reads as its id.
    assert_eq!(
        say(Lang::En, switched(On, "ghost", 1, 0, &[])),
        "ghost: 1 on."
    );
}

#[test]
fn label_manager_sentences() {
    assert_eq!(label_flow_count(Lang::En, 1), "1 flow");
    assert_eq!(label_flow_count(Lang::En, 12), "12 flows");
    assert_eq!(label_flow_count(Lang::Fr, 12), "12 automatisations");
    let names = ["Summer switch".to_owned(), "Morning".to_owned()];
    assert_eq!(
        label_used_by(Lang::En, &names),
        "Used by the label action of: Summer switch, Morning. Those actions will fail until fixed."
    );
    assert!(
        label_used_by(Lang::Fr, &names)
            .starts_with("Utilisée par l'action d'étiquette de\u{a0}: Summer switch, Morning.")
    );
    assert_eq!(remove_label(Lang::En, "Été"), "Remove “Été”");
    assert_eq!(remove_label(Lang::Fr, "Été"), "Retirer «\u{a0}Été\u{a0}»");
    assert_eq!(
        label_color_word(Lang::Fr, nidoo_core::flow::LabelColor::Slate),
        "Ardoise"
    );
}

#[test]
fn label_off_preview_sentences() {
    assert_eq!(
        label_off_preview(Lang::En, 2, "Été"),
        "Turns off the 2 \"Été\" flows."
    );
    assert_eq!(
        label_off_preview(Lang::En, 0, "Été"),
        "No \"Été\" flow to turn off."
    );
    assert_eq!(
        label_off_preview(Lang::Fr, 1, "Été"),
        "Désactive l'automatisation «\u{a0}Été\u{a0}»."
    );
}

#[test]
fn label_preview_sentences() {
    assert_eq!(
        label_preview(Lang::Fr, 5, "Été", 12, &["Hiver"]),
        "Active les 5 automatisations «\u{a0}Été\u{a0}», désactive les 12 «\u{a0}Hiver\u{a0}»."
    );
    assert_eq!(
        label_preview(Lang::En, 5, "Été", 12, &["Hiver"]),
        "Turns on the 5 \"Été\" flows, turns off the 12 \"Hiver\" flows."
    );
    assert_eq!(
        label_preview(Lang::Fr, 1, "Été", 1, &["Hiver", "Printemps"]),
        "Active l'automatisation «\u{a0}Été\u{a0}», désactive l'automatisation «\u{a0}Hiver\u{a0}» / «\u{a0}Printemps\u{a0}»."
    );
    assert_eq!(
        label_preview(Lang::En, 0, "Été", 0, &["Hiver"]),
        "No \"Été\" flow to turn on."
    );
    assert_eq!(
        label_preview(Lang::Fr, 0, "Été", 0, &[]),
        "Aucune automatisation «\u{a0}Été\u{a0}» à activer."
    );
    assert_eq!(
        label_preview(Lang::En, 1, "Été", 0, &[]),
        "Turns on the \"Été\" flow."
    );
}

#[test]
fn every_word_has_both_languages() {
    assert_table(FlowText::ALL, flow_text, SAME);
}

#[test]
fn french_questions_take_a_no_break_space() {
    assert_eq!(ask(Lang::En, "Lamp is on"), "Lamp is on?");
    assert_eq!(ask(Lang::Fr, "Lamp est allumée"), "Lamp est allumée\u{A0}?");
}

#[test]
fn is_dark_sentences_name_the_shift() {
    assert_eq!(
        is_dark_sentence(Lang::En, 0),
        flow_text(Lang::En, FlowText::IsDark)
    );
    assert_eq!(
        is_dark_sentence(Lang::En, -30),
        "Is it dark? (from 30\u{A0}min before civil dusk)"
    );
    assert_eq!(
        is_dark_sentence(Lang::Fr, 15),
        "Fait-il nuit\u{A0}? (dès 15\u{A0}min après le crépuscule civil)"
    );
}

#[test]
fn sun_sentences_in_both_languages() {
    assert_eq!(
        sun_sentence(Lang::Fr, SunWhich::Sunset, 0),
        "Au coucher du soleil"
    );
    assert_eq!(
        sun_sentence(Lang::Fr, SunWhich::Sunrise, 15),
        "15\u{A0}min après le lever du soleil"
    );
    assert_eq!(
        sun_sentence(Lang::Fr, SunWhich::Dawn, -15),
        "15\u{A0}min avant l'aube civile"
    );
    assert_eq!(
        sun_sentence(Lang::Fr, SunWhich::Dusk, 0),
        "Au crépuscule civil"
    );
}
