//! Text-to-speech: the Phonon model, fetched from Hugging Face and run on the hub's CPU.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nidoo_api::{
    DEFAULT_VOICE, DownloadRequest, DownloadView, EngineState, ModelView, RepoListing, Secret,
    SpeechRequest, SpeechTicket, TtsStatus, UploadSession, VoicesView,
};

use engine::{Engine, SpeakerLoader};
use hf::HfClient;
use store::{ModelStore, TtsState};
use upload::Uploads;
use voices::VoiceCatalog;

pub mod download;
pub mod engine;
pub mod hf;
#[cfg(feature = "tts")]
pub mod phonon;
pub mod source;
pub mod speech;
pub mod store;
pub mod tone;
pub mod upload;
pub mod voices;

/// The Hugging Face site every model comes from.
pub const HF_BASE: &str = "https://huggingface.co";

/// Limit the CPU threads the speech engine may use, unless the user already did.
///
/// When `RAYON_NUM_THREADS` is unset, this caps the engine at half the logical
/// cores (at least one); when it is set, that value is the user's override and
/// is left alone. Without the `tts` feature it does nothing.
///
/// This sets an environment variable internally: call it before any other
/// thread exists (first thing in `main`).
pub fn limit_threads() {
    #[cfg(feature = "tts")]
    if std::env::var_os("RAYON_NUM_THREADS").is_none() {
        let cores = std::thread::available_parallelism().map_or(1, usize::from);
        xn::set_num_threads((cores / 2).max(1));
    }
}

/// Why a speech operation failed.
#[derive(Debug, thiserror::Error)]
pub enum TtsError {
    /// The model, voice or repository does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The request is not acceptable as given.
    #[error("{0}")]
    Invalid(String),
    /// The request clashes with the current state.
    #[error("{0}")]
    Conflict(String),
    /// Another speech job is running.
    #[error("speech is busy, try again in a moment")]
    Busy,
    /// Hugging Face (or the network) failed.
    #[error("Hugging Face failed: {0}")]
    Upstream(String),
    /// Hugging Face refused access.
    #[error(
        "Hugging Face refused the download: accept the model's terms on its page, or check the token"
    )]
    Denied,
    /// The disk cannot hold the download.
    #[error("not enough disk space: need {need} bytes, {free} free")]
    NoSpace {
        /// Bytes the download needs.
        need: u64,
        /// Bytes free.
        free: u64,
    },
    /// A file is bigger than the allowed size.
    #[error("too large: {0}")]
    TooLarge(String),
    /// A disk operation failed.
    #[error("disk error: {0}")]
    Io(String),
}

/// Where the speech parts live and how they are built.
pub struct TtsConfig {
    /// The folder holding models, voices, state and token.
    pub root: PathBuf,
    /// The Hugging Face address (injected in tests only).
    pub hf_base: String,
    /// What turns a model folder into a speaker.
    pub loader: Arc<dyn SpeakerLoader>,
}

impl TtsConfig {
    /// The real setup under `data_dir/assist/tts`.
    pub fn for_data_dir(data_dir: &Path) -> TtsConfig {
        #[cfg(feature = "tts")]
        let loader: Arc<dyn SpeakerLoader> = Arc::new(phonon::PhononLoader);
        #[cfg(not(feature = "tts"))]
        let loader: Arc<dyn SpeakerLoader> = Arc::new(engine::NoSpeechLoader);
        TtsConfig {
            root: data_dir.join("assist").join("tts"),
            hf_base: HF_BASE.to_owned(),
            loader,
        }
    }
}

/// How long a freshly loading model is waited for before its voice is left alone.
const READY_WAIT: Duration = Duration::from_secs(120);

