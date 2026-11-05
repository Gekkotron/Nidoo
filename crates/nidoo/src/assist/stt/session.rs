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
mod tests {
    use super::super::engine::fakes::FakeVad;
    use super::*;

    const CFG: SessionCfg = SessionCfg {
        gain: 4.0,
        listen_margin_ms: 300,
    };

    struct Rig {
        s: Session,
        vad: FakeVad,
        now: u64,
        seq: u32,
    }

    impl Rig {
        fn new() -> Rig {
            Rig {
                s: Session::new(CFG),
                vad: FakeVad::default(),
                now: 1_000,
                seq: 0,
            }
        }

        fn on(&mut self, input: Input) -> Vec<Output> {
            self.s.on(input, self.now, &mut self.vad)
        }

        fn wake(&mut self, session: u32) -> Vec<Output> {
            self.on(Input::Wake {
                session,
                kind: WakeKind::WakeWord,
                listen_ctr: 0,
            })
        }

        /// `ms` of audio in 40 ms packets, loud (speech) or silent, 40 ms apart.
        fn audio(&mut self, session: u32, ms: u64, loud: bool) -> Vec<Output> {
            let mut out = Vec::new();
            for _ in 0..ms / 40 {
                self.now += 40;
                self.seq += 1;
                let samples = vec![if loud { 4_000 } else { 0 }; 640];
                out.extend(self.on(Input::Audio {
                    session,
                    seq: self.seq,
                    samples,
                }));
            }
            out
        }

        fn ticks_until(&mut self, now: u64) -> Vec<Output> {
            let mut out = Vec::new();
            while self.now < now {
                self.now = (self.now + 50).min(now);
                out.extend(self.on(Input::Tick));
            }
            out
        }
    }

    fn has_transcribe(out: &[Output]) -> bool {
        out.iter().any(|o| matches!(o, Output::Transcribe(_)))
    }

    /// Wake, a phrase and 0.5 s of silence: the board is stopped and the phrase transcribed.
    fn to_transcribing(r: &mut Rig) {
        assert_eq!(r.wake(1), vec![Output::Hold]);
        assert!(r.audio(1, 800, true).is_empty());
        let out = r.audio(1, 600, false);
        assert_eq!(out.first(), Some(&Output::Stop { session: 1 }), "{out:?}");
        assert!(has_transcribe(&out));
    }

    #[test]
    fn wake_then_phrase_asks_the_assistant() {
        let mut r = Rig::new();
        to_transcribing(&mut r);
        let out = r.on(Input::Transcript("Jarvis, allume la lumière.".into()));
        assert_eq!(out, vec![Output::Ask("allume la lumière.".into())]);
        let out = r.on(Input::Replied {
            clip: Some("c".into()),
            question: false,
        });
        assert!(out.is_empty());
        assert!(
            r.on(Input::Spoken {
                clip: "other".into()
            })
            .is_empty()
        );
        assert_eq!(
            r.on(Input::Spoken { clip: "c".into() }),
            vec![Output::Release]
        );
        assert!(r.s.is_idle());
    }

    #[test]
    fn the_gain_is_applied_before_the_vad() {
        let mut r = Rig::new();
        r.wake(1);
        // The fake VAD hears speech above 0.01 (328 raw): 100 raw is quiet alone, speech ×4.
        r.on(Input::Audio {
            session: 1,
            seq: 1,
            samples: vec![100; 640],
        });
        r.now += 600;
        let out = r.on(Input::End { session: 1 });
        assert!(has_transcribe(&out), "{out:?}");
    }

    #[test]
    fn question_opens_a_follow_up_after_the_margin() {
        let mut r = Rig::new();
        to_transcribing(&mut r);
        r.on(Input::Transcript("règle le chauffage".into()));
        r.on(Input::Replied {
            clip: Some("q".into()),
            question: true,
        });
        let spoken_at = r.now;
        assert!(r.on(Input::Spoken { clip: "q".into() }).is_empty());
        let before = r.ticks_until(spoken_at + 299);
        assert!(!before.contains(&Output::Listen), "{before:?}");
        let out = r.ticks_until(spoken_at + 350);
        assert_eq!(out, vec![Output::Listen]);
        r.s.listening(77);
        let out = r.on(Input::Wake {
            session: 2,
            kind: WakeKind::FollowUp,
            listen_ctr: 77,
        });
        assert!(out.is_empty(), "no new hold: {out:?}");
        let out = r.audio(2, 800, true);
        assert!(out.is_empty());
        let out = r.audio(2, 600, false);
        assert!(has_transcribe(&out));
        assert!(!out.contains(&Output::Release));
    }

