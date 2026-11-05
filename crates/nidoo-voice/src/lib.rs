//! The UDP datagrams between a satellite and the hub (`docs/reference/voice-protocol.md`).
//!
//! Every datagram, little-endian: `"NV"`, version `1`, a type byte, the satellite's 12-hex
//! short id, the body, then the first 8 bytes of `HMAC-SHA256(key, everything before)`.
//! The tag is checked before the body is read, so a datagram from a device without the key
//! never reaches the parser. Authenticated, not encrypted.

use std::net::{Ipv4Addr, SocketAddrV4};

use hmac::{Hmac, Mac};
use sha2::Sha256;

/// The first two bytes of every datagram.
pub const MAGIC: [u8; 2] = *b"NV";
/// The protocol version.
pub const VERSION: u8 = 1;
/// Bytes of the truncated HMAC tag.
pub const TAG_LEN: usize = 8;
/// Bytes of a satellite's short id (lowercase hex).
pub const SID_LEN: usize = 12;
/// The most samples one audio datagram carries (40 ms at 16 kHz).
pub const MAX_SAMPLES: usize = 640;

const HEADER_LEN: usize = 4 + SID_LEN;

const HELLO: u8 = 0x01;
const STOP: u8 = 0x02;
const LISTEN: u8 = 0x03;
const WAKE: u8 = 0x10;
const AUDIO: u8 = 0x11;
const END: u8 = 0x12;

/// The shared secret, in every board's `secrets.yaml` and in the hub's settings.
#[derive(Clone, PartialEq, Eq)]
pub struct Key([u8; 32]);

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(…)")
    }
}

impl Key {
    /// 64 hex characters; `None` otherwise.
    pub fn from_hex(hex: &str) -> Option<Key> {
        let hex = hex.as_bytes();
        if hex.len() != 64 {
            return None;
        }
        let mut bytes = [0u8; 32];
        for (b, pair) in bytes.iter_mut().zip(hex.as_chunks::<2>().0) {
            let digit = |c: u8| char::from(c).to_digit(16);
            *b = u8::try_from(digit(pair[0])? * 16 + digit(pair[1])?).ok()?;
        }
        Some(Key(bytes))
    }

    /// The 64 lowercase hex characters.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// A fresh random key.
    pub fn generate() -> Key {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).expect("the system's random source");
        Key(bytes)
    }

    fn mac(&self) -> Hmac<Sha256> {
        <Hmac<Sha256> as Mac>::new_from_slice(&self.0).expect("HMAC takes any key length")
    }

    /// For tests: a key from its bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Key {
        Key(bytes)
    }
}

/// From the hub to a board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubMsg {
    /// "I am the hub": stream to `hub`, echo `nonce` in your next wake.
    Hello {
        nonce: u64,
        hub_ctr: u64,
        hub: SocketAddrV4,
    },
    /// The phrase is over: close the window `session`.
    Stop { hub_ctr: u64, session: u32 },
    /// Open a follow-up window, without the wake word.
    Listen { hub_ctr: u64 },
}

/// What opened a board's window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeKind {
    WakeWord,
    FollowUp,
}

/// From a board to the hub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardMsg {
    /// A window opened; `listen_ctr` is the Listen's counter for a follow-up, else 0.
    Wake {
        nonce: u64,
        session: u32,
        kind: WakeKind,
        listen_ctr: u64,
    },
    /// 16 kHz mono samples of the window `session`.
    Audio {
        session: u32,
        seq: u32,
        samples: Vec<i16>,
    },
    /// The board closed the window itself (its own time limit).
    End { session: u32 },
}

/// Why a datagram was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PacketError {
    #[error("too short")]
    Short,
    #[error("not a voice packet")]
    Magic,
    #[error("unknown version")]
    Version,
    #[error("unknown type")]
    Type,
    #[error("wrong length")]
    Length,
    #[error("bad satellite id")]
    Sid,
    #[error("bad tag")]
    Tag,
}

