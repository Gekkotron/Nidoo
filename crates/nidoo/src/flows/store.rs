//! The in-memory copy of the retained flow definitions, plus each flow's recent runs.

use std::collections::{BTreeMap, VecDeque};

use nidoo_api::{FlowView, RunStatus, RunView, StepOutcome, StepView, TriggerInfo};
use nidoo_core::flow::{Flow, validate};

/// What `apply` did, so the caller knows whether to republish and re-arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreApplied {
    /// The stored definition (or its invalid state) changed.
    Changed,
    /// The payload is byte-identical to the last one for this id.
    Unchanged,
    /// An empty payload removed a known flow and its runs.
    Removed,
    /// An empty payload for an id the store never held.
    NotFound,
}

/// A retained payload, decoded: either a runnable flow or the reason it is not.
enum Entry {
    Valid(Flow),
    Invalid {
        reason: String,
        /// The `name` the payload carried, when it was readable.
        name: Option<String>,
        /// The `area` the payload carried, when it was readable.
        area: Option<String>,
    },
}

/// The decoded copy of the retained flow definitions, with each flow's run history.
///
/// Invalid payloads are kept (with the reason) so the UI can show and fix them instead of
/// silently losing them. Runs live outside the definitions, so they survive an edit.
pub struct FlowStore {
    entries: BTreeMap<String, Entry>,
    last_payload: BTreeMap<String, Vec<u8>>,
    runs: BTreeMap<String, VecDeque<RunView>>,
    active: BTreeMap<String, String>,
    counter: u64,
    runs_kept: usize,
}