    #[test]
    fn margin_zero_listens_at_once() {
        let mut r = Rig::new();
        r.s = Session::new(SessionCfg {
            gain: 4.0,
            listen_margin_ms: 0,
        });
        to_transcribing(&mut r);
        r.on(Input::Transcript("règle".into()));
        r.on(Input::Replied {
            clip: Some("q".into()),
            question: true,
        });
        assert_eq!(
            r.on(Input::Spoken { clip: "q".into() }),
            vec![Output::Listen]
        );
    }

    #[test]
    fn follow_up_without_answer_releases() {
        // No window opens within 2 s.
        let mut r = Rig::new();
        to_transcribing(&mut r);
        r.on(Input::Transcript("règle".into()));
        r.on(Input::Replied {
            clip: Some("q".into()),
            question: true,
        });
        r.on(Input::Spoken { clip: "q".into() });
        let start = r.now;
        r.ticks_until(start + 300);
        r.s.listening(5);
        let out = r.ticks_until(start + 300 + FOLLOW_UP_START_MS + 100);
        assert_eq!(out, vec![Output::Release]);
        assert!(r.s.is_idle());

        // A window opens, then nothing is said until the board closes it.
        let mut r = Rig::new();
        to_transcribing(&mut r);
        r.on(Input::Transcript("règle".into()));
        r.on(Input::Replied {
            clip: Some("q".into()),
            question: true,
        });
        r.on(Input::Spoken { clip: "q".into() });
        r.ticks_until(r.now + 300);
        r.s.listening(5);
        r.on(Input::Wake {
            session: 2,
            kind: WakeKind::FollowUp,
            listen_ctr: 5,
        });
        r.audio(2, 2_000, false);
        let out = r.on(Input::End { session: 2 });
        assert_eq!(out, vec![Output::Release]);
        assert!(r.s.is_idle());
    }

    #[test]
    fn follow_up_with_a_wrong_counter_is_ignored() {
        let mut r = Rig::new();
        to_transcribing(&mut r);
        r.on(Input::Transcript("règle".into()));
        r.on(Input::Replied {
            clip: Some("q".into()),
            question: true,
        });
        r.on(Input::Spoken { clip: "q".into() });
        r.ticks_until(r.now + 300);
        r.s.listening(5);
        let out = r.on(Input::Wake {
            session: 2,
            kind: WakeKind::FollowUp,
            listen_ctr: 6,
        });
        assert!(out.is_empty());
        assert!(r.audio(2, 800, true).is_empty(), "not listening");
    }

    #[test]
    fn no_speech_closes_silently() {
        let mut r = Rig::new();
        r.wake(1);
        assert!(r.audio(1, 2_000, false).is_empty());
        assert_eq!(r.on(Input::End { session: 1 }), vec![Output::Release]);
        assert!(r.s.is_idle());
    }

    #[test]
    fn empty_transcript_closes_silently() {
        let mut r = Rig::new();
        to_transcribing(&mut r);
        assert_eq!(
            r.on(Input::Transcript("Jarvis.".into())),
            vec![Output::Release]
        );
        assert!(r.s.is_idle());
    }

    #[test]
    fn no_clip_releases_at_once() {
        let mut r = Rig::new();
        to_transcribing(&mut r);
        r.on(Input::Transcript("allume".into()));
        let out = r.on(Input::Replied {
            clip: None,
            question: true,
        });
        assert_eq!(out, vec![Output::Release]);
        assert!(r.s.is_idle());
    }

    #[test]
    fn spoken_never_comes_gives_up() {
        let mut r = Rig::new();
        to_transcribing(&mut r);
        r.on(Input::Transcript("allume".into()));
        r.on(Input::Replied {
            clip: Some("c".into()),
            question: false,
        });
        let out = r.ticks_until(r.now + SPOKEN_WAIT_MS + 100);
        assert_eq!(out.last(), Some(&Output::Release), "{out:?}");
        assert_eq!(out.iter().filter(|o| **o == Output::Release).count(), 1);
        assert!(r.s.is_idle());
    }