/// The datagram for `msg` to the board `sid`.
pub fn encode_hub(key: &Key, sid: &str, msg: &HubMsg) -> Vec<u8> {
    let (kind, body) = match *msg {
        HubMsg::Hello {
            nonce,
            hub_ctr,
            hub,
        } => {
            let mut b = Vec::with_capacity(22);
            b.extend_from_slice(&nonce.to_le_bytes());
            b.extend_from_slice(&hub_ctr.to_le_bytes());
            b.extend_from_slice(&hub.ip().octets());
            b.extend_from_slice(&hub.port().to_le_bytes());
            (HELLO, b)
        }
        HubMsg::Stop { hub_ctr, session } => {
            let mut b = hub_ctr.to_le_bytes().to_vec();
            b.extend_from_slice(&session.to_le_bytes());
            (STOP, b)
        }
        HubMsg::Listen { hub_ctr } => (LISTEN, hub_ctr.to_le_bytes().to_vec()),
    };
    seal(key, kind, sid, &body)
}

/// A board's datagram: its satellite id and message.
pub fn decode_board(key: &Key, datagram: &[u8]) -> Result<(String, BoardMsg), PacketError> {
    let (kind, sid, body) = open(key, datagram)?;
    let mut r = Reader(body);
    let msg = match kind {
        WAKE => {
            let nonce = r.u64()?;
            let session = r.u32()?;
            let kind = match r.u8()? {
                0 => WakeKind::WakeWord,
                1 => WakeKind::FollowUp,
                _ => return Err(PacketError::Length),
            };
            let listen_ctr = r.u64()?;
            BoardMsg::Wake {
                nonce,
                session,
                kind,
                listen_ctr,
            }
        }
        AUDIO => {
            let session = r.u32()?;
            let seq = r.u32()?;
            let rest = r.rest();
            let (pairs, odd) = rest.as_chunks::<2>();
            if !odd.is_empty() || pairs.is_empty() || pairs.len() > MAX_SAMPLES {
                return Err(PacketError::Length);
            }
            let samples = pairs.iter().map(|p| i16::from_le_bytes(*p)).collect();
            BoardMsg::Audio {
                session,
                seq,
                samples,
            }
        }
        END => BoardMsg::End { session: r.u32()? },
        _ => return Err(PacketError::Type),
    };
    r.done()?;
    Ok((sid, msg))
}

/// The datagram a board sends for `msg` (tests, the protocol's vectors).
pub fn encode_board(key: &Key, sid: &str, msg: &BoardMsg) -> Vec<u8> {
    let (kind, body) = match msg {
        BoardMsg::Wake {
            nonce,
            session,
            kind,
            listen_ctr,
        } => {
            let mut b = nonce.to_le_bytes().to_vec();
            b.extend_from_slice(&session.to_le_bytes());
            b.push(match kind {
                WakeKind::WakeWord => 0,
                WakeKind::FollowUp => 1,
            });
            b.extend_from_slice(&listen_ctr.to_le_bytes());
            (WAKE, b)
        }
        BoardMsg::Audio {
            session,
            seq,
            samples,
        } => {
            let mut b = session.to_le_bytes().to_vec();
            b.extend_from_slice(&seq.to_le_bytes());
            for s in samples {
                b.extend_from_slice(&s.to_le_bytes());
            }
            (AUDIO, b)
        }
        BoardMsg::End { session } => (END, session.to_le_bytes().to_vec()),
    };
    seal(key, kind, sid, &body)
}

