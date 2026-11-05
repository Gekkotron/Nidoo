//! The registry: the pure state machine behind the hub. It takes broker messages classified
//! by [`Registry::route`], applies them with [`Registry::apply`], and exposes the effective
//! entities, their states, the devices, the areas and the bridge status.
//!
//! No I/O, no clock, no logging: the caller passes `at` with every state message and reports
//! what [`Registry::template_failures`] holds.

mod areas;
mod folding;
mod ignored;
mod remote;
mod routing;

use std::collections::{BTreeMap, BTreeSet};

use crate::discovery::DiscoveryTopic;
use crate::model::{
    Area, AreasDocument, BridgeInfo, Device, Entity, EntityState, GenericState, GenericValue, Kind,
    StateValue, SwitchType, UNASSIGNED_AREA, UiOverride,
};
use crate::state::{Applied, StateError, apply_parts};
use crate::template::Templates;

pub use ignored::Ignored;

/// Where the registry listens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryConfig {
    /// Home Assistant discovery prefixes (`nidoo`, `homeassistant`).
    pub discovery_prefixes: Vec<String>,
    /// The sub-tree holding overrides (`<ui_prefix>/<unique_id>`) and areas (`<ui_prefix>/_areas`).
    pub ui_prefix: String,
    /// The Zigbee2MQTT base topic (`zigbee2mqtt`), for `bridge/{devices,info,state}`.
    pub z2m_base: String,
    /// Sub-trees directly under each discovery prefix that are never discovery.
    pub reserved: Vec<String>,
}

impl RegistryConfig {
    /// The reserved sub-trees under a discovery prefix: `ui`, `automation`, `virtual`,
    /// `push` (the hub's notification topics, `nidoo/push/...`) and `weather` (the hub's weather
    /// states and availability), `solar` (the hub's solar forecast states, availability and cache)
    /// and `network` (the hub's network presence definitions, states and availability).
    pub fn default_reserved() -> Vec<String> {
        vec![
            "ui".to_owned(),
            "automation".to_owned(),
            "virtual".to_owned(),
            "push".to_owned(),
            "weather".to_owned(),
            "solar".to_owned(),
            "network".to_owned(),
        ]
    }
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            discovery_prefixes: vec!["nidoo".to_owned()],
            ui_prefix: "nidoo/ui".to_owned(),
            z2m_base: "zigbee2mqtt".to_owned(),
            reserved: Self::default_reserved(),
        }
    }
}

/// A broker message, classified for [`Registry::apply`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inbound {
    Discovery(DiscoveryTopic, Vec<u8>),
    Override {
        unique_id: String,
        payload: Vec<u8>,
    },
    Areas(Vec<u8>),
    /// A message the hub itself publishes (the `_types` table): nothing to apply.
    Ignored,
    Z2mDevices(Vec<u8>),
    Z2mBridgeInfo(Vec<u8>),
    Z2mBridgeState(Vec<u8>),
    State {
        topic: String,
        payload: Vec<u8>,
        at: u64,
    },
    Irrelevant,
}

/// What an [`Registry::apply`] changed, deduplicated and in this order.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Change {
    EntityUpserted(String),
    EntityRemoved(String),
    /// The entity's value, attributes or availability differ from before the message.
    StateChanged(String),
    /// The entity read something from the message (so `last_update` moved) but nothing
    /// differs: a device echoing its state, or a re-delivered report.
    StateReported(String),
    /// A message arrived on one of the entity's state topics but the entity read nothing from it
    /// and nothing differs: the value stayed unknown (a `null` field, a missing key, a template
    /// that renders nothing). The topic still spoke, which a liveness check cares about.
    StateTouched(String),
    AreasChanged,
    DevicesChanged,
    BridgeChanged,
    IgnoredChanged,
    SubscriptionsChanged,
}

