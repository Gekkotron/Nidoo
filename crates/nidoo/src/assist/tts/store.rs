//! The speech models kept on disk.
//!
//! Layout under the root: `models/<id>/` (files + `manifest.json`),
//! `models/.staging-<random>/` for work in progress, `state.json`, `token`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use nidoo_api::{DEFAULT_VOICE, ModelSource, ModelView, Secret};
use serde::{Deserialize, Serialize};

use super::TtsError;

/// The weight files the engine can run, in order of preference.
pub const WEIGHT_FILES: [&str; 3] = ["model.q8.gguf", "model.q4k.gguf", "model.safetensors"];

/// The small files a model folder needs besides its weights.
pub const SMALL_FILES: [&str; 9] = [
    "config.json",
    "generation_config.json",
    "processor_config.json",
    "tokenizer.json",
    "tokenizer.5.0.0.json",
    "tokenizer.model",
    "tokenizer_config.json",
    "chat_template.jinja",
    "default-voice.safetensors",
];

/// Files a model folder must hold (plus exactly one weight file).
pub const REQUIRED_FILES: [&str; 2] = ["config.json", "tokenizer.json"];

/// The largest weight file accepted, in bytes (200 MiB).
pub const MAX_WEIGHTS_BYTES: u64 = 200 * 1024 * 1024;

/// The largest non-weight file accepted, in bytes (8 MiB).
pub const MAX_SMALL_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// The quantisation a weight file holds, from its name.
pub fn quant_of(weights: &str) -> Option<&'static str> {
    match weights {
        "model.q8.gguf" => Some("q8"),
        "model.q4k.gguf" => Some("q4k"),
        "model.safetensors" => Some("f32"),
        _ => None,
    }
}

/// One file of an installed model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestFile {
    /// The file name inside the model folder.
    pub name: String,
    /// Its size in bytes.
    pub size: u64,
    /// Its SHA-256, when the source gave one.
    pub sha256: Option<String>,
}

/// What is known about an installed model, kept as `manifest.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// The model id, the folder name.
    pub id: String,
    /// The Hugging Face repository, when downloaded.
    pub repo: Option<String>,
    /// The repository revision, when downloaded.
    pub revision: Option<String>,
    /// The weight file in use.
    pub weights: String,
    /// Every file of the model.
    pub files: Vec<ManifestFile>,
    /// The declared licence.
    pub licence: Option<String>,
    /// The languages the model speaks.
    pub languages: Vec<String>,
    /// How the model arrived.
    pub source: ModelSource,
    /// Install time, Unix seconds.
    pub installed_at: i64,
}

impl Manifest {
    /// The settings-screen view of this model; `used_by` lists the languages speaking with it.
    pub fn view(&self, used_by: Vec<String>) -> ModelView {
        ModelView {
            id: self.id.clone(),
            repo: self.repo.clone(),
            revision: self.revision.clone(),
            weights: self.weights.clone(),
            size: self.files.iter().map(|f| f.size).sum(),
            licence: self.licence.clone(),
            languages: self.languages.clone(),
            source: self.source.clone(),
            installed_at: self.installed_at,
            used_by,
        }
    }
}

/// The chosen model and voice, kept as `state.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TtsState {
    /// The languages the hub speaks, the house language first.
    #[serde(default)]
    pub languages: Vec<super::languages::LanguageEntry>,
    /// The single active model of hubs before languages; read once, never written.
    #[serde(default, skip_serializing)]
    pub active: Option<String>,
    /// The single voice of hubs before languages; read once, never written.
    #[serde(default = "default_voice", skip_serializing)]
    pub voice: String,
}

fn default_voice() -> String {
    DEFAULT_VOICE.to_owned()
}

impl Default for TtsState {
    fn default() -> Self {
        Self {
            languages: Vec::new(),
            active: None,
            voice: default_voice(),
        }
    }
}