    #[test]
    fn a_stuck_assistant_gives_up_too() {
        let mut r = Rig::new();
        to_transcribing(&mut r);
        let out = r.ticks_until(r.now + SPOKEN_WAIT_MS + 100);
        assert_eq!(out.last(), Some(&Output::Release), "no transcript: {out:?}");
        let mut r = Rig::new();
        to_transcribing(&mut r);
        r.on(Input::Transcript("allume".into()));
        let out = r.ticks_until(r.now + SPOKEN_WAIT_MS + 100);
        assert_eq!(out.last(), Some(&Output::Release), "no reply: {out:?}");
    }

    #[test]
    fn wake_while_busy_is_ignored() {
        let mut r = Rig::new();
        to_transcribing(&mut r);
        assert!(r.wake(2).is_empty(), "transcribing");
        r.on(Input::Transcript("allume".into()));
        assert!(r.wake(3).is_empty(), "thinking");
        r.on(Input::Replied {
            clip: Some("c".into()),
            question: false,
        });
        assert!(r.wake(4).is_empty(), "speaking");
        assert!(r.audio(4, 400, true).is_empty());
        assert_eq!(
            r.on(Input::Spoken { clip: "c".into() }),
            vec![Output::Release]
        );
    }

    #[test]
    fn a_follow_up_wake_while_idle_is_ignored() {
        let mut r = Rig::new();
        let out = r.on(Input::Wake {
            session: 1,
            kind: WakeKind::FollowUp,
            listen_ctr: 0,
        });
        assert!(out.is_empty());
        assert!(r.s.is_idle());
    }

    #[test]
    fn second_wake_in_the_same_room_is_ignored() {
        let mut rooms = Rooms::default();
        assert!(rooms.admit("a", "living"));
        assert!(!rooms.admit("b", "living"));
        assert!(rooms.admit("c", "kitchen"));
        assert!(rooms.admit("a", "living"), "the same satellite again");
        rooms.done("a");
        assert!(rooms.admit("b", "living"));
    }

    #[test]
    fn board_silence_ends_the_window() {
        let mut r = Rig::new();
        r.wake(1);
        r.audio(1, 400, true);
        let out = r.ticks_until(r.now + IDLE_GAP_MS + 50);
        assert_eq!(out.first(), Some(&Output::Stop { session: 1 }), "{out:?}");
        assert!(has_transcribe(&out));
    }

    #[test]
    fn hold_is_refreshed_every_10_s() {
        let mut r = Rig::new();
        to_transcribing(&mut r);
        r.on(Input::Transcript("allume".into()));
        r.on(Input::Replied {
            clip: Some("c".into()),
            question: false,
        });
        let out = r.ticks_until(r.now + 25_000);
        assert_eq!(
            out.iter().filter(|o| **o == Output::Hold).count(),
            2,
            "{out:?}"
        );
    }

    #[test]
    fn stale_audio_is_dropped() {
        let mut r = Rig::new();
        r.wake(1);
        r.audio(1, 400, true);
        let last = r.seq;
        // Another session's or an older packet adds nothing: only silence was added after.
        r.on(Input::Audio {
            session: 9,
            seq: last + 1,
            samples: vec![4_000; 640],
        });
        r.on(Input::Audio {
            session: 1,
            seq: last,
            samples: vec![4_000; 640],
        });
        let out = r.audio(1, 600, false);
        let Some(Output::Transcribe(phrase)) =
            out.iter().find(|o| matches!(o, Output::Transcribe(_)))
        else {
            panic!("{out:?}")
        };
        assert_eq!(
            phrase.len(),
            10 * 640,
            "only the session's own, newer packets"
        );
    }

    /// The hub's question is spoken (clip `c1`), the window reopens and a phrase is heard,
    /// up to the transcription.
    fn ask_to_transcribing(r: &mut Rig) {
        assert_eq!(r.on(Input::AskStart), vec![Output::Hold]);
        assert!(r.on(Input::AskClip(Some("c1".into()))).is_empty());
        let spoken_at = r.now;
        assert!(r.on(Input::Spoken { clip: "c1".into() }).is_empty());
        assert_eq!(r.ticks_until(spoken_at + 350), vec![Output::Listen]);
        r.s.listening(7);
        let out = r.on(Input::Wake {
            session: 2,
            kind: WakeKind::FollowUp,
            listen_ctr: 7,
        });
        assert!(out.is_empty(), "{out:?}");
        assert!(r.audio(2, 800, true).is_empty());
        let out = r.audio(2, 600, false);
        assert!(has_transcribe(&out), "{out:?}");
    }

