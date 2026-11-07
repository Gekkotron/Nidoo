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
            WsEvent::WeatherChanged | WsEvent::HomeSettingsChanged { .. } => {}
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
mod tests {
    use super::*;
    use nidoo_api::{AlertChannel, AlertSeverity, AlertView, AreaView, DeviceView};
    use nidoo_core::model::{
        Availability, Capabilities, EntityCategory, EntityState, Kind, StateValue, UiOverride,
    };

    fn entity(id: &str) -> EntityView {
        EntityView {
            id: id.into(),
            device_id: None,
            name: id.into(),
            device_name: None,
            label: None,
            kind: Kind::Light,
            component: "light".to_string(),
            protocol_kind: Kind::Light,
            device_class: None,
            unit: None,
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

    fn area(id: &str) -> AreaView {
        AreaView {
            id: id.into(),
            name: id.into(),
            icon: None,
            order: None,
            entity_count: 0,
            primary_count: 0,
            group_names: Default::default(),
            group_favorites: Default::default(),
        }
    }

    fn device(id: &str) -> DeviceView {
        DeviceView {
            id: id.into(),
            ieee: "0x1".into(),
            friendly_name: id.into(),
            model: None,
            vendor: None,
            description: None,
            supported: true,
            interview_state: "SUCCESSFUL".into(),
            power_source: None,
            entity_ids: vec![],
            availability: Availability::Online,
            battery: None,
            linkquality: None,
            update: None,
        }
    }

    fn snapshot(ids: &[&str]) -> Snapshot {
        Snapshot {
            connection: ConnectionStatus::Online,
            bridge: BridgeView {
                version: Some("2.0".into()),
                coordinator: None,
                permit_join: true,
                state: Availability::Online,
            },
            areas: vec![area("salon")],
            devices: vec![device("d1")],
            entities: ids.iter().map(|id| entity(id)).collect(),
            ignored: vec![IgnoredView {
                topic: "t".into(),
                reason: "r".into(),
            }],
            alerts: vec![],
            flows: vec![],
            generated_at: 42,
        }
    }

    fn state_with(ids: &[&str]) -> HouseState {
        HouseState::from_snapshot(snapshot(ids))
    }

    #[test]
    fn snapshot_replaces_the_cache() {
        let mut s = state_with(&["old"]);
        s.apply(WsEvent::Snapshot(snapshot(&["new"])));
        assert!(!s.entities.contains_key("old"));
        assert!(s.entities.contains_key("new"));
    }

    #[test]
    fn entity_upserted_patches_one_entity() {
        let mut s = state_with(&["a"]);
        s.apply(WsEvent::EntityUpserted(entity("b")));
        assert_eq!(s.entities.len(), 2);
        assert_eq!(s.entities["b"], entity("b"));
    }

    #[test]
    fn state_changed_updates_only_that_entity_state() {
        let mut s = state_with(&["a", "b"]);
        let mut attributes = serde_json::Map::new();
        attributes.insert("brightness".into(), serde_json::json!(10));
        let next = EntityState {
            value: StateValue::Number(3.0),
            attributes,
            availability: Availability::Offline,
            last_update: Some(99),
        };
        s.apply(WsEvent::StateChanged {
            id: "a".into(),
            state: next.clone(),
        });
        assert_eq!(s.entities["a"].state, next);
        assert_eq!(s.entities["b"].state, EntityState::default());
        assert_eq!(s.entities["a"].name, "a");
    }

    #[test]
    fn areas_changed_replaces_the_chip_list() {
        let mut s = state_with(&[]);
        s.apply(WsEvent::AreasChanged {
            areas: vec![area("b"), area("a")],
        });
        let ids: Vec<_> = s.areas.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["b", "a"]);
    }

    #[test]
    fn devices_changed_replaces_the_device_list() {
        let mut s = state_with(&[]);
        s.apply(WsEvent::DevicesChanged {
            devices: vec![device("x")],
        });
        assert_eq!(s.devices, vec![device("x")]);
    }

    #[test]
    fn bridge_changed_updates_bridge_only() {
        let mut s = state_with(&["a"]);
        let before = s.clone();
        let bridge = BridgeView {
            version: None,
            coordinator: Some("c".into()),
            permit_join: false,
            state: Availability::Offline,
        };
        s.apply(WsEvent::BridgeChanged(bridge.clone()));
        assert_eq!(s.bridge, bridge);
        assert_eq!(
            HouseState {
                bridge: before.bridge.clone(),
                ..s
            },
            before
        );
    }

    #[test]
    fn connection_changed_updates_connection_only() {
        let mut s = state_with(&["a"]);
        let before = s.clone();
        let offline = ConnectionStatus::Offline {
            reason: "down".into(),
        };
        s.apply(WsEvent::ConnectionChanged(offline.clone()));
        assert_eq!(s.connection, offline);
        assert_eq!(
            HouseState {
                connection: before.connection.clone(),
                ..s
            },
            before
        );
    }

    #[test]
    fn command_failed_is_a_noop_on_state() {
        let mut s = state_with(&["a"]);
        let before = s.clone();
        s.apply(WsEvent::CommandFailed {
            id: "a".into(),
            reason: "nope".into(),
        });
        assert_eq!(s, before);
    }

    #[test]
    fn alerts_changed_replaces_the_list() {
        let alert = |id: &str| AlertView {
            id: id.into(),
            channel: AlertChannel::Camera,
            severity: AlertSeverity::Info,
            title: id.into(),
            body: String::new(),
            image_url: None,
            video_url: None,
            entity_id: None,
            camera: None,
            actions: vec![],
            source: None,
            silent: false,
            resolved: false,
            resolved_at: None,
            sent_at: 1,
        };
        let mut s = state_with(&[]);
        s.apply(WsEvent::AlertsChanged {
            alerts: vec![alert("b"), alert("a")],
        });
        let ids: Vec<_> = s.alerts.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["b", "a"]);
        s.apply(WsEvent::Snapshot(snapshot(&[])));
        assert!(s.alerts.is_empty());
    }

