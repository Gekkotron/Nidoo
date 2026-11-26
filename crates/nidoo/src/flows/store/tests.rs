use super::*;

const REFERENCE: &[u8] = br#"{
      "id": "living", "name": "Living LED on motion", "enabled": true, "mode": "restart",
      "nodes": [
        { "id": "motion", "type": "trigger.state", "params": { "entity": "m" } },
        { "id": "moving", "type": "if", "params": { "op": "eq", "value": true } },
        { "id": "on", "type": "command", "params": { "entity": "l", "command": { "type": "turn_on" } } }
      ],
      "edges": [
        { "from": "motion", "to": "moving" },
        { "from": "moving", "port": "true", "to": "on" }
      ]
    }"#;

fn store_with_flow() -> FlowStore {
    let mut store = FlowStore::new(20);
    assert_eq!(store.apply("living", REFERENCE), StoreApplied::Changed);
    store
}

#[test]
fn valid_payload_is_stored_and_listed() {
    let store = store_with_flow();
    assert_eq!(store.flows().len(), 1);
    assert!(store.exists("living"));
    let views = store.views();
    assert_eq!(views.len(), 1);
    assert!(views[0].invalid.is_none());
    assert_eq!(views[0].definition.id, "living");
}

#[test]
fn same_payload_is_unchanged() {
    let mut store = store_with_flow();
    assert_eq!(store.apply("living", REFERENCE), StoreApplied::Unchanged);
    let mut bad = FlowStore::new(1);
    assert_eq!(bad.apply("x", b"{"), StoreApplied::Changed);
    assert_eq!(bad.apply("x", b"{"), StoreApplied::Unchanged);
}

#[test]
fn empty_payload_removes_and_drops_runs() {
    let mut store = store_with_flow();
    store.begin_run("living", None, 1).unwrap();
    assert_eq!(store.apply("living", b""), StoreApplied::Removed);
    assert!(store.get("living").is_none());
    assert!(!store.exists("living"));
    assert_eq!(store.active_run("living"), None);
    // Re-adding starts with no history.
    store.apply("living", REFERENCE);
    assert!(store.get("living").unwrap().runs.is_empty());
}

#[test]
fn empty_payload_for_unknown_id_is_not_found() {
    let mut store = FlowStore::new(20);
    assert_eq!(store.apply("nope", b""), StoreApplied::NotFound);
}

#[test]
fn invalid_json_is_kept_as_invalid_with_reason() {
    let mut store = FlowStore::new(20);
    assert_eq!(store.apply("living", b"{"), StoreApplied::Changed);
    let views = store.views();
    assert!(!views[0].invalid.as_ref().unwrap().is_empty());
    assert_eq!(views[0].definition.id, "living");
    assert_eq!(views[0].definition.name, "living");
    assert_eq!(views[0].definition.area, None);
    assert!(views[0].definition.nodes.is_empty());
    assert!(store.flows().is_empty());
    assert!(store.exists("living"));
}

#[test]
fn invalid_stub_keeps_name_and_area_when_readable() {
    let mut store = FlowStore::new(20);
    store.apply(
        "living",
        br#"{"id":"living","name":"Living LED","area":"living","nodes":[{"id":"x","type":"nope"}]}"#,
    );
    let view = store.get("living").unwrap();
    assert!(view.invalid.is_some());
    assert_eq!(view.definition.name, "Living LED");
    assert_eq!(view.definition.area.as_deref(), Some("living"));
}

#[test]
fn id_mismatch_is_invalid() {
    let mut store = FlowStore::new(20);
    store.apply("living", &payload_with_id("other"));
    let view = store.get("living").unwrap();
    let reason = view.invalid.unwrap();
    assert!(reason.starts_with("id mismatch"), "{reason}");
    assert_eq!(view.definition.id, "living");
    assert!(store.flows().is_empty());
}

fn payload_with_id(id: &str) -> Vec<u8> {
    String::from_utf8(REFERENCE.to_vec())
        .unwrap()
        .replace("\"id\": \"living\"", &format!("\"id\": \"{id}\""))
        .into_bytes()
}

#[test]
fn missing_id_is_filled_from_topic() {
    let mut store = FlowStore::new(20);
    let payload = String::from_utf8(REFERENCE.to_vec())
        .unwrap()
        .replace("\"id\": \"living\", ", "");
    store.apply("living", payload.as_bytes());
    let flows = store.flows();
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].id, "living");
}

#[test]
fn runs_are_newest_first_and_bounded() {
    let mut store = FlowStore::new(2);
    store.apply("living", REFERENCE);
    for at in 1..=3 {
        store.begin_run("living", None, at).unwrap();
    }
    let runs = store.get("living").unwrap().runs;
    let ids: Vec<_> = runs.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["r3", "r2"]);
}

