//! The hub asking a question out loud: a flow step hands [`Hub::ask_aloud`] a target and a
//! sentence, the voice service speaks it on a free satellite, listens, and sends back what
//! was heard.

use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use crate::hub::Hub;
use crate::sound::SpeakTarget;

/// The whole question (speaking, listening, answering) is bounded by this.
pub const ASK_LIMIT: Duration = Duration::from_secs(45);

/// One question for the voice service.
pub struct AskRequest {
    pub target: SpeakTarget,
    pub text: String,
    pub reply: oneshot::Sender<AskOutcome>,
}

/// How a question ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskOutcome {
    /// `sid` heard this answer.
    Heard { sid: String, text: String },
    /// Nothing usable was heard in time.
    Silence,
    /// No satellite could ask (the detail says why).
    NoSatellite(String),
    /// The hub has no speech input.
    Off,
}

impl Hub {
    /// Hands the voice service its way in; the second call is a no-op.
    pub fn set_voice_asks(&self, tx: mpsc::Sender<AskRequest>) {
        let _ = self.voice_asks.set(tx);
    }

    /// Asks `text` aloud on the satellites of `target` and waits for the answer, at most
    /// [`ASK_LIMIT`].
    pub async fn ask_aloud(&self, target: SpeakTarget, text: &str) -> AskOutcome {
        self.ask_aloud_within(target, text, ASK_LIMIT).await
    }

    /// [`Hub::ask_aloud`] with its own time limit.
    pub async fn ask_aloud_within(
        &self,
        target: SpeakTarget,
        text: &str,
        limit: Duration,
    ) -> AskOutcome {
        let Some(tx) = self.voice_asks.get() else {
            return AskOutcome::Off;
        };
        let (reply, answer) = oneshot::channel();
        let request = AskRequest {
            target,
            text: text.to_owned(),
            reply,
        };
        if tx.send(request).await.is_err() {
            return AskOutcome::Off;
        }
        match tokio::time::timeout(limit, answer).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) | Err(_) => AskOutcome::Silence,
        }
    }
}
