//! The satellite's I/O around [`Brain`]: the voice socket, the Sendspin server and its
//! mDNS record, the playout, the menu's controls and a 20 ms clock (the board's rate).

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::brain::{Action, Brain, Trainer, UiState, WakeDetector};
use crate::playout::{Playout, SharedPlayout};
use crate::sendspin::{self, LocalPlayerState, PlayerConfig, SendspinEvent};
use crate::store::{KeyError, Store};
use crate::voice::GREETING_TTL;

/// How often the brain looks at the clock (window deadlines, greeting, wake-mute).
const TICK: Duration = Duration::from_millis(20);
/// What we tell `nidoo-sound` we can hold: about 5 s of 48 kHz stereo 16-bit.
const BUFFER_CAPACITY: u32 = 1_048_576;

/// The shell's microphone: opens or closes the real input stream. An `Err` means the
/// microphone cannot be used (no access).
pub trait MicSwitch: Send {
    fn set(&mut self, on: bool) -> anyhow::Result<()>;
}

/// What the menu asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    SetListening(bool),
    /// The clipboard's text, bare hex or the hub's `voice_key: "…"` line.
    SetVoiceKey(String),
    TrainWake,
    CancelTraining,
    Quit,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Where `player.key`, `settings.json` and `head.bin` live.
    pub store_dir: PathBuf,
    /// The player name Nidoo shows (the computer's name).
    pub name: String,
    /// The voice UDP port (4961; 0 = any, for tests).
    pub voice_port: u16,
    /// The Sendspin TCP port (0 = any).
    pub sendspin_port: u16,
    /// Publish `_sendspin._tcp` over mDNS.
    pub advertise: bool,
}

pub struct Handles {
    /// For the speaker's callback.
    pub playout: SharedPlayout,
    pub control: mpsc::Sender<Control>,
    pub ui: watch::Receiver<UiState>,
    /// The ports actually bound.
    pub voice_port: u16,
    pub sendspin_port: u16,
    /// Ends after [`Control::Quit`] (the End datagram is sent by then).
    pub done: JoinHandle<()>,
}

/// Starts the satellite. Samples on `mic_rx` are 16 kHz mono; the detector and trainer
/// are called on the runtime's task, so their `push` must return quickly.
pub async fn start(
    cfg: Config,
    mic: Box<dyn MicSwitch>,
    mic_rx: mpsc::Receiver<Vec<i16>>,
    detector: Box<dyn WakeDetector>,
    trainer: Box<dyn Trainer>,
) -> anyhow::Result<Handles> {
    start_with_greeting_ttl(cfg, GREETING_TTL, mic, mic_rx, detector, trainer).await
}

/// [`start`] with another greeting lifetime, so tests need not wait 15 s.
#[doc(hidden)]
pub async fn start_with_greeting_ttl(
    cfg: Config,
    greeting_ttl: Duration,
    mic: Box<dyn MicSwitch>,
    mic_rx: mpsc::Receiver<Vec<i16>>,
    detector: Box<dyn WakeDetector>,
    trainer: Box<dyn Trainer>,
) -> anyhow::Result<Handles> {
    let store = Store::open(&cfg.store_dir)?;
    let udp = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, cfg.voice_port))
        .await
        .with_context(|| format!("binding the voice port {}", cfg.voice_port))?;
    let voice_port = udp.local_addr()?.port();
    let tcp = TcpListener::bind((Ipv4Addr::UNSPECIFIED, cfg.sendspin_port))
        .await
        .context("binding the Sendspin port")?;
    let sendspin_port = tcp.local_addr()?.port();
    let mdns = if cfg.advertise {
        Some(sendspin::advertise(sendspin_port, &cfg.name)?)
    } else {
        None
    };

    let settings = store.settings().clone();
    let identity = Arc::new(store.identity().clone());
    let (sendspin_tx, sendspin_rx) = mpsc::channel(256);
    let (local_tx, local_rx) = watch::channel(LocalPlayerState {
        volume: 100,
        muted: false,
    });
    let server = tokio::spawn(sendspin::serve(
        tcp,
        identity.clone(),
        PlayerConfig {
            name: cfg.name.clone(),
            buffer_capacity: BUFFER_CAPACITY,
        },
        sendspin_tx,
        local_rx,
    ));

    let brain = Brain::new(
        identity.sid.clone(),
        settings.voice_key.clone(),
        settings.listening,
        greeting_ttl,
        detector,
        trainer,
    );
    let (ui_tx, ui) = watch::channel(brain.ui(Instant::now()));
    let (control, control_rx) = mpsc::channel(16);
    let playout: SharedPlayout = Arc::new(Mutex::new(Playout::default()));

    let runtime = Runtime {
        brain,
        store,
        udp,
        mic,
        playout: playout.clone(),
        local: local_tx,
        ui: ui_tx,
    };
    let done = tokio::spawn(async move {
        runtime.run(control_rx, sendspin_rx, mic_rx).await;
        server.abort();
        if let Some(mdns) = mdns {
            let _ = mdns.shutdown();
        }
    });
    Ok(Handles {
        playout,
        control,
        ui,
        voice_port,
        sendspin_port,
        done,
    })
}

