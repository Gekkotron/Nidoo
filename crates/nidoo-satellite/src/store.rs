//! The satellite's own files: its Sendspin identity and its settings.
//!
//! Both live in one directory (on a Mac, `~/Library/Application Support/Nidoo Satellite/`),
//! every file mode `0600`. Keep `player.key`: a new key makes the satellite a stranger to
//! `nidoo-sound`, which then lists it as a new player to Allow and place in a room.

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use anyhow::Context;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use nidoo_voice::Key;
use serde::{Deserialize, Serialize};

const PLAYER_KEY: &str = "player.key";
const SETTINGS: &str = "settings.json";

/// The satellite's Sendspin identity: a Curve25519 keypair made once and kept.
#[derive(Clone)]
pub struct PlayerIdentity {
    /// The base64url public key, as `client/init` sends it.
    pub client_id: String,
    /// `nidoo-sound`'s short id for this player (`nidoo_sound::state::short_id`).
    pub sid: String,
    private: [u8; 32],
}

impl std::fmt::Debug for PlayerIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlayerIdentity")
            .field("client_id", &self.client_id)
            .field("sid", &self.sid)
            .finish_non_exhaustive()
    }
}

impl PlayerIdentity {
    fn from_private(private: [u8; 32]) -> Self {
        let secret = x25519_dalek::StaticSecret::from(private);
        let public = x25519_dalek::PublicKey::from(&secret).to_bytes();
        let client_id = URL_SAFE_NO_PAD.encode(public);
        Self {
            sid: nidoo_sound::state::short_id(&client_id),
            client_id,
            private,
        }
    }

    /// The static private key, for the Noise handshake.
    pub fn private_key(&self) -> [u8; 32] {
        self.private
    }
}

/// What the owner chose in the menu.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// The house's shared voice key (`Key`'s `Debug` never shows it).
    pub voice_key: Option<Key>,
    /// The microphone switch.
    pub listening: bool,
    /// Overrides the wake-word threshold when set.
    pub wake_threshold: Option<f32>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            voice_key: None,
            listening: true,
            wake_threshold: None,
        }
    }
}

/// `settings.json` as stored: the key as hex.
#[derive(Serialize, Deserialize)]
struct SettingsFile {
    #[serde(default)]
    voice_key: Option<String>,
    #[serde(default = "yes")]
    listening: bool,
    #[serde(default)]
    wake_threshold: Option<f32>,
}

fn yes() -> bool {
    true
}

/// Why `set_voice_key` refused the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("that is not a voice key (64 hex characters)")]
    NotAKey,
    #[error("the key could not be saved")]
    NotSaved,
}

/// The satellite's directory.
#[derive(Debug)]
pub struct Store {
    dir: PathBuf,
    identity: PlayerIdentity,
    settings: Settings,
}

impl Store {
    /// Opens `dir` (created `0700` if missing), making the identity on first use.
    pub fn open(dir: &Path) -> anyhow::Result<Store> {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .with_context(|| format!("creating {}", dir.display()))?;
        let identity = load_or_create_identity(&dir.join(PLAYER_KEY))?;
        let settings = read_settings(&dir.join(SETTINGS))?;
        Ok(Store {
            dir: dir.to_path_buf(),
            identity,
            settings,
        })
    }

    /// The directory the files live in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn identity(&self) -> &PlayerIdentity {
        &self.identity
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Switches the microphone and keeps the choice.
    pub fn set_listening(&mut self, on: bool) -> anyhow::Result<()> {
        let mut next = self.settings.clone();
        next.listening = on;
        self.save(next)
    }

    /// Stores the key in `text` (bare hex or the hub's `voice_key: "…"` line). Junk is
    /// refused and the old key kept.
    pub fn set_voice_key(&mut self, text: &str) -> Result<(), KeyError> {
        let key = parse_voice_key(text).ok_or(KeyError::NotAKey)?;
        let mut next = self.settings.clone();
        next.voice_key = Some(key);
        self.save(next).map_err(|_| KeyError::NotSaved)
    }

    fn save(&mut self, next: Settings) -> anyhow::Result<()> {
        write_settings(&self.dir.join(SETTINGS), &next)?;
        self.settings = next;
        Ok(())
    }
}

/// The key in `text`: 64 hex characters, alone or as the hub's `voice_key: "…"` line.
pub fn parse_voice_key(text: &str) -> Option<Key> {
    let text = text.trim();
    let value = text.strip_prefix("voice_key:").unwrap_or(text).trim();
    let value = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(value);
    Key::from_hex(value)
}

