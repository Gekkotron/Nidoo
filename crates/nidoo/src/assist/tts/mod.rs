//! Text-to-speech: the Phonon model, fetched from Hugging Face and run on the hub's CPU.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use nidoo_api::{
    AvailableLanguage, DEFAULT_VOICE, DownloadRequest, DownloadView, EngineState, LanguageRequest,
    LanguageView, ModelView, RepoListing, Secret, SpeechRequest, SpeechTicket, TtsStatus,
    UploadSession, VoicesView,
};

use engine::{Engine, SpeakerLoader};
use hf::HfClient;
use languages::LanguageEntry;
use store::{ModelStore, TtsState};
use upload::Uploads;
use voices::VoiceCatalog;

pub mod download;
pub mod engine;
pub mod hf;
pub mod languages;
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

/// The speech facade: store, engines, downloads, uploads and voices behind one handle.
pub struct Tts {
    store: Arc<ModelStore>,
    /// What turns a model folder into a speaker, for each engine started.
    loader: Arc<dyn SpeakerLoader>,
    /// One engine per model some language uses, by model id.
    engines: Mutex<HashMap<String, Arc<Engine>>>,
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
    /// Opens the store, moves an older single-model state to languages and loads every
    /// model a language uses. Never fails: problems show as each language's engine state.
    pub fn start(cfg: TtsConfig) -> Arc<Tts> {
        let store = Arc::new(ModelStore::open(&cfg.root));
        let tts = Arc::new(Tts {
            loader: cfg.loader,
            engines: Mutex::new(HashMap::new()),
            downloads: download::Downloads::default(),
            uploads: Uploads::new(Arc::clone(&store)),
            catalog: VoiceCatalog::new(cfg.root.join("voices")),
            hf_base: cfg.hf_base,
            state_lock: Mutex::new(()),
            voice_lock: tokio::sync::Mutex::new(()),
            clips: speech::Clips::default(),
            store,
        });
        let mut state = tts.store.read_state();
        let migrating = state.active.is_some();
        let before = state.languages.clone();
        languages::migrate(&mut state, |id| {
            tts.store
                .get(id)
                .map(|m| m.languages.clone())
                .unwrap_or_default()
        });
        for entry in &mut state.languages {
            if entry
                .model
                .as_deref()
                .is_some_and(|m| tts.store.get(m).is_none())
            {
                entry.model = None;
                entry.voice = DEFAULT_VOICE.to_owned();
            }
        }
        if migrating || state.languages != before {
            tts.save_state(|s| *s = state);
        }
        tts.reconcile();
        tts
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

    /// The languages the hub speaks, the house language first.
    fn languages(&self) -> Vec<LanguageEntry> {
        self.store.read_state().languages
    }

    fn engines(&self) -> MutexGuard<'_, HashMap<String, Arc<Engine>>> {
        self.engines.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The engine of a model some language uses.
    pub fn engine_for(&self, model: &str) -> Option<Arc<Engine>> {
        self.engines().get(model).cloned()
    }

    /// Starts an engine for each model a language uses and drops the others (their thread
    /// ends once its queue is spoken). A model used by several languages has one engine.
    fn reconcile(self: &Arc<Self>) {
        let wanted = languages::models_in_use(&self.languages());
        let mut started = Vec::new();
        {
            let mut engines = self.engines();
            engines.retain(|id, _| wanted.contains(id));
            for id in &wanted {
                if engines.contains_key(id) {
                    continue;
                }
                let Some(manifest) = self.store.get(id) else {
                    continue;
                };
                let engine = Arc::new(engine::start(Arc::clone(&self.loader)));
                engine.load(id, self.store.dir(id), manifest.weights.clone());
                engines.insert(id.clone(), engine);
                started.push(id.clone());
            }
        }
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            for id in started {
                let tts = Arc::clone(self);
                handle.spawn(async move { tts.apply_voices(&id).await });
            }
        }
    }

