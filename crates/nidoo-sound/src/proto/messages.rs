//! JSON messages of the Sendspin protocol (1.0.0-rc1): the subset a server
//! with the `player@v1` role needs. Unknown fields are ignored; unknown
//! message types parse as [`ClientMsg::Other`].

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

/// The only core message format version (`client/init`, `server/init`).
pub const PROTOCOL_VERSION: u32 = 1;
/// The role this server activates.
pub const ROLE_PLAYER_V1: &str = "player@v1";
/// The activity that lets a player play.
pub const ACTIVITY_PLAYBACK: &str = "playback";

/// An audio format (`supported_formats` entry, `stream/start` player object).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioFormat {
    pub codec: Cow<'static, str>,
    pub sample_rate: u32,
    pub channels: u8,
    /// Meaningful for `pcm` and `flac`; ignored for `opus`.
    #[serde(default)]
    pub bit_depth: u8,
}

/// The one format the server streams: raw PCM, 48 kHz, stereo, 16 bit.
pub const PCM_48K_STEREO_16: AudioFormat = AudioFormat {
    codec: Cow::Borrowed("pcm"),
    sample_rate: 48_000,
    channels: 2,
    bit_depth: 16,
};

// ---------------------------------------------------------------- client → server

#[derive(Debug, Clone, PartialEq)]
pub enum ClientMsg {
    /// `client/init`
    Init(ClientInit),
    /// `noise/handshake`
    NoiseHandshake(NoiseHandshake),
    /// `client/hello`
    Hello(ClientHello),
    /// `client/time`
    Time(ClientTime),
    /// `client/state`
    State(ClientState),
    /// `client/goodbye`
    Goodbye(ClientGoodbye),
    /// Any other message type: ignored.
    Other,
}

impl<'de> Deserialize<'de> for ClientMsg {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        #[derive(Deserialize)]
        struct Envelope {
            #[serde(rename = "type")]
            kind: String,
            payload: serde_json::Value,
        }

        fn payload<T: serde::de::DeserializeOwned, E: Error>(v: serde_json::Value) -> Result<T, E> {
            serde_json::from_value(v).map_err(E::custom)
        }