struct Runtime {
    brain: Brain,
    store: Store,
    udp: UdpSocket,
    mic: Box<dyn MicSwitch>,
    playout: SharedPlayout,
    local: watch::Sender<LocalPlayerState>,
    ui: watch::Sender<UiState>,
}

impl Runtime {
    async fn run(
        mut self,
        mut control: mpsc::Receiver<Control>,
        mut sendspin: mpsc::Receiver<SendspinEvent>,
        mut mic_rx: mpsc::Receiver<Vec<i16>>,
    ) {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut buf = vec![0u8; 2048];
        let mut sendspin_open = true;
        loop {
            let actions = tokio::select! {
                c = control.recv() => match c {
                    Some(Control::Quit) | None => {
                        let actions = self.brain.quit(Instant::now());
                        self.apply(actions).await;
                        self.playout().clear();
                        self.publish();
                        return;
                    }
                    Some(c) => self.control(c),
                },
                ev = sendspin.recv(), if sendspin_open => match ev {
                    Some(ev) => self.sendspin(ev),
                    None => {
                        // The Sendspin server stopped: no music can reach us any more.
                        tracing::error!("the Sendspin server stopped");
                        sendspin_open = false;
                        self.playout().clear();
                        self.brain.set_sendspin(false, Instant::now())
                    }
                },
                got = self.udp.recv_from(&mut buf) => match got {
                    Ok((n, _)) => self.brain.on_datagram(&buf[..n], Instant::now()),
                    Err(e) => {
                        tracing::debug!(error = %e, "voice socket");
                        Vec::new()
                    }
                },
                Some(samples) = mic_rx.recv() => self.brain.on_mic(&samples, Instant::now()),
                _ = tick.tick() => self.brain.tick(Instant::now()),
            };
            self.apply(actions).await;
            self.publish();
        }
    }

    fn control(&mut self, c: Control) -> Vec<Action> {
        let now = Instant::now();
        match c {
            Control::SetListening(on) => {
                if let Err(e) = self.store.set_listening(on) {
                    tracing::warn!(error = %e, "keeping the Listening choice");
                }
                self.brain.set_listening(on, now)
            }
            Control::SetVoiceKey(text) => match self.store.set_voice_key(&text) {
                Ok(()) => match self.store.settings().voice_key.clone() {
                    Some(key) => self.brain.set_key(key, now),
                    None => Vec::new(),
                },
                Err(KeyError::NotAKey) => {
                    self.brain.refuse_key(
                        "That is not a voice key — copy it again from Create key".into(),
                    );
                    Vec::new()
                }
                Err(KeyError::NotSaved) => {
                    self.brain
                        .refuse_key("The voice key could not be saved".into());
                    Vec::new()
                }
            },
            Control::TrainWake => self.brain.train(now),
            Control::CancelTraining => self.brain.cancel_training(now),
            Control::Quit => unreachable!("handled by run"),
        }
    }

