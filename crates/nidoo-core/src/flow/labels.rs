//! Flow labels and choices: the catalog document and the plan of a label switch.
//! Pure data and helpers, so the hub and the web compute the same switch.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::answer::normalize;
use super::model::Flow;

/// The longest label or choice id.
const MAX_ID_LEN: usize = 64;

/// The colour of a label; the web maps each name to its light and dark theme values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum LabelColor {
    #[default]
    Blue,
    Amber,
    Green,
    Red,
    Purple,
    Teal,
    Pink,
    Slate,
}

impl LabelColor {
    /// Every colour, in the order new labels take them.
    pub const ALL: [LabelColor; 8] = [
        LabelColor::Blue,
        LabelColor::Amber,
        LabelColor::Green,
        LabelColor::Red,
        LabelColor::Purple,
        LabelColor::Teal,
        LabelColor::Pink,
        LabelColor::Slate,
    ];
}

/// A name the user gives to a group of flows ("Hiver", "Vacances").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct FlowLabel {
    /// `[a-z0-9_-]{1,64}`, made from the name on creation and never changed by a rename.
    pub id: String,
    /// Free text shown in the UI.
    pub name: String,
    #[serde(default)]
    pub color: LabelColor,
    /// The choice this label belongs to (a choice id); none for a standalone label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice: Option<String>,
    /// Fields this hub does not know, preserved verbatim.
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// A named set of labels where one is current ("Saison: Hiver | Été").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct FlowChoice {
    /// `[a-z0-9_-]{1,64}`, made from the name on creation and never changed by a rename.
    pub id: String,
    /// Free text shown in the UI.
    pub name: String,
    /// The label picked last (a label id); none until the first pick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    /// Fields this hub does not know, preserved verbatim.
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

/// Every label and choice of the house, in display order. Unknown keys are kept.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct FlowLabelCatalog {
    #[serde(default)]
    pub labels: Vec<FlowLabel>,
    #[serde(default)]
    pub choices: Vec<FlowChoice>,
    /// Fields this hub does not know, preserved verbatim.
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Value>,
}

impl FlowLabelCatalog {
    /// Decodes the retained document; an empty payload is an empty catalog. The error is
    /// serde's message.
    pub fn decode(payload: &[u8]) -> Result<Self, String> {
        if payload.iter().all(u8::is_ascii_whitespace) {
            return Ok(Self::default());
        }
        serde_json::from_slice(payload).map_err(|e| e.to_string())
    }

    /// Encodes as compact JSON; `{}` only if serialisation somehow fails.
    pub fn encode(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_owned())
    }

    /// The label with this id.
    pub fn label(&self, id: &str) -> Option<&FlowLabel> {
        self.labels.iter().find(|label| label.id == id)
    }

    /// The choice with this id.
    pub fn choice(&self, id: &str) -> Option<&FlowChoice> {
        self.choices.iter().find(|choice| choice.id == id)
    }

    /// The labels of a choice, in catalog order.
    pub fn members(&self, choice: &str) -> Vec<&FlowLabel> {
        self.labels
            .iter()
            .filter(|label| label.choice.as_deref() == Some(choice))
            .collect()
    }
}

/// What switching a label does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum LabelAction {
    /// Turn the label's flows on and the flows of its choice's other labels off; only for a
    /// label in a choice.
    #[default]
    Pick,
    /// Turn every flow carrying the label on.
    On,
    /// Turn every flow carrying the label off.
    Off,
}

/// Why a label switch cannot run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabelError {
    /// No label has this id.
    UnknownLabel(String),
    /// A pick names a label that belongs to no choice.
    NotInChoice(String),
    /// A pick in a choice (this id) that has fewer than two labels.
    IncompleteChoice(String),
}

impl fmt::Display for LabelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LabelError::UnknownLabel(id) => write!(f, "unknown label `{id}`"),
            LabelError::NotInChoice(id) => write!(f, "label `{id}` is no longer in a choice"),
            LabelError::IncompleteChoice(id) => {
                write!(f, "choice `{id}` has fewer than two labels")
            }
        }
    }
}

impl std::error::Error for LabelError {}

