//! The flow side of the hub: ingesting retained definitions, the writers behind the REST and
//! MCP flow tools, and the run bookkeeping the engine reports through.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;

use nidoo_api::{
    ConnectionStatus, FlowView, FlowWriteOutcome, RunStatus, RunView, StepView, TriggerInfo,
    WsEvent,
};
use nidoo_core::command::{Publish, map_command};
use nidoo_core::flow::template::parse;
use nidoo_core::flow::{
    Flow, FlowMode, LabelAction, LabelError, NodeKind, Reading, Scalar, field_scalar, fields,
    headline_scalar, math_names, uses_sun_or_is_dark, validate,
};
use nidoo_core::model::{Command, Kind, UNASSIGNED_AREA};
use nidoo_core::types::{Placement, type_def};

use super::engine::manual_trigger;
use super::{EngineRequest, StoreApplied};
use crate::clock;
use crate::hub::{Hub, HubError};

/// The passes of each `limit` node, by (flow, node): its `max`, its window and the times of the
/// passes still inside it, oldest first.
pub(crate) type LimitCounts =
    BTreeMap<(String, String), (u32, Duration, VecDeque<chrono::DateTime<chrono::Utc>>)>;

/// The warning attached to a sun flow written while the house point is the Paris fallback.
const PARIS_FALLBACK_WARNING: &str = "Running sun flows against the Paris fallback. Set the house location in the settings to use your house's point.";

impl Hub {
    /// Applies a retained flow payload. A change (or removal) is broadcast and tells the engine
    /// to reload; an identical payload or the removal of an unknown flow does nothing.
    pub(crate) fn ingest_flow(&self, id: &str, payload: &[u8]) -> bool {
        let before = self.watched_button_topics();
        let applied = self.flow_store().apply(id, payload);
        // A local write applies first and the broker's echo is then `Unchanged`, so the MQTT
        // task is woken here, whoever wrote the flow, when the watched button topics moved.
        let moved = |hub: &Hub| hub.watched_button_topics().keys().ne(before.keys());
        match applied {
            StoreApplied::Changed | StoreApplied::Removed => {
                self.broadcast(WsEvent::FlowsChanged {
                    flows: self.flows(),
                });
                self.send_engine(EngineRequest::Reload);
                if moved(self) {
                    self.subscriptions_changed.notify_one();
                }
                true
            }
            StoreApplied::Unchanged | StoreApplied::NotFound => false,
        }
    }

    /// The `command_topic` of every button some enabled flow triggers on, with its entity id.
    /// A press is a message on this topic; the hub subscribes to it only while watched.
    pub(crate) fn watched_button_topics(&self) -> BTreeMap<String, Vec<String>> {
        let mut watched: BTreeSet<String> = BTreeSet::new();
        for flow in self.flow_definitions().iter().filter(|flow| flow.enabled) {
            for node in flow.triggers() {
                if let NodeKind::TriggerState(params) = &node.kind {
                    watched.insert(params.entity.clone());
                }
            }
        }
        let reg = self.read();
        let mut topics: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for id in watched {
            let Some(entity) = reg.entity(&id).filter(|e| e.kind == Kind::Button) else {
                continue;
            };
            if let Some(topic) = entity
                .bindings
                .config
                .get("command_topic")
                .and_then(serde_json::Value::as_str)
            {
                topics.entry(topic.to_owned()).or_default().push(id);
            }
        }
        topics
    }

    /// Every flow with its runs, valid or not, sorted by id.
    pub fn flows(&self) -> Vec<FlowView> {
        self.flow_store().views()
    }

    /// One flow with its runs.
    pub fn flow(&self, id: &str) -> Option<FlowView> {
        self.flow_store().get(id)
    }

    /// The valid definitions, enabled or not; what the engine arms from.
    pub fn flow_definitions(&self) -> Vec<Flow> {
        self.flow_store().flows()
    }

