//! The whole house at one instant, sent to a client on connect.

use std::collections::BTreeMap;

use nidoo_core::model::{EntityState, Kind};
use nidoo_core::registry::Registry;
use nidoo_core::types::Placement;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::alerts::AlertView;
use crate::events::ConnectionStatus;
use crate::flows::FlowView;
use crate::views::{
    AreaView, BridgeView, DeviceView, EntityView, IgnoredView, is_primary, placement_of,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Snapshot {
    pub connection: ConnectionStatus,
    pub bridge: BridgeView,
    pub areas: Vec<AreaView>,
    pub devices: Vec<DeviceView>,
    pub entities: Vec<EntityView>,
    pub ignored: Vec<IgnoredView>,
    /// The push alerts (newest first); empty until the hub reads the history topic.
    #[serde(default)]
    pub alerts: Vec<AlertView>,
    /// The flows with their recent runs; empty until the hub fills it.
    #[serde(default)]
    pub flows: Vec<FlowView>,
    pub generated_at: u64,
}

/// Builds the snapshot of `reg`, stamped `at` (unix seconds).
pub fn snapshot(reg: &Registry, connection: ConnectionStatus, at: u64) -> Snapshot {
    Snapshot {
        connection,
        bridge: reg.bridge().into(),
        areas: area_views(reg),
        devices: device_views(reg),
        entities: entity_views(reg),
        ignored: reg.ignored().iter().map(IgnoredView::from).collect(),
        alerts: Vec::new(),
        flows: Vec::new(),
        generated_at: at,
    }
}

/// Every entity of `reg` with its state, by unique id.
pub fn entity_views(reg: &Registry) -> Vec<EntityView> {
    let no_state = EntityState::default();
    reg.entities()
        .map(|e| EntityView::from(e, reg.state(&e.unique_id).unwrap_or(&no_state)))
        .collect()
}

/// The effective areas of `reg`, each with its entity count and its primary entity count.
pub fn area_views(reg: &Registry) -> Vec<AreaView> {
    let mut per_area: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for entity in reg.entities() {
        let (entities, primary) = per_area.entry(entity.area.as_str()).or_default();
        *entities += 1;
        // A generic entity placed as a signal, target or update is not a Home card.
        let card = entity.kind != Kind::Generic
            || placement_of(&entity.component).is_none_or(|p| p == Placement::Card);
        if card
            && is_primary(
                entity.category,
                entity.enabled_by_default,
                entity.part_of.is_some(),
                entity.ui.hidden,
            )
        {
            *primary += 1;
        }
    }
    reg.effective_areas()
        .iter()
        .map(|a| {
            let (entities, primary) = per_area.get(a.id.as_str()).copied().unwrap_or((0, 0));
            AreaView::new(a, entities, primary)
        })
        .collect()
}

/// The devices of `reg`, each with what its entities report.
pub fn device_views(reg: &Registry) -> Vec<DeviceView> {
    let no_state = EntityState::default();
    reg.devices()
        .map(|device| {
            let members: Vec<_> = reg
                .entities()
                .filter(|e| e.device_id.as_deref() == Some(device.id.as_str()))
                .map(|e| (e, reg.state(&e.unique_id).unwrap_or(&no_state)))
                .collect();
            DeviceView::new(device, &members)
        })
        .collect()
}
