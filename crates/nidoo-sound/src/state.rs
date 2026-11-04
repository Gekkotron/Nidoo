//! Each satellite's volume, mute and approval, remembered across restarts in
//! `<data_dir>/sound/players.json` (written atomically: temp file + rename).
//!
//! Satellites are keyed by their Sendspin `client_id`; MQTT topics use the
//! short id (first 12 hex chars of its SHA-256).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const DEFAULT_VOLUME: u8 = 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerPrefs {
    pub client_id: String,
    pub name: String,
    pub volume: u8,
    pub muted: bool,
    /// Approved by the operator: only allowed players get audio.
    pub allowed: bool,
}

/// First 12 hex characters of SHA-256(`client_id`): topic-safe and stable.
pub fn short_id(client_id: &str) -> String {
    let digest = Sha256::digest(client_id.as_bytes());
    digest[..6].iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    players: Vec<PlayerPrefs>,
}

#[derive(Debug)]
pub struct StateStore {
    path: PathBuf,
    players: BTreeMap<String, PlayerPrefs>,
}

impl StateStore {
    /// Loads the file; a missing file is an empty store, an unreadable one an
    /// error (the file is left untouched, never reset).
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let players = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<File>(&bytes)
                .with_context(|| format!("{} is not a valid players file", path.display()))?
                .players
                .into_iter()
                .map(|p| (p.client_id.clone(), p))
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(Self { path, players })
    }

    pub fn get(&self, client_id: &str) -> Option<&PlayerPrefs> {
        self.players.get(client_id)
    }

    pub fn by_short(&self, short: &str) -> Option<&PlayerPrefs> {
        self.players
            .values()
            .find(|p| short_id(&p.client_id) == short)
    }

    pub fn all(&self) -> impl Iterator<Item = &PlayerPrefs> {
        self.players.values()
    }

    /// Records a connected player: creates it with defaults (volume 30, not
    /// muted, not allowed) or updates its name. Saves on change; a failed
    /// save is logged and retried with the next change.
    pub fn observe(&mut self, client_id: &str, name: &str) -> &PlayerPrefs {
        let changed = match self.players.get_mut(client_id) {
            Some(p) if p.name == name => false,
            Some(p) => {
                p.name = name.to_owned();
                true
            }
            None => {
                self.players.insert(
                    client_id.to_owned(),
                    PlayerPrefs {
                        client_id: client_id.to_owned(),
                        name: name.to_owned(),
                        volume: DEFAULT_VOLUME,
                        muted: false,
                        allowed: false,
                    },
                );
                true
            }
        };
        if changed && let Err(e) = self.save() {
            tracing::error!(error = %format!("{e:#}"), "saving player prefs");
        }
        &self.players[client_id]
    }

    /// Changes one player's prefs and saves. `None` for an unknown short id.
    pub fn update(
        &mut self,
        short: &str,
        f: impl FnOnce(&mut PlayerPrefs),
    ) -> anyhow::Result<Option<PlayerPrefs>> {
        let Some(p) = self
            .players
            .values_mut()
            .find(|p| short_id(&p.client_id) == short)
        else {
            return Ok(None);
        };
        f(p);
        let p = p.clone();
        self.save()?;
        Ok(Some(p))
    }

    fn save(&self) -> anyhow::Result<()> {
        let dir = self
            .path
            .parent()
            .context("players file has no directory")?;
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let file = File {
            players: self.players.values().cloned().collect(),
        };
        let tmp = self.path.with_extension("json.tmp");
        {
            let mut f = std::fs::File::create(&tmp)
                .with_context(|| format!("creating {}", tmp.display()))?;
            serde_json::to_writer_pretty(&mut f, &file)?;
            f.write_all(b"\n")?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("replacing {}", self.path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "9kDIII64_oap3z6x5vV92ffB6OvJ6WU5778PBR5u-Bs";

    fn store() -> (tempfile::TempDir, StateStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = StateStore::load(dir.path().join("sound/players.json")).unwrap();
        (dir, s)
    }

    #[test]
    fn defaults_for_new_player() {
        let (_d, mut s) = store();
        let p = s.observe(ID, "Kitchen").clone();
        assert_eq!((p.volume, p.muted, p.allowed), (30, false, false));
        assert_eq!(p.name, "Kitchen");
        assert_eq!(s.get(ID), Some(&p));
        assert_eq!(s.by_short(&short_id(ID)), Some(&p));
    }

    #[test]
    fn observe_updates_the_name_and_keeps_prefs() {
        let (_d, mut s) = store();
        s.observe(ID, "Kitchen");
        s.update(&short_id(ID), |p| p.volume = 70).unwrap();
        let p = s.observe(ID, "Living room");
        assert_eq!((p.name.as_str(), p.volume), ("Living room", 70));
    }

    #[test]
    fn short_id_is_12_hex() {
        use sha2::{Digest, Sha256};
        let want = hex::encode(Sha256::digest(ID.as_bytes()))[..12].to_owned();
        assert_eq!(short_id(ID), want);
        assert_eq!(short_id(ID).len(), 12);
    }

    #[test]
    fn save_and_reload_round_trip() {
        let (d, mut s) = store();
        s.observe(ID, "Kitchen");
        let p = s
            .update(&short_id(ID), |p| {
                p.volume = 55;
                p.muted = true;
                p.allowed = true;
            })
            .unwrap()
            .unwrap();
        assert_eq!((p.volume, p.muted, p.allowed), (55, true, true));
        let again = StateStore::load(d.path().join("sound/players.json")).unwrap();
        assert_eq!(again.get(ID), Some(&p));
        assert_eq!(again.all().count(), 1);
    }

    #[test]
    fn update_of_unknown_player_is_none() {
        let (_d, mut s) = store();
        assert_eq!(s.update("000000000000", |p| p.volume = 1).unwrap(), None);
    }

    #[test]
    fn corrupt_file_is_an_error_not_a_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("players.json");
        std::fs::write(&path, b"{not json").unwrap();
        assert!(StateStore::load(path.clone()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{not json");
    }

    #[test]
    fn save_leaves_no_temp_file() {
        let (d, mut s) = store();
        s.observe(ID, "Kitchen");
        let names: Vec<_> = std::fs::read_dir(d.path().join("sound"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["players.json"]);
    }
}
