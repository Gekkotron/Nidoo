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
        /// The string entries of the payload's `labels` array, when it was readable.
        labels: Vec<String>,
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
    /// JSON strings (and the strings of its `labels` array), so the list can show and place the
    /// stub.
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
            labels: value
                .as_ref()
                .and_then(|v| v.get("labels"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
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
            Entry::Invalid {
                reason,
                name,
                area,
                labels,
            } => (
                Flow {
                    id: id.to_owned(),
                    name: name.clone().unwrap_or_else(|| id.to_owned()),
                    area: area.clone(),
                    labels: labels.clone(),
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
mod tests;
