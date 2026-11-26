use super::*;
use nidoo_api::{AreaView, FlowView, RunStatus, RunView};
use nidoo_core::flow::{Flow, FlowLabelCatalog};
use nidoo_core::model::UNASSIGNED_AREA;

fn run(status: RunStatus, started_at: u64) -> RunView {
    RunView {
        id: format!("r{started_at}"),
        flow_id: "f".into(),
        status,
        started_at,
        ended_at: None,
        trigger: None,
        manual: false,
        steps: vec![],
    }
}

fn flow(id: &str, name: &str, triggers: &[&str]) -> Flow {
    let nodes: Vec<String> = triggers
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let params = match *t {
                "trigger.state" => r#"{ "entity": "m" }"#,
                "trigger.schedule" => r#"{ "cron": "* * * * *" }"#,
                "trigger.sun" => r#"{ "which": "sunset" }"#,
                "trigger.off_peak" => r#"{ "edge": "start" }"#,
                "trigger.silent" => r#"{ "entity": "m", "for": "50m" }"#,
                "trigger.manual" => "{}",
                _ => r#"{ "entity": "r" }"#,
            };
            format!(r#"{{ "id": "t{i}", "type": "{t}", "params": {params} }}"#)
        })
        .collect();
    Flow::decode(
        format!(
            r#"{{ "id": "{id}", "name": "{name}", "nodes": [{}], "edges": [] }}"#,
            nodes.join(",")
        )
        .as_bytes(),
    )
    .unwrap()
}

fn view(id: &str, name: &str, triggers: &[&str]) -> FlowView {
    FlowView {
        definition: flow(id, name, triggers),
        invalid: None,
        active_run: None,
        runs: vec![],
    }
}

fn invalid(id: &str) -> FlowView {
    FlowView {
        definition: flow(id, id, &[]),
        invalid: Some("broken".into()),
        active_run: None,
        runs: vec![],
    }
}

#[test]
fn row_key_changes_only_with_what_the_row_shows() {
    let base = view("f", "Hall light", &["trigger.state"]);
    let key = |v: &FlowView| row_key(&row(v.clone()));
    assert_eq!(key(&base), key(&base));
    assert_eq!(key(&base).0, "f");

    let mut running = base.clone();
    running.runs = vec![run(RunStatus::Running, 5)];
    assert_ne!(key(&running), key(&base), "a new run shows");
    let mut stepped = running.clone();
    stepped.runs[0].steps.push(nidoo_api::StepView {
        node: "t0".into(),
        at: 6,
        outcome: nidoo_api::StepOutcome::Ok,
        detail: String::new(),
        ports: None,
    });
    assert_eq!(key(&stepped), key(&running), "a step shows nothing new");
    let mut done = running.clone();
    done.runs[0].status = RunStatus::Done;
    assert_ne!(key(&done), key(&running), "the run ending shows");

    let mut off = base.clone();
    off.definition.enabled = false;
    assert_ne!(key(&off), key(&base));
    let mut renamed = base.clone();
    renamed.definition.name = "Porch light".into();
    assert_ne!(key(&renamed), key(&base));
    let moved = row_in(base.clone(), "living", "Living room");
    assert_ne!(row_key(&moved), key(&base));
    let mut broken = base.clone();
    broken.invalid = Some("unknown entity".into());
    assert_ne!(key(&broken), key(&base));
}

#[test]
fn clear_stale_area_drops_unknown_and_unassigned() {
    let areas = vec![area("living", "Living room")];
    let with = |a: &str| {
        let mut f = Flow {
            area: Some(a.into()),
            ..Flow::default()
        };
        clear_stale_area(&mut f, &areas);
        f.area
    };
    assert_eq!(with("ghost"), None);
    assert_eq!(with(UNASSIGNED_AREA), None);
    assert_eq!(with("living"), Some("living".into()));
}

fn area(id: &str, name: &str) -> AreaView {
    AreaView {
        id: id.into(),
        name: name.into(),
        icon: None,
        order: None,
        entity_count: 0,
        primary_count: 0,
        group_names: Default::default(),
        group_favorites: Default::default(),
    }
}

