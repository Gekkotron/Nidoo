//! Flow labels on the hub: the retained catalog document and its edits.
//!
//! Every write that touches labels across flows takes [`Hub::labels_pass`] and re-reads the
//! flows inside it, so a flow save never brings back a label deleted meanwhile.

use std::collections::BTreeMap;

use nidoo_api::{
    ConnectionStatus, FlowLabelsView, LabelEdit, LabelSwitchResult, SkippedFlow, WsEvent,
};
use nidoo_core::command::Publish;
use nidoo_core::flow::{
    Flow, FlowChoice, FlowLabel, FlowLabelCatalog, LabelAction, label_id, next_color, plan_switch,
};

use crate::hub::{Hub, HubError};

impl Hub {
    /// Applies the retained catalog: empty means no labels, an undecodable payload keeps the
    /// previous catalog. A change is broadcast.
    pub(crate) fn ingest_flow_labels(&self, payload: &[u8]) {
        let next = match FlowLabelCatalog::decode(payload) {
            Ok(catalog) => catalog,
            Err(err) => {
                tracing::warn!(%err, "ignoring an undecodable flow label catalog");
                return;
            }
        };
        {
            let mut slot = self.flow_label_slot();
            if *slot == next {
                return;
            }
            *slot = next;
        }
        self.broadcast(WsEvent::FlowLabelsChanged {
            labels: self.flow_labels(),
        });
    }

    /// The catalog, with how many flows (valid or invalid) carry each of its labels.
    pub fn flow_labels(&self) -> FlowLabelsView {
        let catalog = self.flow_label_catalog();
        let mut counts = BTreeMap::new();
        for view in self.flow_store().views() {
            for id in &view.definition.labels {
                if catalog.label(id).is_some() {
                    *counts.entry(id.clone()).or_insert(0) += 1;
                }
            }
        }
        FlowLabelsView { catalog, counts }
    }

    /// The catalog as last applied.
    pub(crate) fn flow_label_catalog(&self) -> FlowLabelCatalog {
        self.flow_label_slot().clone()
    }

    /// Applies one catalog edit and publishes the catalog. Deleting a label first rewrites every
    /// valid flow carrying it. Refused while the broker link is down, before anything is written.
    pub async fn edit_flow_labels(&self, edit: LabelEdit) -> Result<FlowLabelsView, HubError> {
        let _pass = self.labels_pass.lock().await;
        self.require_online()?;
        let mut catalog = self.flow_label_catalog();
        let mut deleted = None;
        match edit {
            LabelEdit::CreateLabel { name, choice } => {
                let name = clean_name(&name)?;
                if let Some(choice) = &choice {
                    known_choice(&catalog, choice)?;
                }
                let id = new_id(&catalog, &name);
                let color = next_color(&catalog);
                catalog.labels.push(FlowLabel {
                    id,
                    name,
                    color,
                    choice,
                    extra: BTreeMap::new(),
                });
            }
            LabelEdit::CreateChoice { name } => {
                let name = clean_name(&name)?;
                let id = new_id(&catalog, &name);
                catalog.choices.push(FlowChoice {
                    id,
                    name,
                    current: None,
                    extra: BTreeMap::new(),
                });
            }
            LabelEdit::RenameLabel { id, name } => {
                label_mut(&mut catalog, &id)?.name = clean_name(&name)?;
            }
            LabelEdit::RecolorLabel { id, color } => {
                label_mut(&mut catalog, &id)?.color = color;
            }
            LabelEdit::MoveLabel { id, choice } => {
                if let Some(choice) = &choice {
                    known_choice(&catalog, choice)?;
                }
                let label = label_mut(&mut catalog, &id)?;
                let left = std::mem::replace(&mut label.choice, choice);
                if let Some(left) = left.filter(|left| Some(left) != label.choice.as_ref()) {
                    clear_current(&mut catalog, &left, &id);
                }
            }
            LabelEdit::RenameChoice { id, name } => {
                let name = clean_name(&name)?;
                choice_mut(&mut catalog, &id)?.name = name;
            }
            LabelEdit::DeleteLabel { id } => {
                let label = catalog
                    .label(&id)
                    .cloned()
                    .ok_or_else(|| unknown_label(&id))?;
                if let Some(choice) = &label.choice {
                    clear_current(&mut catalog, choice, &id);
                }
                catalog.labels.retain(|l| l.id != id);
                deleted = Some(id);
            }
            LabelEdit::DeleteChoice { id } => {
                known_choice(&catalog, &id)?;
                catalog.choices.retain(|c| c.id != id);
                for label in &mut catalog.labels {
                    if label.choice.as_deref() == Some(id.as_str()) {
                        label.choice = None;
                    }
                }
            }
        }
        if let Some(id) = deleted {
            // Bound first: the store's guard must be released before `write_flow` re-takes it.
            let flows = self.flow_store().flows();
            for mut flow in flows {
                if flow.labels.contains(&id) {
                    flow.labels.retain(|l| *l != id);
                    self.write_flow(&flow)?;
                }
            }
        }
        self.write_flow_labels(&catalog)?;
        Ok(self.flow_labels())
    }

