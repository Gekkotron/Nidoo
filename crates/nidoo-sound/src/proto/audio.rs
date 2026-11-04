//! Binary messages: player audio chunks (type 4) and fragment reassembly (type 1).

/// Binary message id of a JSON message body.
pub const MSG_JSON: u8 = 0;
/// Binary message id of a fragment.
pub const MSG_FRAGMENT: u8 = 1;
/// Binary message id of a player audio chunk.
pub const MSG_PLAYER_AUDIO: u8 = 4;
/// Bytes before the audio payload: id, timestamp (8), send_ahead (4).
pub const CHUNK_HEADER_LEN: usize = 13;

const FLAG_LAST: u8 = 0b01;
const FLAG_FIRST: u8 = 0b10;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProtoError {
    #[error("empty binary message")]
    Empty,
    #[error("fragment message too short")]
    ShortFragment,
    #[error("reserved fragment flag bits set: {0:#04b}")]
    ReservedFlags(u8),
    #[error("first fragment while another fragmented message is in flight")]
    FirstWhileInFlight,
    #[error("continuation fragment with no fragmented message in flight")]
    ContinuationWithoutFirst,
    #[error("non-fragment message while a fragmented message is in flight")]
    InterleavedMessage,
    #[error("fragment declares orig_type 1")]
    NestedFragment,
}

/// A player audio chunk: `[4][timestamp i64 BE][send_ahead u32 BE][data]`.
pub fn encode_chunk(timestamp_us: i64, send_ahead_us: u32, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(CHUNK_HEADER_LEN + data.len());
    out.push(MSG_PLAYER_AUDIO);
    out.extend_from_slice(&timestamp_us.to_be_bytes());
    out.extend_from_slice(&send_ahead_us.to_be_bytes());
    out.extend_from_slice(data);
    out
}

/// Reassembles fragmented binary messages (after decryption) and enforces the
/// spec's malformed-sequence rules. Every error means: close the connection.
#[derive(Debug, Default)]
pub struct Reassembler {
    in_flight: Option<(u8, Vec<u8>)>,
}

impl Reassembler {
    /// Feeds one decrypted binary message. Returns the complete message
    /// `(type, payload)` when one is ready.
    pub fn push(&mut self, msg: &[u8]) -> Result<Option<(u8, Vec<u8>)>, ProtoError> {
        let (&id, rest) = msg.split_first().ok_or(ProtoError::Empty)?;
        if id != MSG_FRAGMENT {
            if self.in_flight.is_some() {
                return Err(ProtoError::InterleavedMessage);
            }
            return Ok(Some((id, rest.to_vec())));
        }
        let (&flags, rest) = rest.split_first().ok_or(ProtoError::ShortFragment)?;
        if flags & !(FLAG_FIRST | FLAG_LAST) != 0 {
            return Err(ProtoError::ReservedFlags(flags));
        }
        if flags & FLAG_FIRST != 0 {
            if self.in_flight.is_some() {
                return Err(ProtoError::FirstWhileInFlight);
            }
            let (&orig, data) = rest.split_first().ok_or(ProtoError::ShortFragment)?;
            if orig == MSG_FRAGMENT {
                return Err(ProtoError::NestedFragment);
            }
            self.in_flight = Some((orig, data.to_vec()));
        } else {
            let (_, buf) = self
                .in_flight
                .as_mut()
                .ok_or(ProtoError::ContinuationWithoutFirst)?;
            buf.extend_from_slice(rest);
        }
        if flags & FLAG_LAST != 0 {
            return Ok(self.in_flight.take());
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_chunk_layout() {
        let b = encode_chunk(0x0102030405060708, 500_000, &[9, 9]);
        assert_eq!(b, [4, 1, 2, 3, 4, 5, 6, 7, 8, 0, 0x07, 0xA1, 0x20, 9, 9]);
    }

    #[test]
    fn whole_message_passes_through() {
        let mut r = Reassembler::default();
        assert_eq!(r.push(&[0, b'{', b'}']).unwrap(), Some((0, b"{}".to_vec())));
    }

    #[test]
    fn fragments_reassemble_and_bad_sequences_error() {
        let mut r = Reassembler::default();
        assert_eq!(r.push(&[1, 0b10, 4, 1, 2]).unwrap(), None);
        assert_eq!(r.push(&[1, 0b00, 3]).unwrap(), None);
        assert_eq!(
            r.push(&[1, 0b01, 4, 5]).unwrap(),
            Some((4, vec![1, 2, 3, 4, 5]))
        );

        // A single fragment that is both first and last.
        assert_eq!(
            r.push(&[1, 0b11, 0, b'x']).unwrap(),
            Some((0, b"x".to_vec()))
        );

        // Last without first.
        assert!(Reassembler::default().push(&[1, 0b01, 9]).is_err());
        // Reserved flag bit.
        assert!(Reassembler::default().push(&[1, 0b110, 4, 1]).is_err());
        // orig_type 1.
        assert!(Reassembler::default().push(&[1, 0b10, 1, 1]).is_err());
        // First while one is in flight.
        let mut r = Reassembler::default();
        r.push(&[1, 0b10, 4, 1]).unwrap();
        assert!(r.push(&[1, 0b10, 4, 1]).is_err());
        // Non-fragment while one is in flight.
        let mut r = Reassembler::default();
        r.push(&[1, 0b10, 4, 1]).unwrap();
        assert!(r.push(&[0, b'{', b'}']).is_err());
        // Empty message.
        assert!(Reassembler::default().push(&[]).is_err());
    }
}