/// The house as the registry knows it.
#[derive(Debug)]
pub struct Registry {
    config: RegistryConfig,
    templates: Templates,
    entities: BTreeMap<String, Entity>,
    states: BTreeMap<String, EntityState>,
    /// Overrides by unique id, kept even before the entity is discovered.
    overrides: BTreeMap<String, UiOverride>,
    areas: AreasDocument,
    devices: Vec<Device>,
    bridge: BridgeInfo,
    ignored: ignored::IgnoredSet,
    /// Subscribed topic → entities reading it, sorted.
    topic_index: BTreeMap<String, Vec<String>>,
    subscriptions: BTreeSet<String>,
    /// Last payload per discovery topic and per override id, to skip re-deliveries.
    last_discovery: BTreeMap<String, Vec<u8>>,
    last_override: BTreeMap<String, Vec<u8>>,
    /// Discovery topic → the entities it produced, and the reverse.
    produced: BTreeMap<String, Vec<String>>,
    origin: BTreeMap<String, String>,
    /// `device_automation` discovery topic → (remote entity id, action object id). The remote
    /// is not in `produced`/`origin`: many topics contribute to it.
    device_automation_actions: BTreeMap<String, (String, String)>,
    template_failures: BTreeSet<String>,
    /// Entities a message on one of their state topics reached since boot, whether it read a
    /// value or not: [`Self::restore_state`] leaves them alone.
    spoken: BTreeSet<String>,
    /// Last payload (and its `at`) per subscribed availability topic. The broker sends a
    /// retained message once per subscription, so an entity discovered while the topic is
    /// already subscribed would otherwise never hear it: [`Self::store`] replays it.
    last_availability: BTreeMap<String, (String, u64)>,
}

impl Registry {
    pub fn new(config: RegistryConfig) -> Self {
        Self {
            config,
            templates: Templates::new(),
            entities: BTreeMap::new(),
            states: BTreeMap::new(),
            overrides: BTreeMap::new(),
            areas: AreasDocument::default(),
            devices: Vec::new(),
            bridge: BridgeInfo::default(),
            ignored: ignored::IgnoredSet::default(),
            topic_index: BTreeMap::new(),
            subscriptions: BTreeSet::new(),
            last_discovery: BTreeMap::new(),
            last_override: BTreeMap::new(),
            produced: BTreeMap::new(),
            origin: BTreeMap::new(),
            device_automation_actions: BTreeMap::new(),
            template_failures: BTreeSet::new(),
            spoken: BTreeSet::new(),
            last_availability: BTreeMap::new(),
        }
    }

    pub fn config(&self) -> &RegistryConfig {
        &self.config
    }

    /// The template engine the registry renders with, for command mapping.
    pub fn templates(&self) -> &Templates {
        &self.templates
    }

    /// Classifies a message: `<ui_prefix>/_areas` → [`Inbound::Areas`], `<ui_prefix>/<id>` →
    /// [`Inbound::Override`], `<z2m>/bridge/{devices,info,state}`, a discovery topic, a topic
    /// in [`subscription_topics`](Self::subscription_topics) → [`Inbound::State`] with `at`,
    /// else [`Inbound::Irrelevant`].
    pub fn route(&self, topic: &str, payload: Vec<u8>, at: u64) -> Inbound {
        routing::route(self, topic, payload, at)
    }