fn row(v: FlowView) -> ListRow {
    rows(
        Lang::En,
        &[v],
        &[],
        &BTreeMap::new(),
        &FlowLabelCatalog::default(),
    )
    .remove(0)
}

/// The house of the examples: a season choice and a standalone label.
fn seasons() -> FlowLabelCatalog {
    serde_json::from_value(serde_json::json!({
        "labels": [
            {"id": "hiver", "name": "Hiver", "color": "blue", "choice": "saison"},
            {"id": "ete", "name": "Été", "color": "amber", "choice": "saison"},
            {"id": "vacances", "name": "Vacances", "color": "green"}
        ],
        "choices": [
            {"id": "saison", "name": "Saison", "current": "hiver"},
            {"id": "mode", "name": "Mode"}
        ]
    }))
    .unwrap()
}

fn labelled(id: &str, labels: &[&str]) -> FlowView {
    let mut v = view(id, id, &["trigger.manual"]);
    v.definition.labels = labels.iter().map(|l| (*l).to_owned()).collect();
    v
}

fn label_rows(flows: &[FlowView], catalog: &FlowLabelCatalog) -> Vec<ListRow> {
    rows(Lang::En, flows, &[], &BTreeMap::new(), catalog)
}

fn picking(name: &str, labels: &[&str], picks: Option<&str>) -> FlowView {
    let mut nodes = vec![serde_json::json!({"id": "go", "type": "trigger.manual", "params": {}})];
    if let Some(label) = picks {
        nodes.push(
            serde_json::json!({"id": "pick", "type": "flow.label", "params": {"label": label}}),
        );
    }
    let json = serde_json::json!({"id": name, "name": name, "labels": labels, "nodes": nodes});
    FlowView {
        definition: Flow::decode(json.to_string().as_bytes()).unwrap(),
        invalid: None,
        active_run: None,
        runs: vec![],
    }
}

fn two_labels() -> FlowLabelCatalog {
    serde_json::from_value(serde_json::json!({
        "labels": [{"id": "ete", "name": "Été"}, {"id": "hiver", "name": "Hiver"}]
    }))
    .unwrap()
}

#[test]
fn clear_stale_labels_keeps_known_ids_only() {
    let mut flow = picking("a", &["ete", "gone"], None).definition;
    clear_stale_labels(&mut flow, &two_labels());
    assert_eq!(flow.labels, ["ete"]);
    // A catalog emptied by deleting its last label drops that label too.
    clear_stale_labels(&mut flow, &FlowLabelCatalog::default());
    assert!(flow.labels.is_empty());
}

#[test]
fn label_users_and_counts() {
    let flows = [
        picking("Zeta", &["ete"], Some("ete")),
        picking("Alpha", &["ete", "hiver"], Some("ete")),
        picking("Other", &[], Some("hiver")),
    ];
    assert_eq!(label_users(&flows, "ete"), ["Alpha", "Zeta"]);
    assert!(label_users(&flows, "vacances").is_empty());
    let counts = label_flow_counts(&flows);
    assert_eq!(counts.get("ete"), Some(&2));
    assert_eq!(counts.get("hiver"), Some(&1));
}

#[test]
fn new_label_id_and_name_lookup() {
    let before = two_labels();
    let mut after = before.clone();
    after.labels.push(
        serde_json::from_value(serde_json::json!({"id": "vacances", "name": "Vacances"})).unwrap(),
    );
    assert_eq!(new_label_id(&before, &after).as_deref(), Some("vacances"));
    assert_eq!(new_label_id(&before, &before), None);
    assert_eq!(
        label_by_name(&before, " été ").map(|l| l.id.as_str()),
        Some("ete")
    );
    assert_eq!(
        label_by_name(&before, "ETE").map(|l| l.id.as_str()),
        Some("ete")
    );
    assert!(label_by_name(&before, "Printemps").is_none());
}

#[test]
fn toggled_labels_sorts_and_dedups() {
    let labels = ["hiver".to_owned(), "ete".to_owned()];
    assert_eq!(
        toggled_labels(&labels, "vacances"),
        ["ete", "hiver", "vacances"]
    );
    assert_eq!(toggled_labels(&labels, "hiver"), ["ete"]);
}