    /// Validates `flow` against the house, publishes it retained and applies it locally so the
    /// returned view already reflects it. Every entity a node names must exist and every
    /// `command` node must map for that entity, so a flow that could never work is refused now
    /// rather than failing at 3 a.m.
    /// The outcome carries a warning when the flow reads the sun while the house point is the
    /// Paris fallback.
    pub async fn set_flow(&self, mut flow: Flow) -> Result<FlowWriteOutcome, HubError> {
        validate(&flow).map_err(|err| HubError::InvalidFlow(err.to_string()))?;
        // Held until the write: a label deleted meanwhile cannot come back with this save.
        let _pass = self.labels_pass.lock().await;
        self.check_house(&flow)?;
        self.check_label_nodes(&flow)?;
        self.check_labels(&mut flow)?;
        self.check_area(&mut flow)?;
        self.check_speak_rooms(&mut flow)?;
        let warning = (self.fallback_active() && uses_sun_or_is_dark(&flow))
            .then(|| PARIS_FALLBACK_WARNING.to_owned());
        let flow = self.write_flow(&flow)?;
        Ok(FlowWriteOutcome { flow, warning })
    }

    /// Removes a flow, valid or not: publishes an empty retained payload and applies it.
    pub async fn delete_flow(&self, id: &str) -> Result<(), HubError> {
        if !self.flow_store().exists(id) {
            return Err(HubError::UnknownFlow(id.to_owned()));
        }
        let topic = self.flow_topic(id);
        self.publish(Publish {
            topic: topic.clone(),
            payload: String::new(),
            retain: true,
        })?;
        self.ingest_flow(id, b"");
        Ok(())
    }

    /// Flips `enabled`. The house checks are skipped on purpose: an entity may have vanished
    /// since the flow was saved, and switching the flow off must still work.
    pub async fn set_flow_enabled(&self, id: &str, enabled: bool) -> Result<FlowView, HubError> {
        let view = self
            .flow(id)
            .ok_or_else(|| HubError::UnknownFlow(id.to_owned()))?;
        if let Some(reason) = view.invalid {
            return Err(HubError::InvalidFlow(reason));
        }
        let mut flow = view.definition;
        flow.enabled = enabled;
        self.write_flow(&flow)
    }

    /// Moves a flow to another room (or to none with `None`) without touching the rest. Like
    /// [`Self::set_flow_enabled`] it skips the entity checks, so a flow whose entity vanished
    /// can still be filed.
    pub async fn set_flow_area(
        &self,
        id: &str,
        area: Option<String>,
    ) -> Result<FlowView, HubError> {
        let view = self
            .flow(id)
            .ok_or_else(|| HubError::UnknownFlow(id.to_owned()))?;
        if let Some(reason) = view.invalid {
            return Err(HubError::InvalidFlow(reason));
        }
        let mut flow = view.definition;
        flow.area = area;
        self.check_area(&mut flow)?;
        self.write_flow(&flow)
    }

    /// Sets a flow's labels without touching the rest. Like [`Self::set_flow_area`] it skips
    /// the entity checks; the labels are sorted and de-duplicated, and each must be in the
    /// catalog or already on the flow.
    pub async fn set_flow_labels(
        &self,
        id: &str,
        labels: Vec<String>,
    ) -> Result<FlowView, HubError> {
        let _pass = self.labels_pass.lock().await;
        let view = self
            .flow(id)
            .ok_or_else(|| HubError::UnknownFlow(id.to_owned()))?;
        if let Some(reason) = view.invalid {
            return Err(HubError::InvalidFlow(reason));
        }
        let mut flow = view.definition;
        flow.labels = labels;
        self.check_labels(&mut flow)?;
        self.write_flow(&flow)
    }

    /// [`Self::set_flow_enabled`] for a `flow.enable` node of run `run_id`. A run disabling its
    /// own flow is announced to the engine first (before the write's reload) so it finishes
    /// its remaining steps instead of being cancelled with the flow.
    pub(crate) async fn set_flow_enabled_by_run(
        &self,
        id: &str,
        enabled: bool,
        run_id: &str,
    ) -> Result<FlowView, HubError> {
        if !enabled && self.flow_store().active_run(id).as_deref() == Some(run_id) {
            self.send_engine(EngineRequest::KeepRun {
                flow_id: id.to_owned(),
                run_id: run_id.to_owned(),
            });
        }
        self.set_flow_enabled(id, enabled).await
    }

    /// Starts a manual run from the flow's first trigger ("Run now").
    pub fn run_flow(&self, id: &str) -> Result<RunView, HubError> {
        self.run_flow_from(id, None)
    }