fn io_err(what: &str, e: std::io::Error) -> TtsError {
    TtsError::Io(format!("{what}: {e}"))
}

/// A model id or file name that cannot escape its folder.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && name != ".." && !name.contains(['/', '\\', '\0'])
}

/// Write `bytes` to `path` through a temp file in the same folder.
fn write_atomic(path: &Path, bytes: &[u8], private: bool) -> Result<(), TtsError> {
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    let _ = fs::remove_file(&tmp);
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let result = opts
        .open(&tmp)
        .and_then(|mut f| f.write_all(bytes).and_then(|()| f.sync_all()))
        .and_then(|()| fs::rename(&tmp, path));
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(io_err("write", e));
    }
    Ok(())
}

fn read_manifest(dir: &Path, id: &str) -> Option<Manifest> {
    let bytes = fs::read(dir.join("manifest.json")).ok()?;
    let m: Manifest = serde_json::from_slice(&bytes).ok()?;
    (m.id == id).then_some(m)
}

/// The speech models, the chosen one and the token, under one root.
#[derive(Debug, Clone)]
pub struct ModelStore {
    root: PathBuf,
}

impl ModelStore {
    /// Open the store, creating its folders and removing leftovers of
    /// interrupted installs. Never fails; problems are logged.
    pub fn open(root: &Path) -> ModelStore {
        let store = ModelStore {
            root: root.to_path_buf(),
        };
        for d in [store.models_dir(), root.join("voices")] {
            if let Err(e) = fs::create_dir_all(&d) {
                tracing::warn!("cannot create {}: {e}", d.display());
            }
        }
        match fs::read_dir(store.models_dir()) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let path = entry.path();
                    let keep = !name.starts_with(".staging-")
                        && path.is_dir()
                        && read_manifest(&path, &name).is_some();
                    if !keep && let Err(e) = remove_any(&path) {
                        tracing::warn!("cannot delete {}: {e}", path.display());
                    }
                }
            }
            Err(e) => tracing::warn!("cannot read {}: {e}", store.models_dir().display()),
        }
        store
    }

    fn models_dir(&self) -> PathBuf {
        self.root.join("models")
    }

    /// Every installed model, sorted by id.
    pub fn list(&self) -> Vec<Manifest> {
        let mut out: Vec<Manifest> = fs::read_dir(self.models_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                is_plain_name(&name)
                    .then(|| read_manifest(&e.path(), &name))
                    .flatten()
            })
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// One installed model.
    pub fn get(&self, id: &str) -> Option<Manifest> {
        if !is_plain_name(id) {
            return None;
        }
        read_manifest(&self.dir(id), id)
    }

    /// The folder of a model id.
    pub fn dir(&self, id: &str) -> PathBuf {
        self.models_dir().join(id)
    }

    /// Delete an installed model.
    pub fn remove(&self, id: &str) -> Result<(), TtsError> {
        if self.get(id).is_none() {
            return Err(TtsError::NotFound(id.to_owned()));
        }
        fs::remove_dir_all(self.dir(id)).map_err(|e| io_err("remove model", e))
    }

    /// A fresh folder for a download or upload in progress.
    pub fn staging(&self) -> Result<Staging, TtsError> {
        let mut raw = [0u8; 8];
        getrandom::getrandom(&mut raw).map_err(|e| TtsError::Io(format!("random: {e}")))?;
        let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        let dir = self.models_dir().join(format!(".staging-{hex}"));
        fs::create_dir_all(&dir).map_err(|e| io_err("create staging", e))?;
        Ok(Staging { dir, done: false })
    }

    /// `base`, or `base-2`, `base-3`… when that id is taken.
    pub fn free_id(&self, base: &str) -> String {
        if !self.dir(base).exists() {
            return base.to_owned();
        }
        (2u32..)
            .map(|n| format!("{base}-{n}"))
            .find(|id| !self.dir(id).exists())
            .unwrap_or_else(|| base.to_owned())
    }

    /// The chosen model and voice; defaults when unreadable.
    pub fn read_state(&self) -> TtsState {
        fs::read(self.root.join("state.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Save the chosen model and voice.
    pub fn write_state(&self, s: &TtsState) -> Result<(), TtsError> {
        let bytes = serde_json::to_vec_pretty(s).map_err(|e| TtsError::Io(e.to_string()))?;
        write_atomic(&self.root.join("state.json"), &bytes, false)
    }

    /// The Hugging Face token, when one is saved.
    pub fn token(&self) -> Option<String> {
        let raw = fs::read_to_string(self.root.join("token")).ok()?;
        let t = raw.trim();
        (!t.is_empty()).then(|| t.to_owned())
    }

    /// Keep, clear or replace the token. An empty value clears it.
    pub fn set_token(&self, input: Secret) -> Result<(), TtsError> {
        let path = self.root.join("token");
        let value = match input {
            Secret::Keep => return Ok(()),
            Secret::Clear => String::new(),
            Secret::Set(v) => v.trim().to_owned(),
        };
        if value.is_empty() {
            return match fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(io_err("remove token", e)),
            };
        }
        write_atomic(&path, value.as_bytes(), true)
    }
}

fn remove_any(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// A model folder being filled. Deleted on drop unless finished.
#[derive(Debug)]
pub struct Staging {
    /// The folder to put the files in.
    pub dir: PathBuf,
    done: bool,
}

impl Staging {
    /// Check the files, write the manifest and move the folder into the
    /// store. On any failure the staging folder is deleted.
    pub fn finish(
        mut self,
        store: &ModelStore,
        mut manifest: Manifest,
    ) -> Result<Manifest, TtsError> {
        if !is_plain_name(&manifest.id) {
            return Err(TtsError::Invalid(format!("bad model id: {}", manifest.id)));
        }
        let target = store.dir(&manifest.id);
        if target.exists() {
            return Err(TtsError::Conflict(format!(
                "model {} is already installed",
                manifest.id
            )));
        }
        for f in &manifest.files {
            if !is_plain_name(&f.name) {
                return Err(TtsError::Invalid(format!("bad file name: {}", f.name)));
            }
            let len = fs::metadata(self.dir.join(&f.name))
                .map_err(|_| TtsError::Invalid(format!("missing file: {}", f.name)))?
                .len();
            if len != f.size {
                return Err(TtsError::Invalid(format!(
                    "{} is {len} bytes, expected {}",
                    f.name, f.size
                )));
            }
        }
        for name in REQUIRED_FILES {
            if !self.dir.join(name).is_file() {
                return Err(TtsError::Invalid(format!("missing required file: {name}")));
            }
        }
        let present: Vec<&str> = WEIGHT_FILES
            .iter()
            .copied()
            .filter(|n| self.dir.join(n).is_file())
            .collect();
        if present != [manifest.weights.as_str()] {
            return Err(TtsError::Invalid(
                "a model needs exactly one weight file".to_owned(),
            ));
        }
        if let Ok(bytes) = fs::read(self.dir.join("config.json"))
            && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes)
            && let Some(list) = v.get("languages").and_then(|l| l.as_array())
        {
            manifest.languages = list
                .iter()
                .filter_map(|l| l.as_str().map(str::to_owned))
                .collect();
        }
        let bytes =
            serde_json::to_vec_pretty(&manifest).map_err(|e| TtsError::Io(e.to_string()))?;
        write_atomic(&self.dir.join("manifest.json"), &bytes, false)?;
        fs::rename(&self.dir, &target).map_err(|e| io_err("install model", e))?;
        self.done = true;
        Ok(manifest)
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.done
            && let Err(e) = fs::remove_dir_all(&self.dir)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("cannot delete {}: {e}", self.dir.display());
        }
    }
}

#[cfg(test)]
mod tests;
