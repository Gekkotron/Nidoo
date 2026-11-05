//! Sendspin JSON as a player sees it: what the server sends, and what we answer.
//! `nidoo-sound`'s own types read the client side and write the server side; these
//! are the mirror image.

use nidoo_sound::proto::messages::{
    AudioFormat, PCM_48K_STEREO_16, PROTOCOL_VERSION, ROLE_PLAYER_V1,
};
use serde::Deserialize;
use serde_json::json;

/// The only suite we offer (the server must support both).
pub const SUITE: &str = "25519_ChaChaPoly_SHA256";

/// A message from the server. Unknown types and fields are ignored.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMsg {
    Init {
        server_id: String,
        version: u32,
    },
    Error {
        reason: String,
    },
    NoiseHandshake {
        data: String,
    },
    Hello,
    Time {
        client_transmitted: i64,
        server_received: i64,
        server_transmitted: i64,
    },
    /// `server/command` for the player role.
    Command {
        volume: Option<u8>,
        mute: Option<bool>,
    },
    /// `stream/start`; `None` when it has no player object.
    StreamStart(Option<AudioFormat>),
    StreamEnd,
    Other,
}

impl<'de> Deserialize<'de> for ServerMsg {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        #[derive(Deserialize)]
        struct Envelope {
            #[serde(rename = "type")]
            kind: String,
            #[serde(default)]
            payload: serde_json::Value,
        }
        #[derive(Deserialize)]
        struct Init {
            server_id: String,
            version: u32,
        }
        #[derive(Deserialize)]
        struct Reason {
            reason: String,
        }
        #[derive(Deserialize)]
        struct Data {
            data: String,
        }
        #[derive(Deserialize)]
        struct Time {
            client_transmitted: i64,
            server_received: i64,
            server_transmitted: i64,
        }
        #[derive(Deserialize)]
        struct Command {
            #[serde(default)]
            player: Option<PlayerCommand>,
        }
        #[derive(Deserialize)]
        struct PlayerCommand {
            #[serde(default)]
            volume: Option<u8>,
            #[serde(default)]
            mute: Option<bool>,
        }
        #[derive(Deserialize)]
        struct Start {
            #[serde(default)]
            player: Option<AudioFormat>,
        }

        fn payload<T: serde::de::DeserializeOwned, E: Error>(v: serde_json::Value) -> Result<T, E> {
            serde_json::from_value(v).map_err(E::custom)
        }

        let Envelope { kind, payload: p } = Envelope::deserialize(d)?;
        Ok(match kind.as_str() {
            "server/init" => {
                let Init { server_id, version } = payload(p)?;
                Self::Init { server_id, version }
            }
            "server/error" => Self::Error {
                reason: payload::<Reason, _>(p)?.reason,
            },
            "noise/handshake" => Self::NoiseHandshake {
                data: payload::<Data, _>(p)?.data,
            },
            "server/hello" => Self::Hello,
            "server/time" => {
                let t: Time = payload(p)?;
                Self::Time {
                    client_transmitted: t.client_transmitted,
                    server_received: t.server_received,
                    server_transmitted: t.server_transmitted,
                }
            }
            "server/command" => match payload::<Command, _>(p)?.player {
                Some(c) => Self::Command {
                    volume: c.volume,
                    mute: c.mute,
                },
                None => Self::Other,
            },
            "stream/start" => Self::StreamStart(payload::<Start, _>(p)?.player),
            "stream/end" => Self::StreamEnd,
            _ => Self::Other,
        })
    }
}

/// `client/init` (cleartext).
pub fn client_init(client_id: &str) -> String {
    json!({
        "type": "client/init",
        "payload": { "client_id": client_id, "version": PROTOCOL_VERSION, "suite": SUITE },
    })
    .to_string()
}

/// `noise/handshake` with a base64url message.
pub fn noise_handshake(data: &str) -> String {
    json!({ "type": "noise/handshake", "payload": { "data": data } }).to_string()
}

/// `client/hello` for the `player@v1` role, PCM 48 kHz stereo 16-bit only.
pub fn client_hello(client_id: &str, name: &str, buffer_capacity: u32) -> String {
    json!({
        "type": "client/hello",
        "payload": {
            "client_id": client_id,
            "name": name,
            "supported_roles": [ROLE_PLAYER_V1],
            "player@v1_support": {
                "supported_formats": [PCM_48K_STEREO_16],
                "buffer_capacity": buffer_capacity,
                "supported_commands": ["volume", "mute"],
            },
        },
    })
    .to_string()
}

