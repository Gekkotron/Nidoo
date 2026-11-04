//! Uploading a model from the user's computer: one file per request, streamed to disk.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use futures_util::StreamExt;
use nidoo_api::{ModelSource, ModelView, UploadSession};
use tokio::io::AsyncWriteExt;

use super::TtsError;
use super::store::{
    MAX_SMALL_FILE_BYTES, MAX_WEIGHTS_BYTES, Manifest, ManifestFile, ModelStore, SMALL_FILES,
    Staging, WEIGHT_FILES, quant_of,
};

struct Session {
    id: String,
    staging: Staging,
    files: Vec<ManifestFile>,
    /// Names whose bytes are still arriving.
    pending: BTreeSet<String>,
}

/// The one upload the hub keeps open at a time.
pub struct Uploads {
    store: Arc<ModelStore>,
    session: Mutex<Option<Session>>,
}

fn io(e: std::io::Error) -> TtsError {
    TtsError::Io(e.to_string())
}

fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_owned();
    if out.is_empty() { "model".into() } else { out }
}

/// Ends a session when dropped while armed.
struct EndOnDrop<'a> {
    uploads: &'a Uploads,
    id: &'a str,
    armed: bool,
}

impl Drop for EndOnDrop<'_> {
    fn drop(&mut self) {
        if self.armed {
            let mut guard = self.uploads.lock();
            if guard.as_ref().is_some_and(|s| s.id == self.id) {
                *guard = None;
            }
        }
    }
}

impl Uploads {
    /// No session open.
    pub fn new(store: Arc<ModelStore>) -> Uploads {
        Uploads {
            store,
            session: Mutex::new(None),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Session>> {
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Opens a session; a second one while one is open is a conflict.
    pub fn open(&self) -> Result<UploadSession, TtsError> {
        let mut guard = self.lock();
        // A session nobody is writing to is abandoned (a reload, a closed tab): replace it.
        if guard.as_ref().is_some_and(|s| !s.pending.is_empty()) {
            return Err(TtsError::Conflict("an upload is in progress".into()));
        }
        let mut raw = [0u8; 16];
        getrandom::getrandom(&mut raw).map_err(|e| TtsError::Io(format!("random: {e}")))?;
        let id: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        *guard = Some(Session {
            id: id.clone(),
            staging: self.store.staging()?,
            files: Vec::new(),
            pending: BTreeSet::new(),
        });
        Ok(UploadSession { id })
    }

    /// Streams one file into the session's folder, counting bytes.
    pub async fn file(&self, id: &str, name: &str, body: Body) -> Result<(), TtsError> {
        let is_weight = WEIGHT_FILES.contains(&name);
        if !is_weight && !SMALL_FILES.contains(&name) {
            return Err(TtsError::Invalid(format!("unknown file name: {name}")));
        }
        let cap = if is_weight {
            MAX_WEIGHTS_BYTES
        } else {
            MAX_SMALL_FILE_BYTES
        };
        let path = {
            let mut guard = self.lock();
            let s = guard
                .as_mut()
                .filter(|s| s.id == id)
                .ok_or_else(|| TtsError::NotFound("upload".into()))?;
            if s.files.iter().any(|f| f.name == name) || s.pending.contains(name) {
                return Err(TtsError::Conflict(format!("{name} was already sent")));
            }
            let other_weight = |n: &str| WEIGHT_FILES.contains(&n);
            if is_weight
                && (s.files.iter().any(|f| other_weight(&f.name))
                    || s.pending.iter().any(|n| other_weight(n)))
            {
                return Err(TtsError::Invalid(
                    "a model needs exactly one weight file".into(),
                ));
            }
            s.pending.insert(name.to_owned());
            s.staging.dir.join(name)
        };
        // If this future is dropped (the client went away), the guard ends the session.
        let mut cleanup = EndOnDrop {
            uploads: self,
            id,
            armed: true,
        };
        let result = Self::write(&path, body, cap).await;
        cleanup.armed = false;
        let mut guard = self.lock();
        match result {
            Ok(size) => {
                if let Some(s) = guard.as_mut().filter(|s| s.id == id) {
                    s.pending.remove(name);
                    s.files.push(ManifestFile {
                        name: name.to_owned(),
                        size,
                        sha256: None,
                    });
                }
                Ok(())
            }
            Err(e) => {
                // A failed or oversize file ends the session and deletes its folder.
                if guard.as_ref().is_some_and(|s| s.id == id) {
                    *guard = None;
                }
                Err(e)
            }
        }
    }

    async fn write(path: &std::path::Path, body: Body, cap: u64) -> Result<u64, TtsError> {
        let mut out = tokio::fs::File::create(path).await.map_err(io)?;
        let mut stream = body.into_data_stream();
        let mut got = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| TtsError::Invalid("the upload broke off".into()))?;
            got += chunk.len() as u64;
            if got > cap {
                return Err(TtsError::TooLarge(format!("over {cap} bytes")));
            }
            out.write_all(&chunk).await.map_err(io)?;
        }
        out.flush().await.map_err(io)?;
        Ok(got)
    }

    /// Checks and installs the session's folder as a model.
    pub fn finish(&self, id: &str) -> Result<ModelView, TtsError> {
        let session = {
            let mut guard = self.lock();
            if !guard.as_ref().is_some_and(|s| s.id == id) {
                return Err(TtsError::NotFound("upload".into()));
            }
            if guard.as_ref().is_some_and(|s| !s.pending.is_empty()) {
                return Err(TtsError::Conflict("a file is still arriving".into()));
            }
            guard.take().expect("checked above")
        };
        let weights = session
            .files
            .iter()
            .find(|f| WEIGHT_FILES.contains(&f.name.as_str()))
            .map(|f| f.name.clone())
            .ok_or_else(|| TtsError::Invalid("a model needs a weight file".into()))?;
        let quant = quant_of(&weights).unwrap_or("f32");
        let name = std::fs::read(session.staging.dir.join("config.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| v.get("model_id").and_then(|m| m.as_str()).map(slug))
            .unwrap_or_else(|| "model".into());
        let manifest = Manifest {
            id: self.store.free_id(&format!("upload--{name}--{quant}")),
            repo: None,
            revision: None,
            weights,
            files: session.files.clone(),
            licence: None,
            languages: vec![],
            source: ModelSource::Upload,
            installed_at: chrono::Utc::now().timestamp(),
        };
        let manifest = session.staging.finish(&self.store, manifest)?;
        Ok(manifest.view(Vec::new()))
    }

    /// Drops a session and its folder.
    pub fn abort(&self, id: &str) -> Result<(), TtsError> {
        let mut guard = self.lock();
        if guard.as_ref().is_some_and(|s| s.id == id) {
            *guard = None;
            Ok(())
        } else {
            Err(TtsError::NotFound("upload".into()))
        }
    }
}