    /// Starts a manual run: creates it and asks the engine to execute it, from the Manual card
    /// `node` or, without one, from the first trigger. Refused while the broker link is down,
    /// like the other writers: the run's commands could not be published. A run already active
    /// refuses it, except a card press on a `restart` flow, which replaces that run.
    pub fn run_flow_from(&self, id: &str, node: Option<&str>) -> Result<RunView, HubError> {
        if !matches!(*self.connection().borrow(), ConnectionStatus::Online) {
            return Err(HubError::Publish("broker offline".to_owned()));
        }
        let run = {
            let mut store = self.flow_store();
            let view = store
                .get(id)
                .ok_or_else(|| HubError::UnknownFlow(id.to_owned()))?;
            if let Some(reason) = view.invalid {
                return Err(HubError::InvalidFlow(reason));
            }
            let definition = view.definition;
            if !definition.enabled {
                return Err(HubError::Rejected("flow is disabled".to_owned()));
            }
            if let Some(node) = node
                && !definition
                    .nodes
                    .iter()
                    .any(|n| n.id == node && matches!(n.kind, NodeKind::TriggerManual(_)))
            {
                return Err(HubError::Rejected(format!(
                    "`{node}` is not a manual trigger of this flow"
                )));
            }
            let replaces = node.is_some() && definition.mode == FlowMode::Restart;
            if store.active_run(id).is_some() && !replaces {
                return Err(HubError::Rejected("a run is active".to_owned()));
            }
            let start = node
                .map(str::to_owned)
                .or_else(|| definition.triggers().first().map(|n| n.id.clone()));
            let trigger = start
                .as_deref()
                .and_then(|start| manual_trigger(&definition, start));
            store
                .begin_manual_run(id, trigger, clock::now())
                .ok_or_else(|| HubError::UnknownFlow(id.to_owned()))?
        };
        self.broadcast_run(id, &run);
        self.send_engine(EngineRequest::Run {
            flow_id: id.to_owned(),
            run_id: run.id.clone(),
            node: node.map(str::to_owned),
        });
        Ok(run)
    }

    /// Starts a `Running` run for the engine (a trigger fired).
    pub fn begin_run(
        &self,
        flow_id: &str,
        trigger: Option<TriggerInfo>,
    ) -> Result<RunView, HubError> {
        let run = self
            .flow_store()
            .begin_run(flow_id, trigger, clock::now())
            .ok_or_else(|| HubError::UnknownFlow(flow_id.to_owned()))?;
        self.broadcast_run(flow_id, &run);
        Ok(run)
    }

    /// Records a trigger that was dropped because a run was active.
    pub fn add_skipped_run(
        &self,
        flow_id: &str,
        trigger_node: &str,
        trigger: Option<TriggerInfo>,
    ) -> Option<RunView> {
        let run =
            self.flow_store()
                .add_skipped_run(flow_id, trigger_node, trigger, clock::now())?;
        self.broadcast_run(flow_id, &run);
        Some(run)
    }

    /// Records a scheduled tick that could not run, with the reason.
    pub fn add_skipped_run_because(
        &self,
        flow_id: &str,
        trigger_node: &str,
        reason: &str,
    ) -> Option<RunView> {
        let run = self.flow_store().add_skipped_run_because(
            flow_id,
            trigger_node,
            None,
            clock::now(),
            reason,
        )?;
        self.broadcast_run(flow_id, &run);
        Some(run)
    }

    /// Appends a step to a run, but only while it is the flow's active run (checked under the
    /// store lock). Returns whether it was: `false` means the run was cancelled, ended or its
    /// flow deleted, and the caller must stop acting.
    pub fn record_step(&self, flow_id: &str, run_id: &str, step: StepView) -> bool {
        let run = {
            let mut store = self.flow_store();
            if store.active_run(flow_id).as_deref() != Some(run_id) {
                return false;
            }
            store.record_step(flow_id, run_id, step)
        };
        match run {
            Some(run) => {
                self.broadcast_run(flow_id, &run);
                true
            }
            None => false,
        }
    }

    /// Finishes a run.
    pub fn end_run(&self, flow_id: &str, run_id: &str, status: RunStatus) {
        let run = self
            .flow_store()
            .end_run(flow_id, run_id, status, clock::now());
        if let Some(run) = run {
            self.broadcast_run(flow_id, &run);
        }
    }

    /// The headline value of an entity, `None` for an unknown entity or an absent value.
    /// An entity's value with its unit, as a notify placeholder reads it.
    pub fn reading(&self, id: &str) -> Option<Reading> {
        let reg = self.read();
        let entity = reg.entity(id)?;
        Reading::of(
            &entity.component,
            entity.device_class.as_deref(),
            entity.unit.as_deref(),
            &reg.state(id)?.value,
        )
    }

