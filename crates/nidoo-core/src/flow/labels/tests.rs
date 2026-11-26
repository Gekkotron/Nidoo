use serde_json::json;

use super::*;

fn catalog() -> FlowLabelCatalog {
    FlowLabelCatalog::decode(
        r#"{
              "labels": [
                {"id": "hiver", "name": "Hiver", "color": "blue", "choice": "saison"},
                {"id": "ete", "name": "Été", "color": "amber", "choice": "saison"},
                {"id": "vacances", "name": "Vacances", "color": "green"},
                {"id": "seul", "name": "Seul", "color": "red", "choice": "solo"}
              ],
              "choices": [
                {"id": "saison", "name": "Saison", "current": "hiver"},
                {"id": "solo", "name": "Solo"}
              ]
            }"#
        .as_bytes(),
    )
    .unwrap()
}

fn flow(id: &str, enabled: bool, labels: &[&str]) -> Flow {
    Flow {
        id: id.into(),
        name: id.into(),
        enabled,
        labels: labels.iter().map(|l| (*l).to_owned()).collect(),
        ..Flow::default()
    }
}

fn plan(flows: &[Flow], invalid: &[&str], label: &str, action: LabelAction) -> SwitchPlan {
    try_plan(flows, invalid, label, action).unwrap()
}

fn try_plan(
    flows: &[Flow],
    invalid: &[&str],
    label: &str,
    action: LabelAction,
) -> Result<SwitchPlan, LabelError> {
    let pairs: Vec<(&Flow, bool)> = flows
        .iter()
        .map(|f| (f, invalid.contains(&f.id.as_str())))
        .collect();
    plan_switch(&catalog(), &pairs, label, action)
}

#[test]
fn pick_turns_on_the_label_and_off_the_rest_of_the_choice() {
    let flows = [
        flow("heat", true, &["hiver"]),
        flow("heat2", true, &["hiver"]),
        flow("fan", false, &["ete"]),
    ];
    let plan = plan(&flows, &[], "ete", LabelAction::Pick);
    assert_eq!(plan.enable, ["fan"]);
    assert_eq!(plan.disable, ["heat", "heat2"]);
    assert!(plan.skipped_invalid.is_empty());
    assert_eq!(plan.choice.as_deref(), Some("saison"));
}

#[test]
fn pick_keeps_a_flow_carrying_both_labels_on() {
    let flows = [
        flow("both", false, &["hiver", "ete"]),
        flow("w", true, &["hiver"]),
    ];
    let plan = plan(&flows, &[], "ete", LabelAction::Pick);
    assert_eq!(plan.enable, ["both"]);
    assert_eq!(plan.disable, ["w"]);
    let flows = [flow("both", true, &["hiver", "ete"])];
    let plan = super::tests::plan(&flows, &[], "ete", LabelAction::Pick);
    assert!(plan.enable.is_empty() && plan.disable.is_empty());
}

#[test]
fn flows_already_right_are_not_listed() {
    let flows = [flow("heat", false, &["hiver"]), flow("fan", true, &["ete"])];
    let plan = plan(&flows, &[], "ete", LabelAction::Pick);
    assert!(plan.enable.is_empty());
    assert!(plan.disable.is_empty());
}

#[test]
fn untouched_flows_without_the_choice() {
    let flows = [
        flow("away", true, &["vacances"]),
        flow("plain", false, &[]),
        flow("ghost", true, &["unknown"]),
    ];
    let plan = plan(&flows, &[], "ete", LabelAction::Pick);
    assert_eq!(
        plan,
        SwitchPlan {
            choice: Some("saison".into()),
            ..SwitchPlan::default()
        }
    );
}

#[test]
fn pick_errors() {
    let flows = [flow("a", true, &["hiver"])];
    assert_eq!(
        try_plan(&flows, &[], "nope", LabelAction::Pick),
        Err(LabelError::UnknownLabel("nope".into()))
    );
    assert_eq!(
        try_plan(&flows, &[], "vacances", LabelAction::Pick),
        Err(LabelError::NotInChoice("vacances".into()))
    );
    assert_eq!(
        try_plan(&flows, &[], "seul", LabelAction::Pick),
        Err(LabelError::IncompleteChoice("solo".into()))
    );
    assert_eq!(
        try_plan(&flows, &[], "nope", LabelAction::On),
        Err(LabelError::UnknownLabel("nope".into()))
    );
    assert_eq!(
        LabelError::UnknownLabel("x".into()).to_string(),
        "unknown label `x`"
    );
    assert_eq!(
        LabelError::NotInChoice("x".into()).to_string(),
        "label `x` is no longer in a choice"
    );
    assert_eq!(
        LabelError::IncompleteChoice("x".into()).to_string(),
        "choice `x` has fewer than two labels"
    );
}

