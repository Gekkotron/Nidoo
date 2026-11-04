//! Speech input: satellites stream their microphone to the hub after a wake word, the hub
//! finds the end of the phrase, transcribes it and answers through the assistant.
//!
//! [`Stt`] holds what the screens and the voice service share: the settings, the shared key,
//! the model files and their download.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nidoo_api::{DownloadView, EngineState, STT_MODEL_ID, SttSettings, SttStatus};
use tokio::sync::watch;

pub mod engine;
pub mod guard;
pub mod models;
pub mod packet;
pub mod session;
#[cfg(feature = "stt")]
pub mod sherpa;
pub mod store;

use engine::{EngineLoader, Engines, default_loader};
use models::{Pinned, SttModels, pinned_models};
use packet::Key;
use store::SttStore;

/// Why a speech input operation failed.
#[derive(Debug, thiserror::Error)]
pub enum SttError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("not enough disk space: need {need} bytes, {free} free")]
    NoSpace { need: u64, free: u64 },
    #[error("the download failed: {0}")]
    Upstream(String),
    #[error("disk error: {0}")]
    Io(String),
}

/// What the voice service needs to know, sent on every change.
#[derive(Debug, Clone, PartialEq)]
pub struct SttSnapshot {
    pub settings: SttSettings,
    pub key: Option<Key>,
    pub installed: bool,
}

/// Speech input's settings, key and models behind one handle.
pub struct Stt {
    store: SttStore,
    models: SttModels,
    pinned: Vec<Pinned>,
    port_in_use: u16,
    loader: Arc<dyn EngineLoader>,
    engine: Mutex<(EngineState, Option<Engines>)>,
    /// The latest load; an older one that finishes late is dropped.
    load_gen: AtomicU64,
    snapshot: watch::Sender<SttSnapshot>,
}

impl Stt {
    /// Speech input under `root` (`data/assist/stt`), with the pinned models.
    pub fn start(root: PathBuf) -> Arc<Stt> {
        Self::start_with(
            &root,
            pinned_models(),
            SttModels::default(),
            default_loader(),
        )
    }

    /// With other models, download job and loader (tests). Loads the engines in the
    /// background when the model is installed.
    pub fn start_with(
        root: &Path,
        pinned: Vec<Pinned>,
        models: SttModels,
        loader: Arc<dyn EngineLoader>,
    ) -> Arc<Stt> {
        let store = SttStore::open(root);
        let snapshot = SttSnapshot {
            settings: store.settings(),
            key: store.key(),
            installed: store.installed(),
        };
        let stt = Arc::new(Stt {
            port_in_use: snapshot.settings.port,
            store,
            models,
            pinned,
            loader,
            engine: Mutex::new((EngineState::NoModel, None)),
            load_gen: AtomicU64::new(0),
            snapshot: watch::channel(snapshot).0,
        });
        stt.load();
        stt
    }

    fn engine(&self) -> std::sync::MutexGuard<'_, (EngineState, Option<Engines>)> {
        self.engine.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The loaded recognizer and detector factory; `None` until ready.
    pub fn engines(&self) -> Option<Engines> {
        self.engine().1.clone()
    }

