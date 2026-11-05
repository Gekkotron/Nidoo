//! Saying a sentence on the speaker satellites published by the `nidoo-sound` service.
//!
//! The hub turns "room X", "every room" or "this satellite" into the short ids of the
//! satellites that can speak (Allowed and Connected both on, the service not offline), has the sentence synthesised by
//! [`Tts::speak`](crate::assist::tts::Tts::speak), then asks `nidoo-sound` to play the clip on
//! those satellites with one message on [`SPEAK_TOPIC`]. A muted satellite counts as able: it
//! stays silent, as it does for music.

use nidoo_api::SpeechRequest;
use nidoo_core::command::Publish;
use nidoo_core::model::{Availability, SOUND_DEVICE_PREFIX, StateValue, UNASSIGNED_AREA};
use nidoo_core::registry::Registry;

use crate::assist::tts::TtsError;
use crate::hub::{Hub, HubError};

/// The topic `nidoo-sound` reads sentences from; not retained (a sentence is said once).
pub const SPEAK_TOPIC: &str = "nidoo-sound/speak";
/// Holds the music of some satellites while someone talks to the house; not retained.
pub const HOLD_TOPIC: &str = "nidoo-sound/hold";
/// `nidoo-sound` reports a sentence finished playing (or was dropped); not retained.
pub const SPOKEN_TOPIC: &str = "nidoo-sound/spoken";

/// The command topic that makes `nidoo-sound` forget satellite `sid`: its saved prefs and its
/// retained messages (the hub then drops its entities). Not retained.
pub fn remove_topic(sid: &str) -> String {
    format!("nidoo-sound/{sid}/remove/set")
}

/// Why a satellite was not removed.
#[derive(Debug, thiserror::Error)]
pub enum RemoveError {
    #[error("unknown satellite `{0}`")]
    UnknownSatellite(String),
    #[error("the sound service is offline")]
    ServiceOffline,
    #[error(transparent)]
    Publish(HubError),
}

/// A [`SPOKEN_TOPIC`] report.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Spoken {
    pub id: String,
    pub played: bool,
}

/// The report in a [`SPOKEN_TOPIC`] payload.
pub fn parse_spoken(payload: &[u8]) -> Option<Spoken> {
    serde_json::from_slice(payload).ok()
}

/// Where a sentence is said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeakTarget {
    /// Every satellite of the house.
    All,
    /// The satellites of one room, by area id or name.
    Room(String),
    /// One satellite, by short id.
    Satellite(String),
}

/// Why a sentence was not sent.
#[derive(Debug, thiserror::Error)]
pub enum SpeakError {
    #[error("unknown room `{0}`")]
    UnknownRoom(String),
    #[error("no satellite can speak in this room")]
    NoSatelliteInRoom,
    #[error("no satellite can speak in the house")]
    NoSatellite,
    #[error("this satellite cannot speak now")]
    SatelliteCannotSpeak,
    #[error("speech is not set up on this hub")]
    NotSetUp,
    #[error(transparent)]
    Tts(TtsError),
    #[error(transparent)]
    Publish(HubError),
}

/// The short ids of the satellites of `target` that can speak now, sorted.
pub fn speaking_satellites(
    reg: &Registry,
    target: &SpeakTarget,
) -> Result<Vec<String>, SpeakError> {
    let room = match target {
        SpeakTarget::Room(raw) => {
            let resolved = reg.resolve_area(raw);
            let known = reg
                .effective_areas()
                .iter()
                .any(|area| area.id == resolved && area.id != UNASSIGNED_AREA);
            if !known {
                return Err(SpeakError::UnknownRoom(raw.clone()));
            }
            Some(resolved)
        }
        SpeakTarget::All | SpeakTarget::Satellite(_) => None,
    };
    // An offline entity (the sound service is down: its last will) keeps its retained ON.
    let is_on = |id: &str| {
        reg.state(id).is_some_and(|state| {
            state.value == StateValue::OnOff(true) && state.availability != Availability::Offline
        })
    };
    let mut players: Vec<String> = reg
        .entities()
        .filter_map(|entity| {
            let device = entity.device_id.as_deref()?;
            let sid = device.strip_prefix(SOUND_DEVICE_PREFIX)?;
            (entity.unique_id == format!("{device}_volume")).then_some((device, sid, entity))
        })
        .filter(|(_, sid, entity)| match (target, &room) {
            (SpeakTarget::Satellite(wanted), _) => sid == wanted,
            (_, Some(room)) => &entity.area == room,
            _ => true,
        })
        .filter(|(device, _, _)| {
            is_on(&format!("{device}_allowed")) && is_on(&format!("{device}_connected"))
        })
        .map(|(_, sid, _)| sid.to_owned())
        .collect();
    players.sort();
    players.dedup();
    if players.is_empty() {
        return Err(match target {
            SpeakTarget::All => SpeakError::NoSatellite,
            SpeakTarget::Room(_) => SpeakError::NoSatelliteInRoom,
            SpeakTarget::Satellite(_) => SpeakError::SatelliteCannotSpeak,
        });
    }
    Ok(players)
}