#[test]
fn on_off_work_for_any_label() {
    let flows = [
        flow("a", false, &["vacances"]),
        flow("b", true, &["vacances"]),
        flow("c", true, &["hiver"]),
    ];
    let on = plan(&flows, &[], "vacances", LabelAction::On);
    assert_eq!(on.enable, ["a"]);
    assert!(on.disable.is_empty());
    assert_eq!(on.choice, None);
    let off = plan(&flows, &[], "vacances", LabelAction::Off);
    assert_eq!(off.disable, ["b"]);
    let off = plan(&flows, &[], "hiver", LabelAction::Off);
    assert_eq!(off.disable, ["c"]);
    assert_eq!(off.choice, None);
}

#[test]
fn invalid_flows_are_skipped() {
    let flows = [
        flow("bad", true, &["hiver"]),
        flow("bad2", true, &["ete"]),
        flow("other", true, &["vacances"]),
        flow("w", true, &["hiver"]),
    ];
    let plan = plan(&flows, &["bad", "bad2", "other"], "ete", LabelAction::Pick);
    assert_eq!(plan.skipped_invalid, ["bad", "bad2"]);
    assert_eq!(plan.disable, ["w"]);
    assert!(plan.enable.is_empty());
}

#[test]
fn label_id_folds_and_suffixes() {
    assert_eq!(label_id("Été", &[]), "ete");
    assert_eq!(label_id("Été", &["ete"]), "ete-2");
    assert_eq!(label_id("Été", &["ete", "ete-2"]), "ete-3");
    assert_eq!(label_id("  ", &[]), "label");
    assert_eq!(label_id("Mode Télétravail!", &[]), "mode-teletravail");
    let long = "a".repeat(80);
    assert_eq!(label_id(&long, &[]).len(), 64);
    let taken = "a".repeat(64);
    let next = label_id(&long, &[&taken]);
    assert_eq!(next.len(), 64);
    assert!(next.ends_with("-2"));
}

#[test]
fn next_color_is_least_used() {
    assert_eq!(next_color(&FlowLabelCatalog::default()), LabelColor::Blue);
    // blue, amber, green, red are used once each: purple is next.
    assert_eq!(next_color(&catalog()), LabelColor::Purple);
    let mut full = FlowLabelCatalog::default();
    for (i, color) in LabelColor::ALL.iter().enumerate() {
        full.labels.push(FlowLabel {
            id: format!("l{i}"),
            name: format!("L{i}"),
            color: *color,
            choice: None,
            extra: BTreeMap::new(),
        });
    }
    full.labels[0].color = LabelColor::Amber;
    assert_eq!(next_color(&full), LabelColor::Blue);
}

#[test]
fn catalog_round_trips_unknown_keys() {
    let value = json!({
        "labels": [{"id": "a", "name": "A", "color": "teal", "icon": "leaf"}],
        "choices": [{"id": "c", "name": "C", "order": 2}],
        "version": 3
    });
    let catalog = FlowLabelCatalog::decode(value.to_string().as_bytes()).unwrap();
    assert_eq!(catalog.labels[0].color, LabelColor::Teal);
    assert_eq!(catalog.label("a").map(|l| l.name.as_str()), Some("A"));
    assert_eq!(catalog.choice("c").map(|c| c.current.clone()), Some(None));
    let back: Value = serde_json::from_str(&catalog.encode()).unwrap();
    assert_eq!(back, value);
}

#[test]
fn empty_payload_is_an_empty_catalog() {
    assert_eq!(
        FlowLabelCatalog::decode(b"").unwrap(),
        FlowLabelCatalog::default()
    );
    assert_eq!(
        FlowLabelCatalog::decode(b"{}").unwrap(),
        FlowLabelCatalog::default()
    );
    assert!(FlowLabelCatalog::decode(b"[").is_err());
    assert_eq!(
        FlowLabelCatalog::default().encode(),
        r#"{"labels":[],"choices":[]}"#
    );
}

#[test]
fn members_are_in_catalog_order() {
    let catalog = catalog();
    let ids: Vec<&str> = catalog
        .members("saison")
        .iter()
        .map(|l| l.id.as_str())
        .collect();
    assert_eq!(ids, ["hiver", "ete"]);
    assert!(catalog.members("nope").is_empty());
}
