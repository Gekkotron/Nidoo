//! Speech input on disk, under `data/assist/stt/`: `settings.json`, the satellites' shared
//! `key` (owner-only), the Parakeet folder and Silero's `silero_vad.onnx`. No audio is ever
//! written here.

use std::path::{Path, PathBuf};

pub use nidoo_api::SttSettings;

use super::SttError;
use super::packet::Key;

/// The settings file.
pub const SETTINGS_FILE: &str = "settings.json";
/// The shared key file: 64 hex characters, mode 0600.
pub const KEY_FILE: &str = "key";
/// The recognition model's folder.
pub const MODEL_DIR: &str = "parakeet-tdt-0.6b-v3-int8";
/// The end-of-phrase model.
pub const VAD_FILE: &str = "silero_vad.onnx";
/// The files the model folder must hold.
pub const MODEL_FILES: [&str; 4] = [
    "encoder.int8.onnx",
    "decoder.int8.onnx",
    "joiner.int8.onnx",
    "tokens.txt",
];

/// Speech input's folder.
#[derive(Debug)]
pub struct SttStore {
    root: PathBuf,
}

/// Refuses settings out of their ranges.
pub fn check_settings(s: &SttSettings) -> Result<(), SttError> {
    let bad = |what: &str| Err(SttError::Invalid(what.to_owned()));
    if !(1..=16).contains(&s.threads) {
        return bad("threads must be 1 to 16");
    }
    if !(1.0..=16.0).contains(&s.gain) {
        return bad("gain must be 1 to 16");
    }
    if s.port < 1024 {
        return bad("port must be 1024 to 65535");
    }
    if s.listen_margin_ms > 2_000 {
        return bad("the follow-up margin must be 0 to 2000 ms");
    }
    Ok(())
}

impl SttStore {
    /// The store at `root`, created when missing (a failure shows on the first write).
    pub fn open(root: &Path) -> SttStore {
        if let Err(e) = std::fs::create_dir_all(root) {
            tracing::warn!(path = %root.display(), "cannot create the speech input folder: {e}");
        }
        SttStore {
            root: root.to_owned(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The saved settings, or the defaults.
    pub fn settings(&self) -> SttSettings {
        std::fs::read(self.root.join(SETTINGS_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice::<SttSettings>(&b).ok())
            .filter(|s| check_settings(s).is_ok())
            .unwrap_or_default()
    }

    /// Checks and saves the settings.
    pub fn set_settings(&self, s: SttSettings) -> Result<(), SttError> {
        check_settings(&s)?;
        let json = serde_json::to_vec_pretty(&s).map_err(|e| SttError::Io(e.to_string()))?;
        write_atomic(&self.root.join(SETTINGS_FILE), &json, false)
    }

    /// The shared key, when one was made.
    pub fn key(&self) -> Option<Key> {
        let text = std::fs::read_to_string(self.root.join(KEY_FILE)).ok()?;
        Key::from_hex(text.trim())
    }

    /// Makes a new key, replacing the old one.
    pub fn new_key(&self) -> Result<Key, SttError> {
        let key = Key::generate();
        write_atomic(&self.root.join(KEY_FILE), key.to_hex().as_bytes(), true)?;
        Ok(key)
    }

    pub fn model_dir(&self) -> PathBuf {
        self.root.join(MODEL_DIR)
    }

    pub fn vad_path(&self) -> PathBuf {
        self.root.join(VAD_FILE)
    }

    /// Every model file is on disk.
    pub fn installed(&self) -> bool {
        let dir = self.model_dir();
        MODEL_FILES.iter().all(|f| dir.join(f).is_file()) && self.vad_path().is_file()
    }

    /// Removes the model files.
    pub fn remove_model(&self) -> Result<(), SttError> {
        let io = |e: std::io::Error| SttError::Io(format!("remove the model: {e}"));
        match std::fs::remove_dir_all(self.model_dir()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(io(e)),
            _ => {}
        }
        match std::fs::remove_file(self.vad_path()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(io(e)),
            _ => Ok(()),
        }
    }
}

/// Writes `bytes` to `path` through a temporary file; `private` makes it owner-only.
fn write_atomic(path: &Path, bytes: &[u8], private: bool) -> Result<(), SttError> {
    let io = |e: std::io::Error| SttError::Io(format!("write {}: {e}", path.display()));
    let tmp = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    {
        use std::io::Write;
        let mut file = options.open(&tmp).map_err(io)?;
        file.write_all(bytes).map_err(io)?;
        file.sync_all().map_err(io)?;
    }
    std::fs::rename(&tmp, path).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> (tempfile::TempDir, SttStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SttStore::open(&dir.path().join("stt"));
        (dir, store)
    }

    #[test]
    fn settings_default_and_round_trip() {
        let (_d, store) = open();
        assert_eq!(store.settings(), SttSettings::default());
        assert_eq!(
            SttSettings::default(),
            SttSettings {
                threads: 4,
                gain: 4.0,
                port: 4960,
                listen_margin_ms: 300
            }
        );
        let changed = SttSettings {
            threads: 2,
            gain: 6.5,
            port: 5000,
            listen_margin_ms: 0,
        };
        store.set_settings(changed.clone()).unwrap();
        assert_eq!(store.settings(), changed);
    }

    #[test]
    fn settings_out_of_range_are_refused() {
        let (_d, store) = open();
        let ok = SttSettings::default();
        for bad in [
            SttSettings {
                threads: 0,
                ..ok.clone()
            },
            SttSettings {
                threads: 17,
                ..ok.clone()
            },
            SttSettings {
                gain: 0.5,
                ..ok.clone()
            },
            SttSettings {
                gain: f32::NAN,
                ..ok.clone()
            },
            SttSettings {
                port: 80,
                ..ok.clone()
            },
            SttSettings {
                listen_margin_ms: 2001,
                ..ok.clone()
            },
        ] {
            assert!(
                matches!(store.set_settings(bad.clone()), Err(SttError::Invalid(_))),
                "{bad:?}"
            );
        }
        assert_eq!(store.settings(), ok, "nothing was saved");
    }

    #[test]
    fn new_key_replaces_the_old() {
        let (_d, store) = open();
        assert_eq!(store.key(), None);
        let first = store.new_key().unwrap();
        assert_eq!(store.key(), Some(first.clone()));
        let second = store.new_key().unwrap();
        assert_ne!(first, second);
        assert_eq!(store.key(), Some(second));
    }

    #[cfg(unix)]
    #[test]
    fn key_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let (_d, store) = open();
        store.new_key().unwrap();
        let mode = std::fs::metadata(store.root().join(KEY_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn installed_needs_every_file() {
        let (_d, store) = open();
        assert!(!store.installed());
        std::fs::create_dir_all(store.model_dir()).unwrap();
        for name in MODEL_FILES {
            std::fs::write(store.model_dir().join(name), b"x").unwrap();
        }
        assert!(!store.installed(), "no VAD yet");
        std::fs::write(store.vad_path(), b"x").unwrap();
        assert!(store.installed());
        store.remove_model().unwrap();
        assert!(!store.installed());
    }
}