    /// (Re)loads the engines on a thread of their own when the model is on disk (~2 s).
    fn load(self: &Arc<Self>) {
        let generation = self.load_gen.fetch_add(1, Ordering::SeqCst) + 1;
        if !self.store.installed() || !self.loader.built() {
            *self.engine() = (EngineState::NoModel, None);
            return;
        }
        *self.engine() = (
            EngineState::Loading {
                model: STT_MODEL_ID.to_owned(),
            },
            None,
        );
        let me = Arc::clone(self);
        let threads = self.store.settings().threads;
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let loaded = me
                .loader
                .load(&me.store.model_dir(), &me.store.vad_path(), threads);
            if me.load_gen.load(Ordering::SeqCst) != generation {
                return;
            }
            *me.engine() = match loaded {
                Ok(engines) => {
                    tracing::info!(
                        threads,
                        ms = started.elapsed().as_millis(),
                        "speech input ready"
                    );
                    let ready = EngineState::Ready {
                        model: STT_MODEL_ID.to_owned(),
                    };
                    (ready, Some(engines))
                }
                Err(message) => {
                    tracing::warn!("speech input cannot start: {message}");
                    let error = EngineState::Error {
                        model: Some(STT_MODEL_ID.to_owned()),
                        message,
                    };
                    (error, None)
                }
            };
            me.refresh();
        });
    }

    fn refresh(&self) {
        self.snapshot.send_replace(SttSnapshot {
            settings: self.store.settings(),
            key: self.store.key(),
            installed: self.store.installed(),
        });
    }

    /// Changes to settings, key and model files.
    pub fn subscribe(&self) -> watch::Receiver<SttSnapshot> {
        self.snapshot.subscribe()
    }

    pub fn snapshot(&self) -> SttSnapshot {
        self.snapshot.borrow().clone()
    }

    /// The UDP port bound at start; a changed port applies when the hub starts again.
    pub fn port_in_use(&self) -> u16 {
        self.port_in_use
    }

    pub fn status(&self) -> SttStatus {
        SttStatus {
            built: self.loader.built(),
            installed: self.store.installed(),
            engine: self.engine().0.clone(),
            download: self.models.current(),
            settings: self.store.settings(),
            has_key: self.store.key().is_some(),
            port_in_use: self.port_in_use,
        }
    }

    /// Saves the settings; a threads change reloads the recognizer.
    pub fn set_settings(self: &Arc<Self>, settings: SttSettings) -> Result<SttStatus, SttError> {
        let reload = settings.threads != self.store.settings().threads;
        self.store.set_settings(settings)?;
        if reload {
            self.load();
        }
        self.refresh();
        Ok(self.status())
    }

    pub fn key(&self) -> Option<Key> {
        self.store.key()
    }

    /// A new shared key; satellites with the old one stop being heard.
    pub fn new_key(&self) -> Result<Key, SttError> {
        let key = self.store.new_key()?;
        self.refresh();
        Ok(key)
    }

    /// Starts the model download.
    pub fn start_download(self: &Arc<Self>) -> Result<DownloadView, SttError> {
        if self.store.installed() {
            return Err(SttError::Conflict("the model is already installed".into()));
        }
        let me = Arc::clone(self);
        self.models.start(
            self.store.root(),
            self.store.model_dir(),
            self.store.vad_path(),
            self.pinned.clone(),
            move || {
                me.refresh();
                me.load();
            },
        )
    }

    pub fn cancel_download(&self) -> Result<(), SttError> {
        self.models.cancel()
    }

    pub fn remove_model(&self) -> Result<(), SttError> {
        if self
            .models
            .current()
            .is_some_and(|v| v.state == nidoo_api::JobState::Running)
        {
            return Err(SttError::Conflict("a download is running".into()));
        }
        self.store.remove_model()?;
        self.load_gen.fetch_add(1, Ordering::SeqCst);
        *self.engine() = (EngineState::NoModel, None);
        self.refresh();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::engine::{EngineLoader, Engines, fakes};
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingLoader(AtomicUsize);

    impl EngineLoader for CountingLoader {
        fn built(&self) -> bool {
            true
        }

        fn load(&self, _: &Path, _: &Path, threads: u8) -> Result<Engines, String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            if threads == 13 {
                return Err("thirteen".into());
            }
            Ok(fakes::engines("heard"))
        }
    }

    fn install(root: &Path) {
        let dir = root.join(store::MODEL_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        for f in store::MODEL_FILES {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        std::fs::write(root.join(store::VAD_FILE), b"x").unwrap();
    }

    fn wait_engine(stt: &Stt, want: &str) -> EngineState {
        for _ in 0..200 {
            let state = stt.status().engine;
            let name = match &state {
                EngineState::Ready { .. } => "ready",
                EngineState::Error { .. } => "error",
                EngineState::Loading { .. } => "loading",
                EngineState::NoModel => "no_model",
            };
            if name == want {
                return state;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the engine never became {want}: {:?}", stt.status().engine)
    }

    #[test]
    fn engines_load_when_the_model_is_installed() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path());
        let loader = Arc::new(CountingLoader(AtomicUsize::new(0)));
        let stt = Stt::start_with(dir.path(), Vec::new(), SttModels::default(), loader.clone());
        wait_engine(&stt, "ready");
        assert!(stt.engines().is_some());
        assert!(stt.status().built);
        // A threads change reloads; a gain change does not.
        let mut s = stt.status().settings;
        s.gain = 8.0;
        stt.set_settings(s.clone()).unwrap();
        s.threads = 2;
        stt.set_settings(s.clone()).unwrap();
        wait_engine(&stt, "ready");
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(loader.0.load(Ordering::SeqCst), 2);
        // A failed load shows as an error and leaves no engines.
        s.threads = 13;
        stt.set_settings(s).unwrap();
        wait_engine(&stt, "error");
        assert!(stt.engines().is_none());
        stt.remove_model().unwrap();
        assert_eq!(stt.status().engine, EngineState::NoModel);
    }

    #[test]
    fn nothing_loads_without_the_model() {
        let dir = tempfile::tempdir().unwrap();
        let loader = Arc::new(CountingLoader(AtomicUsize::new(0)));
        let stt = Stt::start_with(dir.path(), Vec::new(), SttModels::default(), loader.clone());
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(loader.0.load(Ordering::SeqCst), 0);
        assert!(stt.engines().is_none());
        assert_eq!(stt.status().engine, EngineState::NoModel);
    }
}
