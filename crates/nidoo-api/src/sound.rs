//! Saying a sentence on the speaker satellites (`POST /api/v1/sound/speak`).

use serde::{Deserialize, Serialize};

/// A sentence to say on the satellites of one room, of the whole house, or on one satellite.
/// `room` and `satellite` both left out means every room; both set is refused.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SoundSpeakRequest {
    /// The text to say.
    pub text: String,
    /// An area id or name; every room when left out.
    #[serde(default)]
    pub room: Option<String>,
    /// One satellite's short id (its device id without `nidoo_sound_`).
    #[serde(default)]
    pub satellite: Option<String>,
    /// The language code to speak in; the house language when left out.
    #[serde(default)]
    pub language: Option<String>,
    /// The voice to use; the language's chosen voice when left out.
    #[serde(default)]
    pub voice: Option<String>,
}

/// The answer to a speak request: how many satellites were asked to say it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SoundSpeakReply {
    /// The number of satellites the sentence was sent to.
    pub players: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_defaults_to_every_room() {
        let req: SoundSpeakRequest = serde_json::from_value(json!({"text": "hello"})).unwrap();
        assert_eq!(req.room, None);
        assert_eq!(req.satellite, None);
        assert_eq!(req.language, None);
        assert_eq!(req.voice, None);
    }

    #[test]
    fn reply_round_trips() {
        let reply = SoundSpeakReply { players: 2 };
        let v = serde_json::to_value(&reply).unwrap();
        assert_eq!(v, json!({"players": 2}));
        assert_eq!(serde_json::from_value::<SoundSpeakReply>(v).unwrap(), reply);
    }
}
