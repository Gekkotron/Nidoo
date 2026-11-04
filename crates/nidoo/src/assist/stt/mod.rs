//! Speech input: satellites stream their microphone to the hub after a wake word, the hub
//! finds the end of the phrase, transcribes it and answers through the assistant.
//!
//! [`Stt`] holds what the screens and the voice service share: the settings, the shared key,
//! the model files and their download.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nidoo_api::{DownloadView, EngineState, SttSettings, SttStatus};
use tokio::sync::watch;

pub mod guard;
pub mod models;
pub mod packet;
pub mod store;

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
    engine: Mutex<EngineState>,
    snapshot: watch::Sender<SttSnapshot>,
}

impl Stt {
    /// Speech input under `root` (`data/assist/stt`), with the pinned models.
    pub fn start(root: PathBuf) -> Arc<Stt> {
        Self::start_with(&root, pinned_models(), SttModels::default())
    }

    /// With other models and download job (tests).
    pub fn start_with(root: &Path, pinned: Vec<Pinned>, models: SttModels) -> Arc<Stt> {
        let store = SttStore::open(root);
        let snapshot = SttSnapshot {
            settings: store.settings(),
            key: store.key(),
            installed: store.installed(),
        };
        Arc::new(Stt {
            port_in_use: snapshot.settings.port,
            store,
            models,
            pinned,
            engine: Mutex::new(EngineState::NoModel),
            snapshot: watch::channel(snapshot).0,
        })
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
            built: cfg!(feature = "stt"),
            installed: self.store.installed(),
            engine: self
                .engine
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
            download: self.models.current(),
            settings: self.store.settings(),
            has_key: self.store.key().is_some(),
            port_in_use: self.port_in_use,
        }
    }

    pub fn set_settings(&self, settings: SttSettings) -> Result<SttStatus, SttError> {
        self.store.set_settings(settings)?;
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
            move || me.refresh(),
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
        *self.engine.lock().unwrap_or_else(|e| e.into_inner()) = EngineState::NoModel;
        self.refresh();
        Ok(())
    }
}
