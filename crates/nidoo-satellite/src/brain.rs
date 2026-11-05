//! The satellite's decisions, with no I/O: hub datagrams, microphone samples, menu
//! choices and clock ticks in; datagrams to send, microphone and wake-mute switches out.
//! `run.rs` does the I/O around it.

use std::net::SocketAddrV4;
use std::time::{Duration, Instant};

use nidoo_voice::{Key, WakeKind};

use crate::voice::{VoiceEvent, VoiceLink};
use crate::wake_head::TrainProgress;

/// The playback stays muted at least this long after a wake word (the board's rule).
pub const WAKE_MUTE_FLOOR: Duration = Duration::from_secs(1);

const MIC_NEEDED: &str = "Microphone access needed (System Settings → Privacy)";

/// The wake-word engine, fed 16 kHz mono samples while no voice window is open.
pub trait WakeDetector: Send {
    /// True when the wake word was heard in what was pushed so far.
    fn push(&mut self, samples_16k: &[i16]) -> bool;
    /// Forgets what it heard (after a window, so the same words cannot fire again).
    fn reset(&mut self);
}

/// "Train Hey Jarvis": the shell's trainer saves the head and makes the detector
/// reload it.
pub trait Trainer: Send {
    fn start(&mut self);
    fn push(&mut self, samples_16k: &[i16]) -> TrainProgress;
    fn cancel(&mut self);
}

/// The menu-bar icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconState {
    /// The hub cannot be reached (or this Mac is not set up yet).
    Gray,
    Ready,
    /// A voice window is open, or training is listening.
    Listening,
    /// Listening is off, or the microphone cannot be opened.
    MicOff,
}

/// What the menu shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiState {
    pub icon: IconState,
    pub status: String,
    pub listening: bool,
    /// "Train Hey Jarvis" is running (the menu offers Cancel).
    pub training: bool,
}

/// What the I/O side does for the brain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// A datagram for the hub, at the address inside its Hello.
    Send(SocketAddrV4, Vec<u8>),
    /// Open (true) or close the microphone. A failure is reported with
    /// [`Brain::mic_failed`].
    Mic(bool),
    /// Mute our playback for a voice window, or let it play again.
    WakeMuted(bool),
}

pub struct Brain {
    sid: String,
    greeting_ttl: Duration,
    link: Option<VoiceLink>,
    detector: Box<dyn WakeDetector>,
    trainer: Box<dyn Trainer>,
    sendspin: bool,
    listening: bool,
    was_greeted: bool,
    mic_on: bool,
    mic_error: bool,
    training: Option<String>,
    /// A one-off message (a refused key, a training result) shown until the next change.
    notice: Option<String>,
    wake_at: Option<Instant>,
    wake_muted: bool,
    unmute_at: Option<Instant>,
}

impl Brain {
    pub fn new(
        sid: String,
        key: Option<Key>,
        listening: bool,
        greeting_ttl: Duration,
        detector: Box<dyn WakeDetector>,
        trainer: Box<dyn Trainer>,
    ) -> Self {
        let link = key.map(|k| VoiceLink::with_greeting_ttl(k, sid.clone(), greeting_ttl));
        Self {
            sid,
            greeting_ttl,
            link,
            detector,
            trainer,
            sendspin: false,
            listening,
            was_greeted: false,
            mic_on: false,
            mic_error: false,
            training: None,
            notice: None,
            wake_at: None,
            wake_muted: false,
            unmute_at: None,
        }
    }

    pub fn on_datagram(&mut self, bytes: &[u8], now: Instant) -> Vec<Action> {
        let mut out = Vec::new();
        let event = self.link.as_mut().and_then(|l| l.on_datagram(bytes, now));
        match event {
            Some(VoiceEvent::Greeted) | None => {}
            Some(VoiceEvent::Stop) => self.window_closed(now),
            Some(VoiceEvent::Listen { listen_ctr }) => {
                if self.voice_ok(now) && self.mic_on && self.training.is_none() {
                    self.open(WakeKind::FollowUp, listen_ctr, now, &mut out);
                }
            }
        }
        self.settle(now, &mut out);
        out
    }

