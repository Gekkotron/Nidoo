//! Wire types for text-to-speech: the engine's state, the model and voice catalogues, model
//! downloads and the speech endpoint. Pure data; the hub does the work.

use serde::{Deserialize, Serialize};

use crate::setup::Secret;

/// The voice a speech request uses when it names none.
pub const DEFAULT_VOICE: &str = "default";

/// The longest text one speech may hold, in characters; the hub refuses a longer one.
pub const MAX_SPEECH_CHARS: usize = 500;

/// Where the speech engine stands.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EngineState {
    /// No model is installed or chosen.
    NoModel,
    /// The model is being loaded.
    Loading {
        /// The model id being loaded.
        model: String,
    },
    /// The model is loaded and can speak.
    Ready {
        /// The model id in use.
        model: String,
    },
    /// The engine could not start.
    Error {
        /// The model id involved, when there is one.
        model: Option<String>,
        /// What went wrong, for the screen.
        message: String,
    },
}

/// One language the hub speaks, as the status shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LanguageView {
    /// The language code (`fr`, `en`, `pt-br`).
    pub code: String,
    /// The model that speaks it, when one is set.
    pub model: Option<String>,
    /// The voice it speaks with.
    pub voice: String,
    /// Where its model's engine stands (`no_model` when it has none).
    pub engine: EngineState,
}

/// The speech section as a read shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TtsStatus {
    /// The languages the hub speaks, the house language first.
    pub languages: Vec<LanguageView>,
    /// Whether a Hugging Face token is stored (the token itself is never returned).
    pub token_set: bool,
    /// The download in progress or the last one, when there is one.
    pub download: Option<DownloadView>,
    /// Whether the speech build is included in this hub.
    pub speech_built: bool,
}

/// How a model got onto the disk.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    /// Downloaded from Hugging Face.
    Download,
    /// Uploaded by the user.
    Upload,
}

/// An installed model as the settings screen lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelView {
    /// The model id, the folder name on disk.
    pub id: String,
    /// The Hugging Face repository it came from, when downloaded.
    pub repo: Option<String>,
    /// The repository revision it was fetched at, when downloaded.
    pub revision: Option<String>,
    /// The weight file in use.
    pub weights: String,
    /// The size on disk, in bytes.
    pub size: u64,
    /// The licence the repository declares, when it does.
    pub licence: Option<String>,
    /// The languages the model speaks (empty when its config lists none).
    pub languages: Vec<String>,
    /// How the model got onto the disk.
    pub source: ModelSource,
    /// When the model was installed, as a Unix timestamp in seconds.
    pub installed_at: i64,
    /// The languages that speak with this model, in list order.
    pub used_by: Vec<String>,
}

/// A request to look at a Hugging Face repository before downloading it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InspectRequest {
    /// The model page URL or the `owner/repo` name.
    pub url: String,
}

/// One weight file a repository offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeightFile {
    /// The file name in the repository.
    pub name: String,
    /// The file size, in bytes.
    pub size: u64,
}

/// What a repository offers, as the hub read it before any download.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoListing {
    /// The `owner/repo` name.
    pub repo: String,
    /// The revision that was read.
    pub revision: String,
    /// Whether the repository needs an accepted licence and a token.
    pub gated: bool,
    /// The licence the repository declares, when it does.
    pub licence: Option<String>,
    /// The weight files the repository offers.
    pub weights: Vec<WeightFile>,
    /// The total size of the small files the model needs, in bytes.
    pub small_bytes: u64,
}

/// A request to download one weight file of a repository.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DownloadRequest {
    /// The `owner/repo` name.
    pub repo: String,
    /// The revision to fetch.
    pub revision: String,
    /// The weight file to fetch, by name.
    pub weights: String,
}

/// Where a download job stands.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Files are still arriving.
    Running,
    /// The model is installed.
    Done,
    /// The download stopped on an error.
    Failed,
    /// The user cancelled the download.
    Cancelled,
}

