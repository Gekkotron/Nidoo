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
mod tests;