/// A hub datagram as a board reads it (tests, the protocol's vectors).
pub fn decode_hub(key: &Key, datagram: &[u8]) -> Result<(String, HubMsg), PacketError> {
    let (kind, sid, body) = open(key, datagram)?;
    let mut r = Reader(body);
    let msg = match kind {
        HELLO => {
            let nonce = r.u64()?;
            let hub_ctr = r.u64()?;
            let [a, b, c, d] = r.take::<4>()?;
            let port = u16::from_le_bytes(r.take::<2>()?);
            HubMsg::Hello {
                nonce,
                hub_ctr,
                hub: SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), port),
            }
        }
        STOP => HubMsg::Stop {
            hub_ctr: r.u64()?,
            session: r.u32()?,
        },
        LISTEN => HubMsg::Listen { hub_ctr: r.u64()? },
        _ => return Err(PacketError::Type),
    };
    r.done()?;
    Ok((sid, msg))
}

/// Header, body and tag.
fn seal(key: &Key, kind: u8, sid: &str, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + body.len() + TAG_LEN);
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.push(kind);
    out.extend_from_slice(sid.as_bytes());
    out.extend_from_slice(body);
    let mut mac = key.mac();
    mac.update(&out);
    out.extend_from_slice(&mac.finalize().into_bytes()[..TAG_LEN]);
    out
}

/// Checks the frame and its tag; the type, the satellite id and the body.
fn open<'a>(key: &Key, datagram: &'a [u8]) -> Result<(u8, String, &'a [u8]), PacketError> {
    if datagram.len() < HEADER_LEN + TAG_LEN {
        return Err(PacketError::Short);
    }
    if datagram[..2] != MAGIC {
        return Err(PacketError::Magic);
    }
    if datagram[2] != VERSION {
        return Err(PacketError::Version);
    }
    let (signed, tag) = datagram.split_at(datagram.len() - TAG_LEN);
    let mut mac = key.mac();
    mac.update(signed);
    mac.verify_truncated_left(tag)
        .map_err(|_| PacketError::Tag)?;
    let sid = &signed[4..HEADER_LEN];
    if !sid
        .iter()
        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
    {
        return Err(PacketError::Sid);
    }
    let sid = String::from_utf8(sid.to_vec()).map_err(|_| PacketError::Sid)?;
    Ok((signed[3], sid, &signed[HEADER_LEN..]))
}