    pub fn on_mic(&mut self, samples: &[i16], now: Instant) -> Vec<Action> {
        let mut out = Vec::new();
        if !self.mic_on {
            // Queued before the microphone closed.
            return out;
        }
        if self.training.is_some() {
            match self.trainer.push(samples) {
                p @ (TrainProgress::Done(_) | TrainProgress::Failed(_)) => {
                    self.training = None;
                    self.notice = Some(p.status());
                    self.detector.reset();
                }
                p => self.training = Some(p.status()),
            }
        } else if let Some(link) = self.link.as_mut().filter(|l| l.window_open()) {
            for d in link.audio(samples) {
                send(link, d, &mut out);
            }
        } else if self.detector.push(samples) {
            self.open(WakeKind::WakeWord, 0, now, &mut out);
        }
        self.settle(now, &mut out);
        out
    }

    pub fn tick(&mut self, now: Instant) -> Vec<Action> {
        let mut out = Vec::new();
        if let Some(link) = self.link.as_mut()
            && let Some(end) = link.tick(now)
        {
            send(link, end, &mut out);
            self.window_closed(now);
        }
        self.settle(now, &mut out);
        out
    }

    /// The Sendspin connection came or went.
    pub fn set_sendspin(&mut self, connected: bool, now: Instant) -> Vec<Action> {
        if self.sendspin != connected {
            self.sendspin = connected;
            self.notice = None;
        }
        self.tick(now)
    }

    pub fn set_listening(&mut self, on: bool, now: Instant) -> Vec<Action> {
        self.listening = on;
        self.notice = None;
        self.tick(now)
    }

    /// A new voice key: any open window ends, and the link waits for a Hello under it.
    pub fn set_key(&mut self, key: Key, now: Instant) -> Vec<Action> {
        let mut out = Vec::new();
        self.close_window(now, &mut out);
        self.link = Some(VoiceLink::with_greeting_ttl(
            key,
            self.sid.clone(),
            self.greeting_ttl,
        ));
        self.notice = None;
        self.settle(now, &mut out);
        out
    }

    /// The text given as a voice key was refused; the old key stays.
    pub fn refuse_key(&mut self, reason: String) {
        self.notice = Some(reason);
    }

    /// The microphone could not be opened. It is tried again once it is no longer
    /// wanted and then wanted again (Listening off and on, or the next Hello).
    pub fn mic_failed(&mut self) {
        self.mic_on = false;
        self.mic_error = true;
        self.notice = None;
        if self.training.take().is_some() {
            // Training opens the mic outside the normal rule: say why it stopped.
            self.trainer.cancel();
            self.notice = Some(MIC_NEEDED.into());
        }
    }

    /// "Train Hey Jarvis": ignored while a voice window is open.
    pub fn train(&mut self, now: Instant) -> Vec<Action> {
        if self.window_open() || self.training.is_some() {
            return Vec::new();
        }
        self.trainer.start();
        self.training = Some("Say Hey Jarvis".into());
        self.notice = None;
        self.mic_error = false;
        self.tick(now)
    }

    pub fn cancel_training(&mut self, now: Instant) -> Vec<Action> {
        if self.training.take().is_none() {
            return Vec::new();
        }
        self.trainer.cancel();
        self.detector.reset();
        self.tick(now)
    }

    /// Leaving the house: the window ends, training is dropped, the mic closes.
    pub fn quit(&mut self, now: Instant) -> Vec<Action> {
        let mut out = Vec::new();
        if self.training.take().is_some() {
            self.trainer.cancel();
        }
        self.close_window(now, &mut out);
        if self.mic_on {
            self.mic_on = false;
            out.push(Action::Mic(false));
        }
        out
    }

    pub fn ui(&self, now: Instant) -> UiState {
        let greeted = self.greeted(now);
        let icon = if self.training.is_some() {
            IconState::Listening
        } else if !self.sendspin || !greeted {
            IconState::Gray
        } else if self.window_open() {
            IconState::Listening
        } else if !self.listening || self.mic_error {
            IconState::MicOff
        } else {
            IconState::Ready
        };
        let status = if let Some(training) = &self.training {
            training.clone()
        } else if let Some(notice) = &self.notice {
            notice.clone()
        } else if !self.sendspin {
            "Waiting for the hub…".into()
        } else if self.link.is_none() {
            "Set the voice key (Create key in Nidoo)".into()
        } else if !greeted {
            "Allow this Mac in Nidoo and give it a room".into()
        } else if self.mic_error {
            MIC_NEEDED.into()
        } else {
            "Connected".into()
        };
        UiState {
            icon,
            status,
            listening: self.listening,
            training: self.training.is_some(),
        }
    }