#[test]
fn end_run_clears_active_only_for_that_run() {
    let mut store = store_with_flow();
    let first = store.begin_run("living", None, 1).unwrap();
    let second = store.begin_run("living", None, 2).unwrap();
    assert_eq!(store.active_run("living"), Some(second.id.clone()));
    let ended = store
        .end_run("living", &first.id, RunStatus::Cancelled, 3)
        .unwrap();
    assert_eq!(ended.status, RunStatus::Cancelled);
    assert_eq!(ended.ended_at, Some(3));
    assert_eq!(store.active_run("living"), Some(second.id.clone()));
    store.end_run("living", &second.id, RunStatus::Done, 4);
    assert_eq!(store.active_run("living"), None);
}

#[test]
fn steps_and_skipped_runs_are_recorded() {
    let mut store = store_with_flow();
    let run = store.begin_run("living", None, 1).unwrap();
    let step = StepView {
        node: "on".into(),
        at: 2,
        outcome: StepOutcome::Ok,
        detail: "sent".into(),
        ports: None,
    };
    let updated = store.record_step("living", &run.id, step).unwrap();
    assert_eq!(updated.steps.len(), 1);
    let skipped = store.add_skipped_run("living", "motion", None, 5).unwrap();
    assert_eq!(skipped.status, RunStatus::Skipped);
    assert_eq!(skipped.ended_at, Some(5));
    assert_eq!(skipped.steps[0].detail, "a run is active");
    // A skipped run does not take over the active slot.
    assert_eq!(store.active_run("living"), Some(run.id));
    assert!(store.begin_run("ghost", None, 1).is_none());
    assert!(
        store
            .record_step("ghost", "r1", skipped.steps[0].clone())
            .is_none()
    );
}

#[test]
fn ending_the_only_kept_run_clears_active_and_keeps_it() {
    let mut store = FlowStore::new(1);
    store.apply("living", REFERENCE);
    let run = store.begin_run("living", None, 1).unwrap();
    store.add_skipped_run("living", "motion", None, 2).unwrap();
    store
        .end_run("living", &run.id, RunStatus::Done, 3)
        .unwrap();
    assert_eq!(store.active_run("living"), None);
    let runs = store.get("living").unwrap().runs;
    assert!(runs.iter().any(|r| r.id == run.id));
}

#[test]
fn running_run_survives_skipped_runs() {
    let mut store = FlowStore::new(2);
    store.apply("living", REFERENCE);
    let run = store.begin_run("living", None, 1).unwrap();
    for at in 2..=4 {
        store.add_skipped_run("living", "motion", None, at).unwrap();
    }
    let view = store.get("living").unwrap();
    assert_eq!(view.active_run.as_deref(), Some(run.id.as_str()));
    assert!(view.runs.iter().any(|r| r.id == run.id));
    assert!(
        store
            .record_step(
                "living",
                &run.id,
                StepView {
                    node: "on".into(),
                    at: 5,
                    outcome: StepOutcome::Ok,
                    detail: String::new(),
                    ports: None,
                }
            )
            .is_some()
    );
}

#[test]
fn end_run_of_a_missing_run_returns_none() {
    let mut store = store_with_flow();
    let run = store.begin_run("living", None, 1).unwrap();
    // Removing and re-adding the flow drops its runs.
    store.apply("living", b"");
    store.apply("living", REFERENCE);
    assert!(
        store
            .end_run("living", &run.id, RunStatus::Done, 2)
            .is_none()
    );
    assert_eq!(store.active_run("living"), None);
}

#[test]
fn end_run_clears_a_dangling_active_id() {
    let mut store = store_with_flow();
    store.active.insert("living".into(), "ghost".into());
    assert!(
        store
            .end_run("living", "ghost", RunStatus::Done, 2)
            .is_none()
    );
    assert_eq!(store.active_run("living"), None);
}

#[test]
fn hostile_json_shapes_do_not_panic() {
    let mut store = FlowStore::new(20);
    for payload in [&b"[]"[..], b"5", b"\"x\"", b"null"] {
        store.apply("living", payload);
        assert!(store.get("living").unwrap().invalid.is_some());
        store.apply("living", b"");
    }
}

#[test]
fn null_or_empty_id_is_filled_from_topic() {
    for fragment in ["\"id\": null", "\"id\": \"\""] {
        let mut store = FlowStore::new(20);
        let payload = String::from_utf8(REFERENCE.to_vec())
            .unwrap()
            .replace("\"id\": \"living\"", fragment);
        store.apply("living", payload.as_bytes());
        assert_eq!(store.flows().len(), 1, "{fragment}");
        assert_eq!(store.flows()[0].id, "living");
    }
}
