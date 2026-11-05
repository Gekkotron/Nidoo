//! openWakeWord's model files (CC BY-NC-SA 4.0, so never committed): downloaded on
//! first run into the store's `models/` folder and checked against their SHA-256.

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, bail};
use sha2::{Digest, Sha256};

const BASE: &str = "https://github.com/dscripka/openWakeWord/releases/download/v0.5.1/";
const FILES: [(&str, &str); 3] = [
    (
        "melspectrogram.onnx",
        "ba2b0e0f8b7b875369a2c89cb13360ff53bac436f2895cced9f479fa65eb176f",
    ),
    (
        "embedding_model.onnx",
        "70d164290c1d095d1d4ee149bc5e00543250a7316b59f31d056cff7bd3075c1f",
    ),
    (
        "hey_jarvis_v0.1.onnx",
        "94a13cfe60075b132f6a472e7e462e8123ee70861bc3fb58434a73712ee0d2cb",
    ),
];
pub const CLASSIFIER: &str = "hey_jarvis_v0.1.onnx";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Download {
    Pending,
    Ready,
    /// The last attempt failed; another one follows a minute later.
    Failed(String),
}

/// The models folder and where its download is.
pub struct Models {
    pub dir: PathBuf,
    state: Mutex<Download>,
}

impl Models {
    /// Starts fetching whatever is missing, retrying every minute until it is all there.
    pub fn spawn_download(dir: PathBuf) -> Arc<Models> {
        let models = Arc::new(Models {
            dir,
            state: Mutex::new(Download::Pending),
        });
        let m = models.clone();
        std::thread::Builder::new()
            .name("models".into())
            .spawn(move || {
                loop {
                    match ensure(&m.dir) {
                        Ok(()) => {
                            m.set(Download::Ready);
                            return;
                        }
                        Err(e) => {
                            tracing::warn!(error = %format!("{e:#}"), "wake word model download");
                            m.set(Download::Failed(e.to_string()));
                        }
                    }
                    std::thread::sleep(Duration::from_secs(60));
                }
            })
            .expect("spawning the download thread");
        models
    }

    /// Models already in `dir` (tests).
    #[cfg(test)]
    pub fn ready_at(dir: PathBuf) -> Arc<Models> {
        Arc::new(Models {
            dir,
            state: Mutex::new(Download::Ready),
        })
    }

    pub fn state(&self) -> Download {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn ready(&self) -> bool {
        self.state() == Download::Ready
    }

    fn set(&self, s: Download) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = s;
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn ensure(dir: &Path) -> anyhow::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("creating {}", dir.display()))?;
    // Like every file in the store: this user only.
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    for (name, sha) in FILES {
        let path = dir.join(name);
        if std::fs::read(&path).is_ok_and(|b| sha256_hex(&b) == sha) {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            continue;
        }
        tracing::info!(name, "downloading");
        let mut body = ureq::get(&format!("{BASE}{name}"))
            .call()
            .with_context(|| format!("fetching {name}"))?
            .into_body();
        let bytes = body
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_vec()
            .with_context(|| format!("reading {name}"))?;
        if sha256_hex(&bytes) != sha {
            bail!("{name}: unexpected checksum");
        }
        let part = dir.join(format!("{name}.part"));
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&part)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        std::fs::rename(&part, &path)?;
    }
    Ok(())
}