#[test]
fn label_reach_counts_carried_flows_whatever_their_state() {
    let catalog: FlowLabelCatalog = serde_json::from_value(serde_json::json!({
        "labels": [
            {"id": "hiver", "name": "Hiver", "choice": "saison"},
            {"id": "ete", "name": "Été", "choice": "saison"},
            {"id": "vacances", "name": "Vacances"}
        ],
        "choices": [{"id": "saison", "name": "Saison"}]
    }))
    .unwrap();
    let mut off = picking("fan", &["ete"], None);
    off.definition.enabled = false;
    let mut broken = picking("broken", &["ete"], None);
    broken.invalid = Some("bad".into());
    let flows = [
        picking("heat", &["hiver"], None),
        picking("both", &["hiver", "ete"], None),
        off,
        broken,
    ];
    // Été: "both" and "fan" (off now) on; "heat" off; the invalid one is skipped.
    assert_eq!(
        label_reach(&catalog, &flows, "ete", LabelAction::Pick),
        Some((2, 1))
    );
    assert_eq!(
        label_reach(&catalog, &flows, "ete", LabelAction::Off),
        Some((0, 2))
    );
    assert_eq!(
        label_reach(&catalog, &flows, "hiver", LabelAction::On),
        Some((2, 0))
    );
    assert_eq!(
        label_reach(&catalog, &flows, "vacances", LabelAction::Pick),
        None
    );
    assert_eq!(label_reach(&catalog, &flows, "gone", LabelAction::On), None);
}

#[test]
fn label_color_names_are_the_wire_names() {
    for color in nidoo_core::flow::LabelColor::ALL {
        let wire = serde_json::to_value(color).unwrap();
        assert_eq!(wire, label_color_name(color));
    }
}

#[test]
fn grouping_label_decodes() {
    assert_eq!(
        decode_prefs(r#"{"grouping":"label"}"#).grouping,
        Grouping::Label
    );
    let p = FlowListPrefs {
        grouping: Grouping::Label,
        ..FlowListPrefs::default()
    };
    assert_eq!(decode_prefs(&encode_prefs(&p)), p);
}

#[test]
fn rows_resolve_labels_in_catalog_order_and_drop_unknown_ids() {
    let catalog = seasons();
    let out = label_rows(&[labelled("a", &["vacances", "ghost", "hiver"])], &catalog);
    let ids: Vec<&str> = out[0].labels.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(ids, ["hiver", "vacances"]);
}

#[test]
fn label_sections_order_and_duplicates() {
    let catalog = seasons();
    let list = label_rows(
        &[
            labelled("both", &["hiver", "ete"]),
            labelled("winter_away", &["hiver", "vacances"]),
            labelled("plain", &[]),
            labelled("stale", &["ghost"]),
        ],
        &catalog,
    );
    let out = label_sections(Lang::Fr, list, &catalog);
    let keys: Vec<&str> = out.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "choice:saison",
            "choice:mode",
            "label:vacances",
            NO_LABEL_KEY
        ]
    );
    assert_eq!(ids(&out[0].rows), ["both", "winter_away"]);
    assert_eq!(ids(&out[2].rows), ["winter_away"]);
    assert_eq!(ids(&out[3].rows), ["plain", "stale"]);
    assert_eq!(out[3].title, "Sans étiquette");
    assert_eq!(out[0].title, "Saison");
    match &out[0].head {
        SectionHead::Choice { choice, members } => {
            assert_eq!(choice.id, "saison");
            let m: Vec<&str> = members.iter().map(|l| l.id.as_str()).collect();
            assert_eq!(m, ["hiver", "ete"]);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(&out[2].head, SectionHead::Label(l) if l.id == "vacances"));
    assert_eq!(out[3].head, SectionHead::NoLabel);
}