    pub fn entity_scalar(&self, id: &str) -> Option<Scalar> {
        self.entity_value(id, None)
    }

    /// An entity's headline value, or `field` of its state; `None` for an unknown entity or an
    /// absent value.
    pub fn entity_value(&self, id: &str, field: Option<&str>) -> Option<Scalar> {
        let reg = self.read();
        let entity = reg.entity(id)?;
        let value = &reg.state(id)?.value;
        match field {
            None => headline_scalar(&entity.component, value),
            Some(field) => field_scalar(value, field),
        }
    }

    /// The `(min, max, step)` of a number entity, `None` when it is unknown.
    pub(crate) fn number_limits(
        &self,
        id: &str,
    ) -> Option<(Option<f64>, Option<f64>, Option<f64>)> {
        let reg = self.read();
        let caps = &reg.entity(id)?.capabilities;
        Some((caps.min, caps.max, caps.step))
    }

    fn broadcast_run(&self, flow_id: &str, run: &RunView) {
        self.broadcast(WsEvent::FlowRunChanged {
            flow_id: flow_id.to_owned(),
            run: run.clone(),
        });
    }

    /// Resolves `flow.area` to an area id and refuses an unknown room. The area already stored
    /// for this flow is always accepted, so a room deleted later does not block re-saving or
    /// toggling the flow; `Unassigned` is never a flow's room (no area means that).
    fn check_area(&self, flow: &mut Flow) -> Result<(), HubError> {
        let Some(raw) = flow.area.clone() else {
            return Ok(());
        };
        let stored = self
            .flow_store()
            .get(&flow.id)
            .filter(|view| view.invalid.is_none())
            .and_then(|view| view.definition.area);
        let reg = self.read();
        let resolved = reg.resolve_area(&raw);
        let known = reg
            .effective_areas()
            .iter()
            .any(|area| area.id == resolved && area.id != UNASSIGNED_AREA);
        if stored.as_deref() == Some(resolved.as_str()) || known {
            flow.area = Some(resolved);
            Ok(())
        } else {
            Err(HubError::InvalidFlow(format!("unknown area `{raw}`")))
        }
    }

    /// Sorts and de-duplicates `flow.labels` and refuses a label the catalog does not have,
    /// unless the stored flow already carries it (a label deleted later never blocks a save).
    fn check_labels(&self, flow: &mut Flow) -> Result<(), HubError> {
        flow.labels.sort();
        flow.labels.dedup();
        let stored = self
            .flow_store()
            .get(&flow.id)
            .map(|view| view.definition.labels)
            .unwrap_or_default();
        let catalog = self.flow_label_catalog();
        match flow
            .labels
            .iter()
            .find(|id| catalog.label(id).is_none() && !stored.contains(id))
        {
            Some(id) => Err(HubError::InvalidFlow(format!("unknown label `{id}`"))),
            None => Ok(()),
        }
    }

    /// Refuses a `flow.label` node whose label is unknown, or that picks a label in no choice.
    /// A node identical to the one stored on this flow passes, so a flow broken by a catalog
    /// edit can still be saved to fix another node.
    fn check_label_nodes(&self, flow: &Flow) -> Result<(), HubError> {
        let stored = self.flow_store().get(&flow.id).map(|view| view.definition);
        let catalog = self.flow_label_catalog();
        for node in &flow.nodes {
            let NodeKind::FlowLabel(params) = &node.kind else {
                continue;
            };
            let unchanged = stored
                .as_ref()
                .and_then(|stored| stored.node(&node.id))
                .is_some_and(|old| old.kind == node.kind);
            if unchanged {
                continue;
            }
            let error = match catalog.label(&params.label) {
                None => Some(LabelError::UnknownLabel(params.label.clone())),
                Some(label)
                    if params.action == LabelAction::Pick
                        && label
                            .choice
                            .as_deref()
                            .is_none_or(|choice| catalog.choice(choice).is_none()) =>
                {
                    Some(LabelError::NotInChoice(params.label.clone()))
                }
                Some(_) => None,
            };
            if let Some(error) = error {
                return Err(HubError::InvalidFlow(format!(
                    "{error} in node `{}`",
                    node.id
                )));
            }
        }
        Ok(())
    }

