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
mod tests;