#[test]
fn empty_choice_section_is_kept() {
    let catalog = seasons();
    let out = label_sections(
        Lang::En,
        label_rows(&[labelled("x", &["hiver"])], &catalog),
        &catalog,
    );
    let keys: Vec<&str> = out.iter().map(|s| s.key.as_str()).collect();
    // `mode` and `vacances` carry no flow but keep their switch; "No label" is empty and dropped.
    assert_eq!(keys, ["choice:saison", "choice:mode", "label:vacances"]);
    assert!(out[1].rows.is_empty());
    assert!(matches!(&out[1].head, SectionHead::Choice { members, .. } if members.is_empty()));
}

#[test]
fn room_sections_have_plain_heads() {
    let out = sections(Lang::En, vec![row(view("a", "A", &[]))], &[]);
    assert_eq!(out[0].head, SectionHead::Plain);
}

#[test]
fn query_matches_label_names() {
    let catalog = seasons();
    let list = label_rows(&[labelled("a", &["ete"]), labelled("b", &[])], &catalog);
    let f = Filters {
        query: "ETE".into(),
        ..Filters::default()
    };
    assert_eq!(ids(&filter(&list, &f)), ["a"]);
}

#[test]
fn row_key_changes_with_labels() {
    let catalog = seasons();
    let key = |labels: &[&str], catalog: &FlowLabelCatalog| {
        row_key(&label_rows(&[labelled("a", labels)], catalog)[0])
    };
    assert_ne!(key(&["hiver"], &catalog), key(&[], &catalog));
    assert_ne!(key(&["hiver"], &catalog), key(&["ete"], &catalog));
    let mut recoloured = catalog.clone();
    recoloured.labels[0].color = nidoo_core::flow::LabelColor::Red;
    assert_ne!(key(&["hiver"], &catalog), key(&["hiver"], &recoloured));
    let mut renamed = catalog.clone();
    renamed.labels[0].name = "Winter".into();
    assert_ne!(key(&["hiver"], &catalog), key(&["hiver"], &renamed));
}

fn row_in(v: FlowView, room: &str, room_name: &str) -> ListRow {
    let mut r = row(v);
    r.room = Some(room.into());
    r.room_name = Some(room_name.into());
    r
}

fn failing_sun(id: &str) -> FlowView {
    let mut v = view(id, id, &["trigger.sun"]);
    v.runs = vec![run(RunStatus::Failed, 1)];
    v
}

fn ids(rows: &[ListRow]) -> Vec<&str> {
    rows.iter().map(|r| r.view.definition.id.as_str()).collect()
}

#[test]
fn attention_on_invalid_and_latest_finished_failure() {
    let with = |statuses: &[RunStatus]| {
        let mut v = view("f", "F", &[]);
        v.runs = statuses.iter().map(|s| run(*s, 1)).collect();
        v
    };
    assert!(needs_attention(&invalid("x")));
    assert!(needs_attention(&with(&[RunStatus::Failed])));
    assert!(needs_attention(&with(&[
        RunStatus::Skipped,
        RunStatus::Failed
    ])));
    assert!(!needs_attention(&with(&[
        RunStatus::Done,
        RunStatus::Failed
    ])));
    assert!(!needs_attention(&with(&[
        RunStatus::Running,
        RunStatus::Done
    ])));
    assert!(!needs_attention(&with(&[])));
}

#[test]
fn kinds_per_trigger_type_and_invalid() {
    let v = view("f", "F", &["trigger.state", "trigger.schedule"]);
    assert_eq!(
        trigger_kinds(&v),
        BTreeSet::from([TriggerKind::State, TriggerKind::Schedule])
    );
    let all = view(
        "f",
        "F",
        &["trigger.sun", "trigger.signal", "trigger.state"],
    );
    assert_eq!(
        trigger_kinds(&all),
        BTreeSet::from([TriggerKind::Sun, TriggerKind::Signal, TriggerKind::State])
    );
    assert!(trigger_kinds(&invalid("x")).is_empty());
}

#[test]
fn kinds_off_peak_is_its_own_kind() {
    let only = view("f", "F", &["trigger.off_peak"]);
    assert_eq!(trigger_kinds(&only), BTreeSet::from([TriggerKind::OffPeak]));
}

