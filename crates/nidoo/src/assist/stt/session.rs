//! One spoken exchange with one satellite, from the wake word to the last reply.
//!
//! Pure: the voice service feeds it what the board sends, what the recognizer and the
//! assistant return and a clock tick, and carries out what it asks (hold the room's music,
//! close the board's window, transcribe, ask the assistant, reopen the window). Every way
//! back to idle releases the hold, and every wait has a limit, so a lost reply never leaves
//! a room silent.

use std::collections::HashMap;

use super::engine::{Vad, strip_wake_word};
use super::packet::WakeKind;

/// No packet for this long ends the window (the board went quiet or off the network).
pub const IDLE_GAP_MS: u64 = 1_000;
/// The board closes a wake word window after this long.
pub const WINDOW_MS: u64 = 8_000;
/// The board closes a follow-up window after this long.
pub const FOLLOW_UP_MS: u64 = 6_000;
/// A follow-up window must open this soon after the Listen.
pub const FOLLOW_UP_START_MS: u64 = 2_000;
/// The longest wait for a transcript, a reply or the end of its playback.
pub const SPOKEN_WAIT_MS: u64 = 60_000;
/// How often the hold is sent again while the exchange lasts.
pub const HOLD_REFRESH_MS: u64 = 10_000;

/// What happened.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Wake {
        session: u32,
        kind: WakeKind,
        listen_ctr: u64,
    },
    Audio {
        session: u32,
        seq: u32,
        samples: Vec<i16>,
    },
    End {
        session: u32,
    },
    /// The recognizer's text for the phrase.
    Transcript(String),
    /// The assistant answered; `clip` is the reply being spoken (`None`: nothing is said).
    Replied {
        clip: Option<String>,
        question: bool,
    },
    /// A clip finished playing (or was dropped).
    Spoken {
        clip: String,
    },
    Tick,
    /// The hub starts an exchange by speaking its own question (ignored unless idle).
    AskStart,
    /// The question's clip is ready (`None`: it could not be made).
    AskClip(Option<String>),
    /// The hub gave the question up: end the exchange without an answer.
    AskCancel,
}

/// What to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    /// Hold the room's music (sent again every [`HOLD_REFRESH_MS`]).
    Hold,
    /// Give the music back.
    Release,
    /// Close the board's window.
    Stop { session: u32 },
    /// Transcribe this phrase (16 kHz, -1.0 to 1.0).
    Transcribe(Vec<f32>),
    /// Ask the assistant.
    Ask(String),
    /// Reopen the board's window; report its counter with [`Session::listening`].
    Listen,
    /// The hub's question was answered with this text (`None`: nothing usable was heard).
    /// Always comes before the [`Output::Release`] of an asking session.
    Answer(Option<String>),
}

/// Session tuning from the settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SessionCfg {
    pub gain: f32,
    pub listen_margin_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
enum Phase {
    Idle,
    Listening {
        session: u32,
        last_seq: Option<u32>,
        last_packet: u64,
        until: u64,
    },
    Transcribing {
        since: u64,
    },
    /// The hub's question is being made into a clip.
    Preparing {
        since: u64,
    },
    Thinking {
        since: u64,
    },
    Speaking {
        clip: String,
        question: bool,
        since: u64,
    },
    Margin {
        until: u64,
    },
    FollowUp {
        listen_ctr: Option<u64>,
        until: u64,
    },
}

/// One satellite's exchange.
#[derive(Debug)]
pub struct Session {
    cfg: SessionCfg,
    phase: Phase,
    last_hold: u64,
    /// The exchange is the hub's question: the transcript goes back as an answer.
    asking: bool,
}

fn listening(session: u32, now: u64, window: u64) -> Phase {
    Phase::Listening {
        session,
        last_seq: None,
        last_packet: now,
        until: now + window,
    }
}

fn follow_up(now: u64) -> Phase {
    Phase::FollowUp {
        listen_ctr: None,
        until: now + FOLLOW_UP_START_MS,
    }
}

