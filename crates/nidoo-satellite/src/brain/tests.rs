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
    assert!(a.contains(&Action::Chime), "the wake word rings the chime");
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
    assert!(!a.contains(&Action::Chime), "a follow-up has no chime");
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
