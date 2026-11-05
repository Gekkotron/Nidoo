//! The board side of the voice protocol (`docs/reference/voice-protocol.md`), as a state
//! machine with no I/O: hub datagrams in, board datagrams out. It does what
//! `satellite/nidoo_voice.h` does on an ESP32.

use std::net::SocketAddrV4;
use std::time::{Duration, Instant};

use nidoo_voice::{BoardMsg, HubMsg, Key, MAX_SAMPLES, WakeKind, decode_hub, encode_board};

/// How long a Hello keeps the satellite greeted: three missed Hellos (one every 5 s).
pub const GREETING_TTL: Duration = Duration::from_secs(15);
/// The longest window after the wake word.
pub const WAKE_WINDOW: Duration = Duration::from_secs(8);
/// The longest follow-up window.
pub const FOLLOW_UP_WINDOW: Duration = Duration::from_secs(6);

/// What a hub datagram meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceEvent {
    /// A Hello: the hub is there.
    Greeted,
    /// The hub closed the open window (no End is sent).
    Stop,
    /// The hub asks for a follow-up window, to be opened with this counter.
    Listen { listen_ctr: u64 },
}

#[derive(Debug)]
struct Window {
    session: u32,
    seq: u32,
    deadline: Instant,
}

/// One satellite's voice link to the hub.
#[derive(Debug)]
pub struct VoiceLink {
    key: Key,
    sid: String,
    last_hub_ctr: u64,
    nonce: Option<u64>,
    hub: Option<SocketAddrV4>,
    greeted_at: Option<Instant>,
    greeting_ttl: Duration,
    session: u32,
    window: Option<Window>,
}

impl VoiceLink {
    pub fn new(key: Key, sid: String) -> Self {
        Self::with_greeting_ttl(key, sid, GREETING_TTL)
    }

    /// Like [`VoiceLink::new`] with another greeting lifetime (tests use a short one).
    pub fn with_greeting_ttl(key: Key, sid: String, greeting_ttl: Duration) -> Self {
        Self {
            key,
            sid,
            last_hub_ctr: 0,
            nonce: None,
            hub: None,
            greeted_at: None,
            greeting_ttl,
            session: 0,
            window: None,
        }
    }

    /// Reads one hub datagram. Bad tags, other satellites' datagrams and replays (a
    /// `hub_ctr` not above the last one accepted) are ignored.
    pub fn on_datagram(&mut self, bytes: &[u8], now: Instant) -> Option<VoiceEvent> {
        let (sid, msg) = decode_hub(&self.key, bytes).ok()?;
        if sid != self.sid {
            return None;
        }
        let ctr = match msg {
            HubMsg::Hello { hub_ctr, .. }
            | HubMsg::Stop { hub_ctr, .. }
            | HubMsg::Listen { hub_ctr } => hub_ctr,
        };
        if ctr <= self.last_hub_ctr {
            return None;
        }
        self.last_hub_ctr = ctr;
        match msg {
            HubMsg::Hello { nonce, hub, .. } => {
                self.nonce = Some(nonce);
                self.hub = Some(hub);
                self.greeted_at = Some(now);
                Some(VoiceEvent::Greeted)
            }
            HubMsg::Stop { session, .. } => {
                let open = self.window.as_ref().map(|w| w.session);
                (open == Some(session)).then(|| {
                    self.window = None;
                    VoiceEvent::Stop
                })
            }
            HubMsg::Listen { hub_ctr } => self.window.is_none().then_some(VoiceEvent::Listen {
                listen_ctr: hub_ctr,
            }),
        }
    }

    /// True when a Hello came within the greeting lifetime (`GREETING_TTL` by default).
    pub fn greeted(&self, now: Instant) -> bool {
        self.greeted_at
            .is_some_and(|at| now.saturating_duration_since(at) < self.greeting_ttl)
    }

    /// Where to send datagrams: the address inside the last Hello.
    pub fn hub(&self) -> Option<SocketAddrV4> {
        self.hub
    }

    /// Opens a window and returns its Wake, or `None` when not greeted or already open.
    pub fn open(&mut self, kind: WakeKind, listen_ctr: u64, now: Instant) -> Option<Vec<u8>> {
        if self.window.is_some() || !self.greeted(now) {
            return None;
        }
        let nonce = self.nonce?;
        self.session += 1;
        let length = match kind {
            WakeKind::WakeWord => WAKE_WINDOW,
            WakeKind::FollowUp => FOLLOW_UP_WINDOW,
        };
        self.window = Some(Window {
            session: self.session,
            seq: 0,
            deadline: now + length,
        });
        let listen_ctr = match kind {
            WakeKind::WakeWord => 0,
            WakeKind::FollowUp => listen_ctr,
        };
        Some(self.encode(&BoardMsg::Wake {
            nonce,
            session: self.session,
            kind,
            listen_ctr,
        }))
    }

