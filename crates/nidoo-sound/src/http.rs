//! HTTP API: lists the approved satellites and sets their volume and mute.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use serde::Serialize;

use crate::group::Published;
use crate::state::{PlayerPrefs, short_id};

/// One approved satellite, as the API shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SatelliteView {
    pub id: String,
    pub name: String,
    pub volume: u8,
    pub muted: bool,
    pub connected: bool,
}

/// The satellites known to the group, kept up to date from its `Published` events.
#[derive(Clone, Default)]
pub struct Satellites {
    inner: Arc<RwLock<BTreeMap<String, (PlayerPrefs, bool)>>>,
}

impl Satellites {
    pub fn apply(&self, msg: &Published) {
        let mut map = self.inner.write().unwrap();
        match msg {
            Published::Player {
                short_id,
                prefs,
                connected,
                ..
            } => {
                map.insert(short_id.clone(), (prefs.clone(), *connected));
            }
            Published::Removed { prefs } => {
                map.remove(&short_id(&prefs.client_id));
            }
            Published::Spoken { .. } => {}
        }
    }

    /// The approved satellites, sorted by name and then id.
    pub fn list(&self) -> Vec<SatelliteView> {
        let map = self.inner.read().unwrap();
        let mut out: Vec<SatelliteView> = map
            .iter()
            .filter(|(_, (prefs, _))| prefs.allowed)
            .map(|(id, (prefs, connected))| SatelliteView {
                id: id.clone(),
                name: prefs.name.clone(),
                volume: prefs.volume,
                muted: prefs.muted,
                connected: *connected,
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        out
    }

    pub fn is_approved(&self, id: &str) -> bool {
        self.inner
            .read()
            .unwrap()
            .get(id)
            .is_some_and(|(prefs, _)| prefs.allowed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs(client_id: &str, name: &str, allowed: bool) -> PlayerPrefs {
        PlayerPrefs {
            client_id: client_id.to_owned(),
            name: name.to_owned(),
            volume: 45,
            muted: false,
            allowed,
        }
    }

    fn player(prefs: PlayerPrefs, connected: bool) -> Published {
        Published::Player {
            short_id: short_id(&prefs.client_id),
            prefs,
            connected,
            address: None,
        }
    }

    #[test]
    fn list_keeps_only_approved_sorted_by_name() {
        let s = Satellites::default();
        s.apply(&player(prefs("living-id", "Living room", true), true));
        s.apply(&player(prefs("kitchen-id", "Kitchen", true), true));
        s.apply(&player(prefs("garage-id", "Garage", false), true));
        let list = s.list();
        let names: Vec<&str> = list.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["Kitchen", "Living room"]);
        assert_eq!(list[0].id, short_id("kitchen-id"));
    }

    #[test]
    fn player_update_replaces_the_entry() {
        let s = Satellites::default();
        s.apply(&player(prefs("kitchen-id", "Kitchen", true), true));
        let mut p = prefs("kitchen-id", "Kitchen", true);
        p.volume = 70;
        p.muted = true;
        s.apply(&player(p, false));
        let list = s.list();
        assert_eq!(list.len(), 1);
        assert_eq!(
            (list[0].volume, list[0].muted, list[0].connected),
            (70, true, false)
        );
    }

    #[test]
    fn removed_and_unapproved_leave_the_list() {
        let s = Satellites::default();
        let kitchen = prefs("kitchen-id", "Kitchen", true);
        s.apply(&player(kitchen.clone(), true));
        s.apply(&Published::Removed {
            prefs: kitchen.clone(),
        });
        assert!(s.list().is_empty());

        s.apply(&player(kitchen.clone(), true));
        assert!(s.is_approved(&short_id("kitchen-id")));
        let mut off = kitchen;
        off.allowed = false;
        s.apply(&player(off, true));
        assert!(s.list().is_empty());
        assert!(!s.is_approved(&short_id("kitchen-id")));
    }
}