        let Envelope { kind, payload: p } = Envelope::deserialize(d)?;
        if !p.is_object() {
            return Err(D::Error::custom("payload is not an object"));
        }
        Ok(match kind.as_str() {
            "client/init" => Self::Init(payload(p)?),
            "noise/handshake" => Self::NoiseHandshake(payload(p)?),
            "client/hello" => Self::Hello(payload(p)?),
            "client/time" => Self::Time(payload(p)?),
            "client/state" => Self::State(payload(p)?),
            "client/goodbye" => Self::Goodbye(payload(p)?),
            _ => Self::Other,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ClientInit {
    pub client_id: String,
    pub version: u32,
    pub suite: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoiseHandshake {
    /// base64url (no padding) Noise handshake message.
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ClientHello {
    pub name: String,
    #[serde(default)]
    pub supported_roles: Vec<String>,
    #[serde(rename = "player@v1_support", default)]
    pub player_v1_support: Option<PlayerSupport>,
    #[serde(default)]
    pub unpaired_access: Option<UnpairedAccess>,
}

impl ClientHello {
    /// The `player@v1` support object, when the client lists that role.
    pub fn player_support(&self) -> Option<&PlayerSupport> {
        if self.supported_roles.iter().any(|r| r == ROLE_PLAYER_V1) {
            self.player_v1_support.as_ref()
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PlayerSupport {
    pub supported_formats: Vec<AudioFormat>,
    /// Bytes of outstanding audio chunks (13-byte header + payload) the player holds.
    pub buffer_capacity: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct UnpairedAccess {
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct ClientTime {
    pub client_transmitted: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ClientState {
    pub available: bool,
    #[serde(default)]
    pub player: Option<PlayerState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PlayerState {
    #[serde(default)]
    pub volume: Option<u8>,
    #[serde(default)]
    pub muted: Option<bool>,
    #[serde(default)]
    pub output_delay_ms: Option<u32>,
    #[serde(default)]
    pub required_lead_time_ms: Option<u32>,
    #[serde(default)]
    pub min_buffer_ms: Option<u32>,
    #[serde(default)]
    pub supported_commands: Vec<String>,
    #[serde(default)]
    pub format: Option<AudioFormat>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ClientGoodbye {
    pub reason: String,
}

// ---------------------------------------------------------------- server → client

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum ServerMsg {
    #[serde(rename = "server/init")]
    Init(ServerInit),
    #[serde(rename = "server/error")]
    Error(ServerError),
    #[serde(rename = "noise/handshake")]
    NoiseHandshake(NoiseHandshake),
    #[serde(rename = "server/hello")]
    Hello(ServerHello),
    #[serde(rename = "server/activate")]
    Activate(ServerActivate),
    #[serde(rename = "server/time")]
    Time(ServerTime),
    #[serde(rename = "server/command")]
    Command(ServerCommand),
    #[serde(rename = "stream/start")]
    StreamStart(StreamStart),
    #[serde(rename = "stream/end")]
    StreamEnd(StreamEnd),
    #[serde(rename = "group/update")]
    GroupUpdate(GroupUpdate),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerInit {
    pub server_id: String,
    pub version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerError {
    /// `unsupported_version`, `unsupported_suite` or `malformed`.
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerHello {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerActivate {
    pub activities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_roles: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ServerTime {
    pub client_transmitted: i64,
    pub server_received: i64,
    pub server_transmitted: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerCommand {
    pub player: PlayerCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlayerCommand {
    /// `volume` or `mute`.
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mute: Option<bool>,
}

impl PlayerCommand {
    pub fn volume(volume: u8) -> Self {
        Self {
            command: "volume".into(),
            volume: Some(volume.min(100)),
            mute: None,
        }
    }

    pub fn mute(mute: bool) -> Self {
        Self {
            command: "mute".into(),
            volume: None,
            mute: Some(mute),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamStart {
    pub server_transmitted: i64,
    pub player: AudioFormat,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct StreamEnd {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackState {
    Playing,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GroupUpdate {
    pub playback_state: PlaybackState,
    pub group_id: String,
    pub group_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped after the rc1 `client/hello` definition with a `player@v1_support` object.
    const CLIENT_HELLO_EXAMPLE: &str = r#"{
        "type": "client/hello",
        "payload": {
            "name": "Kitchen satellite",
            "device_info": {"product_name": "XIAO", "manufacturer": "Seeed", "software_version": "2026.10.2"},
            "supported_roles": ["player@v1"],
            "player@v1_support": {
                "supported_formats": [
                    {"codec": "pcm", "channels": 2, "sample_rate": 48000, "bit_depth": 16}
                ],
                "buffer_capacity": 1000000
            },
            "supported_pair_methods": {"pairing_psk": {}},
            "unpaired_access": {"enabled": true},
            "client_id": "9kDIII64_oap3z6x5vV92ffB6OvJ6WU5778PBR5u-Bs"
        }
    }"#;

    #[test]
    fn client_hello_parses_player_support() {
        let m: ClientMsg = serde_json::from_str(CLIENT_HELLO_EXAMPLE).unwrap();
        let ClientMsg::Hello(h) = m else {
            panic!("not a hello: {m:?}")
        };
        assert_eq!(h.name, "Kitchen satellite");
        let support = h.player_support().unwrap();
        assert!(support.supported_formats.contains(&PCM_48K_STEREO_16));
        assert_eq!(support.buffer_capacity, 1_000_000);
    }

    #[test]
    fn client_init_parses() {
        let m: ClientMsg = serde_json::from_str(
            r#"{"type":"client/init","payload":{"client_id":"9kDIII64_oap3z6x5vV92ffB6OvJ6WU5778PBR5u-Bs","version":1,"suite":"25519_ChaChaPoly_SHA256"}}"#,
        )
        .unwrap();
        let ClientMsg::Init(i) = m else { panic!() };
        assert_eq!(i.client_id.len(), 43);
        assert_eq!(i.version, 1);
        assert_eq!(i.suite, "25519_ChaChaPoly_SHA256");
    }

    #[test]
    fn client_state_parses_player_object() {
        let m: ClientMsg = serde_json::from_str(
            r#"{"type":"client/state","payload":{"available":true,"player":{"volume":30,"muted":false,"output_delay_ms":0,"required_lead_time_ms":200,"min_buffer_ms":150,"supported_commands":["volume","mute"]}}}"#,
        )
        .unwrap();
        let ClientMsg::State(s) = m else { panic!() };
        assert!(s.available);
        let p = s.player.unwrap();
        assert_eq!(
            (p.volume, p.muted, p.min_buffer_ms),
            (Some(30), Some(false), Some(150))
        );
        assert_eq!(p.supported_commands, vec!["volume", "mute"]);
    }

    #[test]
    fn unknown_type_is_other() {
        let m: ClientMsg =
            serde_json::from_str(r#"{"type":"client/whatever","payload":{}}"#).unwrap();
        assert!(matches!(m, ClientMsg::Other));
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let m: ClientMsg = serde_json::from_str(
            r#"{"type":"client/time","payload":{"client_transmitted":42,"future_field":1}}"#,
        )
        .unwrap();
        assert!(matches!(
            m,
            ClientMsg::Time(ClientTime {
                client_transmitted: 42
            })
        ));
    }

    #[test]
    fn server_messages_use_wire_names() {
        let j = |m: &ServerMsg| serde_json::to_value(m).unwrap();
        let v = j(&ServerMsg::Activate(ServerActivate {
            activities: vec!["playback".into()],
            active_roles: Some(vec!["player@v1".into()]),
        }));
        assert_eq!(
            v,
            serde_json::json!({"type":"server/activate","payload":{"activities":["playback"],"active_roles":["player@v1"]}})
        );
        let v = j(&ServerMsg::Command(ServerCommand {
            player: PlayerCommand::volume(10),
        }));
        assert_eq!(
            v,
            serde_json::json!({"type":"server/command","payload":{"player":{"command":"volume","volume":10}}})
        );
        let v = j(&ServerMsg::StreamStart(StreamStart {
            server_transmitted: 7,
            player: PCM_48K_STEREO_16,
        }));
        assert_eq!(
            v,
            serde_json::json!({"type":"stream/start","payload":{"server_transmitted":7,"player":{"codec":"pcm","sample_rate":48000,"channels":2,"bit_depth":16}}})
        );
        let v = j(&ServerMsg::StreamEnd(StreamEnd::default()));
        assert_eq!(v, serde_json::json!({"type":"stream/end","payload":{}}));
        let v = j(&ServerMsg::Time(ServerTime {
            client_transmitted: 1,
            server_received: 2,
            server_transmitted: 3,
        }));
        assert_eq!(v["type"], "server/time");
        let v = j(&ServerMsg::GroupUpdate(GroupUpdate {
            playback_state: PlaybackState::Playing,
            group_id: "g".into(),
            group_name: "House".into(),
        }));
        assert_eq!(
            v,
            serde_json::json!({"type":"group/update","payload":{"playback_state":"playing","group_id":"g","group_name":"House"}})
        );
    }
}