/// The flows a label switch changes, each list sorted by flow id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SwitchPlan {
    /// Flows to turn on (they are off now).
    pub enable: Vec<String>,
    /// Flows to turn off (they are on now).
    pub disable: Vec<String>,
    /// Invalid flows the switch targets but cannot write.
    pub skipped_invalid: Vec<String>,
    /// The choice whose current label becomes the picked one (`Pick` only).
    pub choice: Option<String>,
}

/// Plans switching `label`: `flows` pairs each flow with whether it is invalid. Only flows whose
/// `enabled` differs from the target are listed, so nothing already right is rewritten. Every
/// invalid flow the switch targets is skipped, whatever its stub says about `enabled`.
pub fn plan_switch(
    catalog: &FlowLabelCatalog,
    flows: &[(&Flow, bool)],
    label: &str,
    action: LabelAction,
) -> Result<SwitchPlan, LabelError> {
    let found = catalog
        .label(label)
        .ok_or_else(|| LabelError::UnknownLabel(label.to_owned()))?;
    let (choice, others) = match action {
        LabelAction::Pick => {
            let choice = found
                .choice
                .as_deref()
                .filter(|choice| catalog.choice(choice).is_some())
                .ok_or_else(|| LabelError::NotInChoice(label.to_owned()))?;
            let members = catalog.members(choice);
            if members.len() < 2 {
                return Err(LabelError::IncompleteChoice(choice.to_owned()));
            }
            let others: Vec<&str> = members
                .iter()
                .map(|member| member.id.as_str())
                .filter(|id| *id != label)
                .collect();
            (Some(choice.to_owned()), others)
        }
        LabelAction::On | LabelAction::Off => (None, Vec::new()),
    };
    let mut plan = SwitchPlan {
        choice,
        ..SwitchPlan::default()
    };
    for (flow, invalid) in flows {
        let carries = |id: &str| flow.labels.iter().any(|l| l == id);
        let target = if carries(label) {
            action != LabelAction::Off
        } else if others.iter().any(|other| carries(other)) {
            false
        } else {
            continue;
        };
        if *invalid {
            plan.skipped_invalid.push(flow.id.clone());
        } else if flow.enabled != target {
            let list = if target {
                &mut plan.enable
            } else {
                &mut plan.disable
            };
            list.push(flow.id.clone());
        }
    }
    plan.enable.sort();
    plan.disable.sort();
    plan.skipped_invalid.sort();
    Ok(plan)
}

/// A new id for a label or choice named `name`: lower-case, accents folded, every other run of
/// characters turned into `-`, at most 64 long, `label` when nothing is left, and suffixed `-2`,
/// `-3`… while it is `taken`.
pub fn label_id(name: &str, taken: &[&str]) -> String {
    let mut base = String::new();
    for c in normalize(name).chars() {
        if c.is_ascii_alphanumeric() {
            base.push(c);
        } else if !base.ends_with('-') {
            base.push('-');
        }
    }
    let base = cap(base.trim_matches('-'), MAX_ID_LEN);
    let base = if base.is_empty() { "label" } else { base };
    if !taken.contains(&base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| {
            let suffix = format!("-{n}");
            format!("{}{suffix}", cap(base, MAX_ID_LEN - suffix.len()))
        })
        .find(|id| !taken.contains(&id.as_str()))
        .unwrap_or_else(|| base.to_owned())
}

/// `id` (ASCII) cut to `len` bytes, without a trailing `-`.
fn cap(id: &str, len: usize) -> &str {
    id[..id.len().min(len)].trim_end_matches('-')
}

