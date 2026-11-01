//! Read-only views of the registry's entities, devices, areas, bridge and ignored topics.

use std::collections::{BTreeMap, BTreeSet};

use nidoo_core::model::{
    Area, Availability, BridgeInfo, Capabilities, Device, Entity, EntityCategory, EntityState,
    GenericValue, Kind, StateValue, UiOverride,
};
use nidoo_core::registry::Ignored;
use nidoo_core::types::{Placement, type_def};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// An entity as the UI shows it, with its current state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EntityView {
    pub id: String,
    pub device_id: Option<String>,
    /// The effective name: the override's when set and not blank, else the discovered one.
    pub name: String,
    pub device_name: Option<String>,
    pub label: Option<String>,
    pub kind: Kind,
    pub component: String,
    pub protocol_kind: Kind,
    pub device_class: Option<String>,
    pub unit: Option<String>,
    pub icon: Option<String>,
    pub category: EntityCategory,
    pub enabled_by_default: bool,
    pub area: String,
    pub part_of: Option<String>,
    pub capabilities: Capabilities,
    pub ui: UiOverride,
    pub state: EntityState,
}

impl EntityView {
    /// True when the UI shows this entity by default: primary category, enabled by default,
    /// not folded into another entity, not hidden by its override.
    pub fn is_primary(&self) -> bool {
        is_primary(
            self.category,
            self.enabled_by_default,
            self.part_of.is_some(),
            self.ui.hidden,
        )
    }

    pub fn from(entity: &Entity, state: &EntityState) -> Self {
        let mut masked = state.clone();
        mask_secret(entity, &mut masked);
        let name = match entity.ui.name.as_deref() {
            Some(name) if !name.trim().is_empty() => name.to_owned(),
            _ => entity.name.clone(),
        };
        Self {
            id: entity.unique_id.clone(),
            device_id: entity.device_id.clone(),
            name,
            device_name: entity.device_name.clone(),
            label: entity.label.clone(),
            kind: entity.kind,
            component: entity.component.clone(),
            protocol_kind: entity.protocol_kind,
            device_class: entity.device_class.clone(),
            unit: entity.unit.clone(),
            icon: entity.icon.clone(),
            category: entity.category,
            enabled_by_default: entity.enabled_by_default,
            area: entity.area.clone(),
            part_of: entity.part_of.clone(),
            capabilities: entity.capabilities.clone(),
            ui: entity.ui.clone(),
            state: masked,
        }
    }
}

/// What a secret entity shows in place of its value.
const MASK: &str = "\u{2022}\u{2022}\u{2022}\u{2022}";

/// Blanks every field of a secret entity's generic state (a password-mode text), for any
/// copy of the state that leaves the hub. A no-op for every other entity.
pub fn mask_secret(entity: &Entity, state: &mut EntityState) {
    if !entity
        .capabilities
        .generic
        .as_ref()
        .is_some_and(|g| g.secret)
    {
        return;
    }
    if let StateValue::Generic(gs) = &mut state.value {
        for value in gs.fields.values_mut() {
            *value = GenericValue::Text(MASK.to_owned());
        }
    }
}

/// A Zigbee device with what its diagnostic entities report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceView {
    pub id: String,
    pub ieee: String,
    pub friendly_name: String,
    pub model: Option<String>,
    pub vendor: Option<String>,
    pub description: Option<String>,
    pub supported: bool,
    pub interview_state: String,
    pub power_source: Option<String>,
    pub entity_ids: Vec<String>,
    pub availability: Availability,
    pub battery: Option<u8>,
    pub linkquality: Option<u32>,
    /// Firmware versions, from the device's `update` entity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<UpdateView>,
}

/// What a device's `update` entity reports about its firmware.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UpdateView {
    pub installed: Option<String>,
    pub latest: Option<String>,
    /// Both versions are known and differ.
    pub available: bool,
}

impl UpdateView {
    fn from_state(state: &EntityState) -> Self {
        let text = |key: &str| match &state.value {
            StateValue::Generic(g) => match g.fields.get(key) {
                Some(GenericValue::Text(t)) => Some(t.clone()),
                _ => None,
            },
            _ => None,
        };
        let installed = text("installed");
        let latest = text("latest");
        let available = installed.is_some() && latest.is_some() && installed != latest;
        Self {
            installed,
            latest,
            available,
        }
    }
}

