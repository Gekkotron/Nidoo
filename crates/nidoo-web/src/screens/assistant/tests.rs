use super::*;
use nidoo_assist::view::ActionTrace;

#[test]
fn chips_live_as_long_as_the_hub_keeps_the_question() {
    let window = PENDING_TTL_S as f64 * 1000.0;
    // Just asked: the whole window, whatever the clocks say.
    assert_eq!(chip_window_ms(1_000, 1_000_000.0), window);
    assert_eq!(chip_window_ms(1_000, 990_000.0), window);
    assert_eq!(chip_window_ms(1_000, 1_030_000.0), window - 30_000.0);
    // Expired, long ago or just now.
    assert_eq!(chip_window_ms(1_000, 1_060_000.0), 0.0);
    assert_eq!(chip_window_ms(1_000, 9_000_000.0), 0.0);
}

fn h(sent: &[&str]) -> History {
    History::new(sent.iter().map(|s| s.to_string()).collect())
}

#[test]
fn up_walks_back_and_down_gives_back_the_typed_sentence() {
    let mut h = h(&["allume le salon", "éteins le salon"]);
    assert_eq!(h.up("mets le"), Some("éteins le salon".into()));
    assert_eq!(h.up("éteins le salon"), Some("allume le salon".into()));
    // The oldest: up stays put.
    assert_eq!(h.up("allume le salon"), None);
    assert_eq!(h.down(), Some("éteins le salon".into()));
    assert_eq!(h.down(), Some("mets le".into()));
    // Not browsing any more.
    assert_eq!(h.down(), None);
}

#[test]
fn nothing_sent_nothing_to_recall() {
    let mut h = h(&[]);
    assert_eq!(h.up(""), None);
    assert_eq!(h.down(), None);
}

#[test]
fn an_edit_stops_browsing() {
    let mut h = h(&["a", "b"]);
    assert_eq!(h.up(""), Some("b".into()));
    h.edited();
    assert_eq!(h.down(), None);
    assert_eq!(h.up("b!"), Some("b".into()));
    assert_eq!(h.down(), Some("b!".into()));
}

#[test]
fn sending_keeps_the_sentence_once_and_caps_the_list() {
    let mut h = h(&["a"]);
    h.push("a".into());
    h.push("b".into());
    assert_eq!(h.sent(), ["a", "b"]);
    for i in 0..HISTORY_MAX + 5 {
        h.push(format!("s{i}"));
    }
    assert_eq!(h.sent().len(), HISTORY_MAX);
    assert_eq!(h.sent().last().map(String::as_str), Some("s54"));
}

fn turn(id: &str, at: u64) -> AssistTurn {
    AssistTurn {
        id: id.into(),
        at,
        client: "c".into(),
        text: "allume le salon".into(),
        simulate: false,
        outcome: nidoo_assist::view::OutcomeKind::Acted,
        reply: "C'est fait.".into(),
        question: None,
        trace: Trace::default(),
        wrong: false,
    }
}

fn ids(list: &[AssistTurn]) -> Vec<&str> {
    list.iter().map(|t| t.id.as_str()).collect()
}

#[test]
fn a_clear_hides_the_turns_up_to_the_mark() {
    let list = vec![turn("a", 10), turn("b", 20), turn("c", 20), turn("d", 30)];
    assert_eq!(ids(&after_clear(list.clone(), None)), ["a", "b", "c", "d"]);
    // The marked turn is in the list: only what came after it, even within the same second.
    let mark = ClearMark {
        id: "b".into(),
        at: 20,
    };
    assert_eq!(ids(&after_clear(list.clone(), Some(&mark))), ["c", "d"]);
    // The marked turn has aged out of the list or the hub forgot it: go by its time.
    let mark = ClearMark {
        id: "gone".into(),
        at: 20,
    };
    assert_eq!(ids(&after_clear(list.clone(), Some(&mark))), ["d"]);
    let mark = ClearMark {
        id: "d".into(),
        at: 30,
    };
    assert!(after_clear(list, Some(&mark)).is_empty());
}