    /// Switches a label: `pick` turns its flows on and the flows of its choice's other labels
    /// off, then makes it the choice's current label; `on` / `off` switch every flow carrying it.
    /// Only flows not already in the target state are written. Invalid flows and failed writes
    /// are reported in `skipped`; the choice's `current` is set even then. `by_run` is the
    /// (flow, run) of a `flow.label` step, whose own flow is switched so the run survives.
    pub async fn switch_flow_label(
        &self,
        label: &str,
        action: LabelAction,
        by_run: Option<(&str, &str)>,
    ) -> Result<LabelSwitchResult, HubError> {
        let _pass = self.labels_pass.lock().await;
        self.require_online()?;
        let catalog = self.flow_label_catalog();
        let views = self.flow_store().views();
        let pairs: Vec<(&Flow, bool)> = views
            .iter()
            .map(|view| (&view.definition, view.invalid.is_some()))
            .collect();
        let plan = plan_switch(&catalog, &pairs, label, action)
            .map_err(|err| HubError::Rejected(err.to_string()))?;
        let name_of = |id: &str| {
            views
                .iter()
                .find(|view| view.definition.id == id)
                .map_or_else(|| id.to_owned(), |view| view.definition.name.clone())
        };
        let skipped = |id: &str, reason: String| SkippedFlow {
            id: id.to_owned(),
            name: name_of(id),
            reason,
        };
        let mut result = LabelSwitchResult {
            label: label.to_owned(),
            action,
            enabled: Vec::new(),
            disabled: Vec::new(),
            skipped: plan
                .skipped_invalid
                .iter()
                .map(|id| skipped(id, "invalid".to_owned()))
                .collect(),
        };
        let mut writes: Vec<(&String, bool)> = plan
            .enable
            .iter()
            .map(|id| (id, true))
            .chain(plan.disable.iter().map(|id| (id, false)))
            .collect();
        // The run's own flow goes first: its `KeepRun` must reach the engine before the reload
        // of any other write, or that reload already sees the flow off and cancels the run.
        let own = |id: &String| by_run.is_some_and(|(flow, _)| flow == id.as_str());
        writes.sort_by_key(|(id, _)| !own(id));
        for (id, on) in writes {
            let written = match by_run {
                Some((flow, run)) if flow == id.as_str() => {
                    self.set_flow_enabled_by_run(id, on, run).await
                }
                _ => self.set_flow_enabled(id, on).await,
            };
            match written {
                Ok(_) if on => result.enabled.push(id.clone()),
                Ok(_) => result.disabled.push(id.clone()),
                Err(err) => result.skipped.push(skipped(id, err.to_string())),
            }
        }
        result.enabled.sort();
        result.disabled.sort();
        if let Some(choice) = plan.choice {
            let mut catalog = self.flow_label_catalog();
            if let Some(choice) = catalog.choices.iter_mut().find(|c| c.id == choice)
                && choice.current.as_deref() != Some(label)
            {
                choice.current = Some(label.to_owned());
                self.write_flow_labels(&catalog)?;
            }
        }
        Ok(result)
    }

    /// Publishes the catalog retained and applies it at once.
    pub(crate) fn write_flow_labels(&self, catalog: &FlowLabelCatalog) -> Result<(), HubError> {
        let payload = catalog.encode();
        self.publish(Publish {
            topic: self.flow_labels_topic.clone(),
            payload: payload.clone(),
            retain: true,
        })?;
        self.ingest_flow_labels(payload.as_bytes());
        Ok(())
    }

    /// `broker offline` unless the broker link is up: label writers check before any write.
    pub(crate) fn require_online(&self) -> Result<(), HubError> {
        if matches!(*self.connection().borrow(), ConnectionStatus::Online) {
            Ok(())
        } else {
            Err(HubError::Publish("broker offline".to_owned()))
        }
    }
}