    /// Applies one message and reports what changed. Nothing here panics on a hostile payload.
    pub fn apply(&mut self, inbound: Inbound) -> Vec<Change> {
        let mut changes = Vec::new();
        match inbound {
            Inbound::Discovery(topic, payload) => {
                let before = self.subscriptions.clone();
                routing::apply_discovery(self, &topic, payload, &mut changes);
                self.recompute(&mut changes);
                if self.subscriptions != before {
                    changes.push(Change::SubscriptionsChanged);
                }
            }
            Inbound::Override { unique_id, payload } => {
                self.apply_override(unique_id, payload, &mut changes);
            }
            Inbound::Areas(payload) => self.apply_areas(&payload, &mut changes),
            Inbound::Ignored => {}
            Inbound::Z2mDevices(payload) => {
                if let Ok(devices) = Device::parse_list(&payload)
                    && devices != self.devices
                {
                    self.devices = devices;
                    changes.push(Change::DevicesChanged);
                    let before = self.subscriptions.clone();
                    self.sync_exposed_actions(&mut changes);
                    self.recompute(&mut changes);
                    if self.subscriptions != before {
                        changes.push(Change::SubscriptionsChanged);
                    }
                }
            }
            Inbound::Z2mBridgeInfo(payload) => {
                let mut bridge = self.bridge.clone();
                bridge.apply_info(&payload);
                self.replace_bridge(bridge, &mut changes);
            }
            Inbound::Z2mBridgeState(payload) => self.apply_bridge_state(&payload, &mut changes),
            Inbound::State { topic, payload, at } => {
                if topic == format!("{}/bridge/state", self.config.z2m_base) {
                    self.apply_bridge_state(&payload, &mut changes);
                }
                self.apply_state(&topic, &payload, at, &mut changes);
            }
            Inbound::Irrelevant => {}
        }
        changes.sort();
        changes.dedup();
        changes
    }

    /// Records a signal (a remote press, an event, a tag scan) on a signal-placed entity: its
    /// `last` field becomes `signal` and its `at` field the time in milliseconds, so a repeated
    /// signal still changes the state. `at_ms` is kept strictly increasing per entity.
    pub fn record_signal(&mut self, unique_id: &str, signal: &str, at_ms: u64) -> Vec<Change> {
        let Some(state) = self.states.get_mut(unique_id) else {
            return Vec::new();
        };
        if !matches!(state.value, StateValue::Generic(_)) {
            state.value = StateValue::Generic(GenericState::default());
        }
        let StateValue::Generic(generic) = &mut state.value else {
            return Vec::new();
        };
        let previous = match generic.fields.get("at") {
            Some(GenericValue::Number(n)) => *n,
            _ => 0.0,
        };
        let at = (at_ms as f64).max(previous + 1.0);
        generic
            .fields
            .insert("last".to_owned(), GenericValue::Text(signal.to_owned()));
        generic
            .fields
            .insert("at".to_owned(), GenericValue::Number(at));
        state.last_update = Some(at_ms / 1000);
        vec![Change::StateChanged(unique_id.to_owned())]
    }

    /// Puts a recorded value back on an entity that has never spoken since boot: it exists,
    /// its value is [`StateValue::Unknown`] and no message reached its state topic. An
    /// availability message (Z2M's `bridge/state` at boot) stamps `last_update` but is not a
    /// report, so it does not count. `at_secs` is when the value was recorded, not now.
    /// Anything else (an unknown id, a value already read, a message that rendered nothing)
    /// is left alone. Availability is not touched.
    pub fn restore_state(
        &mut self,
        unique_id: &str,
        value: StateValue,
        at_secs: u64,
    ) -> Vec<Change> {
        if !self.entities.contains_key(unique_id) {
            return Vec::new();
        }
        let Some(state) = self.states.get_mut(unique_id) else {
            return Vec::new();
        };
        if state.value != StateValue::Unknown || self.spoken.contains(unique_id) {
            return Vec::new();
        }
        state.value = value;
        state.last_update = Some(at_secs);
        vec![Change::StateChanged(unique_id.to_owned())]
    }

    pub fn entity(&self, unique_id: &str) -> Option<&Entity> {
        self.entities.get(unique_id)
    }