    fn flow_view(id: &str) -> nidoo_api::FlowView {
        nidoo_api::FlowView {
            definition: nidoo_core::flow::Flow {
                id: id.into(),
                name: id.into(),
                ..Default::default()
            },
            invalid: None,
            active_run: None,
            runs: vec![],
        }
    }

    fn run(id: &str, status: nidoo_api::RunStatus) -> nidoo_api::RunView {
        nidoo_api::RunView {
            id: id.into(),
            flow_id: "f".into(),
            status,
            started_at: 1,
            ended_at: None,
            trigger: None,
            manual: false,
            steps: vec![],
        }
    }

    #[test]
    fn house_applies_run_events() {
        use nidoo_api::RunStatus::{Done, Running};
        let mut s = state_with(&[]);
        s.apply(WsEvent::FlowsChanged {
            flows: vec![flow_view("f")],
        });
        // A run for an unknown flow is ignored.
        s.apply(WsEvent::FlowRunChanged {
            flow_id: "ghost".into(),
            run: run("x", Running),
        });
        assert_eq!(s.flows.len(), 1);
        s.apply(WsEvent::FlowRunChanged {
            flow_id: "f".into(),
            run: run("r1", Running),
        });
        assert_eq!(s.flows[0].active_run.as_deref(), Some("r1"));
        assert_eq!(s.flows[0].runs.len(), 1);
        s.apply(WsEvent::FlowRunChanged {
            flow_id: "f".into(),
            run: run("r2", Running),
        });
        let ids: Vec<_> = s.flows[0].runs.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["r2", "r1"]);
        // Finishing r1 replaces it in place and leaves r2 active.
        s.apply(WsEvent::FlowRunChanged {
            flow_id: "f".into(),
            run: run("r1", Done),
        });
        assert_eq!(s.flows[0].runs[1].status, Done);
        assert_eq!(s.flows[0].active_run.as_deref(), Some("r2"));
        s.apply(WsEvent::FlowRunChanged {
            flow_id: "f".into(),
            run: run("r2", Done),
        });
        assert_eq!(s.flows[0].active_run, None);
        // A late Running event for a finished run changes nothing.
        s.apply(WsEvent::FlowRunChanged {
            flow_id: "f".into(),
            run: run("r2", Running),
        });
        assert_eq!(s.flows[0].active_run, None);
        assert_eq!(s.flows[0].runs[0].status, Done);
        // The history is capped at 20.
        for i in 0..25 {
            s.apply(WsEvent::FlowRunChanged {
                flow_id: "f".into(),
                run: run(&format!("n{i}"), Done),
            });
        }
        assert_eq!(s.flows[0].runs.len(), 20);
        assert_eq!(s.flows[0].runs[0].id, "n24");
    }

    #[test]
    fn entity_removed_drops_the_entry() {
        let mut s = state_with(&["a", "b"]);
        s.apply(WsEvent::EntityRemoved { id: "a".into() });
        assert!(!s.entities.contains_key("a"));
        assert!(s.entities.contains_key("b"));
    }

    #[test]
    fn from_snapshot_populates_every_field() {
        let s = state_with(&["a"]);
        assert_eq!(s.connection, ConnectionStatus::Online);
        assert_eq!(s.bridge.version.as_deref(), Some("2.0"));
        assert_eq!(s.areas.len(), 1);
        assert_eq!(s.devices.len(), 1);
        assert_eq!(s.entities.len(), 1);
        assert_eq!(s.ignored.len(), 1);
        assert_eq!(s.generated_at, 42);
    }

    #[test]
    fn snapshot_after_lagging_replaces_stale_entries() {
        let mut s = state_with(&["a", "stale"]);
        s.apply(WsEvent::Snapshot(snapshot(&["a"])));
        assert!(!s.entities.contains_key("stale"));
        assert_eq!(s.entities.len(), 1);
    }

    #[test]
    fn part_of_children_returns_folded_entities() {
        let mut state = HouseState::new();
        state.apply(WsEvent::EntityUpserted(entity("cover.hall")));
        let mut child = entity("cover.hall_moving");
        child.part_of = Some("cover.hall".into());
        state.apply(WsEvent::EntityUpserted(child));
        assert_eq!(state.part_of_children("cover.hall").len(), 1);
        assert_eq!(
            state.part_of_children("cover.hall")[0].id,
            "cover.hall_moving"
        );
        assert!(state.part_of_children("cover.hall_moving").is_empty());
    }
}