    /// Resolves the room of every `speak` and `ask` node to an area id (a name is accepted, as for
    /// `flow.area`) and refuses one the house does not have.
    fn check_speak_rooms(&self, flow: &mut Flow) -> Result<(), HubError> {
        let reg = self.read();
        let areas = reg.effective_areas();
        for node in &mut flow.nodes {
            let room = match &mut node.kind {
                NodeKind::Speak(params) => &mut params.room,
                NodeKind::Ask(params) => &mut params.room,
                _ => continue,
            };
            let Some(raw) = room.clone() else {
                continue;
            };
            let resolved = reg.resolve_area(&raw);
            if !areas
                .iter()
                .any(|area| area.id == resolved && area.id != UNASSIGNED_AREA)
            {
                return Err(HubError::InvalidFlow(format!(
                    "unknown area `{raw}` in node `{}`",
                    node.id
                )));
            }
            *room = Some(resolved);
        }
        Ok(())
    }

    /// Checks every entity a node names exists and every command maps; publishes nothing.
    fn check_house(&self, flow: &Flow) -> Result<(), HubError> {
        let reg = self.read();
        let results = math_names(flow);
        for node in &flow.nodes {
            if let NodeKind::Math(params) = &node.kind {
                if reg.entity(&params.name).is_some() {
                    return Err(HubError::InvalidFlow(format!(
                        "node `{}`: result name `{}` is an entity id",
                        node.id, params.name
                    )));
                }
                let inputs = params.inputs.iter().map(|input| &input.entity);
                for entity in inputs.chain(params.save_to.as_ref()) {
                    if reg.entity(entity).is_none() {
                        return Err(HubError::InvalidFlow(format!(
                            "unknown entity `{entity}` in node `{}`",
                            node.id
                        )));
                    }
                }
                if let Some(target) = &params.save_to
                    && reg.entity(target).is_some_and(|e| e.kind != Kind::Number)
                {
                    return Err(HubError::InvalidFlow(format!(
                        "node `{}`: `{target}` is not a number entity",
                        node.id
                    )));
                }
            }
            if let NodeKind::Ask(params) = &node.kind
                && let Some(name) = &params.name
                && reg.entity(name).is_some()
            {
                return Err(HubError::InvalidFlow(format!(
                    "node `{}`: result name `{name}` is an entity id",
                    node.id
                )));
            }
            let texts = match &node.kind {
                NodeKind::Notify(params) => vec![Some(&params.title), params.body.as_ref()],
                NodeKind::Speak(params) => vec![Some(&params.text)],
                NodeKind::Ask(params) => vec![Some(&params.text)],
                _ => Vec::new(),
            };
            // `validate` already refused a text that does not parse.
            for template in texts.into_iter().flatten().filter_map(|t| parse(t).ok()) {
                if let Some(id) = template
                    .entities()
                    .into_iter()
                    .find(|id| !results.contains(id) && reg.entity(id).is_none())
                {
                    return Err(HubError::InvalidFlow(format!(
                        "unknown entity `{id}` in node `{}`",
                        node.id
                    )));
                }
            }
            let entity = match &node.kind {
                NodeKind::TriggerState(params) => Some(&params.entity),
                NodeKind::TriggerSignal(params) => Some(&params.entity),
                NodeKind::TriggerSilent(params) => Some(&params.entity),
                NodeKind::If(params) => params.entity.as_ref(),
                NodeKind::Switch(params) => params.entity.as_ref(),
                NodeKind::Command(params) => Some(&params.entity),
                NodeKind::FlowEnable(params) => {
                    if let Some(target) = &params.flow
                        && *target != flow.id
                        && !self.flow_store().exists(target)
                    {
                        return Err(HubError::InvalidFlow(format!(
                            "unknown flow `{target}` in node `{}`",
                            node.id
                        )));
                    }
                    None
                }
                NodeKind::Delay(_)
                | NodeKind::Limit(_)
                | NodeKind::FlowLabel(_)
                | NodeKind::BackupRun(_)
                | NodeKind::Notify(_)
                | NodeKind::Speak(_)
                | NodeKind::Ask(_)
                | NodeKind::TimeBetween(_)
                | NodeKind::TriggerSchedule(_)
                | NodeKind::TriggerSun(_)
                | NodeKind::TriggerOffPeak(_)
                | NodeKind::TriggerManual(_)
                | NodeKind::Math(_) => None,
            };
            let Some(entity) = entity else { continue };
            let Some(found) = reg.entity(entity) else {
                return Err(HubError::InvalidFlow(format!(
                    "unknown entity `{entity}` in node `{}`",
                    node.id
                )));
            };
            let (field, average) = match &node.kind {
                NodeKind::TriggerState(params) => (params.field.as_deref(), None),
                NodeKind::If(params) => (params.field.as_deref(), params.average.as_ref()),
                NodeKind::Switch(params) => (params.field.as_deref(), params.average.as_ref()),
                _ => (None, None),
            };
            if let Some(field) = field {
                let reported = fields(
                    found.kind,
                    &found.component,
                    &found.capabilities,
                    found.unit.as_deref(),
                );
                let Some(info) = reported.iter().find(|info| info.name == field) else {
                    return Err(HubError::InvalidFlow(format!(
                        "node `{}`: `{entity}` reports no field `{field}`",
                        node.id
                    )));
                };
                if average.is_some() && !info.recorded {
                    return Err(HubError::InvalidFlow(format!(
                        "node `{}`: `{field}` of `{entity}` has no history to average",
                        node.id
                    )));
                }
                // An If or Switch without an entity downstream reads this field too.
                if matches!(node.kind, NodeKind::TriggerState(_))
                    && !info.recorded
                    && let Some(averaging) = averaging_trigger_value(flow, &node.id)
                {
                    return Err(HubError::InvalidFlow(format!(
                        "node `{averaging}`: `{field}` of `{entity}` has no history to average"
                    )));
                }
            }
            let signal_source =
                type_def(&found.component).is_some_and(|def| def.placement == Placement::Signal);
            match &node.kind {
                NodeKind::TriggerState(_) if signal_source => {
                    return Err(HubError::InvalidFlow(format!(
                        "node `{}`: `{entity}` sends signals, trigger on them with trigger.signal",
                        node.id
                    )));
                }
                NodeKind::TriggerSilent(_) if signal_source || found.kind == Kind::Button => {
                    return Err(HubError::InvalidFlow(format!(
                        "node `{}`: `{entity}` never reports state, so it cannot go silent",
                        node.id
                    )));
                }
                NodeKind::TriggerSignal(params) => {
                    if !signal_source {
                        return Err(HubError::InvalidFlow(format!(
                            "node `{}`: `{entity}` sends no signals, use trigger.state",
                            node.id
                        )));
                    }
                    if let Some(signal) = &params.signal
                        && !announced_signals(found).contains(signal)
                    {
                        return Err(HubError::InvalidFlow(format!(
                            "node `{}`: `{entity}` does not announce the signal `{signal}`",
                            node.id
                        )));
                    }
                }
                _ => {}
            }
            if let NodeKind::TriggerState(params) = &node.kind
                && found.kind == Kind::Button
                && [&params.to, &params.from]
                    .into_iter()
                    .any(|filter| filter.as_ref().is_some_and(|v| !v.is_null()))
            {
                return Err(HubError::InvalidFlow(format!(
                    "node `{}`: a button trigger takes no to/from",
                    node.id
                )));
            }
            if let NodeKind::Command(params) = &node.kind {
                if matches!(
                    params.command,
                    Command::Arm { code: Some(_), .. } | Command::Disarm { code: Some(_) }
                ) {
                    return Err(HubError::InvalidFlow(format!(
                        "node `{}`: alarm codes are not allowed in flows (they would be stored in clear on the broker)",
                        node.id
                    )));
                }
                let state = reg.state(entity).cloned().unwrap_or_default();
                map_command(found, &state, &params.command, reg.templates())
                    .map_err(|err| HubError::InvalidFlow(format!("node `{}`: {err}", node.id)))?;
            }
        }
        Ok(())
    }