impl DeviceView {
    /// `members` are the device's entities with their states.
    pub fn new(device: &Device, members: &[(&Entity, &EntityState)]) -> Self {
        let any = |wanted: Availability| members.iter().any(|(_, s)| s.availability == wanted);
        let availability = if any(Availability::Online) {
            Availability::Online
        } else if any(Availability::Offline) {
            Availability::Offline
        } else {
            Availability::Unknown
        };
        let number = |suffix: &str| {
            members
                .iter()
                .find_map(|(entity, state)| match state.value {
                    StateValue::Number(n) if entity.unique_id.ends_with(suffix) => Some(n),
                    _ => None,
                })
        };
        // A device can carry more than one `update` entity: the first that reports a version wins.
        let mut updates = members
            .iter()
            .filter(|(entity, _)| placement_of(&entity.component) == Some(Placement::Update))
            .map(|(_, state)| UpdateView::from_state(state))
            .collect::<Vec<_>>();
        let known = updates
            .iter()
            .position(|u| u.installed.is_some() || u.latest.is_some())
            .unwrap_or(0);
        let update = (known < updates.len()).then(|| updates.swap_remove(known));
        Self {
            id: device.id.clone(),
            ieee: device.ieee.clone(),
            friendly_name: device.friendly_name.clone(),
            model: device.model.clone(),
            vendor: device.vendor.clone(),
            description: device.description.clone(),
            supported: device.supported,
            interview_state: device.interview_state.clone(),
            power_source: device.power_source.clone(),
            entity_ids: members.iter().map(|(e, _)| e.unique_id.clone()).collect(),
            availability,
            // `as` saturates, so the clamp only needs to bound the top.
            battery: number("_battery_zigbee2mqtt").map(|n| n.clamp(0.0, 100.0) as u8),
            linkquality: number("_linkquality_zigbee2mqtt").map(|n| n as u32),
            update,
        }
    }
}

/// A room with the number of entities in it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AreaView {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub order: Option<i32>,
    /// Every entity whose effective area this is, diagnostics and hidden ones included.
    pub entity_count: usize,
    /// The entities the UI shows by default (see [`EntityView::is_primary`]); what a room
    /// chip or card count should display.
    pub primary_count: usize,
    /// Custom room group card names by kind key (`cover`, `light`).
    #[serde(default)]
    pub group_names: BTreeMap<String, String>,
    /// Kind keys whose room group shows in the Favorites chip on Home.
    #[serde(default)]
    pub group_favorites: BTreeSet<String>,
}

impl AreaView {
    pub fn new(area: &Area, entity_count: usize, primary_count: usize) -> Self {
        Self {
            id: area.id.clone(),
            name: area.name.clone(),
            icon: area.icon.clone(),
            order: area.order,
            entity_count,
            primary_count,
            group_names: area.group_names.clone(),
            group_favorites: area.group_favorites.clone(),
        }
    }
}

/// Where the type table places a generic component; `None` for a component it does not list.
pub(crate) fn placement_of(component: &str) -> Option<Placement> {
    type_def(component).map(|def| def.placement)
}

/// The primary rule shared by [`EntityView::is_primary`] and the area counts.
pub(crate) fn is_primary(
    category: EntityCategory,
    enabled_by_default: bool,
    folded: bool,
    hidden: bool,
) -> bool {
    category == EntityCategory::Primary && enabled_by_default && !folded && !hidden
}

/// The Zigbee2MQTT bridge.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BridgeView {
    pub version: Option<String>,
    pub coordinator: Option<String>,
    pub permit_join: bool,
    pub state: Availability,
}

impl From<&BridgeInfo> for BridgeView {
    fn from(bridge: &BridgeInfo) -> Self {
        Self {
            version: bridge.version.clone(),
            coordinator: bridge.coordinator.clone(),
            permit_join: bridge.permit_join,
            state: bridge.state,
        }
    }
}

/// A discovery topic that did not become an entity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IgnoredView {
    pub topic: String,
    pub reason: String,
}

impl From<&Ignored> for IgnoredView {
    fn from(ignored: &Ignored) -> Self {
        Self {
            topic: ignored.topic.clone(),
            reason: ignored.reason.clone(),
        }
    }
}