/// The room of satellite `sid`: its Volume's area, `None` when it has none.
pub fn room_of(reg: &Registry, sid: &str) -> Option<String> {
    reg.entity(&format!("{SOUND_DEVICE_PREFIX}{sid}_volume"))
        .map(|e| e.area.clone())
        .filter(|area| area != UNASSIGNED_AREA)
}

/// The satellites whose music is held while `sid` listens: every Allowed one of its room
/// (muted or offline ones too, harmlessly), or `sid` alone when it has no room.
pub fn room_satellites(reg: &Registry, sid: &str) -> Vec<String> {
    let Some(room) = room_of(reg, sid) else {
        return vec![sid.to_owned()];
    };
    let allowed = |device: &str| {
        reg.state(&format!("{device}_allowed"))
            .is_some_and(|s| s.value == StateValue::OnOff(true))
    };
    let mut players: Vec<String> = reg
        .entities()
        .filter_map(|entity| {
            let device = entity.device_id.as_deref()?;
            let other = device.strip_prefix(SOUND_DEVICE_PREFIX)?;
            (entity.unique_id == format!("{device}_volume")
                && entity.area == room
                && allowed(device))
            .then(|| other.to_owned())
        })
        .collect();
    if !players.iter().any(|p| p == sid) {
        players.push(sid.to_owned());
    }
    players.sort();
    players.dedup();
    players
}

/// The satellites the hub greets for speech input: able to speak (Allowed, Connected, the
/// service up) and with a known IPv4 address.
pub fn voice_satellites(reg: &Registry) -> Vec<(String, std::net::Ipv4Addr)> {
    speaking_satellites(reg, &SpeakTarget::All)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|sid| {
            let state = reg.state(&format!("{SOUND_DEVICE_PREFIX}{sid}_address"))?;
            let StateValue::Text(text) = &state.value else {
                return None;
            };
            let ip = text.parse().ok()?;
            Some((sid, ip))
        })
        .collect()
}

impl Hub {
    /// Asks `nidoo-sound` to forget satellite `sid` and clears the overrides (room, name) of
    /// its entities, so a board that comes back starts like a first install. Refused while the
    /// sound service is offline: the command is not retained and would be lost.
    pub async fn remove_satellite(&self, sid: &str) -> Result<(), RemoveError> {
        let device = format!("{SOUND_DEVICE_PREFIX}{sid}");
        let (entities, offline) = {
            let reg = self.read();
            let entities: Vec<(String, bool)> = reg
                .entities()
                .filter(|e| e.device_id.as_deref() == Some(device.as_str()))
                .map(|e| {
                    (
                        e.unique_id.clone(),
                        reg.raw_override(&e.unique_id).is_some(),
                    )
                })
                .collect();
            let offline = entities.iter().any(|(id, _)| {
                reg.state(id)
                    .is_some_and(|s| s.availability == Availability::Offline)
            });
            (entities, offline)
        };
        if entities.is_empty() {
            return Err(RemoveError::UnknownSatellite(sid.to_owned()));
        }
        if offline {
            return Err(RemoveError::ServiceOffline);
        }
        self.publish(Publish {
            topic: remove_topic(sid),
            payload: "PRESS".to_owned(),
            retain: false,
        })
        .map_err(RemoveError::Publish)?;
        for (id, _) in entities.iter().filter(|(_, has)| *has) {
            if let Err(err) = self.clear_override(id).await {
                tracing::warn!(%id, %err, "clearing a removed satellite's override");
            }
        }
        Ok(())
    }

    /// Says `text` on the satellites of `target`; the number of satellites it was sent to.
    pub async fn speak_in(
        &self,
        target: SpeakTarget,
        text: &str,
        language: Option<String>,
        voice: Option<String>,
    ) -> Result<usize, SpeakError> {
        let (_, players) = self.speak_clip(target, text, language, voice).await?;
        Ok(players.len())
    }