    fn greeted(&self, now: Instant) -> bool {
        self.link.as_ref().is_some_and(|l| l.greeted(now))
    }

    fn window_open(&self) -> bool {
        self.link.as_ref().is_some_and(|l| l.window_open())
    }

    /// Voice may be used: Listening on, a key, and a recent Hello.
    fn voice_ok(&self, now: Instant) -> bool {
        self.listening && self.greeted(now)
    }

    fn open(&mut self, kind: WakeKind, listen_ctr: u64, now: Instant, out: &mut Vec<Action>) {
        let Some(link) = self.link.as_mut() else {
            return;
        };
        let Some(wake) = link.open(kind, listen_ctr, now) else {
            return;
        };
        if kind == WakeKind::WakeWord {
            out.push(Action::WakeMuted(true));
            self.wake_muted = true;
            self.wake_at = Some(now);
            self.unmute_at = None;
        }
        send(link, wake, out);
        self.notice = None;
    }

    fn close_window(&mut self, now: Instant, out: &mut Vec<Action>) {
        if let Some(link) = self.link.as_mut()
            && let Some(end) = link.close_local()
        {
            send(link, end, out);
            self.window_closed(now);
        }
    }

    /// After a Stop, a timeout or a local close: the detector starts afresh and the
    /// playback comes back, no earlier than the floor after the wake word.
    fn window_closed(&mut self, now: Instant) {
        self.detector.reset();
        if self.wake_muted {
            let floor = self.wake_at.map_or(now, |at| at + WAKE_MUTE_FLOOR);
            self.unmute_at = Some(floor.max(now));
        }
        self.wake_at = None;
    }

    /// Applies the rules that follow from the current state.
    fn settle(&mut self, now: Instant, out: &mut Vec<Action>) {
        let greeted = self.greeted(now);
        if greeted != self.was_greeted {
            self.was_greeted = greeted;
            self.notice = None;
        }
        let voice_ok = self.voice_ok(now);
        if !voice_ok || self.training.is_some() {
            self.close_window(now, out);
        }
        let want = self.training.is_some() || voice_ok;
        if !want {
            self.mic_error = false;
            if self.mic_on {
                self.mic_on = false;
                out.push(Action::Mic(false));
            }
        } else if !self.mic_on && !self.mic_error {
            self.mic_on = true;
            out.push(Action::Mic(true));
        }
        if self.wake_muted && self.unmute_at.is_some_and(|at| now >= at) {
            self.wake_muted = false;
            self.unmute_at = None;
            out.push(Action::WakeMuted(false));
        }
    }
}