    /// Every entity, by unique id.
    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values()
    }

    pub fn state(&self, unique_id: &str) -> Option<&EntityState> {
        self.states.get(unique_id)
    }

    pub fn devices(&self) -> impl Iterator<Item = &Device> {
        self.devices.iter()
    }

    pub fn device(&self, id: &str) -> Option<&Device> {
        self.devices.iter().find(|device| device.id == id)
    }

    /// The declared areas, as retained on `<ui_prefix>/_areas`.
    pub fn areas_document(&self) -> &AreasDocument {
        &self.areas
    }

    /// Declared areas by `order` then name, then the implicit ones carried by entities (named
    /// by their id), then `Unassigned` when any entity has it.
    pub fn effective_areas(&self) -> Vec<Area> {
        areas::effective_list(&self.areas.areas, self.entities.values())
    }

    /// The area id `raw` stands for: a declared id as is, a declared name (ignoring case) as
    /// its id, anything else as written (an implicit area). What a writer should store so the
    /// retained override and the effective area agree.
    pub fn resolve_area(&self, raw: &str) -> String {
        areas::resolve(raw, &self.areas.areas)
    }

    pub fn bridge(&self) -> &BridgeInfo {
        &self.bridge
    }

    /// Discovery topics that did not become entities, by topic.
    pub fn ignored(&self) -> &[Ignored] {
        self.ignored.as_slice()
    }

    /// Every topic some entity reads: the set to subscribe to.
    pub fn subscription_topics(&self) -> &BTreeSet<String> {
        &self.subscriptions
    }

    /// The override retained for `unique_id`, whether or not the entity exists.
    pub fn raw_override(&self, unique_id: &str) -> Option<&UiOverride> {
        self.overrides.get(unique_id)
    }

    /// The entities reading `topic`, sorted by unique id.
    pub fn entities_on_topic(&self, topic: &str) -> &[String] {
        self.topic_index.get(topic).map_or(&[], Vec::as_slice)
    }

    /// Entities whose template failed on their last message; their value is `Unknown` until a
    /// message maps again. The caller logs each id once.
    pub fn template_failures(&self) -> &BTreeSet<String> {
        &self.template_failures
    }

    fn apply_override(&mut self, unique_id: String, payload: Vec<u8>, changes: &mut Vec<Change>) {
        if self.last_override.get(&unique_id) == Some(&payload) {
            return;
        }
        let was_inverted = self.overrides.get(&unique_id).is_some_and(|ui| ui.invert);
        if payload.is_empty() {
            self.overrides.remove(&unique_id);
            self.last_override.remove(&unique_id);
        } else {
            let Some(ui) = UiOverride::decode(&payload) else {
                return;
            };
            self.overrides.insert(unique_id.clone(), ui);
            self.last_override.insert(unique_id.clone(), payload);
        }
        if self.overrides.get(&unique_id).is_some_and(|ui| ui.invert) != was_inverted {
            self.flip_switch(&unique_id, changes);
        }
        self.recompute(changes);
    }

    /// Flips a switch's stored on/off when its `invert` override changes, so the value reads
    /// right before the next report. Only here: a re-announce keeps the override, and a
    /// flip on every recompute would invert the value twice.
    fn flip_switch(&mut self, unique_id: &str, changes: &mut Vec<Change>) {
        let is_switch = self
            .entities
            .get(unique_id)
            .is_some_and(|e| e.protocol_kind == Kind::Switch);
        if let Some(state) = self.states.get_mut(unique_id)
            && is_switch
            && let StateValue::OnOff(on) = state.value
        {
            state.value = StateValue::OnOff(!on);
            changes.push(Change::StateChanged(unique_id.to_owned()));
        }
    }

    fn apply_areas(&mut self, payload: &[u8], changes: &mut Vec<Change>) {
        let document = if payload.is_empty() {
            AreasDocument::default()
        } else {
            let Some(document) = AreasDocument::decode(payload) else {
                return;
            };
            document
        };
        if document == self.areas {
            return;
        }
        self.areas = document;
        changes.push(Change::AreasChanged);
        self.recompute(changes);
    }

    fn apply_bridge_state(&mut self, payload: &[u8], changes: &mut Vec<Change>) {
        let mut bridge = self.bridge.clone();
        bridge.apply_state(payload);
        self.replace_bridge(bridge, changes);
    }

    fn replace_bridge(&mut self, bridge: BridgeInfo, changes: &mut Vec<Change>) {
        if bridge != self.bridge {
            self.bridge = bridge;
            changes.push(Change::BridgeChanged);
        }
    }

    /// Hands the message to every entity reading `topic`: [`Change::StateChanged`] when its
    /// value, attributes or availability differ afterwards, [`Change::StateReported`] when
    /// something was read but nothing differs, [`Change::StateTouched`] when nothing was read
    /// and nothing differs. A template failure leaves the value `Unknown`
    /// and lists the entity in `template_failures`; it is cleared once the kind mapper stores
    /// a value again (an availability update on the same entity does not clear it).
    fn apply_state(&mut self, topic: &str, payload: &[u8], at: u64, changes: &mut Vec<Change>) {
        let Ok(text) = std::str::from_utf8(payload) else {
            return;
        };
        let Some(ids) = self.topic_index.get(topic) else {
            return;
        };
        if ids.iter().any(|id| {
            self.entities
                .get(id)
                .is_some_and(|e| e.bindings.availability_topics().contains(topic))
        }) {
            self.last_availability
                .insert(topic.to_owned(), (text.to_owned(), at));
        }
        for id in ids {
            let (Some(entity), Some(state)) = (self.entities.get(id), self.states.get_mut(id))
            else {
                continue;
            };
            if !entity.bindings.is_availability_only(topic) {
                self.spoken.insert(id.clone());
            }
            let before = state.clone();
            let reported = match apply_parts(entity, state, topic, text, at, &self.templates) {
                Ok(parts) => {
                    if parts.value == Applied::Changed {
                        self.template_failures.remove(id);
                    }
                    parts.overall() == Applied::Changed
                }
                Err(StateError::Template(_)) => {
                    state.value = StateValue::Unknown;
                    self.template_failures.insert(id.clone());
                    false
                }
            };
            if differs(&before, state) {
                changes.push(Change::StateChanged(id.clone()));
            } else if reported {
                changes.push(Change::StateReported(id.clone()));
            } else {
                changes.push(Change::StateTouched(id.clone()));
            }
        }
    }

    /// Stores a freshly parsed entity produced by the discovery topic `origin`, replacing any
    /// previous one with the same unique id. Its state, if any, is kept; a re-parse that
    /// yields the same entity is not a change.
    fn upsert(&mut self, entity: Entity, origin: &str, changes: &mut Vec<Change>) {
        let id = entity.unique_id.clone();
        if let Some(previous_origin) = self.origin.insert(id.clone(), origin.to_owned())
            && previous_origin != origin
            && let Some(ids) = self.produced.get_mut(&previous_origin)
        {
            ids.retain(|other| *other != id);
        }
        if let Some(previous) = self.entities.get(&id)
            && as_discovered(previous) == entity
        {
            return;
        }
        self.store(entity, changes);
    }

    /// Indexes and stores `entity`, replacing any entity with the same id, and reports it.
    /// An availability topic it did not read before gets the word that topic last said.
    fn store(&mut self, entity: Entity, changes: &mut Vec<Change>) {
        let id = entity.unique_id.clone();
        let mut heard = BTreeSet::new();
        if let Some(previous) = self.entities.get(&id) {
            let previous = previous.clone();
            heard = previous.bindings.availability_topics();
            self.unindex(&id, &previous);
        }
        self.index(&id, &entity);
        self.forget_unread_availability();
        self.states.entry(id.clone()).or_default();
        self.entities.insert(id.clone(), entity);
        changes.push(Change::EntityUpserted(id.clone()));
        self.replay_availability(&id, &heard, changes);
    }

    /// Applies the remembered word of each availability-only topic of `id` not in `heard`.
    fn replay_availability(
        &mut self,
        id: &str,
        heard: &BTreeSet<String>,
        changes: &mut Vec<Change>,
    ) {
        let (Some(entity), Some(state)) = (self.entities.get(id), self.states.get_mut(id)) else {
            return;
        };
        let before = state.clone();
        for topic in entity.bindings.availability_topics() {
            if heard.contains(&topic) || !entity.bindings.is_availability_only(&topic) {
                continue;
            }
            let Some((text, at)) = self.last_availability.get(&topic) else {
                continue;
            };
            // A template failure leaves the availability as it was; the next message retries.
            let _ = apply_parts(entity, state, &topic, text, *at, &self.templates);
        }
        if differs(&before, state) {
            changes.push(Change::StateChanged(id.to_owned()));
        }
    }

    fn remove_entity(&mut self, id: &str, changes: &mut Vec<Change>) {
        let Some(entity) = self.entities.remove(id) else {
            return;
        };
        self.unindex(id, &entity);
        self.forget_unread_availability();
        self.states.remove(id);
        self.origin.remove(id);
        self.template_failures.remove(id);
        changes.push(Change::EntityRemoved(id.to_owned()));
    }

    fn index(&mut self, id: &str, entity: &Entity) {
        for topic in entity.bindings.topics() {
            let ids = self.topic_index.entry(topic.clone()).or_default();
            if let Err(at) = ids.binary_search_by(|other| other.as_str().cmp(id)) {
                ids.insert(at, id.to_owned());
            }
            self.subscriptions.insert(topic);
        }
    }

    /// Drops the remembered word of the topics no entity reads any more: the hub unsubscribes
    /// from them, and the next subscription brings the retained word back.
    fn forget_unread_availability(&mut self) {
        let index = &self.topic_index;
        self.last_availability
            .retain(|topic, _| index.contains_key(topic));
    }

    fn unindex(&mut self, id: &str, entity: &Entity) {
        for topic in entity.bindings.topics() {
            let Some(ids) = self.topic_index.get_mut(&topic) else {
                continue;
            };
            ids.retain(|other| other != id);
            if ids.is_empty() {
                self.topic_index.remove(&topic);
                self.subscriptions.remove(&topic);
            }
        }
    }

    /// Recomputes every entity's override, area, kind and `part_of`, reporting those that
    /// changed. Cheap at the house's scale (hundreds of entities) and always correct.
    fn recompute(&mut self, changes: &mut Vec<Change>) {
        let declared = &self.areas.areas;
        for (id, entity) in &mut self.entities {
            let ui = self.overrides.get(id).cloned().unwrap_or_default();
            let area = areas::effective(entity, ui.area.as_deref(), declared);
            let kind = SwitchType::effective_kind(entity.protocol_kind, &ui);
            if entity.ui != ui || entity.area != area || entity.kind != kind {
                entity.ui = ui;
                entity.area = area;
                entity.kind = kind;
                changes.push(Change::EntityUpserted(id.clone()));
            }
        }
        let part_of = folding::compute(&self.entities);
        for (id, entity) in &mut self.entities {
            let owner = part_of.get(id).cloned();
            if entity.part_of != owner {
                entity.part_of = owner;
                changes.push(Change::EntityUpserted(id.clone()));
            }
        }
    }
}

/// True when anything but `last_update` differs between the two states.
fn differs(before: &EntityState, after: &EntityState) -> bool {
    before.value != after.value
        || before.availability != after.availability
        || before.attributes != after.attributes
}

/// `entity` with the fields the registry computes reset to what discovery produces, so it
/// compares equal to a fresh parse of the same config.
fn as_discovered(entity: &Entity) -> Entity {
    Entity {
        ui: UiOverride::default(),
        area: UNASSIGNED_AREA.to_owned(),
        kind: entity.protocol_kind,
        part_of: None,
        ..entity.clone()
    }
}