    fn sendspin(&mut self, ev: SendspinEvent) -> Vec<Action> {
        match ev {
            SendspinEvent::Connected => return self.brain.set_sendspin(true, Instant::now()),
            SendspinEvent::Disconnected => {
                self.playout().clear();
                return self.brain.set_sendspin(false, Instant::now());
            }
            SendspinEvent::StreamStart(_) | SendspinEvent::StreamEnd => self.playout().clear(),
            SendspinEvent::Chunk {
                local_play_at_us,
                pcm,
            } => self.playout().push(local_play_at_us, &pcm),
            SendspinEvent::Volume(v) => {
                self.playout().set_volume(v);
                self.local.send_modify(|s| s.volume = v);
            }
            SendspinEvent::Mute(m) => {
                self.playout().set_muted(m);
                self.local.send_modify(|s| s.muted = m);
            }
        }
        Vec::new()
    }

    async fn apply(&mut self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Send(to, bytes) => {
                    if let Err(e) = self.udp.send_to(&bytes, SocketAddr::V4(to)).await {
                        tracing::debug!(error = %e, %to, "sending to the hub");
                    }
                }
                Action::Mic(on) => {
                    if let Err(e) = self.mic.set(on) {
                        tracing::warn!(error = %e, "microphone");
                        if on {
                            self.brain.mic_failed();
                        }
                    }
                }
                Action::WakeMuted(m) => self.playout().set_wake_muted(m),
            }
        }
    }

    fn playout(&self) -> std::sync::MutexGuard<'_, Playout> {
        self.playout.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn publish(&self) {
        let next = self.brain.ui(Instant::now());
        self.ui.send_if_modified(|ui| {
            if *ui == next {
                false
            } else {
                *ui = next;
                true
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wake_head::TrainProgress;

    struct NoMic;
    impl MicSwitch for NoMic {
        fn set(&mut self, _on: bool) -> anyhow::Result<()> {
            Ok(())
        }
    }
    struct NoDetector;
    impl WakeDetector for NoDetector {
        fn push(&mut self, _: &[i16]) -> bool {
            false
        }
        fn reset(&mut self) {}
    }
    struct NoTrainer;
    impl Trainer for NoTrainer {
        fn start(&mut self) {}
        fn push(&mut self, _: &[i16]) -> TrainProgress {
            TrainProgress::Training
        }
        fn cancel(&mut self) {}
    }

    #[tokio::test]
    async fn sendspin_server_ending_shows_waiting_for_the_hub() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let brain = Brain::new(
            store.identity().sid.clone(),
            None,
            true,
            GREETING_TTL,
            Box::new(NoDetector),
            Box::new(NoTrainer),
        );
        let (ui_tx, mut ui) = watch::channel(brain.ui(Instant::now()));
        let (local, _local_rx) = watch::channel(LocalPlayerState {
            volume: 100,
            muted: false,
        });
        let runtime = Runtime {
            brain,
            store,
            udp: UdpSocket::bind("127.0.0.1:0").await.unwrap(),
            mic: Box::new(NoMic),
            playout: Arc::new(Mutex::new(Playout::default())),
            local,
            ui: ui_tx,
        };
        let (_control, control_rx) = mpsc::channel(4);
        let (events, events_rx) = mpsc::channel(4);
        let (_mic, mic_rx) = mpsc::channel(4);
        tokio::spawn(runtime.run(control_rx, events_rx, mic_rx));

        events.send(SendspinEvent::Connected).await.unwrap();
        let wait = Duration::from_secs(2);
        tokio::time::timeout(wait, ui.wait_for(|u| u.status.contains("voice key")))
            .await
            .expect("connected")
            .unwrap();
        // The server task ended (it never sends Disconnected then).
        drop(events);
        tokio::time::timeout(
            wait,
            ui.wait_for(|u| u.status.contains("Waiting for the hub")),
        )
        .await
        .expect("the menu no longer claims a connection")
        .unwrap();
    }
}