fn send(link: &VoiceLink, bytes: Vec<u8>, out: &mut Vec<Action>) {
    if let Some(hub) = link.hub() {
        out.push(Action::Send(hub, bytes));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddrV4};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use nidoo_voice::{BoardMsg, HubMsg, Key, WakeKind, decode_board, encode_hub};

    use crate::wake_head::TrainProgress;

    const SID: &str = "0123456789ab";
    const HUB: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 10), 4960);
    const WAKE: i16 = 12345;

    fn key() -> Key {
        Key::from_bytes(std::array::from_fn(|i| i as u8))
    }

    #[derive(Default)]
    struct Log {
        detector_pushes: usize,
        detector_resets: usize,
        trainer_starts: usize,
        trainer_pushes: usize,
        trainer_cancels: usize,
    }

    struct FakeDetector(Arc<Mutex<Log>>);

    impl WakeDetector for FakeDetector {
        fn push(&mut self, samples_16k: &[i16]) -> bool {
            self.0.lock().unwrap().detector_pushes += 1;
            samples_16k.contains(&WAKE)
        }
        fn reset(&mut self) {
            self.0.lock().unwrap().detector_resets += 1;
        }
    }

    /// Answers each push with the next scripted progress (the last one repeats).
    struct FakeTrainer(Arc<Mutex<Log>>, Vec<TrainProgress>);

    impl Trainer for FakeTrainer {
        fn start(&mut self) {
            self.0.lock().unwrap().trainer_starts += 1;
        }
        fn push(&mut self, _samples_16k: &[i16]) -> TrainProgress {
            let mut log = self.0.lock().unwrap();
            let i = log.trainer_pushes.min(self.1.len() - 1);
            log.trainer_pushes += 1;
            self.1[i].clone()
        }
        fn cancel(&mut self) {
            self.0.lock().unwrap().trainer_cancels += 1;
        }
    }

    struct Rig {
        brain: Brain,
        log: Arc<Mutex<Log>>,
        ctr: u64,
    }

    fn rig_with(key: Option<Key>, listening: bool, script: Vec<TrainProgress>) -> Rig {
        let log = Arc::new(Mutex::new(Log::default()));
        let brain = Brain::new(
            SID.to_string(),
            key,
            listening,
            Duration::from_secs(15),
            Box::new(FakeDetector(log.clone())),
            Box::new(FakeTrainer(log.clone(), script)),
        );
        Rig { brain, log, ctr: 0 }
    }

    fn rig(listening: bool) -> Rig {
        rig_with(
            Some(key()),
            listening,
            vec![TrainProgress::Saying { heard: 1, of: 15 }],
        )
    }

    impl Rig {
        fn hub(&mut self, msg: HubMsg, now: Instant) -> Vec<Action> {
            self.brain.on_datagram(&encode_hub(&key(), SID, &msg), now)
        }
        fn hello(&mut self, now: Instant) -> Vec<Action> {
            self.ctr += 1;
            self.hub(
                HubMsg::Hello {
                    nonce: 42,
                    hub_ctr: self.ctr,
                    hub: HUB,
                },
                now,
            )
        }
        fn stop(&mut self, session: u32, now: Instant) -> Vec<Action> {
            self.ctr += 1;
            self.hub(
                HubMsg::Stop {
                    hub_ctr: self.ctr,
                    session,
                },
                now,
            )
        }
        fn listen(&mut self, now: Instant) -> Vec<Action> {
            self.ctr += 1;
            self.hub(HubMsg::Listen { hub_ctr: self.ctr }, now)
        }
        fn log(&self) -> std::sync::MutexGuard<'_, Log> {
            self.log.lock().unwrap()
        }
    }

    /// The board datagrams among `actions`, all sent to the Hello's address.
    fn sent(actions: &[Action]) -> Vec<BoardMsg> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Send(to, bytes) => {
                    assert_eq!(*to, HUB, "datagrams go to the Hello's address");
                    let (sid, msg) = decode_board(&key(), bytes).unwrap();
                    assert_eq!(sid, SID);
                    Some(msg)
                }
                _ => None,
            })
            .collect()
    }

    fn mic(actions: &[Action]) -> Option<bool> {
        actions.iter().find_map(|a| match a {
            Action::Mic(on) => Some(*on),
            _ => None,
        })
    }

    fn wake_muted(actions: &[Action]) -> Option<bool> {
        actions.iter().find_map(|a| match a {
            Action::WakeMuted(m) => Some(*m),
            _ => None,
        })
    }

    #[test]
    fn mic_closed_until_greeted_and_listening() {
        let t = Instant::now();
        let mut r = rig(true);
        assert_eq!(mic(&r.brain.tick(t)), None, "not greeted yet");
        assert_eq!(mic(&r.hello(t)), Some(true));
        assert_eq!(mic(&r.brain.tick(t + Duration::from_secs(14))), None);
        assert_eq!(
            mic(&r.brain.tick(t + Duration::from_secs(15))),
            Some(false),
            "three missed Hellos close it"
        );

        let mut r = rig(false);
        assert_eq!(mic(&r.hello(t)), None, "Listening off");
        assert_eq!(mic(&r.brain.set_listening(true, t)), Some(true));
        assert_eq!(mic(&r.brain.set_listening(false, t)), Some(false));

        let mut r = rig_with(None, true, vec![TrainProgress::Training]);
        assert_eq!(mic(&r.hello(t)), None, "no key, no Hello understood");
        // Samples that were already queued when the mic closed are dropped.
        assert!(r.brain.on_mic(&[WAKE], t).is_empty());
        assert_eq!(r.log().detector_pushes, 0);
    }

    #[test]
    fn icon_states_cover_spec() {
        use IconState::*;
        // (sendspin, key, greeted, listening, window) → icon
        let table = [
            ((false, true, true, true, false), Gray),
            ((true, false, false, true, false), Gray),
            ((true, true, false, true, false), Gray),
            ((true, true, true, true, false), Ready),
            ((true, true, true, false, false), MicOff),
            ((true, true, true, true, true), Listening),
        ];
        let t = Instant::now();
        for ((sendspin, has_key, greeted, listening, window), icon) in table {
            let mut r = rig_with(has_key.then(key), listening, vec![TrainProgress::Training]);
            r.brain.set_sendspin(sendspin, t);
            if greeted {
                r.hello(t);
            }
            if window {
                r.brain.on_mic(&[WAKE], t);
            }
            let ui = r.brain.ui(t);
            assert_eq!(
                ui.icon,
                icon,
                "{:?}",
                (sendspin, has_key, greeted, listening, window)
            );
            assert_eq!(ui.listening, listening);
        }
    }

    #[test]
    fn status_line_for_each_case() {
        let t = Instant::now();
        let mut r = rig_with(None, true, vec![TrainProgress::Training]);
        assert!(r.brain.ui(t).status.contains("Waiting for the hub"));
        r.brain.set_sendspin(true, t);
        assert!(r.brain.ui(t).status.contains("voice key"));
        r.brain.set_key(key(), t);
        assert!(r.brain.ui(t).status.contains("Allow this Mac"));
        r.hello(t);
        assert_eq!(r.brain.ui(t).status, "Connected");
        r.brain.refuse_key("That is not a voice key".into());
        assert_eq!(r.brain.ui(t).status, "That is not a voice key");
        r.brain.set_listening(false, t);
        assert_eq!(
            r.brain.ui(t).status,
            "Connected",
            "a notice lasts until the next change"
        );
    }

    #[test]
    fn wake_mutes_then_unmutes_not_before_1s() {
        let t = Instant::now();
        let mut r = rig(true);
        r.hello(t);
        let a = r.brain.on_mic(&[0, WAKE, 0], t);
        assert_eq!(wake_muted(&a), Some(true));
        assert_eq!(
            sent(&a),
            vec![BoardMsg::Wake {
                nonce: 42,
                session: 1,
                kind: WakeKind::WakeWord,
                listen_ctr: 0
            }]
        );
        let mute_at = a
            .iter()
            .position(|x| *x == Action::WakeMuted(true))
            .unwrap();
        let send_at = a
            .iter()
            .position(|x| matches!(x, Action::Send(..)))
            .unwrap();
        assert!(mute_at < send_at, "mute first, like the board");

        // While the window is open, samples go to the hub, not to the detector.
        let a = r.brain.on_mic(&[7; 10], t + Duration::from_millis(100));
        assert!(matches!(
            sent(&a)[..],
            [BoardMsg::Audio {
                session: 1,
                seq: 1,
                ..
            }]
        ));
        assert_eq!(r.log().detector_pushes, 1);

        let a = r.stop(1, t + Duration::from_millis(300));
        assert_eq!(wake_muted(&a), None, "not before 1 s");
        assert!(sent(&a).is_empty(), "no End after a Stop");
        assert_eq!(r.log().detector_resets, 1);
        assert!(
            r.brain
                .on_mic(&[7; 10], t + Duration::from_millis(400))
                .is_empty()
        );
        assert_eq!(r.log().detector_pushes, 2, "the detector runs again");
        assert_eq!(
            wake_muted(&r.brain.tick(t + Duration::from_millis(999))),
            None
        );
        assert_eq!(
            wake_muted(&r.brain.tick(t + Duration::from_millis(1000))),
            Some(false)
        );
        assert_eq!(wake_muted(&r.brain.tick(t + Duration::from_secs(2))), None);
    }

    #[test]
    fn window_times_out_with_end_and_unmutes() {
        let t = Instant::now();
        let mut r = rig(true);
        r.brain.set_sendspin(true, t);
        r.hello(t);
        r.brain.on_mic(&[WAKE], t);
        let a = r.brain.tick(t + Duration::from_secs(8));
        assert_eq!(sent(&a), vec![BoardMsg::End { session: 1 }]);
        assert_eq!(wake_muted(&a), Some(false), "the 1 s floor has passed");
        assert_eq!(
            r.brain.ui(t + Duration::from_secs(8)).icon,
            IconState::Ready
        );
    }

    #[test]
    fn listening_off_mid_window_ends_it() {
        let t = Instant::now();
        let mut r = rig(true);
        r.brain.set_sendspin(true, t);
        r.hello(t);
        r.brain.on_mic(&[WAKE], t);
        let at = t + Duration::from_millis(200);
        let a = r.brain.set_listening(false, at);
        assert_eq!(sent(&a), vec![BoardMsg::End { session: 1 }]);
        assert_eq!(mic(&a), Some(false));
        assert_eq!(wake_muted(&a), None, "not before 1 s");
        assert_eq!(r.brain.ui(at).icon, IconState::MicOff);
        assert!(r.brain.on_mic(&[1; 10], at).is_empty(), "nothing leaves");
        assert_eq!(
            wake_muted(&r.brain.tick(t + Duration::from_secs(1))),
            Some(false),
            "the music comes back"
        );
    }

    #[test]
    fn listen_opens_follow_up_without_mute() {
        let t = Instant::now();
        let mut r = rig(true);
        r.brain.set_sendspin(true, t);
        r.hello(t);
        let a = r.listen(t);
        assert_eq!(wake_muted(&a), None);
        assert_eq!(
            sent(&a),
            vec![BoardMsg::Wake {
                nonce: 42,
                session: 1,
                kind: WakeKind::FollowUp,
                listen_ctr: 2
            }]
        );
        assert_eq!(r.brain.ui(t).icon, IconState::Listening);
        let a = r.brain.on_mic(&[WAKE; 4], t);
        assert!(matches!(sent(&a)[..], [BoardMsg::Audio { session: 1, .. }]));
        assert_eq!(r.log().detector_pushes, 0, "no wake word inside a window");
        let a = r.brain.tick(t + Duration::from_secs(6));
        assert_eq!(sent(&a), vec![BoardMsg::End { session: 1 }]);
        assert_eq!(wake_muted(&a), None, "it was never muted");

        // Listening off: a Listen opens nothing.
        r.brain.set_listening(false, t);
        assert!(r.listen(t + Duration::from_secs(7)).is_empty());
    }

    #[test]
    fn mic_error_shows_access_needed() {
        let t = Instant::now();
        let mut r = rig(true);
        r.brain.set_sendspin(true, t);
        assert_eq!(mic(&r.hello(t)), Some(true));
        r.brain.mic_failed();
        let ui = r.brain.ui(t);
        assert!(
            ui.status.contains("Microphone access needed"),
            "{}",
            ui.status
        );
        assert_eq!(ui.icon, IconState::MicOff);
        assert_eq!(mic(&r.brain.tick(t)), None, "no retry loop");
        assert!(r.listen(t).is_empty(), "no window without a mic");
        // Switching Listening off and on again retries.
        assert_eq!(mic(&r.brain.set_listening(false, t)), None);
        assert_eq!(mic(&r.brain.set_listening(true, t)), Some(true));
        assert_eq!(r.brain.ui(t).status, "Connected");
    }

    #[test]
    fn training_opens_mic_and_pauses_wake() {
        let t = Instant::now();
        let mut r = rig_with(
            Some(key()),
            false,
            vec![
                TrainProgress::Saying { heard: 1, of: 15 },
                TrainProgress::Failed("Heard only 1 of 15 — try again".into()),
            ],
        );
        let a = r.brain.train(t);
        assert_eq!(
            mic(&a),
            Some(true),
            "not greeted, Listening off: still opens"
        );
        assert_eq!(r.log().trainer_starts, 1);
        r.brain.on_mic(&[WAKE], t);
        assert_eq!(r.log().detector_pushes, 0);
        assert_eq!(r.log().trainer_pushes, 1);
        assert_eq!(r.brain.ui(t).status, "Say Hey Jarvis — 1 of 15");
        assert_eq!(r.brain.ui(t).icon, IconState::Listening, "the mic is hot");
        assert!(r.brain.ui(t).training);

        // No voice window opens meanwhile.
        r.brain.set_listening(true, t);
        r.hello(t);
        assert!(r.listen(t).is_empty());

        let a = r.brain.on_mic(&[0], t);
        assert_eq!(
            mic(&a),
            None,
            "back to the normal rule: greeted and listening"
        );
        assert_eq!(r.brain.ui(t).status, "Heard only 1 of 15 — try again");
        assert!(!r.brain.ui(t).training);
        assert_eq!(r.log().detector_resets, 1);
        r.brain.on_mic(&[WAKE], t);
        assert_eq!(r.log().detector_pushes, 1, "the detector is fed again");
    }

    #[test]
    fn training_done_returns_to_mic_rule() {
        let t = Instant::now();
        let head = crate::wake_head::train(
            &[vec![1.0; crate::wake_head::FEATURES]],
            &[vec![0.0; crate::wake_head::FEATURES]],
        );
        let mut r = rig_with(Some(key()), false, vec![TrainProgress::Done(head)]);
        assert_eq!(mic(&r.brain.train(t)), Some(true));
        let a = r.brain.on_mic(&[0], t);
        assert_eq!(mic(&a), Some(false), "Listening is off");
        assert_eq!(r.brain.ui(t).status, "Hey Jarvis trained");
    }

    #[test]
    fn training_ignored_while_a_window_is_open() {
        let t = Instant::now();
        let mut r = rig(true);
        r.hello(t);
        r.brain.on_mic(&[WAKE], t);
        assert!(r.brain.train(t).is_empty());
        assert_eq!(r.log().trainer_starts, 0);
    }

    #[test]
    fn training_cancel_restores_listening() {
        let t = Instant::now();
        let mut r = rig(true);
        r.brain.set_sendspin(true, t);
        r.hello(t);
        assert_eq!(mic(&r.brain.train(t)), None, "the mic was already on");
        r.brain.on_mic(&[WAKE], t);
        assert_eq!(r.log().detector_pushes, 0);
        assert_eq!(mic(&r.brain.cancel_training(t)), None);
        assert_eq!(r.log().trainer_cancels, 1);
        assert_eq!(r.brain.ui(t).status, "Connected");
        let a = r.brain.on_mic(&[WAKE], t);
        assert!(
            matches!(sent(&a)[..], [BoardMsg::Wake { .. }]),
            "the wake word works again"
        );

        let mut r = rig(false);
        assert_eq!(mic(&r.brain.train(t)), Some(true));
        assert_eq!(mic(&r.brain.cancel_training(t)), Some(false));
        assert!(r.brain.cancel_training(t).is_empty(), "nothing to cancel");
        assert_eq!(r.log().trainer_cancels, 1);
    }

    #[test]
    fn mic_failure_during_training_ends_it() {
        let t = Instant::now();
        let mut r = rig(false);
        r.brain.train(t);
        r.brain.mic_failed();
        assert_eq!(r.log().trainer_cancels, 1);
        assert!(r.brain.ui(t).status.contains("Microphone access needed"));
    }

    #[test]
    fn new_key_closes_the_window_and_waits_for_a_hello() {
        let t = Instant::now();
        let mut r = rig(true);
        r.hello(t);
        r.brain.on_mic(&[WAKE], t);
        let a = r.brain.set_key(Key::from_bytes([7; 32]), t);
        assert_eq!(sent(&a), vec![BoardMsg::End { session: 1 }]);
        assert_eq!(mic(&a), Some(false));
    }

    #[test]
    fn quit_ends_the_window_and_closes_the_mic() {
        let t = Instant::now();
        let mut r = rig(true);
        r.hello(t);
        r.brain.on_mic(&[WAKE], t);
        let a = r.brain.quit(t);
        assert_eq!(sent(&a), vec![BoardMsg::End { session: 1 }]);
        assert_eq!(mic(&a), Some(false));
    }
}