impl FlowStore {
    /// An empty store that keeps at most `runs_kept` recent runs per flow.
    pub fn new(runs_kept: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            last_payload: BTreeMap::new(),
            runs: BTreeMap::new(),
            active: BTreeMap::new(),
            counter: 0,
            runs_kept: runs_kept.max(1),
        }
    }

    /// Applies the retained payload of the topic `<prefix>/<id>`. An empty payload removes the
    /// flow and its runs. Otherwise the payload is decoded and validated; a failure is stored as
    /// an invalid entry. A missing `id` field is filled from the topic, a different one is an
    /// `id mismatch`. Payloads identical to the last one for this id are `Unchanged`, valid or not.
    pub fn apply(&mut self, id: &str, payload: &[u8]) -> StoreApplied {
        if payload.is_empty() {
            self.last_payload.remove(id);
            self.runs.remove(id);
            self.active.remove(id);
            return match self.entries.remove(id) {
                Some(_) => StoreApplied::Removed,
                None => StoreApplied::NotFound,
            };
        }
        if self
            .last_payload
            .get(id)
            .is_some_and(|last| last == payload)
        {
            return StoreApplied::Unchanged;
        }
        self.last_payload.insert(id.to_owned(), payload.to_vec());
        self.entries
            .insert(id.to_owned(), Self::decode(id, payload));
        StoreApplied::Changed
    }

    /// `Flow::decode` requires `id`, but a flow published without one means "the topic's id".
    /// Returns the payload with `id` inserted when it is a JSON object lacking one (or null),
    /// `None` when nothing needs changing (including when it is not an object at all, which
    /// `Flow::decode` then reports).
    fn fill_missing_id(id: &str, payload: &[u8]) -> Option<Vec<u8>> {
        let mut value: serde_json::Value = serde_json::from_slice(payload).ok()?;
        let object = value.as_object_mut()?;
        if object.get("id").is_some_and(|v| !v.is_null()) {
            return None;
        }
        object.insert("id".to_owned(), serde_json::Value::String(id.to_owned()));
        serde_json::to_vec(&value).ok()
    }

    fn decode(id: &str, payload: &[u8]) -> Entry {
        let filled = Self::fill_missing_id(id, payload);
        let mut flow = match Flow::decode(filled.as_deref().unwrap_or(payload)) {
            Ok(flow) => flow,
            Err(reason) => return Self::invalid(payload, reason),
        };
        if flow.id.is_empty() {
            flow.id = id.to_owned();
        } else if flow.id != id {
            return Self::invalid(
                payload,
                format!("id mismatch: payload says `{}`, topic says `{id}`", flow.id),
            );
        }
        match validate(&flow) {
            Ok(()) => Entry::Valid(flow),
            Err(err) => Self::invalid(payload, err.to_string()),
        }
    }

    /// An invalid entry that still remembers the payload's `name` and `area` when they are
    /// JSON strings, so the list can show and place the stub.
    fn invalid(payload: &[u8], reason: String) -> Entry {
        let value: Option<serde_json::Value> = serde_json::from_slice(payload).ok();
        let text = |key: &str| {
            value
                .as_ref()
                .and_then(|v| v.get(key))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };
        Entry::Invalid {
            reason,
            name: text("name"),
            area: text("area"),
        }
    }

    /// The valid definitions (enabled or not), sorted by id.
    pub fn flows(&self) -> Vec<Flow> {
        self.entries
            .values()
            .filter_map(|entry| match entry {
                Entry::Valid(flow) => Some(flow.clone()),
                Entry::Invalid { .. } => None,
            })
            .collect()
    }

    /// Whether the store holds this id, valid or invalid.
    pub fn exists(&self, id: &str) -> bool {
        self.entries.contains_key(id)
    }

    /// One flow with its runs, or `None` for an unknown id.
    pub fn get(&self, id: &str) -> Option<FlowView> {
        let entry = self.entries.get(id)?;
        let (definition, invalid) = match entry {
            Entry::Valid(flow) => (flow.clone(), None),
            Entry::Invalid { reason, name, area } => (
                Flow {
                    id: id.to_owned(),
                    name: name.clone().unwrap_or_else(|| id.to_owned()),
                    area: area.clone(),
                    ..Default::default()
                },
                Some(reason.clone()),
            ),
        };
        Some(FlowView {
            definition,
            invalid,
            active_run: self.active.get(id).cloned(),
            runs: self
                .runs
                .get(id)
                .map(|runs| runs.iter().cloned().collect())
                .unwrap_or_default(),
        })
    }

    /// Every flow as a view, sorted by id.
    pub fn views(&self) -> Vec<FlowView> {
        self.entries.keys().filter_map(|id| self.get(id)).collect()
    }

    /// The id of the run in progress for this flow, if any.
    pub fn active_run(&self, flow_id: &str) -> Option<String> {
        self.active.get(flow_id).cloned()
    }

    fn is_valid(&self, flow_id: &str) -> bool {
        matches!(self.entries.get(flow_id), Some(Entry::Valid(_)))
    }

    fn push_run(&mut self, flow_id: &str, run: RunView) {
        let runs = self.runs.entry(flow_id.to_owned()).or_default();
        runs.push_front(run);
        // Never evict the run `active` names: it is still going and its steps and end must find
        // it. That can leave the deque one longer than `runs_kept` until the next run is pushed after it ends.
        let active = self.active.get(flow_id);
        while runs.len() > self.runs_kept {
            match runs.iter().rposition(|run| Some(&run.id) != active) {
                Some(index) => runs.remove(index),
                None => break,
            };
        }
    }

    fn next_run_id(&mut self) -> String {
        self.counter += 1;
        format!("r{}", self.counter)
    }

    /// Starts a `Running` run and makes it the active one. `None` when the flow is unknown or
    /// invalid (nothing can run it).
    pub fn begin_run(
        &mut self,
        flow_id: &str,
        trigger: Option<TriggerInfo>,
        at: u64,
    ) -> Option<RunView> {
        self.start_run(flow_id, trigger, false, at)
    }

    /// [`Self::begin_run`] for a run started by hand, marked `manual`.
    pub fn begin_manual_run(
        &mut self,
        flow_id: &str,
        trigger: Option<TriggerInfo>,
        at: u64,
    ) -> Option<RunView> {
        self.start_run(flow_id, trigger, true, at)
    }

    fn start_run(
        &mut self,
        flow_id: &str,
        trigger: Option<TriggerInfo>,
        manual: bool,
        at: u64,
    ) -> Option<RunView> {
        if !self.is_valid(flow_id) {
            return None;
        }
        let run = RunView {
            id: self.next_run_id(),
            flow_id: flow_id.to_owned(),
            status: RunStatus::Running,
            started_at: at,
            ended_at: None,
            trigger,
            manual,
            steps: Vec::new(),
        };
        self.active.insert(flow_id.to_owned(), run.id.clone());
        self.push_run(flow_id, run.clone());
        Some(run)
    }

    /// Records a trigger that fired while another run was active: an already-ended `Skipped`
    /// run with one skipped step on the trigger node. It does not touch the active run. `None`
    /// when the flow is unknown or invalid.
    pub fn add_skipped_run(
        &mut self,
        flow_id: &str,
        trigger_node: &str,
        trigger: Option<TriggerInfo>,
        at: u64,
    ) -> Option<RunView> {
        self.add_skipped_run_because(flow_id, trigger_node, trigger, at, "a run is active")
    }

    /// [`Self::add_skipped_run`] with the reason the trigger was dropped.
    pub fn add_skipped_run_because(
        &mut self,
        flow_id: &str,
        trigger_node: &str,
        trigger: Option<TriggerInfo>,
        at: u64,
        reason: &str,
    ) -> Option<RunView> {
        if !self.is_valid(flow_id) {
            return None;
        }
        let run = RunView {
            id: self.next_run_id(),
            flow_id: flow_id.to_owned(),
            status: RunStatus::Skipped,
            started_at: at,
            ended_at: Some(at),
            trigger,
            manual: false,
            steps: vec![StepView {
                node: trigger_node.to_owned(),
                at,
                outcome: StepOutcome::Skipped,
                detail: reason.to_owned(),
                ports: None,
            }],
        };
        self.push_run(flow_id, run.clone());
        Some(run)
    }

    fn run_mut(&mut self, flow_id: &str, run_id: &str) -> Option<&mut RunView> {
        self.runs
            .get_mut(flow_id)?
            .iter_mut()
            .find(|run| run.id == run_id)
    }

    /// Appends a step to a run and returns the updated run; `None` if the run is unknown
    /// (including one already truncated away).
    pub fn record_step(&mut self, flow_id: &str, run_id: &str, step: StepView) -> Option<RunView> {
        let run = self.run_mut(flow_id, run_id)?;
        run.steps.push(step);
        Some(run.clone())
    }

    /// Finishes a run with `status` and returns it; the flow's active run is cleared only when
    /// it is this one. `None` if the run is unknown.
    pub fn end_run(
        &mut self,
        flow_id: &str,
        run_id: &str,
        status: RunStatus,
        at: u64,
    ) -> Option<RunView> {
        // Clear the active slot first, whether or not the run is still in the history, so a
        // lost run can never leave the flow stuck "active".
        if self
            .active
            .get(flow_id)
            .is_some_and(|active| active == run_id)
        {
            self.active.remove(flow_id);
        }
        let run = self.run_mut(flow_id, run_id)?;
        run.status = status;
        run.ended_at = Some(at);
        Some(run.clone())
    }
}

#[cfg(test)]
mod tests {
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
}
