//! Onboarding: the bootstrap file, the settings the wizard edits, and how the hub boots from them.

pub mod banner;
pub mod boot;
pub mod bundled;
pub mod fs;
pub mod hub_file;
pub mod mobile;
pub mod probe;
pub mod restart;
pub mod routes;
pub mod secret;
pub mod sections;
pub mod settings;
pub mod store;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tokio::sync::Mutex;

use crate::history::db::Db;
pub use hub_file::{HUB_FILE, HubFile, HubFileError};
pub use restart::RestartHook;
use sections::Sections;

/// Which half of the hub is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Not configured yet: only the setup screens are served.
    Setup,
    /// The whole hub.
    Run,
}

/// What [`SetupCtx::write_unless_restarting`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    Saved,
    /// A restart was already pending: nothing saved.
    Restarting,
    /// The write would restart into a hub that cannot boot (the reason): nothing saved.
    Refused(String),
}

/// What the setup code shares: the files, the database and the restart switch.
pub struct SetupCtx {
    pub data_dir: PathBuf,
    pub mode: Mode,
    /// Every write of `hub.json` goes through this lock (see [`SetupCtx::update_file`]).
    pub file: Mutex<HubFile>,
    pub db: Option<Db>,
    pub sections: Mutex<Sections>,
    pub restarting: AtomicBool,
    pub restart: RestartHook,
}

impl SetupCtx {
    pub fn new(
        data_dir: PathBuf,
        mode: Mode,
        file: HubFile,
        db: Option<Db>,
        sections: Sections,
        restart: RestartHook,
    ) -> SetupCtx {
        SetupCtx {
            data_dir,
            mode,
            file: Mutex::new(file),
            db,
            sections: Mutex::new(sections),
            restarting: AtomicBool::new(false),
            restart,
        }
    }

    /// A Run-mode context over an empty `hub.json` that never restarts, for tests and tools
    /// that need a router but not the setup. Its directory is only written if a setup write
    /// route is called.
    #[doc(hidden)]
    pub fn inert() -> Arc<SetupCtx> {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Arc::new(SetupCtx::new(
            std::env::temp_dir().join(format!("nidoo-inert-{}-{n}", std::process::id())),
            Mode::Run,
            HubFile::default(),
            None,
            Sections::default(),
            RestartHook::Record(Arc::default()),
        ))
    }

    /// Like [`SetupCtx::update_file`], but refuses ([`Written::Restarting`], nothing saved) when
    /// a restart is already pending, checked under the file lock. When the edit makes
    /// `restart(before, after)` true, `check(after)` must pass first (else
    /// [`Written::Refused`], nothing saved), and the restart is scheduled while the lock is
    /// still held, so a writer queued behind this one always sees the flag.
    pub async fn write_unless_restarting(
        self: &Arc<Self>,
        delay: std::time::Duration,
        edit: impl FnOnce(&mut HubFile),
        restart: impl FnOnce(&HubFile, &HubFile) -> bool,
        check: impl FnOnce(&HubFile) -> Result<(), String>,
    ) -> std::io::Result<Written> {
        let mut held = self.file.lock().await;
        if self.restarting.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(Written::Restarting);
        }
        let mut next = held.clone();
        edit(&mut next);
        let go = restart(&held, &next);
        if go && let Err(message) = check(&next) {
            return Ok(Written::Refused(message));
        }
        next.save(&self.data_dir)?;
        *held = next;
        if go {
            restart::schedule_restart(self, delay);
        }
        Ok(Written::Saved)
    }

    /// Edits `hub.json` under the lock: changes a copy, saves the copy, and only then replaces
    /// the held value, so a failed save leaves memory and disk alike, and two writers never
    /// meet on `hub.json.tmp`.
    pub async fn update_file(&self, edit: impl FnOnce(&mut HubFile)) -> std::io::Result<()> {
        let mut held = self.file.lock().await;
        let mut next = held.clone();
        edit(&mut next);
        next.save(&self.data_dir)?;
        *held = next;
        Ok(())
    }
}