/// Reads a body front to back.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], PacketError> {
        let (head, rest) = self.0.split_first_chunk::<N>().ok_or(PacketError::Length)?;
        self.0 = rest;
        Ok(*head)
    }

    fn u8(&mut self) -> Result<u8, PacketError> {
        Ok(self.take::<1>()?[0])
    }

    fn u32(&mut self) -> Result<u32, PacketError> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    fn u64(&mut self) -> Result<u64, PacketError> {
        Ok(u64::from_le_bytes(self.take()?))
    }

    fn rest(&mut self) -> &[u8] {
        std::mem::take(&mut self.0)
    }

    fn done(&self) -> Result<(), PacketError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(PacketError::Length)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SID: &str = "0123456789ab";

    fn hex(bytes: Vec<u8>) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn key() -> Key {
        let mut bytes = [0u8; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = i as u8;
        }
        Key::from_bytes(bytes)
    }

    fn hub_messages() -> Vec<HubMsg> {
        vec![
            HubMsg::Hello {
                nonce: 1,
                hub_ctr: 2,
                hub: SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 10), 4960),
            },
            HubMsg::Stop {
                hub_ctr: 3,
                session: 4,
            },
            HubMsg::Listen { hub_ctr: u64::MAX },
        ]
    }

    fn board_messages() -> Vec<BoardMsg> {
        vec![
            BoardMsg::Wake {
                nonce: 9,
                session: 1,
                kind: WakeKind::WakeWord,
                listen_ctr: 0,
            },
            BoardMsg::Wake {
                nonce: 9,
                session: 2,
                kind: WakeKind::FollowUp,
                listen_ctr: 77,
            },
            BoardMsg::Audio {
                session: 2,
                seq: 5,
                samples: (0..640).map(|i| i as i16 - 320).collect(),
            },
            BoardMsg::End { session: 2 },
        ]
    }

    #[test]
    fn every_message_round_trips() {
        for msg in hub_messages() {
            let bytes = encode_hub(&key(), SID, &msg);
            assert_eq!(decode_hub(&key(), &bytes), Ok((SID.to_owned(), msg)));
        }
        for msg in board_messages() {
            let bytes = encode_board(&key(), SID, &msg);
            assert_eq!(decode_board(&key(), &bytes), Ok((SID.to_owned(), msg)));
        }
    }

    #[test]
    fn a_flipped_bit_anywhere_is_refused() {
        let bytes = encode_board(
            &key(),
            SID,
            &BoardMsg::Audio {
                session: 1,
                seq: 7,
                samples: vec![0, 1, -1, 32767],
            },
        );
        for i in 0..bytes.len() {
            let mut bad = bytes.clone();
            bad[i] ^= 0x01;
            assert!(decode_board(&key(), &bad).is_err(), "byte {i}");
        }
    }

    #[test]
    fn wrong_key_is_refused() {
        let bytes = encode_board(&key(), SID, &BoardMsg::End { session: 1 });
        let other = Key::from_bytes([7; 32]);
        assert_eq!(decode_board(&other, &bytes), Err(PacketError::Tag));
    }

    #[test]
    fn short_and_oversized_audio_are_refused() {
        for n in [0usize, MAX_SAMPLES + 1] {
            let bytes = encode_board(
                &key(),
                SID,
                &BoardMsg::Audio {
                    session: 1,
                    seq: 1,
                    samples: vec![0; n],
                },
            );
            assert_eq!(
                decode_board(&key(), &bytes),
                Err(PacketError::Length),
                "{n}"
            );
        }
        assert_eq!(decode_board(&key(), b"NV"), Err(PacketError::Short));
    }

    #[test]
    fn bad_sid_is_refused() {
        for sid in ["0123456789AB", "0123456789zz", "0123456789a"] {
            let bytes = encode_board(&key(), sid, &BoardMsg::End { session: 1 });
            assert!(decode_board(&key(), &bytes).is_err(), "{sid}");
        }
        let bytes = encode_board(&key(), "0123456789AB", &BoardMsg::End { session: 1 });
        assert_eq!(decode_board(&key(), &bytes), Err(PacketError::Sid));
    }

    #[test]
    fn a_board_message_is_not_a_hub_message() {
        let bytes = encode_board(&key(), SID, &BoardMsg::End { session: 1 });
        assert_eq!(decode_hub(&key(), &bytes), Err(PacketError::Type));
    }

    #[test]
    fn key_hex_round_trips() {
        let hex = key().to_hex();
        assert_eq!(hex.len(), 64);
        assert_eq!(&hex[..8], "00010203");
        assert_eq!(Key::from_hex(&hex), Some(key()));
        assert_eq!(Key::from_hex(&hex.to_uppercase()), Some(key()));
        assert_eq!(Key::from_hex(&hex[..62]), None);
        assert_eq!(Key::from_hex(&"zz".repeat(32)), None);
        assert_ne!(Key::generate(), Key::generate());
        assert_eq!(format!("{:?}", key()), "Key(…)");
    }

    // The vectors in docs/reference/voice-protocol.md, computed with Python's hmac module
    // from the layout, not from this code.
    #[test]
    fn golden_hello() {
        let msg = &hub_messages()[0];
        assert_eq!(
            hex(encode_hub(&key(), SID, msg)),
            "4e56010130313233343536373839616201000000000000000200000000000000\
             c000020a6013eebff1b9d2213d37"
        );
    }

    #[test]
    fn golden_audio() {
        let msg = BoardMsg::Audio {
            session: 1,
            seq: 7,
            samples: vec![0, 1, -1, 32767],
        };
        assert_eq!(
            hex(encode_board(&key(), SID, &msg)),
            "4e560111303132333435363738396162010000000700000000000100ffffff7f\
             0c9b91e10bae7b2e"
        );
    }
}