/// A trimmed, non-empty name.
fn clean_name(name: &str) -> Result<String, HubError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(HubError::Rejected("a name is required".to_owned()));
    }
    Ok(name.to_owned())
}

/// A fresh id for `name`, unique among the labels and choices.
fn new_id(catalog: &FlowLabelCatalog, name: &str) -> String {
    let taken: Vec<&str> = catalog
        .labels
        .iter()
        .map(|l| l.id.as_str())
        .chain(catalog.choices.iter().map(|c| c.id.as_str()))
        .collect();
    label_id(name, &taken)
}

fn unknown_label(id: &str) -> HubError {
    HubError::Rejected(format!("unknown label `{id}`"))
}

fn known_choice(catalog: &FlowLabelCatalog, id: &str) -> Result<(), HubError> {
    catalog
        .choice(id)
        .map(|_| ())
        .ok_or_else(|| HubError::Rejected(format!("unknown choice `{id}`")))
}

fn label_mut<'a>(
    catalog: &'a mut FlowLabelCatalog,
    id: &str,
) -> Result<&'a mut FlowLabel, HubError> {
    catalog
        .labels
        .iter_mut()
        .find(|l| l.id == id)
        .ok_or_else(|| unknown_label(id))
}

fn choice_mut<'a>(
    catalog: &'a mut FlowLabelCatalog,
    id: &str,
) -> Result<&'a mut FlowChoice, HubError> {
    catalog
        .choices
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or_else(|| HubError::Rejected(format!("unknown choice `{id}`")))
}

/// Forgets `label` as the current label of `choice`, when it was.
fn clear_current(catalog: &mut FlowLabelCatalog, choice: &str, label: &str) {
    if let Some(choice) = catalog.choices.iter_mut().find(|c| c.id == choice)
        && choice.current.as_deref() == Some(label)
    {
        choice.current = None;
    }
}

/// The run step detail of a label switch: `picked ete: 2 on, 1 off`, `on vacances: 3 on`,
/// `off vacances: 3 off` or `picked ete: nothing to change`, then `, 1 skipped (Name: reason)`
/// when any flow was not switched.
pub(crate) fn switch_detail(result: &LabelSwitchResult) -> String {
    let (on, off) = (result.enabled.len(), result.disabled.len());
    let (verb, counts) = match result.action {
        LabelAction::Pick => ("picked", format!("{on} on, {off} off")),
        LabelAction::On => ("on", format!("{on} on")),
        LabelAction::Off => ("off", format!("{off} off")),
    };
    let counts = if on + off == 0 {
        "nothing to change".to_owned()
    } else {
        counts
    };
    let mut detail = format!("{verb} {}: {counts}", result.label);
    if !result.skipped.is_empty() {
        let names: Vec<String> = result
            .skipped
            .iter()
            .map(|s| format!("{}: {}", s.name, s.reason))
            .collect();
        detail.push_str(&format!(
            ", {} skipped ({})",
            result.skipped.len(),
            names.join(", ")
        ));
    }
    detail
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(action: LabelAction, on: usize, off: usize, skipped: usize) -> LabelSwitchResult {
        let ids = |n: usize, p: &str| (0..n).map(|i| format!("{p}{i}")).collect();
        LabelSwitchResult {
            label: "ete".into(),
            action,
            enabled: ids(on, "on"),
            disabled: ids(off, "off"),
            skipped: (0..skipped)
                .map(|i| SkippedFlow {
                    id: format!("s{i}"),
                    name: format!("S{i}"),
                    reason: "invalid".into(),
                })
                .collect(),
        }
    }

    #[test]
    fn switch_detail_wording() {
        assert_eq!(
            switch_detail(&result(LabelAction::Pick, 2, 1, 0)),
            "picked ete: 2 on, 1 off"
        );
        assert_eq!(
            switch_detail(&result(LabelAction::On, 3, 0, 0)),
            "on ete: 3 on"
        );
        assert_eq!(
            switch_detail(&result(LabelAction::Off, 0, 3, 0)),
            "off ete: 3 off"
        );
        assert_eq!(
            switch_detail(&result(LabelAction::Pick, 0, 0, 0)),
            "picked ete: nothing to change"
        );
        assert_eq!(
            switch_detail(&result(LabelAction::Pick, 1, 0, 2)),
            "picked ete: 1 on, 0 off, 2 skipped (S0: invalid, S1: invalid)"
        );
    }
}