impl Session {
    pub fn new(cfg: SessionCfg) -> Session {
        Session {
            cfg,
            phase: Phase::Idle,
            last_hold: 0,
            asking: false,
        }
    }

    /// The exchange in progress is the hub's own question.
    pub fn is_asking(&self) -> bool {
        self.asking
    }

    #[cfg(test)]
    pub fn is_speaking(&self) -> bool {
        matches!(self.phase, Phase::Speaking { .. })
    }

    pub fn is_idle(&self) -> bool {
        self.phase == Phase::Idle
    }

    /// The window `session` of the board is open and heard.
    pub fn is_listening(&self, session: u32) -> bool {
        matches!(self.phase, Phase::Listening { session: open, .. } if open == session)
    }

    /// The Listen asked for went out with this counter.
    pub fn listening(&mut self, listen_ctr: u64) {
        if let Phase::FollowUp {
            listen_ctr: ctr, ..
        } = &mut self.phase
        {
            *ctr = Some(listen_ctr);
        }
    }

    /// Feeds one input at `now_ms`; what to do, in order.
    pub fn on(&mut self, input: Input, now_ms: u64, vad: &mut dyn Vad) -> Vec<Output> {
        let now = now_ms;
        match (input, &mut self.phase) {
            (
                Input::Wake {
                    session,
                    kind: WakeKind::WakeWord,
                    ..
                },
                Phase::Idle,
            ) => {
                vad.reset();
                self.phase = listening(session, now, WINDOW_MS);
                self.last_hold = now;
                vec![Output::Hold]
            }
            (
                Input::Wake {
                    session,
                    kind: WakeKind::FollowUp,
                    listen_ctr,
                },
                Phase::FollowUp {
                    listen_ctr: Some(expected),
                    ..
                },
            ) if listen_ctr == *expected => {
                vad.reset();
                self.phase = listening(session, now, FOLLOW_UP_MS);
                Vec::new()
            }
            (
                Input::Audio {
                    session,
                    seq,
                    samples,
                },
                Phase::Listening {
                    session: open,
                    last_seq,
                    last_packet,
                    ..
                },
            ) if session == *open && last_seq.is_none_or(|last| seq > last) => {
                *last_seq = Some(seq);
                *last_packet = now;
                let gain = self.cfg.gain;
                let amplified: Vec<f32> = samples
                    .iter()
                    .map(|&v| (f32::from(v) / 32_768.0 * gain).clamp(-1.0, 1.0))
                    .collect();
                vad.accept(&amplified);
                match vad.pop_segment() {
                    Some(phrase) => {
                        self.phase = Phase::Transcribing { since: now };
                        vec![Output::Stop { session }, Output::Transcribe(phrase)]
                    }
                    None => Vec::new(),
                }
            }
            (Input::End { session }, Phase::Listening { session: open, .. })
                if session == *open =>
            {
                self.end_window(None, now, vad)
            }
            (Input::Transcript(text), Phase::Transcribing { .. }) => {
                let text = strip_wake_word(&text);
                if text.is_empty() {
                    self.release()
                } else if self.asking {
                    self.asking = false;
                    let mut out = vec![Output::Answer(Some(text))];
                    out.extend(self.release());
                    out
                } else {
                    self.phase = Phase::Thinking { since: now };
                    vec![Output::Ask(text)]
                }
            }
            (Input::Replied { clip, question }, Phase::Thinking { .. }) => match clip {
                Some(clip) => {
                    self.phase = Phase::Speaking {
                        clip,
                        question,
                        since: now,
                    };
                    Vec::new()
                }
                None => self.release(),
            },
            (
                Input::Spoken { clip },
                Phase::Speaking {
                    clip: playing,
                    question,
                    ..
                },
            ) if clip == *playing => {
                if !*question {
                    return self.release();
                }
                if self.cfg.listen_margin_ms == 0 {
                    self.phase = follow_up(now);
                    return vec![Output::Listen];
                }
                self.phase = Phase::Margin {
                    until: now + self.cfg.listen_margin_ms,
                };
                Vec::new()
            }
            (Input::AskStart, Phase::Idle) => {
                self.asking = true;
                self.phase = Phase::Preparing { since: now };
                self.last_hold = now;
                vec![Output::Hold]
            }
            (Input::AskClip(clip), Phase::Preparing { .. }) => match clip {
                Some(clip) => {
                    self.phase = Phase::Speaking {
                        clip,
                        question: true,
                        since: now,
                    };
                    Vec::new()
                }
                None => self.release(),
            },
            (Input::AskCancel, phase) if self.asking && *phase != Phase::Idle => {
                self.asking = false;
                self.release()
            }
            (Input::Tick, _) => self.on_tick(now, vad),
            _ => Vec::new(),
        }
    }