    #[test]
    fn ask_speaks_then_listens_then_answers() {
        let mut r = Rig::new();
        assert!(!r.s.is_asking());
        ask_to_transcribing(&mut r);
        assert!(r.s.is_asking());
        let out = r.on(Input::Transcript("oui à 21".into()));
        assert_eq!(
            out,
            vec![Output::Answer(Some("oui à 21".into())), Output::Release]
        );
        assert!(r.s.is_idle());
        assert!(!r.s.is_asking());
    }

    #[test]
    fn asked_transcript_is_stripped_of_the_wake_word() {
        let mut r = Rig::new();
        ask_to_transcribing(&mut r);
        let out = r.on(Input::Transcript("hey jarvis oui".into()));
        assert_eq!(
            out,
            vec![Output::Answer(Some("oui".into())), Output::Release]
        );
    }

    #[test]
    fn an_empty_asked_transcript_answers_none() {
        let mut r = Rig::new();
        ask_to_transcribing(&mut r);
        let out = r.on(Input::Transcript("Jarvis.".into()));
        assert_eq!(out, vec![Output::Answer(None), Output::Release]);
        assert!(r.s.is_idle());
    }

    #[test]
    fn ask_silence_answers_none() {
        let mut r = Rig::new();
        r.on(Input::AskStart);
        r.on(Input::AskClip(Some("c1".into())));
        r.on(Input::Spoken { clip: "c1".into() });
        r.ticks_until(r.now + 300);
        r.s.listening(7);
        let out = r.ticks_until(r.now + FOLLOW_UP_START_MS + 100);
        assert_eq!(out, vec![Output::Answer(None), Output::Release]);
        assert!(r.s.is_idle());
        assert!(!r.s.is_asking());
    }

    #[test]
    fn ask_without_clip_answers_none() {
        let mut r = Rig::new();
        r.on(Input::AskStart);
        let out = r.on(Input::AskClip(None));
        assert_eq!(out, vec![Output::Answer(None), Output::Release]);
        assert!(r.s.is_idle());
    }

    #[test]
    fn ask_cancel_releases_without_answer() {
        let mut r = Rig::new();
        r.on(Input::AskStart);
        r.on(Input::AskClip(Some("c1".into())));
        assert_eq!(r.on(Input::AskCancel), vec![Output::Release]);
        assert!(r.s.is_idle());
        assert!(!r.s.is_asking());

        let mut r = Rig::new();
        r.on(Input::AskStart);
        assert_eq!(r.on(Input::AskCancel), vec![Output::Release], "preparing");
    }

    #[test]
    fn ask_cancel_does_not_touch_a_normal_exchange() {
        let mut r = Rig::new();
        to_transcribing(&mut r);
        assert!(r.on(Input::AskCancel).is_empty());
        assert!(!r.s.is_idle());
    }

    #[test]
    fn ask_start_ignored_when_busy() {
        let mut r = Rig::new();
        r.wake(1);
        assert!(r.on(Input::AskStart).is_empty());
        assert!(!r.s.is_asking());
    }

    #[test]
    fn wake_word_during_ask_is_ignored() {
        let mut r = Rig::new();
        r.on(Input::AskStart);
        assert!(r.wake(1).is_empty(), "preparing");
        r.on(Input::AskClip(Some("c1".into())));
        assert!(r.wake(1).is_empty(), "speaking");
        assert!(r.s.is_asking());
        r.on(Input::Spoken { clip: "c1".into() });
        r.ticks_until(r.now + 300);
        r.s.listening(7);
        r.on(Input::Wake {
            session: 2,
            kind: WakeKind::FollowUp,
            listen_ctr: 7,
        });
        r.audio(2, 800, true);
        r.audio(2, 600, false);
        let out = r.on(Input::Transcript("non".into()));
        assert_eq!(
            out,
            vec![Output::Answer(Some("non".into())), Output::Release]
        );
    }

    #[test]
    fn a_stuck_ask_expires_with_none() {
        let mut r = Rig::new();
        r.on(Input::AskStart);
        let out = r.ticks_until(r.now + SPOKEN_WAIT_MS + 100);
        let tail: Vec<_> = out.iter().rev().take(2).rev().cloned().collect();
        assert_eq!(tail, vec![Output::Answer(None), Output::Release], "{out:?}");
        assert!(r.s.is_idle());
    }

    #[test]
    fn the_ask_hold_is_refreshed() {
        let mut r = Rig::new();
        r.on(Input::AskStart);
        let out = r.ticks_until(r.now + 25_000);
        assert_eq!(
            out.iter().filter(|o| **o == Output::Hold).count(),
            2,
            "{out:?}"
        );
    }
}