    /// The Audio datagrams for `samples` (16 kHz mono), or none when no window is open.
    pub fn audio(&mut self, samples: &[i16]) -> Vec<Vec<u8>> {
        let Some(window) = self.window.as_mut() else {
            return Vec::new();
        };
        let mut msgs = Vec::new();
        for chunk in samples.chunks(MAX_SAMPLES) {
            window.seq += 1;
            msgs.push(BoardMsg::Audio {
                session: window.session,
                seq: window.seq,
                samples: chunk.to_vec(),
            });
        }
        msgs.iter().map(|m| self.encode(m)).collect()
    }

    /// Closes the window once its time is up and returns its End.
    pub fn tick(&mut self, now: Instant) -> Option<Vec<u8>> {
        if self.window.as_ref().is_some_and(|w| now >= w.deadline) {
            self.close_local()
        } else {
            None
        }
    }

    /// Closes the open window from this side and returns its End.
    pub fn close_local(&mut self) -> Option<Vec<u8>> {
        let window = self.window.take()?;
        Some(self.encode(&BoardMsg::End {
            session: window.session,
        }))
    }

    pub fn window_open(&self) -> bool {
        self.window.is_some()
    }

    fn encode(&self, msg: &BoardMsg) -> Vec<u8> {
        encode_board(&self.key, &self.sid, msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddrV4};
    use std::time::{Duration, Instant};

    use nidoo_voice::{BoardMsg, HubMsg, Key, WakeKind, decode_board, encode_hub};

    const SID: &str = "0123456789ab";
    const HUB: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 10), 4960);

    fn key() -> Key {
        Key::from_bytes(std::array::from_fn(|i| i as u8))
    }

    fn link() -> VoiceLink {
        VoiceLink::new(key(), SID.to_string())
    }

    fn hello(nonce: u64, hub_ctr: u64) -> Vec<u8> {
        encode_hub(
            &key(),
            SID,
            &HubMsg::Hello {
                nonce,
                hub_ctr,
                hub: HUB,
            },
        )
    }

    fn board(bytes: &[u8]) -> BoardMsg {
        let (sid, msg) = decode_board(&key(), bytes).unwrap();
        assert_eq!(sid, SID);
        msg
    }

    #[test]
    fn no_wake_before_hello() {
        let mut l = link();
        assert_eq!(l.open(WakeKind::WakeWord, 0, Instant::now()), None);
        assert!(!l.window_open());
        assert!(l.audio(&[1, 2, 3]).is_empty());
    }

    #[test]
    fn wake_echoes_latest_nonce_and_sessions_rise() {
        let t = Instant::now();
        let mut l = link();
        assert_eq!(l.on_datagram(&hello(5, 10), t), Some(VoiceEvent::Greeted));
        assert_eq!(l.on_datagram(&hello(9, 11), t), Some(VoiceEvent::Greeted));
        let wake = l.open(WakeKind::WakeWord, 0, t).unwrap();
        assert_eq!(
            board(&wake),
            BoardMsg::Wake {
                nonce: 9,
                session: 1,
                kind: WakeKind::WakeWord,
                listen_ctr: 0
            }
        );
        assert_eq!(l.open(WakeKind::WakeWord, 0, t), None, "already open");
        assert_eq!(
            board(&l.close_local().unwrap()),
            BoardMsg::End { session: 1 }
        );
        assert_eq!(l.close_local(), None, "nothing left to close");
        let wake = l.open(WakeKind::WakeWord, 0, t).unwrap();
        assert!(matches!(board(&wake), BoardMsg::Wake { session: 2, .. }));
    }

    #[test]
    fn streams_to_hello_address_not_source() {
        let mut l = link();
        assert_eq!(l.hub(), None);
        l.on_datagram(&hello(1, 1), Instant::now());
        assert_eq!(l.hub(), Some(HUB));
    }

    #[test]
    fn audio_splits_at_640_with_rising_seq() {
        let t = Instant::now();
        let mut l = link();
        l.on_datagram(&hello(1, 1), t);
        l.open(WakeKind::WakeWord, 0, t).unwrap();
        let samples: Vec<i16> = (0..1500).map(|i| i as i16).collect();
        let out: Vec<BoardMsg> = l.audio(&samples).iter().map(|d| board(d)).collect();
        let shape: Vec<(u32, u32, usize)> = out
            .iter()
            .map(|m| match m {
                BoardMsg::Audio {
                    session,
                    seq,
                    samples,
                } => (*session, *seq, samples.len()),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(shape, vec![(1, 1, 640), (1, 2, 640), (1, 3, 220)]);
        let BoardMsg::Audio { samples: last, .. } = &out[2] else {
            unreachable!()
        };
        assert_eq!(last[0], 1280);
        // A new window starts again at seq 1.
        l.close_local();
        l.open(WakeKind::WakeWord, 0, t).unwrap();
        assert!(matches!(
            board(&l.audio(&[0])[0]),
            BoardMsg::Audio {
                session: 2,
                seq: 1,
                ..
            }
        ));
    }

    #[test]
    fn stop_closes_only_its_session() {
        let t = Instant::now();
        let mut l = link();
        l.on_datagram(&hello(1, 1), t);
        l.open(WakeKind::WakeWord, 0, t).unwrap();
        let other = encode_hub(
            &key(),
            SID,
            &HubMsg::Stop {
                hub_ctr: 2,
                session: 7,
            },
        );
        assert_eq!(l.on_datagram(&other, t), None);
        assert!(l.window_open());
        let ours = encode_hub(
            &key(),
            SID,
            &HubMsg::Stop {
                hub_ctr: 3,
                session: 1,
            },
        );
        assert_eq!(l.on_datagram(&ours, t), Some(VoiceEvent::Stop));
        assert!(!l.window_open());
        assert!(l.audio(&[1]).is_empty());
        assert_eq!(
            l.tick(t + Duration::from_secs(9)),
            None,
            "no End after a Stop"
        );
    }

    #[test]
    fn window_times_out_with_end() {
        let t = Instant::now();
        let mut l = link();
        l.on_datagram(&hello(1, 1), t);
        l.open(WakeKind::WakeWord, 0, t).unwrap();
        assert_eq!(l.tick(t + Duration::from_millis(7_999)), None);
        let end = l.tick(t + Duration::from_secs(8)).unwrap();
        assert_eq!(board(&end), BoardMsg::End { session: 1 });
        assert!(!l.window_open());

        let listen = encode_hub(&key(), SID, &HubMsg::Listen { hub_ctr: 2 });
        l.on_datagram(&listen, t);
        l.open(WakeKind::FollowUp, 2, t).unwrap();
        assert_eq!(l.tick(t + Duration::from_millis(5_999)), None);
        let end = l.tick(t + Duration::from_secs(6)).unwrap();
        assert_eq!(board(&end), BoardMsg::End { session: 2 });
    }

    #[test]
    fn listen_opens_follow_up() {
        let t = Instant::now();
        let mut l = link();
        l.on_datagram(&hello(4, 1), t);
        let listen = encode_hub(&key(), SID, &HubMsg::Listen { hub_ctr: 77 });
        assert_eq!(
            l.on_datagram(&listen, t),
            Some(VoiceEvent::Listen { listen_ctr: 77 })
        );
        let wake = l.open(WakeKind::FollowUp, 77, t).unwrap();
        assert_eq!(
            board(&wake),
            BoardMsg::Wake {
                nonce: 4,
                session: 1,
                kind: WakeKind::FollowUp,
                listen_ctr: 77
            }
        );
        // A Listen while a window is open is not a new follow-up.
        let again = encode_hub(&key(), SID, &HubMsg::Listen { hub_ctr: 78 });
        assert_eq!(l.on_datagram(&again, t), None);
    }

    #[test]
    fn ignores_wrong_key_other_sid_and_stale_ctr() {
        let t = Instant::now();
        let mut l = link();
        let wrong_key = encode_hub(
            &Key::from_bytes([9; 32]),
            SID,
            &HubMsg::Hello {
                nonce: 1,
                hub_ctr: 1,
                hub: HUB,
            },
        );
        assert_eq!(l.on_datagram(&wrong_key, t), None);
        let other_sid = encode_hub(
            &key(),
            "ffffffffffff",
            &HubMsg::Hello {
                nonce: 1,
                hub_ctr: 1,
                hub: HUB,
            },
        );
        assert_eq!(l.on_datagram(&other_sid, t), None);
        assert_eq!(l.on_datagram(b"junk", t), None);
        assert!(!l.greeted(t));
        assert_eq!(l.hub(), None);

        assert_eq!(l.on_datagram(&hello(5, 10), t), Some(VoiceEvent::Greeted));
        assert_eq!(l.on_datagram(&hello(6, 10), t), None, "same ctr");
        assert_eq!(l.on_datagram(&hello(6, 9), t), None, "older ctr");
        let wake = l.open(WakeKind::WakeWord, 0, t).unwrap();
        assert!(matches!(board(&wake), BoardMsg::Wake { nonce: 5, .. }));
        let stale_stop = encode_hub(
            &key(),
            SID,
            &HubMsg::Stop {
                hub_ctr: 3,
                session: 1,
            },
        );
        assert_eq!(l.on_datagram(&stale_stop, t), None);
        assert!(l.window_open());
    }

    #[test]
    fn greeted_expires_after_15s_and_comes_back() {
        let t = Instant::now();
        let mut l = link();
        assert!(!l.greeted(t));
        l.on_datagram(&hello(1, 1), t);
        assert!(l.greeted(t + Duration::from_millis(14_999)));
        assert!(!l.greeted(t + Duration::from_secs(15)));
        assert_eq!(
            l.open(WakeKind::WakeWord, 0, t + Duration::from_secs(15)),
            None
        );
        let later = t + Duration::from_secs(40);
        l.on_datagram(&hello(2, 2), later);
        assert!(l.greeted(later));
        assert!(l.open(WakeKind::WakeWord, 0, later).is_some());
    }
}