fn load_or_create_identity(path: &Path) -> anyhow::Result<PlayerIdentity> {
    let private: [u8; 32] = match std::fs::read(path) {
        Ok(bytes) => bytes.try_into().map_err(|b: Vec<u8>| {
            anyhow::anyhow!("{} holds {} bytes, expected 32", path.display(), b.len())
        })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let key: [u8; 32] = rand::random();
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .with_context(|| format!("creating {}", path.display()))?;
            f.write_all(&key)?;
            f.sync_all()?;
            key
        }
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    Ok(PlayerIdentity::from_private(private))
}

fn read_settings(path: &Path) -> anyhow::Result<Settings> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let file: SettingsFile =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Settings {
        voice_key: file.voice_key.as_deref().and_then(Key::from_hex),
        listening: file.listening,
        wake_threshold: file.wake_threshold,
    })
}

/// Writes `settings` to a `0600` temp file next to `path`, then renames it over `path`.
fn write_settings(path: &Path, settings: &Settings) -> anyhow::Result<()> {
    let file = SettingsFile {
        voice_key: settings.voice_key.as_ref().map(Key::to_hex),
        listening: settings.listening,
        wake_threshold: settings.wake_threshold,
    };
    let body = serde_json::to_vec_pretty(&file)?;
    write_private_atomic(path, &body).with_context(|| format!("writing {}", path.display()))
}

/// Replaces `path` with `bytes` through `<path>.tmp` and one rename; both are `0600`
/// from the start.
pub(crate) fn write_private_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    #[test]
    fn identity_is_stable_across_opens() {
        let dir = tempfile::tempdir().unwrap();
        let a = Store::open(dir.path()).unwrap();
        let b = Store::open(dir.path()).unwrap();
        assert_eq!(a.identity().client_id, b.identity().client_id);
        assert_eq!(a.identity().sid, b.identity().sid);
        assert_eq!(a.identity().private_key(), b.identity().private_key());
        assert_eq!(a.identity().client_id.len(), 43);
        assert_eq!(
            a.identity().sid,
            nidoo_sound::state::short_id(&a.identity().client_id)
        );
    }

    #[test]
    fn files_are_0600() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        store.set_listening(false).unwrap();
        for name in ["player.key", "settings.json"] {
            let mode = std::fs::metadata(dir.path().join(name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{name}");
        }
    }

    #[test]
    fn parse_voice_key_accepts_bare_hex_and_yaml_line() {
        let want = nidoo_voice::Key::from_hex(HEX).unwrap();
        assert_eq!(parse_voice_key(HEX), Some(want.clone()));
        assert_eq!(
            parse_voice_key(&format!("voice_key: \"{HEX}\"")),
            Some(want.clone())
        );
        assert_eq!(parse_voice_key(&format!("  \n{HEX}\n")), Some(want.clone()));
        assert_eq!(
            parse_voice_key(&format!("voice_key: '{HEX}'\n")),
            Some(want.clone())
        );
        assert_eq!(parse_voice_key(&HEX.to_uppercase()), Some(want));
    }

    #[test]
    fn parse_voice_key_rejects_junk() {
        for junk in [
            "",
            &HEX[..63],
            &format!("{HEX}0"),
            "hello",
            "https://example.com/voice_key",
            "voice_key: \"\"",
        ] {
            assert_eq!(parse_voice_key(junk), None, "{junk:?}");
        }
    }

    #[test]
    fn set_voice_key_keeps_old_on_junk() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        assert_eq!(store.settings().voice_key, None);
        store.set_voice_key(HEX).unwrap();
        assert_eq!(store.set_voice_key("junk"), Err(KeyError::NotAKey));
        let want = nidoo_voice::Key::from_hex(HEX);
        assert_eq!(store.settings().voice_key, want);
        assert_eq!(Store::open(dir.path()).unwrap().settings().voice_key, want);
    }

    #[test]
    fn listening_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Store::open(dir.path()).unwrap().settings().listening);
        Store::open(dir.path())
            .unwrap()
            .set_listening(false)
            .unwrap();
        assert!(!Store::open(dir.path()).unwrap().settings().listening);
    }

    #[test]
    fn the_key_is_never_in_debug_output() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        store.set_voice_key(HEX).unwrap();
        let shown = format!("{store:?}");
        assert!(!shown.contains(HEX), "{shown}");
    }
}