    /// Once a freshly started engine is ready, registers the catalogue voice each language
    /// using it chose.
    async fn apply_voices(&self, model: &str) {
        let deadline = tokio::time::Instant::now() + READY_WAIT;
        let engine = loop {
            let Some(engine) = self.engine_for(model) else {
                return;
            };
            match engine.state() {
                EngineState::Ready { .. } => break engine,
                EngineState::Loading { .. } => {}
                _ => return,
            }
            if tokio::time::Instant::now() >= deadline {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        let _guard = self.voice_lock.lock().await;
        for entry in self.languages() {
            if entry.model.as_deref() != Some(model)
                || entry.voice == DEFAULT_VOICE
                || engine.voices().contains(&entry.voice)
            {
                continue;
            }
            if let Ok(path) = self
                .fetch_catalog_voice(&entry.voice, std::slice::from_ref(&entry.code))
                .await
            {
                engine.add_voice(entry.voice.clone(), path);
            }
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

    /// Makes `voice` usable by the language `code` on `model`: a catalogue voice in that
    /// language is downloaded and, when the model's engine runs, registered with it. The
    /// caller holds `voice_lock`.
    async fn register_voice(&self, code: &str, model: &str, voice: &str) -> Result<(), TtsError> {
        if voice == DEFAULT_VOICE {
            return Ok(());
        }
        let engine = self.engine_for(model);
        if let Some(e) = &engine {
            let (list, _) = self.catalog.list(&self.client()).await;
            if voices_for_language(e.voices(), code, &list)
                .iter()
                .any(|v| v == voice)
            {
                return Ok(());
            }
        }
        let path = self.fetch_catalog_voice(voice, &[code.to_owned()]).await?;
        if let Some(engine) = engine {
            engine.add_voice(voice.to_owned(), path);
        }
        Ok(())
    }

    /// Each language with its engine state, the token flag and the current download.
    pub fn status(&self) -> TtsStatus {
        let languages = self
            .languages()
            .into_iter()
            .map(|l| {
                let engine = l
                    .model
                    .as_deref()
                    .and_then(|m| self.engine_for(m))
                    .map_or(EngineState::NoModel, |e| e.state());
                LanguageView {
                    code: l.code,
                    model: l.model,
                    voice: l.voice,
                    engine,
                }
            })
            .collect();
        TtsStatus {
            languages,
            token_set: self.store.token().is_some(),
            download: self.downloads.current(),
            speech_built: cfg!(feature = "tts"),
        }
    }

    /// The installed models, each with the languages that use it.
    pub fn models(&self) -> Vec<ModelView> {
        let list = self.languages();
        self.store
            .list()
            .iter()
            .map(|m| m.view(languages::used_by(&list, &m.id)))
            .collect()
    }

    /// Deletes a model; the languages using it are left without a model.
    pub fn remove(self: &Arc<Self>, id: &str) -> Result<(), TtsError> {
        if self.store.get(id).is_none() {
            return Err(TtsError::NotFound(id.to_owned()));
        }
        self.save_state(|s| languages::detach_model(&mut s.languages, id));
        self.reconcile();
        self.store.remove(id)
    }

    /// Adds a language or changes its model or voice.
    pub async fn set_language(
        self: &Arc<Self>,
        code: &str,
        req: LanguageRequest,
    ) -> Result<TtsStatus, TtsError> {
        let code = languages::normalise_code(code)?;
        let manifest = self
            .store
            .get(&req.model)
            .ok_or_else(|| TtsError::NotFound(req.model.clone()))?;
        if !languages::speaks(&manifest.languages, &code) {
            return Err(TtsError::Invalid(format!(
                "{} does not speak {code}",
                req.model
            )));
        }
        if let Some(voice) = req.voice.as_deref() {
            let _guard = self.voice_lock.lock().await;
            self.register_voice(&code, &req.model, voice).await?;
        }
        self.save_state(|s| {
            languages::upsert(&mut s.languages, &code, &req.model, req.voice.as_deref());
        });
        self.reconcile();
        Ok(self.status())
    }

    /// Stops speaking a language; its model unloads when no other language uses it.
    pub fn remove_language(self: &Arc<Self>, code: &str) -> Result<TtsStatus, TtsError> {
        let code = languages::normalise_code(code)?;
        let mut found = false;
        self.save_state(|s| found = languages::remove(&mut s.languages, &code));
        if !found {
            return Err(TtsError::NotFound(code));
        }
        self.reconcile();
        Ok(self.status())
    }

    /// Makes a language the house language, the one speech uses when none is asked.
    pub fn set_house(&self, code: &str) -> Result<TtsStatus, TtsError> {
        let code = languages::normalise_code(code)?;
        let mut found = false;
        self.save_state(|s| found = languages::make_house(&mut s.languages, &code));
        if !found {
            return Err(TtsError::NotFound(code));
        }
        Ok(self.status())
    }

    /// The languages installed models speak that the hub does not speak yet, by code.
    pub fn available_languages(&self) -> Vec<AvailableLanguage> {
        let models: Vec<(String, Vec<String>)> = self
            .store
            .list()
            .into_iter()
            .map(|m| (m.id.clone(), m.languages.clone()))
            .collect();
        languages::available(&models, &self.languages())
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

    /// The voices on offer to a language (the house language when none is named), and the
    /// one it chose. With no language set up, only the default voice.
    pub async fn voices(&self, language: Option<&str>) -> Result<VoicesView, TtsError> {
        let entry = match language {
            Some(raw) => Some(self.language(raw)?),
            None => self.languages().into_iter().next(),
        };
        Ok(self.voices_of(entry.as_ref()).await)
    }

    /// The configured language `raw` names; unknown or not spoken here is refused.
    fn language(&self, raw: &str) -> Result<LanguageEntry, TtsError> {
        let code = languages::normalise_code(raw)?;
        self.languages()
            .into_iter()
            .find(|l| l.code == code)
            .ok_or_else(|| TtsError::Invalid(format!("this hub does not speak {code}")))
    }

    async fn voices_of(&self, entry: Option<&LanguageEntry>) -> VoicesView {
        let engine = entry
            .and_then(|e| e.model.as_deref())
            .and_then(|m| self.engine_for(m));
        let codes: Vec<String> = entry
            .filter(|e| e.model.is_some())
            .map(|e| vec![e.code.clone()])
            .unwrap_or_default();
        let (list, offline) = if codes.is_empty() {
            (vec![], false)
        } else {
            self.catalog.list(&self.client()).await
        };
        let mut model_voices = match (engine, entry) {
            (Some(e), Some(entry)) => voices_for_language(e.voices(), &entry.code, &list),
            _ => vec![],
        };
        if model_voices.is_empty() {
            model_voices.push(DEFAULT_VOICE.to_owned());
        }
        voices::view(
            &model_voices,
            &codes,
            &list,
            |v| self.catalog.path_of(v).is_ok_and(|p| p.is_file()),
            entry.map_or(DEFAULT_VOICE, |e| e.voice.as_str()),
            offline,
        )
    }

    /// Keeps, clears or replaces the Hugging Face token.
    pub fn set_token(&self, input: Secret) -> Result<(), TtsError> {
        self.store.set_token(input)
    }

    /// Starts speaking a text now; the returned address streams the clip as it fills.
    ///
    /// The language is the one named (it must be set up) or the house language. A named voice
    /// is used without being chosen: a catalogue voice of that language not yet on the disk is
    /// downloaded first, and an unknown one is refused. With no voice named, the language's
    /// voice speaks, or the default when it is not usable.
    pub async fn speak(&self, req: SpeechRequest) -> Result<SpeechTicket, TtsError> {
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
        let house = self
            .languages()
            .into_iter()
            .next()
            .ok_or_else(|| TtsError::Conflict("no language is set up".into()))?;
        let entry = match req.language.as_deref() {
            Some(raw) => self.language(raw)?,
            None => house,
        };
        let model = entry
            .model
            .clone()
            .ok_or_else(|| TtsError::Conflict(format!("{} has no model", entry.code)))?;
        let engine = self
            .engine_for(&model)
            .ok_or_else(|| TtsError::Conflict(format!("{model} is not loaded")))?;
        // Only a ready engine can take a new voice; otherwise the engine says why it cannot speak.
        if let Some(voice) = req.voice.as_deref()
            && matches!(engine.state(), EngineState::Ready { .. })
        {
            let _guard = self.voice_lock.lock().await;
            self.register_voice(&entry.code, &model, voice).await?;
        }
        // A named voice was just registered: the engine takes messages in order, so it knows
        // the voice by the time it speaks, even if `voices()` does not list it yet.
        let voice = match req.voice {
            Some(v) if v == DEFAULT_VOICE => None,
            Some(v) => Some(v),
            None => Some(entry.voice.clone()).filter(|v| engine.voices().contains(v)),
        };
        let (id, clip) = self.clips.create(engine.sample_rate().unwrap_or(24_000));
        let sink = clip_sink(Arc::clone(&clip));
        if let Err(e) = engine.speak(text.to_owned(), Some(entry.code.clone()), voice, sink) {
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

/// The voices an engine holds that the language `code` may use: the model's own, and the
/// catalogue voices in `code`. An engine shared by several languages also holds the other
/// languages' catalogue voices, which stay theirs.
fn voices_for_language(
    engine_voices: Vec<String>,
    code: &str,
    catalog: &[voices::CatalogVoice],
) -> Vec<String> {
    engine_voices
        .into_iter()
        .filter(|v| {
            catalog
                .iter()
                .find(|c| c.name == *v)
                .is_none_or(|c| c.language.as_deref() == Some(code))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::body::to_bytes;

    use super::*;

    #[tokio::test]
    async fn a_dropped_request_ends_its_clip() {
        let clips = speech::Clips::default();
        let (_id, clip) = clips.create(24_000);
        let sink = clip_sink(Arc::clone(&clip));
        // The engine thread died with the request still queued.
        drop(sink);
        let body = Arc::clone(&clip).body(24_000);
        let read = tokio::time::timeout(Duration::from_secs(2), to_bytes(body, usize::MAX))
            .await
            .expect("the clip body hung");
        assert!(
            read.is_err(),
            "the body ends with a failure, not as a clean clip"
        );
    }

    #[tokio::test]
    async fn a_finished_clip_is_not_failed_by_its_sink_dropping() {
        let clips = speech::Clips::default();
        let (_id, clip) = clips.create(24_000);
        let mut sink = clip_sink(Arc::clone(&clip));
        assert!(sink(Some(Ok(vec![0.0; 10]))));
        assert!(sink(None));
        drop(sink);
        let body = Arc::clone(&clip).body(24_000);
        let bytes = to_bytes(body, usize::MAX).await.unwrap();
        assert_eq!(bytes.len(), 44 + 20);
    }
}
