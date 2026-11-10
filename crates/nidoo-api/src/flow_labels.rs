//! Flow labels and choices: `GET`/`POST /api/v1/flow-labels` and the label switch.

use std::collections::BTreeMap;

use nidoo_core::flow::{FlowLabelCatalog, LabelAction, LabelColor};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The label catalog, with how many flows carry each label (valid and invalid flows alike).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FlowLabelsView {
    pub catalog: FlowLabelCatalog,
    /// Flow count per label id; a label no flow carries is absent.
    #[serde(default)]
    pub counts: BTreeMap<String, usize>,
}

/// One change to the label catalog. No edit switches flows; deleting a label removes it from
/// every flow carrying it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum LabelEdit {
    /// A new label, standalone or in a choice (a choice id); its id is made from the name.
    CreateLabel {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        choice: Option<String>,
    },
    /// A new choice, with no labels and no current label yet.
    CreateChoice { name: String },
    /// Renames a label; its id does not change.
    RenameLabel { id: String, name: String },
    /// Changes a label's colour.
    RecolorLabel { id: String, color: LabelColor },
    /// Moves a label into a choice, or out of its choice (`choice` absent).
    MoveLabel {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        choice: Option<String>,
    },
    /// Renames a choice; its id does not change.
    RenameChoice { id: String, name: String },
    /// Deletes a label and removes it from every flow carrying it.
    DeleteLabel { id: String },
    /// Deletes a choice; its labels stay, as standalone labels.
    DeleteChoice { id: String },
}

/// A flow a label switch did not change, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkippedFlow {
    pub id: String,
    pub name: String,
    /// `invalid`, or the error of the failed write.
    pub reason: String,
}

/// What a label switch did: the flows it turned on and off (only those that changed), and the
/// ones it could not switch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LabelSwitchResult {
    pub label: String,
    pub action: LabelAction,
    /// Flow ids turned on.
    pub enabled: Vec<String>,
    /// Flow ids turned off.
    pub disabled: Vec<String>,
    #[serde(default)]
    pub skipped: Vec<SkippedFlow>,
}

/// The body of `POST /api/v1/flow-labels/{id}/switch`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LabelSwitchBody {
    pub action: LabelAction,
}