/// `client/time`.
pub fn client_time(client_transmitted: i64) -> String {
    json!({ "type": "client/time", "payload": { "client_transmitted": client_transmitted } })
        .to_string()
}

/// `client/state`: always available, with the player's volume and mute.
pub fn client_state(volume: u8, muted: bool) -> String {
    json!({
        "type": "client/state",
        "payload": { "available": true, "player": { "volume": volume, "muted": muted } },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_sound::proto::messages::PCM_48K_STEREO_16;

    fn parse(text: &str) -> ServerMsg {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn server_messages_parse() {
        assert_eq!(
            parse(r#"{"type":"server/init","payload":{"server_id":"abc","version":1}}"#),
            ServerMsg::Init {
                server_id: "abc".into(),
                version: 1
            }
        );
        assert_eq!(
            parse(r#"{"type":"server/error","payload":{"reason":"malformed"}}"#),
            ServerMsg::Error {
                reason: "malformed".into()
            }
        );
        assert_eq!(
            parse(r#"{"type":"noise/handshake","payload":{"data":"AAA"}}"#),
            ServerMsg::NoiseHandshake { data: "AAA".into() }
        );
        assert_eq!(
            parse(r#"{"type":"server/hello","payload":{"name":"Nidoo"}}"#),
            ServerMsg::Hello
        );
        assert_eq!(
            parse(
                r#"{"type":"server/time","payload":{"client_transmitted":1,"server_received":2,"server_transmitted":3}}"#
            ),
            ServerMsg::Time {
                client_transmitted: 1,
                server_received: 2,
                server_transmitted: 3
            }
        );
        assert_eq!(
            parse(
                r#"{"type":"server/command","payload":{"player":{"command":"volume","volume":40}}}"#
            ),
            ServerMsg::Command {
                volume: Some(40),
                mute: None
            }
        );
        assert_eq!(
            parse(
                r#"{"type":"server/command","payload":{"player":{"command":"mute","mute":true}}}"#
            ),
            ServerMsg::Command {
                volume: None,
                mute: Some(true)
            }
        );
        assert_eq!(
            parse(
                r#"{"type":"stream/start","payload":{"server_transmitted":5,"player":{"codec":"pcm","sample_rate":48000,"channels":2,"bit_depth":16}}}"#
            ),
            ServerMsg::StreamStart(Some(PCM_48K_STEREO_16))
        );
        assert_eq!(
            parse(r#"{"type":"stream/end","payload":{}}"#),
            ServerMsg::StreamEnd
        );
        assert_eq!(
            parse(r#"{"type":"server/activate","payload":{"activities":["playback"]}}"#),
            ServerMsg::Other
        );
        assert_eq!(
            parse(r#"{"type":"group/update","payload":{}}"#),
            ServerMsg::Other
        );
    }

    #[test]
    fn client_messages_have_the_server_shape() {
        use nidoo_sound::proto::messages::ClientMsg;
        let init: ClientMsg = serde_json::from_str(&client_init("cid")).unwrap();
        let ClientMsg::Init(init) = init else {
            panic!("{init:?}")
        };
        assert_eq!(
            (init.client_id.as_str(), init.version, init.suite.as_str()),
            ("cid", 1, "25519_ChaChaPoly_SHA256")
        );

        let hello: ClientMsg =
            serde_json::from_str(&client_hello("cid", "Test Mac", 1_048_576)).unwrap();
        let ClientMsg::Hello(hello) = hello else {
            panic!("{hello:?}")
        };
        assert_eq!(hello.name, "Test Mac");
        let support = hello.player_support().unwrap();
        assert_eq!(support.supported_formats, vec![PCM_48K_STEREO_16]);
        assert_eq!(support.buffer_capacity, 1_048_576);
        assert_eq!(support.supported_commands, vec!["volume", "mute"]);

        let state: ClientMsg = serde_json::from_str(&client_state(40, true)).unwrap();
        let ClientMsg::State(state) = state else {
            panic!("{state:?}")
        };
        assert!(state.available);
        let player = state.player.unwrap();
        assert_eq!((player.volume, player.muted), (Some(40), Some(true)));

        let time: ClientMsg = serde_json::from_str(&client_time(123)).unwrap();
        assert!(matches!(time, ClientMsg::Time(t) if t.client_transmitted == 123));

        let hs: ClientMsg = serde_json::from_str(&noise_handshake("QUJD")).unwrap();
        assert!(matches!(hs, ClientMsg::NoiseHandshake(h) if h.data == "QUJD"));
    }
}
