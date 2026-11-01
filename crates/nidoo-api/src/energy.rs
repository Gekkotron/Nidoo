//! Entity views → the slice of an entity the energy maths need, shared by the hub (grid day
//! counters) and the web (the Energy tab).

use nidoo_core::energy::{ConsumerIcon, EnergyEntity, EnergyRole};
use nidoo_core::model::{Availability, StateValue};

use crate::EntityView;

/// The slice of an entity the energy maths need.
pub fn energy_entity(e: &EntityView) -> EnergyEntity {
    EnergyEntity {
        id: e.id.clone(),
        device_id: e.device_id.clone(),
        name: e.name.clone(),
        label: e.label.clone(),
        unit: e.unit.clone(),
        value: match e.state.value {
            StateValue::Number(n) => Some(n),
            _ => None,
        },
        available: e.state.availability != Availability::Offline,
        role: e.ui.energy_role.as_deref().and_then(EnergyRole::parse),
        invert: e.ui.energy_invert,
        order: e.ui.order,
        icon: e.ui.energy_icon.as_deref().and_then(ConsumerIcon::parse),
        meter: e.ui.energy_meter.clone(),
    }
}

/// Every view but the hub's groups, as energy entities.
pub fn energy_entities<'a>(views: impl IntoIterator<Item = &'a EntityView>) -> Vec<EnergyEntity> {
    views
        .into_iter()
        .filter(|e| !e.id.starts_with("group_"))
        .map(energy_entity)
        .collect()
}