/// The progress of a model download.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DownloadView {
    /// The model id being installed.
    pub model: String,
    /// Where the job stands.
    pub state: JobState,
    /// Bytes received so far.
    pub done: u64,
    /// Bytes expected in total.
    pub total: u64,
    /// The file being fetched now.
    pub file: String,
    /// The error text, when the job failed.
    pub error: Option<String>,
}

/// A request to add a language or change its model or voice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LanguageRequest {
    /// The installed model that speaks it.
    pub model: String,
    /// The voice to use; left out keeps the current one, or the default with a new model.
    #[serde(default)]
    pub voice: Option<String>,
}

/// A request to make a language the house language.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HouseRequest {
    /// The language code to move first.
    pub code: String,
}

/// A language the installed models speak that the hub does not speak yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AvailableLanguage {
    /// The language code.
    pub code: String,
    /// The installed models that speak it.
    pub models: Vec<String>,
}

/// Where a voice comes from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceSource {
    /// Shipped inside the installed model.
    Model,
    /// Listed in the voice catalogue.
    Catalog,
}

/// A voice as the settings screen lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceView {
    /// The voice name, the id speech requests use.
    pub name: String,
    /// The language code, when the catalogue gives one.
    pub language: Option<String>,
    /// The speaker's gender, when the catalogue gives one.
    pub gender: Option<String>,
    /// The speaker's age group, when the catalogue gives one.
    pub age: Option<String>,
    /// A short description, when the catalogue gives one.
    pub description: Option<String>,
    /// Where the voice comes from.
    pub source: VoiceSource,
    /// Whether the voice file is already on the disk.
    pub on_disk: bool,
}

/// The voices on offer and the one speech uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoicesView {
    /// The voices on offer.
    pub voices: Vec<VoiceView>,
    /// The voice speech uses now.
    pub chosen: String,
    /// Whether the chosen voice was missing, so the default stands in for it.
    pub fell_back: bool,
    /// Whether the voice catalogue could not be reached.
    pub catalog_offline: bool,
}

/// The Hugging Face token in a `PUT`: left out keeps the stored one, `null` clears it, a
/// string sets it.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TokenInput {
    /// The token, following the `Secret` rules.
    #[serde(default)]
    pub token: Secret,
}

/// The id of an upload in progress.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UploadSession {
    /// The session id to send the files under.
    pub id: String,
}

/// A request to speak a text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeechRequest {
    /// The text to speak.
    pub text: String,
    /// The language code to speak in; the house language when left out.
    #[serde(default)]
    pub language: Option<String>,
    /// The voice to use; the language's chosen voice when left out.
    #[serde(default)]
    pub voice: Option<String>,
}

/// The language a voice list is asked for (`?language=`); the house language when left out.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VoicesQuery {
    /// The language code.
    #[serde(default)]
    pub language: Option<String>,
}

/// Where a speech clip can be fetched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeechTicket {
    /// The clip id.
    pub id: String,
    /// The URL the audio streams from.
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn engine_state_is_tagged() {
        let v = serde_json::to_value(EngineState::Ready { model: "m".into() }).unwrap();
        assert_eq!(v, json!({"state": "ready", "model": "m"}));
        let v = serde_json::to_value(EngineState::NoModel).unwrap();
        assert_eq!(v, json!({"state": "no_model"}));
    }

    #[test]
    fn token_input_follows_secret_rules() {
        let keep: TokenInput = serde_json::from_str("{}").unwrap();
        assert_eq!(keep.token, Secret::Keep);
        let clear: TokenInput = serde_json::from_str(r#"{"token": null}"#).unwrap();
        assert_eq!(clear.token, Secret::Clear);
    }

    #[test]
    fn speech_request_voice_is_optional() {
        let r: SpeechRequest = serde_json::from_str(r#"{"text": "Bonjour"}"#).unwrap();
        assert_eq!(r.voice, None);
    }
}