    /// [`Hub::speak_in`], returning the clip id (which `nidoo-sound` reports once played)
    /// and the satellites.
    pub async fn speak_clip(
        &self,
        target: SpeakTarget,
        text: &str,
        language: Option<String>,
        voice: Option<String>,
    ) -> Result<(String, Vec<String>), SpeakError> {
        let players = speaking_satellites(&self.read(), &target)?;
        let tts = self.tts().cloned().ok_or(SpeakError::NotSetUp)?;
        let ticket = tts
            .speak(SpeechRequest {
                text: text.to_owned(),
                language,
                voice,
            })
            .await
            .map_err(SpeakError::Tts)?;
        let payload = serde_json::json!({
            "id": ticket.id,
            "url": ticket.url,
            "players": players,
        });
        self.publish(Publish {
            topic: SPEAK_TOPIC.to_owned(),
            payload: payload.to_string(),
            retain: false,
        })
        .map_err(SpeakError::Publish)?;
        Ok((ticket.id, players))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_core::registry::RegistryConfig;
    use serde_json::json;

    const AT: u64 = 1_700_000_000;

    fn registry() -> Registry {
        Registry::new(RegistryConfig {
            discovery_prefixes: vec!["nidoo".to_owned()],
            ui_prefix: "nidoo/ui".to_owned(),
            z2m_base: "zigbee2mqtt".to_owned(),
            reserved: vec!["ui".to_owned(), "automation".to_owned()],
        })
    }

    fn feed(reg: &mut Registry, topic: &str, payload: &str) {
        let inbound = reg.route(topic, payload.as_bytes().to_vec(), AT);
        reg.apply(inbound);
    }

    /// A satellite as `nidoo-sound` announces it, in `area`, with its switches as given.
    fn satellite(
        reg: &mut Registry,
        sid: &str,
        area: &str,
        allowed: bool,
        connected: bool,
        mute: bool,
    ) {
        let device = format!("{SOUND_DEVICE_PREFIX}{sid}");
        for (component, object) in [
            ("number", "volume"),
            ("switch", "mute"),
            ("switch", "allowed"),
            ("binary_sensor", "connected"),
        ] {
            let mut config = json!({
                "name": object,
                "unique_id": format!("{device}_{object}"),
                "state_topic": format!("nidoo-sound/{sid}/{object}"),
                "payload_on": "ON",
                "payload_off": "OFF",
                "availability_topic": "nidoo-sound/status",
                "device": { "identifiers": [device], "name": sid },
            });
            if component != "binary_sensor" {
                config["command_topic"] = json!(format!("nidoo-sound/{sid}/{object}/set"));
            }
            feed(
                reg,
                &format!("nidoo/{component}/{device}/{object}/config"),
                &config.to_string(),
            );
            feed(
                reg,
                &format!("nidoo/ui/{device}_{object}"),
                &json!({ "area": area }).to_string(),
            );
        }
        let on = |b: bool| if b { "ON" } else { "OFF" };
        feed(reg, &format!("nidoo-sound/{sid}/volume"), "40");
        feed(reg, &format!("nidoo-sound/{sid}/mute"), on(mute));
        feed(reg, &format!("nidoo-sound/{sid}/allowed"), on(allowed));
        feed(reg, &format!("nidoo-sound/{sid}/connected"), on(connected));
    }

    /// Two declared rooms; `a` and `muted` in the living room, `b` in the kitchen, one not
    /// allowed and one disconnected.
    fn house() -> Registry {
        let mut reg = registry();
        feed(
            &mut reg,
            "nidoo/ui/_areas",
            r#"{"areas":[{"id":"sejour","name":"Séjour","order":0},{"id":"kitchen","name":"Kitchen","order":1}]}"#,
        );
        feed(&mut reg, "nidoo-sound/status", "online");
        satellite(&mut reg, "a", "sejour", true, true, false);
        satellite(&mut reg, "muted", "sejour", true, true, true);
        satellite(&mut reg, "b", "kitchen", true, true, false);
        satellite(&mut reg, "refused", "kitchen", false, true, false);
        satellite(&mut reg, "gone", "sejour", true, false, false);
        reg
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn speaking_satellites_all_rooms_picks_every_able_one() {
        let got = speaking_satellites(&house(), &SpeakTarget::All).unwrap();
        assert_eq!(got, ids(&["a", "b", "muted"]));
    }

    #[test]
    fn speaking_satellites_room_picks_only_its_own() {
        let got = speaking_satellites(&house(), &SpeakTarget::Room("kitchen".into())).unwrap();
        assert_eq!(got, ids(&["b"]));
    }

    #[test]
    fn speaking_satellites_room_by_name_or_id() {
        let reg = house();
        for raw in ["Séjour", "séjour", "sejour"] {
            let got = speaking_satellites(&reg, &SpeakTarget::Room(raw.into())).unwrap();
            assert_eq!(got, ids(&["a", "muted"]), "{raw}");
        }
    }

    #[test]
    fn speaking_satellites_one_satellite() {
        let reg = house();
        let got = speaking_satellites(&reg, &SpeakTarget::Satellite("b".into())).unwrap();
        assert_eq!(got, ids(&["b"]));
        let refused = speaking_satellites(&reg, &SpeakTarget::Satellite("refused".into()));
        assert!(
            matches!(refused, Err(SpeakError::SatelliteCannotSpeak)),
            "{refused:?}"
        );
        assert_eq!(
            refused.unwrap_err().to_string(),
            "this satellite cannot speak now"
        );
    }

    #[test]
    fn speaking_satellites_unknown_room() {
        let got = speaking_satellites(&house(), &SpeakTarget::Room("attic".into()));
        assert!(
            matches!(got, Err(SpeakError::UnknownRoom(ref r)) if r == "attic"),
            "{got:?}"
        );
    }

    #[test]
    fn speaking_satellites_none_while_the_sound_service_is_down() {
        // The service's last will marks every satellite entity offline; their retained
        // Allowed and Connected still read ON.
        let mut reg = house();
        feed(&mut reg, "nidoo-sound/status", "offline");
        let got = speaking_satellites(&reg, &SpeakTarget::All);
        assert!(matches!(got, Err(SpeakError::NoSatellite)), "{got:?}");
    }

    #[test]
    fn speaking_satellites_nothing_left_is_an_error() {
        let mut reg = house();
        feed(&mut reg, "nidoo-sound/b/connected", "OFF");
        let got = speaking_satellites(&reg, &SpeakTarget::Room("Kitchen".into()));
        assert!(matches!(got, Err(SpeakError::NoSatelliteInRoom)), "{got:?}");
        let empty = speaking_satellites(&registry(), &SpeakTarget::All);
        assert!(matches!(empty, Err(SpeakError::NoSatellite)), "{empty:?}");
        assert_eq!(
            empty.unwrap_err().to_string(),
            "no satellite can speak in the house"
        );
    }

    fn with_address(reg: &mut Registry, sid: &str, address: &str) {
        let device = format!("{SOUND_DEVICE_PREFIX}{sid}");
        let config = json!({
            "name": "address",
            "unique_id": format!("{device}_address"),
            "state_topic": format!("nidoo-sound/{sid}/address"),
            "availability_topic": "nidoo-sound/status",
            "entity_category": "diagnostic",
            "device": { "identifiers": [device], "name": sid },
        });
        feed(
            reg,
            &format!("nidoo/sensor/{device}/address/config"),
            &config.to_string(),
        );
        feed(reg, &format!("nidoo-sound/{sid}/address"), address);
    }

    #[test]
    fn room_satellites_take_every_allowed_one_of_the_room() {
        let mut reg = house();
        // Muted and disconnected ones are held too: the hold is harmless and they may
        // come back mid-exchange.
        assert_eq!(room_satellites(&reg, "a"), ids(&["a", "gone", "muted"]));
        assert_eq!(
            room_satellites(&reg, "b"),
            ids(&["b"]),
            "refused is not allowed"
        );
        satellite(&mut reg, "lonely", UNASSIGNED_AREA, true, true, false);
        assert_eq!(room_satellites(&reg, "lonely"), ids(&["lonely"]));
        assert_eq!(room_of(&reg, "lonely"), None);
        assert_eq!(room_of(&reg, "a").as_deref(), Some("sejour"));
    }

    #[test]
    fn voice_satellites_need_an_address() {
        let mut reg = house();
        with_address(&mut reg, "a", "192.0.2.10");
        with_address(&mut reg, "b", "");
        with_address(&mut reg, "refused", "192.0.2.11");
        assert_eq!(
            voice_satellites(&reg),
            vec![("a".to_owned(), "192.0.2.10".parse().unwrap())]
        );
    }

    #[test]
    fn parse_spoken_reads_the_sound_report() {
        assert_eq!(
            parse_spoken(br#"{"id":"c","played":true}"#),
            Some(Spoken {
                id: "c".into(),
                played: true
            })
        );
        assert_eq!(parse_spoken(b"nope"), None);
    }
}