#[test]
fn kinds_silent_is_its_own_kind() {
    let only = view("f", "F", &["trigger.silent"]);
    assert_eq!(trigger_kinds(&only), BTreeSet::from([TriggerKind::Silent]));
}

#[test]
fn kinds_manual_card_is_manual() {
    let only = view("f", "F", &["trigger.manual"]);
    assert_eq!(trigger_kinds(&only), BTreeSet::from([TriggerKind::Manual]));
    let both = view("f", "F", &["trigger.state", "trigger.manual"]);
    assert_eq!(
        trigger_kinds(&both),
        BTreeSet::from([TriggerKind::State, TriggerKind::Manual])
    );
    assert!(trigger_kinds(&view("f", "F", &[])).is_empty());
}

#[test]
fn rows_treat_unknown_area_as_no_room() {
    let areas = [area("salon", "Salon"), area(UNASSIGNED_AREA, "Unassigned")];
    let mut ghost = view("a", "A", &[]);
    ghost.definition.area = Some("ghost".into());
    let mut unassigned = view("b", "B", &[]);
    unassigned.definition.area = Some(UNASSIGNED_AREA.into());
    let mut known = view("c", "C", &[]);
    known.definition.area = Some("salon".into());
    let out = rows(
        Lang::En,
        &[ghost, unassigned, known],
        &areas,
        &BTreeMap::new(),
        &FlowLabelCatalog::default(),
    );
    assert_eq!(
        (out[0].room.clone(), out[0].room_name.clone()),
        (None, None)
    );
    assert_eq!(
        (out[1].room.clone(), out[1].room_name.clone()),
        (None, None)
    );
    assert_eq!(out[2].room.as_deref(), Some("salon"));
    assert_eq!(out[2].room_name.as_deref(), Some("Salon"));
    assert_eq!(out[2].trigger_text, "No trigger");
}

#[test]
fn search_folds_case_and_accents() {
    let q = |query: &str| Filters {
        query: query.into(),
        ..Filters::default()
    };
    let by_room = row_in(view("a", "A", &[]), "entree", "Entrée");
    assert_eq!(filter(&[by_room], &q("ENTREE")).len(), 1);
    let by_name = row(view("b", "Lumière Salon", &[]));
    assert_eq!(filter(std::slice::from_ref(&by_name), &q("salon")).len(), 1);
    assert_eq!(
        filter(std::slice::from_ref(&by_name), &q("cuisine")).len(),
        0
    );
    let mut by_trigger = row(view("c", "C", &["trigger.sun"]));
    by_trigger.trigger_text = "At sunset".into();
    assert_eq!(filter(&[by_trigger], &q("SUNSET")).len(), 1);
    let mut v = view("d", "D", &[]);
    v.definition.description = Some("Ferme les volets".into());
    assert_eq!(filter(&[row(v)], &q("volets")).len(), 1);
}

#[test]
fn filters_combine_status_and_triggers() {
    let mut off = view("off", "Off", &["trigger.sun"]);
    off.definition.enabled = false;
    let all = vec![
        row(failing_sun("fail_sun")),
        row(view("ok_sun", "Ok", &["trigger.sun"])),
        row(view("state", "State", &["trigger.state"])),
        row(invalid("bad")),
        row(off),
    ];
    let f = Filters {
        status: StatusFilter::Attention,
        triggers: BTreeSet::from([TriggerKind::Sun]),
        ..Filters::default()
    };
    assert_eq!(ids(&filter(&all, &f)), ["fail_sun"]);
    let attention = Filters {
        status: StatusFilter::Attention,
        ..Filters::default()
    };
    assert_eq!(ids(&filter(&all, &attention)), ["fail_sun", "bad"]);
    let on = Filters {
        status: StatusFilter::On,
        ..Filters::default()
    };
    assert_eq!(ids(&filter(&all, &on)), ["fail_sun", "ok_sun", "state"]);
    let off_only = Filters {
        status: StatusFilter::Off,
        ..Filters::default()
    };
    assert_eq!(ids(&filter(&all, &off_only)), ["off"]);
}