/// The speech facade: store, engine, downloads, uploads and voices behind one handle.
pub struct Tts {
    store: Arc<ModelStore>,
    engine: Engine,
    downloads: download::Downloads,
    uploads: Uploads,
    catalog: VoiceCatalog,
    hf_base: String,
    /// Serialises read-modify-write of `state.json`.
    state_lock: Mutex<()>,
    /// One voice choice at a time (two fetches of a voice share one `.part` file).
    voice_lock: tokio::sync::Mutex<()>,
    clips: speech::Clips,
}

/// Ends its clip when dropped unfinished, so a request that never runs frees its readers.
struct ClipGuard {
    clip: Arc<speech::Clip>,
    finished: bool,
}

impl Drop for ClipGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.clip.finish(Some("speech stopped".into()));
        }
    }
}

/// The sink that fills `clip` from the engine thread.
fn clip_sink(clip: Arc<speech::Clip>) -> engine::Sink {
    let mut guard = ClipGuard {
        clip,
        finished: false,
    };
    Box::new(move |item| {
        match item {
            Some(Ok(samples)) => {
                let mut pcm = Vec::new();
                speech::to_pcm16(&samples, &mut pcm);
                guard.clip.push(&pcm);
                // Nobody can fetch an expired clip nobody is reading: stop early.
                return !(guard.clip.expired() && Arc::strong_count(&guard.clip) == 1);
            }
            Some(Err(message)) => {
                guard.finished = true;
                guard.clip.finish(Some(message));
            }
            None => {
                guard.finished = true;
                guard.clip.finish(None);
            }
        }
        true
    })
}

impl Tts {
    /// Opens the store and loads the chosen model. Never fails: problems show as the
    /// engine's state.
    pub fn start(cfg: TtsConfig) -> Arc<Tts> {
        let store = Arc::new(ModelStore::open(&cfg.root));
        let tts = Arc::new(Tts {
            engine: engine::start(cfg.loader),
            downloads: download::Downloads::default(),
            uploads: Uploads::new(Arc::clone(&store)),
            catalog: VoiceCatalog::new(cfg.root.join("voices")),
            hf_base: cfg.hf_base,
            state_lock: Mutex::new(()),
            voice_lock: tokio::sync::Mutex::new(()),
            clips: speech::Clips::default(),
            store,
        });
        if let Some(id) = tts.store.read_state().active {
            if tts.store.get(&id).is_some() {
                tts.load(&id);
            } else {
                tts.save_state(|s| s.active = None);
            }
        }
        tts
    }

    /// The engine, for speech requests.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The saved voice name.
    pub fn chosen_voice(&self) -> String {
        self.store.read_state().voice
    }

    fn client(&self) -> HfClient {
        HfClient::new(&self.hf_base, self.store.token())
    }

    fn save_state(&self, f: impl FnOnce(&mut TtsState)) {
        let _guard = self.state_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.store.read_state();
        f(&mut state);
        if let Err(e) = self.store.write_state(&state) {
            tracing::warn!("cannot save the speech state: {e}");
        }
    }