    /// Publishes the definition retained and applies it locally (the broker's echo is then a
    /// no-op); the registry lock is not held while publishing.
    pub(crate) fn write_flow(&self, flow: &Flow) -> Result<FlowView, HubError> {
        let payload = flow.encode();
        self.publish(Publish {
            topic: self.flow_topic(&flow.id),
            payload: payload.clone(),
            retain: true,
        })?;
        self.ingest_flow(&flow.id, payload.as_bytes());
        self.flow(&flow.id)
            .ok_or_else(|| HubError::UnknownFlow(flow.id.clone()))
    }
}

impl Hub {
    /// Lets a run through the `limit` node `node` of `flow` when fewer than `max` runs passed it
    /// in the last `per`, and records the pass: `Ok` holds the passes in the window, this one
    /// included. Over the limit nothing is recorded and `Err` holds the passes in the window.
    pub fn limit_pass(
        &self,
        flow: &str,
        node: &str,
        max: u32,
        per: Duration,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<usize, usize> {
        let mut limits = self.limits();
        let (_, _, passes) = limits
            .entry((flow.to_owned(), node.to_owned()))
            .or_insert_with(|| (max, per, VecDeque::new()));
        let window = chrono::Duration::from_std(per).unwrap_or(chrono::Duration::MAX);
        while passes.front().is_some_and(|at| *at <= now - window) {
            passes.pop_front();
        }
        if passes.len() >= max as usize {
            return Err(passes.len());
        }
        passes.push_back(now);
        Ok(passes.len())
    }

    /// Forgets the counts of every `limit` node not in `keep` (its `max` and `per` text, by flow
    /// and node) or whose `max` or window changed.
    pub fn limit_retain(&self, keep: &BTreeMap<(String, String), (u32, String)>) {
        self.limits().retain(|key, (max, per, _)| {
            keep.get(key).is_some_and(|(keep_max, keep_per)| {
                keep_max == max && nidoo_core::flow::window(keep_per).is_ok_and(|p| p == *per)
            })
        });
    }
}

impl Hub {
    /// Starts the flow engine on the runtime. The event subscription is taken before the task
    /// spawns so nothing ingested right after this call is missed. A second call logs a warning
    /// and returns an already finished handle: the request receiver can be taken only once.
    pub fn start_engine(self: &Arc<Self>) -> JoinHandle<()> {
        let Some(requests) = self.take_engine_requests() else {
            tracing::warn!("the flow engine was already started");
            return tokio::spawn(async {});
        };
        let events = self.subscribe();
        let seen = self.subscribe_state_touched();
        tokio::spawn(super::engine::run_with(
            Arc::clone(self),
            events,
            seen,
            requests,
        ))
    }
}

/// The signal names an entity announces: its `signals`, else (an `event`) the options of `last`.
/// The first If or Switch reachable from `trigger` that averages the trigger's value (no
/// `entity` of its own, an `average`).
fn averaging_trigger_value<'a>(flow: &'a Flow, trigger: &str) -> Option<&'a str> {
    let mut seen = BTreeSet::from([trigger]);
    let mut queue = vec![trigger];
    while let Some(id) = queue.pop() {
        for edge in flow.edges.iter().filter(|e| e.from == id) {
            if !seen.insert(edge.to.as_str()) {
                continue;
            }
            let averages = match flow.node(&edge.to).map(|n| &n.kind) {
                Some(NodeKind::If(p)) => p.entity.is_none() && p.average.is_some(),
                Some(NodeKind::Switch(p)) => p.entity.is_none() && p.average.is_some(),
                _ => false,
            };
            if averages {
                return Some(edge.to.as_str());
            }
            queue.push(edge.to.as_str());
        }
    }
    None
}

fn announced_signals(entity: &nidoo_core::model::Entity) -> Vec<String> {
    let Some(generic) = &entity.capabilities.generic else {
        return Vec::new();
    };
    let mut names = generic.signals.clone();
    if let Some(options) = generic.options.get("last") {
        names.extend(options.iter().cloned());
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn silent_names_an_existing_entity() {
        let (tx, _rx) = tokio::sync::mpsc::channel::<Publish>(8);
        let hub = Hub::new(&crate::config::Config::default(), tx);
        let flow = Flow::decode(
            serde_json::json!({
                "id": "quiet", "name": "quiet", "enabled": true, "mode": "single",
                "nodes": [
                    { "id": "t", "type": "trigger.silent",
                      "params": { "entity": "binary_sensor.smoke", "for": "50m" } },
                    { "id": "w", "type": "delay", "params": { "for": "1s" } },
                ],
                "edges": [{ "from": "t", "to": "w" }]
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        match hub.set_flow(flow).await {
            Err(HubError::InvalidFlow(reason)) => {
                assert_eq!(reason, "unknown entity `binary_sensor.smoke` in node `t`");
            }
            other => panic!("expected InvalidFlow, got {other:?}"),
        }
    }
}