    fn on_tick(&mut self, now: u64, vad: &mut dyn Vad) -> Vec<Output> {
        let mut out = Vec::new();
        let expired = match &self.phase {
            Phase::Idle => return out,
            Phase::Listening {
                session,
                last_packet,
                until,
                ..
            } => {
                if now >= last_packet + IDLE_GAP_MS || now >= until + IDLE_GAP_MS {
                    let session = *session;
                    out.extend(self.end_window(Some(session), now, vad));
                    return self.with_refresh(now, out);
                }
                false
            }
            Phase::Transcribing { since }
            | Phase::Preparing { since }
            | Phase::Thinking { since }
            | Phase::Speaking { since, .. } => now >= since + SPOKEN_WAIT_MS,
            Phase::Margin { until } => {
                if now >= *until {
                    self.phase = follow_up(now);
                    out.push(Output::Listen);
                }
                false
            }
            Phase::FollowUp { until, .. } => now >= *until,
        };
        if expired {
            return self.release();
        }
        self.with_refresh(now, out)
    }

    /// Adds a hold refresh when one is due and the exchange goes on.
    fn with_refresh(&mut self, now: u64, mut out: Vec<Output>) -> Vec<Output> {
        if !self.is_idle() && now >= self.last_hold + HOLD_REFRESH_MS {
            self.last_hold = now;
            out.push(Output::Hold);
        }
        out
    }

    /// The window is over: what was heard is transcribed, or the exchange ends. `stop` is
    /// the session to close on the board (it may not know yet).
    fn end_window(&mut self, stop: Option<u32>, now: u64, vad: &mut dyn Vad) -> Vec<Output> {
        vad.flush();
        match vad.pop_segment() {
            Some(phrase) => {
                self.phase = Phase::Transcribing { since: now };
                let mut out: Vec<Output> = stop
                    .map(|session| Output::Stop { session })
                    .into_iter()
                    .collect();
                out.push(Output::Transcribe(phrase));
                out
            }
            None => {
                let mut out: Vec<Output> = stop
                    .map(|session| Output::Stop { session })
                    .into_iter()
                    .collect();
                out.extend(self.release());
                out
            }
        }
    }

    fn release(&mut self) -> Vec<Output> {
        self.phase = Phase::Idle;
        if std::mem::take(&mut self.asking) {
            return vec![Output::Answer(None), Output::Release];
        }
        vec![Output::Release]
    }
}

/// One exchange per room at a time: a second satellite that hears the same wake word is
/// ignored while the first one's exchange lasts.
#[derive(Debug, Default)]
pub struct Rooms {
    busy: HashMap<String, String>,
}

impl Rooms {
    /// Whether `sid` may start an exchange in `room` (the room id, or the sid itself for a
    /// satellite without a room); it then holds the room until [`Rooms::done`].
    pub fn admit(&mut self, sid: &str, room: &str) -> bool {
        match self.busy.get(room) {
            Some(owner) => owner == sid,
            None => {
                self.busy.insert(room.to_owned(), sid.to_owned());
                true
            }
        }
    }

    pub fn done(&mut self, sid: &str) {
        self.busy.retain(|_, owner| owner != sid);
    }
}

#[cfg(test)]
mod tests;