    /// Loads an installed model and, once ready, registers the chosen catalogue voice.
    fn load(self: &Arc<Self>, id: &str) {
        let Some(manifest) = self.store.get(id) else {
            return;
        };
        self.engine
            .load(id, self.store.dir(id), manifest.weights.clone());
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let (tts, id) = (Arc::clone(self), id.to_owned());
            handle.spawn(async move { tts.apply_chosen_voice(&id).await });
        }
    }

    async fn apply_chosen_voice(&self, id: &str) {
        let deadline = tokio::time::Instant::now() + READY_WAIT;
        loop {
            match self.engine.state() {
                EngineState::Ready { model } if model == id => break,
                EngineState::Loading { model } if model == id => {}
                _ => return,
            }
            if tokio::time::Instant::now() >= deadline {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let _guard = self.voice_lock.lock().await;
        let voice = self.chosen_voice();
        if voice == DEFAULT_VOICE || self.engine.voices().contains(&voice) {
            return;
        }
        let languages = self.store.get(id).map(|m| m.languages).unwrap_or_default();
        if let Ok(path) = self.fetch_catalog_voice(&voice, &languages).await {
            self.engine.add_voice(voice, path);
        }
    }

    async fn fetch_catalog_voice(
        &self,
        voice: &str,
        languages: &[String],
    ) -> Result<PathBuf, TtsError> {
        let client = self.client();
        let (list, _) = self.catalog.list(&client).await;
        let found = list
            .iter()
            .find(|v| {
                v.name == voice
                    && v.language
                        .as_ref()
                        .is_some_and(|l| languages.iter().any(|m| m == l))
            })
            .ok_or_else(|| TtsError::Invalid(format!("unknown voice: {voice}")))?;
        self.catalog.fetch(&client, found).await
    }

    /// Engine state, choices, token flag and the current download.
    pub fn status(&self) -> TtsStatus {
        let state = self.store.read_state();
        TtsStatus {
            engine: self.engine.state(),
            active: state.active,
            voice: state.voice,
            token_set: self.store.token().is_some(),
            download: self.downloads.current(),
            speech_built: cfg!(feature = "tts"),
        }
    }

    /// The installed models.
    pub fn models(&self) -> Vec<ModelView> {
        let active = self.store.read_state().active;
        self.store
            .list()
            .iter()
            .map(|m| m.view(active.as_deref() == Some(m.id.as_str())))
            .collect()
    }

    /// Deletes a model; the active one is unloaded and no model stays active.
    pub fn remove(&self, id: &str) -> Result<(), TtsError> {
        if self.store.get(id).is_none() {
            return Err(TtsError::NotFound(id.to_owned()));
        }
        if self.store.read_state().active.as_deref() == Some(id) {
            self.engine.unload();
            self.save_state(|s| s.active = None);
        }
        self.store.remove(id)
    }

    /// Chooses the model in use, or none.
    pub fn set_active(self: &Arc<Self>, model: Option<String>) -> Result<TtsStatus, TtsError> {
        match model {
            Some(id) => {
                if self.store.get(&id).is_none() {
                    return Err(TtsError::NotFound(id));
                }
                self.save_state(|s| s.active = Some(id.clone()));
                self.load(&id);
            }
            None => {
                self.engine.unload();
                self.save_state(|s| s.active = None);
            }
        }
        Ok(self.status())
    }

    /// Reads a Hugging Face repository.
    pub async fn inspect(&self, url: &str) -> Result<RepoListing, TtsError> {
        let src = source::parse_source(url)?;
        self.client().inspect(&src).await
    }

    /// Starts the background download.
    pub fn start_download(&self, req: DownloadRequest) -> Result<DownloadView, TtsError> {
        self.downloads
            .start(Arc::clone(&self.store), self.client(), req, |_| {})
    }

    /// The running or last download.
    pub fn download(&self) -> Option<DownloadView> {
        self.downloads.current()
    }

    /// Stops the running download.
    pub fn cancel_download(&self) -> Result<(), TtsError> {
        self.downloads.cancel()
    }

    /// Opens an upload session.
    pub fn upload_open(&self) -> Result<UploadSession, TtsError> {
        self.uploads.open()
    }

    /// Streams one file of an upload to disk.
    pub async fn upload_file(
        &self,
        id: &str,
        name: &str,
        body: axum::body::Body,
    ) -> Result<(), TtsError> {
        self.uploads.file(id, name, body).await
    }

    /// Closes an upload and installs the model.
    pub fn upload_finish(&self, id: &str) -> Result<ModelView, TtsError> {
        self.uploads.finish(id)
    }

    /// Drops an upload and its files.
    pub fn upload_abort(&self, id: &str) -> Result<(), TtsError> {
        self.uploads.abort(id)
    }

    /// The voices on offer and the chosen one.
    pub async fn voices(&self) -> VoicesView {
        let state = self.store.read_state();
        let languages = state
            .active
            .as_deref()
            .and_then(|id| self.store.get(id))
            .map(|m| m.languages)
            .unwrap_or_default();
        let mut model_voices = self.engine.voices();
        if model_voices.is_empty() {
            model_voices.push(DEFAULT_VOICE.to_owned());
        }
        let (list, offline) = if languages.is_empty() {
            (vec![], false)
        } else {
            self.catalog.list(&self.client()).await
        };
        voices::view(
            &model_voices,
            &languages,
            &list,
            |v| self.catalog.path_of(v).is_ok_and(|p| p.is_file()),
            &state.voice,
            offline,
        )
    }

    /// Chooses the voice speech uses.
    pub async fn set_voice(&self, voice: &str) -> Result<VoicesView, TtsError> {
        {
            let _guard = self.voice_lock.lock().await;
            if voice != DEFAULT_VOICE {
                let active = self.store.read_state().active;
                let manifest = active
                    .as_deref()
                    .and_then(|id| self.store.get(id))
                    .ok_or_else(|| TtsError::Invalid("no model is active".into()))?;
                if !self.engine.voices().iter().any(|v| v == voice) {
                    let path = self.fetch_catalog_voice(voice, &manifest.languages).await?;
                    self.engine.add_voice(voice.to_owned(), path);
                }
            }
            self.save_state(|s| s.voice = voice.to_owned());
        }
        Ok(self.voices().await)
    }

    /// Keeps, clears or replaces the Hugging Face token.
    pub fn set_token(&self, input: Secret) -> Result<(), TtsError> {
        self.store.set_token(input)
    }

    /// Starts speaking a text now; the returned address streams the clip as it fills.
    pub fn speak(&self, req: SpeechRequest) -> Result<SpeechTicket, TtsError> {
        let text = req.text.trim();
        if text.is_empty() {
            return Err(TtsError::Invalid("the text is empty".into()));
        }
        if text.chars().count() > speech::MAX_SPEECH_CHARS {
            return Err(TtsError::Invalid(format!(
                "the text is longer than {} characters",
                speech::MAX_SPEECH_CHARS
            )));
        }
        let wanted = req.voice.unwrap_or_else(|| self.chosen_voice());
        let voice = self.engine.voices().contains(&wanted).then_some(wanted);
        let (id, clip) = self.clips.create();
        let sink = clip_sink(Arc::clone(&clip));
        if let Err(e) = self.engine.speak(text.to_owned(), voice, sink) {
            self.clips.remove(&id);
            return Err(e);
        }
        let url = format!("/api/v1/assist/speech/{id}.wav");
        Ok(SpeechTicket { id, url })
    }

    /// A clip that can still be fetched.
    pub fn clip(&self, id: &str) -> Option<Arc<speech::Clip>> {
        self.clips.get(id)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::body::to_bytes;

    use super::*;

    #[tokio::test]
    async fn a_dropped_request_ends_its_clip() {
        let clips = speech::Clips::default();
        let (_id, clip) = clips.create();
        let sink = clip_sink(Arc::clone(&clip));
        // The engine thread died with the request still queued.
        drop(sink);
        let body = Arc::clone(&clip).body(24_000);
        let read = tokio::time::timeout(Duration::from_secs(2), to_bytes(body, usize::MAX))
            .await
            .expect("the clip body hung");
        assert!(read.is_err(), "the body ends with a failure, not as a clean clip");
    }

    #[tokio::test]
    async fn a_finished_clip_is_not_failed_by_its_sink_dropping() {
        let clips = speech::Clips::default();
        let (_id, clip) = clips.create();
        let mut sink = clip_sink(Arc::clone(&clip));
        assert!(sink(Some(Ok(vec![0.0; 10]))));
        assert!(sink(None));
        drop(sink);
        let body = Arc::clone(&clip).body(24_000);
        let bytes = to_bytes(body, usize::MAX).await.unwrap();
        assert_eq!(bytes.len(), 44 + 20);
    }
}
