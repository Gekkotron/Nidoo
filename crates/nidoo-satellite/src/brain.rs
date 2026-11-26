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
    /// Ring the short wake chime: the wake word was heard (not on a follow-up).
    Chime,
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
            out.push(Action::Chime);
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
mod tests;
