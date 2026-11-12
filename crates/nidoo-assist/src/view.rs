//! The wire types of the assistant: what the web sends, the turns the hub returns and logs.

use nidoo_core::flow::Lang;
use serde::{Deserialize, Serialize};

/// How a sentence was understood, for the "how it understood" panel and the log.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Trace {
    /// The normalised words.
    pub words: Vec<String>,
    pub intent: Option<String>,
    /// The area id.
    pub room: Option<String>,
    /// How the room was found ("mots : chambre, lea", "choix").
    pub room_how: Option<String>,
    pub entities: Vec<String>,
    /// The value and its arithmetic ("19 − 0,5 = 18,5 °C").
    pub value: Option<String>,
    /// The safety check and its result.
    pub gate: Option<String>,
    pub actions: Vec<ActionTrace>,
    pub elapsed_us: u64,
}

/// One action and what happened to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ActionTrace {
    pub target: String,
    pub what: String,
    /// `ok`, `skipped` or `failed: <reason>`; prefixed `simulated` in Simulation.
    pub result: String,
}

/// `POST /api/v1/assist`: a sentence, or the answer to a pending question.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AssistRequest {
    /// The id the page generated once for this browser; one pending question per client.
    pub client: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub reply: Option<PendingReply>,
    /// Plan and reply, but send nothing to the house.
    #[serde(default)]
    pub simulate: bool,
    /// The language of the sentence and of the reply: `fr` (the default) or `en`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<Lang>,
}

impl AssistRequest {
    /// The language of this request: French unless `en` is asked.
    pub fn lang(&self) -> Lang {
        self.lang.unwrap_or(Lang::Fr)
    }
}

/// A tapped answer to a pending question.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum PendingReply {
    Choice { pending: String, choice: usize },
    Yes { pending: String },
    No { pending: String },
}

/// The question a turn leaves pending.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Question {
    pub pending: String,
    pub kind: QuestionKind,
    /// The labels of the choices; empty for a confirmation (Oui / Non).
    pub choices: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum QuestionKind {
    Choice,
    Confirm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum OutcomeKind {
    Acted,
    Answered,
    Asked,
    Confirming,
    NotUnderstood,
    Failed,
}

impl OutcomeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            OutcomeKind::Acted => "acted",
            OutcomeKind::Answered => "answered",
            OutcomeKind::Asked => "asked",
            OutcomeKind::Confirming => "confirming",
            OutcomeKind::NotUnderstood => "not_understood",
            OutcomeKind::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [
            OutcomeKind::Acted,
            OutcomeKind::Answered,
            OutcomeKind::Asked,
            OutcomeKind::Confirming,
            OutcomeKind::NotUnderstood,
            OutcomeKind::Failed,
        ]
        .into_iter()
        .find(|k| k.as_str() == s)
    }
}

/// One exchange: what was typed or tapped, the reply, and how it was understood.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AssistTurn {
    pub id: String,
    /// Unix seconds.
    pub at: u64,
    pub client: String,
    /// The sentence, or the tapped answer ("Oui", a choice's label).
    pub text: String,
    pub simulate: bool,
    pub outcome: OutcomeKind,
    pub reply: String,
    pub question: Option<Question>,
    pub trace: Trace,
    /// Marked "wrongly understood" with the Faux button.
    pub wrong: bool,
}

/// `GET /api/v1/assist/turns`: newest first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AssistTurns {
    pub turns: Vec<AssistTurn>,
    /// The history database keeps turns; false = only the hub's memory (lost on restart).
    pub kept: bool,
}
