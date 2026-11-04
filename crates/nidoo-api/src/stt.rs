//! Wire types for speech input: the recognition model, its settings and the satellites'
//! shared key. Pure data; the hub does the work.

use serde::{Deserialize, Serialize};

use crate::tts::{DownloadView, EngineState};

/// The recognition model's id, as downloads and the engine state name it.
pub const STT_MODEL_ID: &str = "parakeet-tdt-0.6b-v3-int8";

/// How speech input runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SttSettings {
    /// CPU threads for recognition, 1 to 16.
    pub threads: u8,
    /// Microphone gain before the end-of-phrase detection, 1 to 16.
    pub gain: f32,
    /// The hub's UDP port for the satellites, 1024 to 65535; applies when the hub starts.
    pub port: u16,
    /// Wait after a spoken question before the satellite listens again, 0 to 2000 ms.
    pub listen_margin_ms: u32,
}

impl Default for SttSettings {
    fn default() -> Self {
        Self {
            threads: 4,
            gain: 4.0,
            port: 4960,
            listen_margin_ms: 300,
        }
    }
}

/// Speech input as a read shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SttStatus {
    /// The hub was built with speech input.
    pub built: bool,
    /// The model files are on disk.
    pub installed: bool,
    /// Where the recognizer stands.
    pub engine: EngineState,
    /// The download in progress or the last one, when there is one.
    pub download: Option<DownloadView>,
    pub settings: SttSettings,
    /// A shared key exists (read it with `GET …/key`).
    pub has_key: bool,
    /// The UDP port the hub listens on now.
    pub port_in_use: u16,
}

/// The satellites' shared key, 64 hex characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SttKey {
    pub key: String,
}