/// The colour a new label gets: the least used one, ties broken in [`LabelColor::ALL`] order.
pub fn next_color(catalog: &FlowLabelCatalog) -> LabelColor {
    LabelColor::ALL
        .into_iter()
        .min_by_key(|color| {
            catalog
                .labels
                .iter()
                .filter(|label| label.color == *color)
                .count()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn catalog() -> FlowLabelCatalog {
        FlowLabelCatalog::decode(
            r#"{
              "labels": [
                {"id": "hiver", "name": "Hiver", "color": "blue", "choice": "saison"},
                {"id": "ete", "name": "Été", "color": "amber", "choice": "saison"},
                {"id": "vacances", "name": "Vacances", "color": "green"},
                {"id": "seul", "name": "Seul", "color": "red", "choice": "solo"}
              ],
              "choices": [
                {"id": "saison", "name": "Saison", "current": "hiver"},
                {"id": "solo", "name": "Solo"}
              ]
            }"#
            .as_bytes(),
        )
        .unwrap()
    }

    fn flow(id: &str, enabled: bool, labels: &[&str]) -> Flow {
        Flow {
            id: id.into(),
            name: id.into(),
            enabled,
            labels: labels.iter().map(|l| (*l).to_owned()).collect(),
            ..Flow::default()
        }
    }

    fn plan(flows: &[Flow], invalid: &[&str], label: &str, action: LabelAction) -> SwitchPlan {
        try_plan(flows, invalid, label, action).unwrap()
    }

    fn try_plan(
        flows: &[Flow],
        invalid: &[&str],
        label: &str,
        action: LabelAction,
    ) -> Result<SwitchPlan, LabelError> {
        let pairs: Vec<(&Flow, bool)> = flows
            .iter()
            .map(|f| (f, invalid.contains(&f.id.as_str())))
            .collect();
        plan_switch(&catalog(), &pairs, label, action)
    }

    #[test]
    fn pick_turns_on_the_label_and_off_the_rest_of_the_choice() {
        let flows = [
            flow("heat", true, &["hiver"]),
            flow("heat2", true, &["hiver"]),
            flow("fan", false, &["ete"]),
        ];
        let plan = plan(&flows, &[], "ete", LabelAction::Pick);
        assert_eq!(plan.enable, ["fan"]);
        assert_eq!(plan.disable, ["heat", "heat2"]);
        assert!(plan.skipped_invalid.is_empty());
        assert_eq!(plan.choice.as_deref(), Some("saison"));
    }

    #[test]
    fn pick_keeps_a_flow_carrying_both_labels_on() {
        let flows = [
            flow("both", false, &["hiver", "ete"]),
            flow("w", true, &["hiver"]),
        ];
        let plan = plan(&flows, &[], "ete", LabelAction::Pick);
        assert_eq!(plan.enable, ["both"]);
        assert_eq!(plan.disable, ["w"]);
        let flows = [flow("both", true, &["hiver", "ete"])];
        let plan = super::tests::plan(&flows, &[], "ete", LabelAction::Pick);
        assert!(plan.enable.is_empty() && plan.disable.is_empty());
    }

    #[test]
    fn flows_already_right_are_not_listed() {
        let flows = [flow("heat", false, &["hiver"]), flow("fan", true, &["ete"])];
        let plan = plan(&flows, &[], "ete", LabelAction::Pick);
        assert!(plan.enable.is_empty());
        assert!(plan.disable.is_empty());
    }

    #[test]
    fn untouched_flows_without_the_choice() {
        let flows = [
            flow("away", true, &["vacances"]),
            flow("plain", false, &[]),
            flow("ghost", true, &["unknown"]),
        ];
        let plan = plan(&flows, &[], "ete", LabelAction::Pick);
        assert_eq!(
            plan,
            SwitchPlan {
                choice: Some("saison".into()),
                ..SwitchPlan::default()
            }
        );
    }

    #[test]
    fn pick_errors() {
        let flows = [flow("a", true, &["hiver"])];
        assert_eq!(
            try_plan(&flows, &[], "nope", LabelAction::Pick),
            Err(LabelError::UnknownLabel("nope".into()))
        );
        assert_eq!(
            try_plan(&flows, &[], "vacances", LabelAction::Pick),
            Err(LabelError::NotInChoice("vacances".into()))
        );
        assert_eq!(
            try_plan(&flows, &[], "seul", LabelAction::Pick),
            Err(LabelError::IncompleteChoice("solo".into()))
        );
        assert_eq!(
            try_plan(&flows, &[], "nope", LabelAction::On),
            Err(LabelError::UnknownLabel("nope".into()))
        );
        assert_eq!(
            LabelError::UnknownLabel("x".into()).to_string(),
            "unknown label `x`"
        );
        assert_eq!(
            LabelError::NotInChoice("x".into()).to_string(),
            "label `x` is no longer in a choice"
        );
        assert_eq!(
            LabelError::IncompleteChoice("x".into()).to_string(),
            "choice `x` has fewer than two labels"
        );
    }

    #[test]
    fn on_off_work_for_any_label() {
        let flows = [
            flow("a", false, &["vacances"]),
            flow("b", true, &["vacances"]),
            flow("c", true, &["hiver"]),
        ];
        let on = plan(&flows, &[], "vacances", LabelAction::On);
        assert_eq!(on.enable, ["a"]);
        assert!(on.disable.is_empty());
        assert_eq!(on.choice, None);
        let off = plan(&flows, &[], "vacances", LabelAction::Off);
        assert_eq!(off.disable, ["b"]);
        let off = plan(&flows, &[], "hiver", LabelAction::Off);
        assert_eq!(off.disable, ["c"]);
        assert_eq!(off.choice, None);
    }

    #[test]
    fn invalid_flows_are_skipped() {
        let flows = [
            flow("bad", true, &["hiver"]),
            flow("bad2", true, &["ete"]),
            flow("other", true, &["vacances"]),
            flow("w", true, &["hiver"]),
        ];
        let plan = plan(&flows, &["bad", "bad2", "other"], "ete", LabelAction::Pick);
        assert_eq!(plan.skipped_invalid, ["bad", "bad2"]);
        assert_eq!(plan.disable, ["w"]);
        assert!(plan.enable.is_empty());
    }

    #[test]
    fn label_id_folds_and_suffixes() {
        assert_eq!(label_id("Été", &[]), "ete");
        assert_eq!(label_id("Été", &["ete"]), "ete-2");
        assert_eq!(label_id("Été", &["ete", "ete-2"]), "ete-3");
        assert_eq!(label_id("  ", &[]), "label");
        assert_eq!(label_id("Mode Télétravail!", &[]), "mode-teletravail");
        let long = "a".repeat(80);
        assert_eq!(label_id(&long, &[]).len(), 64);
        let taken = "a".repeat(64);
        let next = label_id(&long, &[&taken]);
        assert_eq!(next.len(), 64);
        assert!(next.ends_with("-2"));
    }

    #[test]
    fn next_color_is_least_used() {
        assert_eq!(next_color(&FlowLabelCatalog::default()), LabelColor::Blue);
        // blue, amber, green, red are used once each: purple is next.
        assert_eq!(next_color(&catalog()), LabelColor::Purple);
        let mut full = FlowLabelCatalog::default();
        for (i, color) in LabelColor::ALL.iter().enumerate() {
            full.labels.push(FlowLabel {
                id: format!("l{i}"),
                name: format!("L{i}"),
                color: *color,
                choice: None,
                extra: BTreeMap::new(),
            });
        }
        full.labels[0].color = LabelColor::Amber;
        assert_eq!(next_color(&full), LabelColor::Blue);
    }

    #[test]
    fn catalog_round_trips_unknown_keys() {
        let value = json!({
            "labels": [{"id": "a", "name": "A", "color": "teal", "icon": "leaf"}],
            "choices": [{"id": "c", "name": "C", "order": 2}],
            "version": 3
        });
        let catalog = FlowLabelCatalog::decode(value.to_string().as_bytes()).unwrap();
        assert_eq!(catalog.labels[0].color, LabelColor::Teal);
        assert_eq!(catalog.label("a").map(|l| l.name.as_str()), Some("A"));
        assert_eq!(catalog.choice("c").map(|c| c.current.clone()), Some(None));
        let back: Value = serde_json::from_str(&catalog.encode()).unwrap();
        assert_eq!(back, value);
    }

    #[test]
    fn empty_payload_is_an_empty_catalog() {
        assert_eq!(
            FlowLabelCatalog::decode(b"").unwrap(),
            FlowLabelCatalog::default()
        );
        assert_eq!(
            FlowLabelCatalog::decode(b"{}").unwrap(),
            FlowLabelCatalog::default()
        );
        assert!(FlowLabelCatalog::decode(b"[").is_err());
        assert_eq!(
            FlowLabelCatalog::default().encode(),
            r#"{"labels":[],"choices":[]}"#
        );
    }

    #[test]
    fn members_are_in_catalog_order() {
        let catalog = catalog();
        let ids: Vec<&str> = catalog
            .members("saison")
            .iter()
            .map(|l| l.id.as_str())
            .collect();
        assert_eq!(ids, ["hiver", "ete"]);
        assert!(catalog.members("nope").is_empty());
    }
}
