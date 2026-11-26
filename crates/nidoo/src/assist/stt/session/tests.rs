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
    let Some(Output::Transcribe(phrase)) = out.iter().find(|o| matches!(o, Output::Transcribe(_)))
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
