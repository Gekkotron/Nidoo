//! The pure client-side model of the house: what the hub last told us.

use std::collections::BTreeMap;

use nidoo_api::{
    AlertView, AreaView, BridgeView, ConnectionStatus, DeviceView, EntityView, FlowView,
    IgnoredView, RunStatus, RunView, Snapshot, WsEvent,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HouseState {
    pub connection: ConnectionStatus,
    pub bridge: BridgeView,
    pub areas: Vec<AreaView>,
    pub devices: Vec<DeviceView>,
    pub entities: BTreeMap<String, EntityView>,
    pub ignored: Vec<IgnoredView>,
    /// The push alerts, newest first, as the hub sent them.
    pub alerts: Vec<AlertView>,
    /// The flows with their recent runs, as the hub sent them.
    pub flows: Vec<FlowView>,
    pub generated_at: u64,
}

impl HouseState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_snapshot(s: Snapshot) -> Self {
        Self {
            connection: s.connection,
            bridge: s.bridge,
            areas: s.areas,
            devices: s.devices,
            entities: s.entities.into_iter().map(|e| (e.id.clone(), e)).collect(),
            ignored: s.ignored,
            alerts: s.alerts,
            flows: s.flows,
            generated_at: s.generated_at,
        }
    }

    /// The entities folded into `parent_id` through `part_of`.
    pub fn part_of_children(&self, parent_id: &str) -> Vec<EntityView> {
        self.entities
            .values()
            .filter(|e| e.part_of.as_deref() == Some(parent_id))
            .cloned()
            .collect()
    }

    /// A snapshot replaces everything; every other event patches one field.
    pub fn apply(&mut self, event: WsEvent) {
        match event {
            WsEvent::Snapshot(s) => *self = Self::from_snapshot(s),
            WsEvent::EntityUpserted(view) => {
                self.entities.insert(view.id.clone(), view);
            }
            WsEvent::EntityRemoved { id } => {
                self.entities.remove(&id);
            }
            WsEvent::StateChanged { id, state } => {
                if let Some(entity) = self.entities.get_mut(&id) {
                    entity.state = state;
                }
            }
            WsEvent::AreasChanged { areas } => self.areas = areas,
            WsEvent::DevicesChanged { devices } => self.devices = devices,
            WsEvent::BridgeChanged(bridge) => self.bridge = bridge,
            WsEvent::ConnectionChanged(connection) => self.connection = connection,
            WsEvent::AlertsChanged { alerts } => self.alerts = alerts,
            WsEvent::FlowsChanged { flows } => self.flows = flows,
            WsEvent::FlowRunChanged { flow_id, run } => upsert_run(&mut self.flows, &flow_id, run),
            WsEvent::CommandFailed { .. } => {}
            WsEvent::ButtonPressed { .. } | WsEvent::SignalReceived { .. } => {}
            WsEvent::ClimateChanged { .. } | WsEvent::AlarmChanged => {}
            // The weather and home settings live in `House` only; the header reads them from there.
            WsEvent::WeatherChanged
            | WsEvent::HomeSettingsChanged { .. }
            | WsEvent::FlowLabelsChanged { .. } => {}
            WsEvent::AmbiancesChanged { .. } | WsEvent::AmbianceApplied { .. } => {}
            // The solar forecast lives in `House` only; the card refetches on this event.
            WsEvent::SolarChanged => {}
            // The Assistant page refetches its turns on this event.
            WsEvent::AssistTurnAdded { .. } => {}
        }
    }
}

/// How many runs a flow keeps in the client, matching what the hub sends.
const RUN_HISTORY: usize = 20;

/// Folds one run event into `flows`: the run with the same id is replaced, else it goes in
/// front; the history is capped; `active_run` follows a running run and clears when the active
/// one ends. A flow that is not listed is ignored (the next `FlowsChanged` will carry it).
pub fn upsert_run(flows: &mut [FlowView], flow_id: &str, run: RunView) {
    let Some(flow) = flows.iter_mut().find(|f| f.definition.id == flow_id) else {
        return;
    };
    let held = flow.runs.iter().find(|r| r.id == run.id);
    // A late `Running` broadcast must not re-activate a run the house already saw end.
    if run.status == RunStatus::Running && held.is_some_and(|r| r.status != RunStatus::Running) {
        return;
    }
    if run.status == RunStatus::Running {
        flow.active_run = Some(run.id.clone());
    } else if flow.active_run.as_deref() == Some(run.id.as_str()) {
        flow.active_run = None;
    }
    match flow.runs.iter().position(|r| r.id == run.id) {
        Some(i) => flow.runs[i] = run,
        None => flow.runs.insert(0, run),
    }
    flow.runs.truncate(RUN_HISTORY);
}

#[cfg(test)]
mod tests;