#[test]
fn counts_follow_query_and_triggers_not_status() {
    let mut off = view("off", "Off", &["trigger.sun"]);
    off.definition.enabled = false;
    let all = vec![
        row(failing_sun("fail_sun")),
        row(view("state", "State", &["trigger.state"])),
        row(invalid("bad")),
        row(off),
    ];
    let c = status_counts(&all, "", &BTreeSet::new());
    assert_eq!((c.all, c.on, c.off, c.attention), (4, 2, 1, 2), "{c:?}");
    let sun = BTreeSet::from([TriggerKind::Sun]);
    let c = status_counts(&all, "", &sun);
    assert_eq!((c.all, c.on, c.off, c.attention), (2, 1, 1, 1));
    let c = status_counts(&all, "state", &BTreeSet::new());
    assert_eq!((c.all, c.on, c.off, c.attention), (1, 1, 0, 0));
}

#[test]
fn sort_last_run_newest_first_never_run_last() {
    let mut a = view("a", "Alpha", &[]);
    a.runs = vec![run(RunStatus::Done, 10)];
    let mut b = view("b", "Bravo", &[]);
    b.runs = vec![run(RunStatus::Done, 50), run(RunStatus::Done, 5)];
    let never_z = view("z", "Zulu", &[]);
    let never_c = view("c", "Charlie", &[]);
    let mut tie = view("t", "Able", &[]);
    tie.runs = vec![run(RunStatus::Done, 10)];
    let mut list: Vec<ListRow> = [a, never_z, b, never_c, tie].into_iter().map(row).collect();
    sort_rows(&mut list, Sort::LastRun);
    assert_eq!(ids(&list), ["b", "t", "a", "c", "z"]);
    sort_rows(&mut list, Sort::Name);
    assert_eq!(ids(&list), ["t", "a", "b", "c", "z"]);
}

#[test]
fn sections_follow_area_order_no_room_last() {
    let areas = [
        area("b", "Bedroom"),
        area("a", "Attic"),
        area("empty", "Empty"),
        area(UNASSIGNED_AREA, "Unassigned"),
    ];
    let list = vec![
        row_in(view("in_a", "A", &[]), "a", "Attic"),
        row(view("none", "N", &[])),
        row_in(view("in_b", "B", &[]), "b", "Bedroom"),
        row_in(view("in_b2", "B2", &[]), "b", "Bedroom"),
    ];
    let out = sections(Lang::En, list, &areas);
    let keys: Vec<&str> = out.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, ["b", "a", NO_ROOM_KEY]);
    assert_eq!(out[0].title, "Bedroom");
    assert_eq!(out[2].title, "No room");
    assert_eq!(ids(&out[0].rows), ["in_b", "in_b2"]);
}

#[test]
fn section_flags_attention() {
    let list = vec![
        row_in(view("ok", "Ok", &[]), "a", "Attic"),
        row_in(invalid("bad"), "a", "Attic"),
        row_in(view("ok2", "Ok2", &[]), "b", "Bedroom"),
    ];
    let out = sections(Lang::En, list, &[area("a", "Attic"), area("b", "Bedroom")]);
    assert!(out[0].attention);
    assert!(!out[1].attention);
}

#[test]
fn prefs_round_trip_and_decode_garbage_is_default() {
    assert_eq!(decode_prefs("nope"), FlowListPrefs::default());
    assert_eq!(decode_prefs(""), FlowListPrefs::default());
    let cards = decode_prefs(r#"{"view":"cards"}"#);
    assert_eq!(cards.view, ViewMode::Cards);
    assert_eq!(
        FlowListPrefs {
            view: ViewMode::List,
            ..cards.clone()
        },
        FlowListPrefs::default()
    );
    let p = FlowListPrefs {
        view: ViewMode::Cards,
        grouping: Grouping::None,
        sort: Sort::LastRun,
        status: StatusFilter::Attention,
        triggers: BTreeSet::from([TriggerKind::Sun, TriggerKind::Manual]),
        folded: BTreeSet::from(["salon".to_owned()]),
    };
    assert_eq!(decode_prefs(&encode_prefs(&p)), p);
}