#[test]
fn a_turn_already_shown_stays_single() {
    let mut list = vec![turn("a", 10), turn("b", 20)];
    let mut b = turn("b", 20);
    b.reply = "changed".into();
    assert!(add_turns(&mut list, vec![turn("a", 10), b, turn("c", 30)]));
    assert_eq!(ids(&list), ["a", "b", "c"]);
    // The copy on the page wins: a « Faux » tapped here is not undone by a fetch.
    assert_eq!(list[1].reply, "C'est fait.");
    assert!(!add_turns(&mut list, vec![turn("c", 30)]));
    assert_eq!(ids(&list), ["a", "b", "c"]);
}

#[test]
fn a_late_reply_lands_in_time_order() {
    // A satellite turn fetched while this browser waited for its own reply.
    let mut list = vec![turn("a", 10), turn("sat", 30)];
    assert!(add_turns(&mut list, vec![turn("mine", 20)]));
    assert_eq!(ids(&list), ["a", "mine", "sat"]);
}

#[test]
fn the_mark_is_the_newest_turn() {
    assert_eq!(ClearMark::of(&[]), None);
    assert_eq!(
        ClearMark::of(&[turn("a", 10), turn("b", 20)]),
        Some(ClearMark {
            id: "b".into(),
            at: 20
        })
    );
}

#[test]
fn trace_rows_leave_out_empty_fields() {
    let trace = Trace {
        words: vec!["mets".into(), "le".into(), "salon".into()],
        room: Some("salon".into()),
        room_how: Some("mots : salon".into()),
        value: Some("19 − 0,5 = 18,5 °C".into()),
        actions: vec![ActionTrace {
            target: "salon".into(),
            what: "consigne 18,5 °C".into(),
            result: "ok".into(),
        }],
        elapsed_us: 80,
        ..Trace::default()
    };
    let rows = trace_rows(Lang::En, &trace);
    let labels: Vec<&str> = rows.iter().map(|(l, _)| *l).collect();
    assert_eq!(labels, ["Words", "Room", "Value", "Actions", "Time"]);
    assert_eq!(rows[1].1, "salon (mots : salon)");
    assert_eq!(rows[3].1, "salon → consigne 18,5 °C : ok");
}

fn volume(device: &str, name: &str, area: &str) -> nidoo_api::EntityView {
    use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};
    nidoo_api::EntityView {
        id: format!("{device}_volume"),
        device_id: Some(device.into()),
        name: "Volume".into(),
        device_name: Some(name.into()),
        label: None,
        kind: Kind::Number,
        component: Kind::Number.component().to_string(),
        protocol_kind: Kind::Number,
        device_class: None,
        unit: None,
        icon: None,
        category: EntityCategory::Primary,
        enabled_by_default: true,
        area: area.into(),
        part_of: None,
        capabilities: Capabilities::default(),
        ui: UiOverride::default(),
        state: EntityState::default(),
    }
}

fn areas() -> Vec<nidoo_api::AreaView> {
    let kitchen = serde_json::json!({
        "id": "kitchen", "name": "Kitchen", "icon": null, "order": null,
        "entity_count": 1, "primary_count": 1
    });
    vec![serde_json::from_value(kitchen).unwrap()]
}

#[test]
fn turn_source_names_the_satellite_and_its_room() {
    let entities: std::collections::BTreeMap<_, _> = [
        volume("nidoo_sound_aaaaaaaaaaaa", "Desk speaker", "kitchen"),
        volume(
            "nidoo_sound_bbbbbbbbbbbb",
            "Loose speaker",
            nidoo_core::model::UNASSIGNED_AREA,
        ),
    ]
    .into_iter()
    .map(|e| (e.id.clone(), e))
    .collect();
    let areas = areas();
    assert_eq!(
        turn_source("satellite:aaaaaaaaaaaa", &entities, &areas).as_deref(),
        Some("Desk speaker · Kitchen")
    );
    assert_eq!(
        turn_source("satellite:bbbbbbbbbbbb", &entities, &areas).as_deref(),
        Some("Loose speaker")
    );
    assert_eq!(
        turn_source("satellite:cccccccccccc", &entities, &areas).as_deref(),
        Some("cccccccccccc"),
        "unknown satellite"
    );
    assert_eq!(turn_source("k3j2h1", &entities, &areas), None, "a browser");
}
