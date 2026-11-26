//! The group: every satellite in the house plays the one input, in sync.
//!
//! A single actor owns the player connections, the pacer and the saved
//! prefs. Only **allowed** players get audio; a new player is not allowed
//! until the operator switches it on. Volume and mute are saved, re-applied
//! on every (re)connect and published to the MQTT link on every change.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::clock::Clock;
use crate::discover::{Discovered, DiscoveryEvent};
use crate::hold::{HoldRequest, Holds};
use crate::input::InputEvent;
use crate::level::{Levels, split};
use crate::mixer::{FADE_FRAMES, SpeechMixer, fade_in, scale};
use crate::pacer::{CHUNK_BYTES, CHUNK_US, Pacer};
use crate::player::{PlayerCmd, PlayerEvent, run_player};
use crate::proto::messages::PCM_48K_STEREO_16;
use crate::proto::noise::ServerIdentity;
use crate::speech::Speech;
use crate::state::{PlayerPrefs, StateStore, short_id};

/// How often the pacer is polled while playing.
const TICK: Duration = Duration::from_millis(5);
/// How long after a sentence's last sample its stream ends, when no music plays: room for
/// the satellite's output stage.
const SPEECH_END_MARGIN_US: i64 = 100_000;
/// Pause between connection attempts to an advertised player.
const RECONNECT_DELAY: Duration = Duration::from_secs(2);
/// Slots of a player's command queue that audio leaves free, so stream and
/// volume changes still fit when its connection stops taking audio.
const CONTROL_ROOM: usize = 16;

/// Changes asked for from Nidoo (MQTT commands).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    SetVolume {
        short_id: String,
        volume: u8,
    },
    /// The Speech volume: answers and announcements.
    SetSpeech {
        short_id: String,
        volume: u8,
    },
    SetMute {
        short_id: String,
        muted: bool,
    },
    SetAllowed {
        short_id: String,
        allowed: bool,
    },
    /// Forget the satellite: its prefs and its retained messages. A board still on the
    /// network comes back as a new one, waiting for approval.
    Remove {
        short_id: String,
    },
}

impl Control {
    pub fn short_id(&self) -> &str {
        match self {
            Control::SetVolume { short_id, .. }
            | Control::SetSpeech { short_id, .. }
            | Control::SetMute { short_id, .. }
            | Control::SetAllowed { short_id, .. }
            | Control::Remove { short_id } => short_id,
        }
    }

    fn apply(&self, p: &mut PlayerPrefs) {
        match *self {
            Control::SetVolume { volume, .. } => p.volume = volume.min(100),
            Control::SetSpeech { volume, .. } => p.speech_volume = volume.min(100),
            Control::SetMute { muted, .. } => p.muted = muted,
            Control::SetAllowed { allowed, .. } => p.allowed = allowed,
            Control::Remove { .. } => {}
        }
    }
}

/// What the MQTT link publishes: sent on every change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Published {
    Player {
        short_id: String,
        prefs: PlayerPrefs,
        connected: bool,
        /// Its IPv4 address while connected.
        address: Option<Ipv4Addr>,
    },
    /// A forgotten satellite: its retained messages are cleared.
    Removed { prefs: PlayerPrefs },
    /// A sentence finished playing (`played`), or was dropped.
    Spoken { id: String, played: bool },
}

/// From a connection task to the group.
#[derive(Debug)]
pub enum FromPlayer {
    /// A new connection attempt to `instance`; commands go to `cmds`.
    Link {
        instance: String,
        cmds: mpsc::Sender<PlayerCmd>,
    },
    /// A player that connected to us (client-initiated) and finished its
    /// handshake; it is forgotten once `Gone`. Its events follow under the
    /// same `instance`, which must not be an mDNS instance name.
    Accepted {
        instance: String,
        cmds: mpsc::Sender<PlayerCmd>,
        /// Where the connection came from.
        peer: Option<IpAddr>,
    },
    Event {
        instance: String,
        event: PlayerEvent,
    },
}

/// Starts the connection task for a discovered player.
pub type Connector =
    Arc<dyn Fn(Discovered, mpsc::Sender<FromPlayer>) -> JoinHandle<()> + Send + Sync>;

/// Connects with the Sendspin protocol, reconnecting every 2 s until aborted.
pub fn sendspin_connector(identity: Arc<ServerIdentity>, clock: Arc<dyn Clock>) -> Connector {
    Arc::new(move |d: Discovered, to_group: mpsc::Sender<FromPlayer>| {
        let (identity, clock) = (identity.clone(), clock.clone());
        tokio::spawn(async move {
            loop {
                let (cmds, cmd_rx) = mpsc::channel(256);
                let link = FromPlayer::Link {
                    instance: d.instance.clone(),
                    cmds,
                };
                if to_group.send(link).await.is_err() {
                    return;
                }
                let (ev_tx, mut ev_rx) = mpsc::channel(64);
                let forward = {
                    let (to_group, instance) = (to_group.clone(), d.instance.clone());
                    tokio::spawn(async move {
                        while let Some(event) = ev_rx.recv().await {
                            let msg = FromPlayer::Event {
                                instance: instance.clone(),
                                event,
                            };
                            if to_group.send(msg).await.is_err() {
                                break;
                            }
                        }
                    })
                };
                let result = run_player(
                    d.url.clone(),
                    identity.clone(),
                    clock.clone(),
                    ev_tx,
                    cmd_rx,
                )
                .await;
                if let Err(e) = result {
                    tracing::info!(instance = %d.instance, error = %format!("{e:#}"), "player connection ended");
                }
                let _ = forward.await;
                tokio::time::sleep(RECONNECT_DELAY).await;
            }
        })
    })
}

/// One connection to a player.
struct Link {
    cmds: mpsc::Sender<PlayerCmd>,
    client_id: Option<String>,
    connected: bool,
    streaming: bool,
    /// From the player's latest `client/state`.
    min_buffer_us: i64,
    output_delay_us: i64,
    /// Values we commanded and have not seen echoed yet: earlier reports
    /// are stale and must not overwrite the saved prefs.
    expect_volume: Option<u8>,
    /// The device volume last sent to the player (kept after its echo).
    sent_volume: Option<u8>,
    /// A device volume waiting for the audio that carries its gains: the
    /// level and the time on the shared clock it is sent (the timestamp of the
    /// first chunk made with the new gains). Audio already sent plays out at
    /// the old device volume first.
    pending_volume: Option<(u8, i64)>,
    /// Device volume and sample gains from its saved Music and Speech settings.
    levels: Levels,
    expect_mute: Option<bool>,
    /// Its music was held for the last chunk sent.
    held: bool,
    /// Its music is coming back after a hold: the fade's next frame.
    fade_from: Option<usize>,
    /// Earliest timestamp the next audio chunk may carry in this stream:
    /// timestamps only go up, even when music and speech take turns.
    /// `NO_TS` when the stream has sent nothing yet.
    next_ts: AtomicI64,
    /// Audio chunks dropped in a row because the queue was full.
    dropped: AtomicU64,
}

/// `Link::next_ts` before the first chunk of a stream.
const NO_TS: i64 = i64::MIN;

impl Link {
    fn new(cmds: mpsc::Sender<PlayerCmd>) -> Self {
        Self {
            cmds,
            client_id: None,
            connected: false,
            streaming: false,
            min_buffer_us: 0,
            output_delay_us: 0,
            expect_volume: None,
            sent_volume: None,
            pending_volume: None,
            levels: Levels {
                device: 0,
                music_gain: 1.0,
                speech_gain: 1.0,
            },
            expect_mute: None,
            held: false,
            fade_from: None,
            next_ts: AtomicI64::new(NO_TS),
            dropped: AtomicU64::new(0),
        }
    }

    /// This player's short id, once it said hello.
    fn short_id(&self) -> Option<String> {
        self.client_id.as_deref().map(short_id)
    }

    /// Takes the levels of `prefs`; commands the device volume when it is not the one
    /// last sent or waiting to be (`force`: whatever was sent, as on connect).
    ///
    /// While streaming, the audio already sent carries the old gains, so the device
    /// volume changes with the first chunk made with the new ones; the echo of the
    /// old value is meanwhile not a change on the device (`expect_volume`).
    async fn set_levels(&mut self, prefs: &PlayerPrefs, force: bool) {
        self.levels = split(prefs.volume, prefs.speech_volume);
        let device = self.levels.device;
        if force {
            self.pending_volume = None;
            self.send_volume(device).await;
            return;
        }
        let target = self.pending_volume.map(|(d, _)| d).or(self.sent_volume);
        if target == Some(device) {
            return;
        }
        match self.next_ts().filter(|_| self.streaming) {
            Some(at) => {
                self.pending_volume = Some((device, at));
                self.expect_volume = Some(device);
            }
            None => {
                self.pending_volume = None;
                self.send_volume(device).await;
            }
        }
    }

    async fn send_volume(&mut self, device: u8) {
        self.send(PlayerCmd::Volume(device));
        self.sent_volume = Some(device);
        self.expect_volume = Some(device);
    }

    /// Sends the waiting device volume once its audio plays, or at once when
    /// nothing is streaming any more.
    async fn flush_volume(&mut self, now: i64) {
        if let Some((device, at)) = self.pending_volume
            && (now >= at || !self.streaming)
        {
            self.pending_volume = None;
            self.send_volume(device).await;
        }
    }

    fn next_ts(&self) -> Option<i64> {
        Some(self.next_ts.load(Ordering::Relaxed)).filter(|&t| t != NO_TS)
    }

    /// Queues one audio chunk unless it would go back in time in this stream.
    fn audio(&self, timestamp_us: i64, data: bytes::Bytes) {
        if self.next_ts().is_some_and(|next| timestamp_us < next) {
            return;
        }
        self.next_ts
            .store(timestamp_us + CHUNK_US, Ordering::Relaxed);
        // The group is the queue's only sender: the room it sees stays free.
        let queued = self.cmds.capacity() > CONTROL_ROOM
            && self
                .cmds
                .try_send(PlayerCmd::Audio { timestamp_us, data })
                .is_ok();
        if queued {
            let dropped = self.dropped.swap(0, Ordering::Relaxed);
            if dropped > 0 {
                tracing::info!(client_id = ?self.client_id, dropped, "player takes audio again");
            }
        } else if self.dropped.fetch_add(1, Ordering::Relaxed) == 0 && !self.cmds.is_closed() {
            tracing::warn!(client_id = ?self.client_id, "player queue full, dropping audio");
        }
    }

    /// Queues a command without waiting: one stuck player must not hold up the
    /// group. A command that finds no room is dropped; the stuck connection
    /// ends ([`crate::player::Liveness::write_stall`]) and the reconnect sends
    /// the player's whole state again.
    fn send(&self, cmd: PlayerCmd) {
        if matches!(cmd, PlayerCmd::StreamStart(_) | PlayerCmd::StreamEnd) {
            self.next_ts.store(NO_TS, Ordering::Relaxed);
        }
        match self.cmds.try_send(cmd) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(cmd)) => {
                tracing::warn!(client_id = ?self.client_id, ?cmd, "player queue full, command dropped");
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::debug!(client_id = ?self.client_id, "player connection gone");
            }
        }
    }
}

/// An advertised (or recently advertised) player, or one that connected to us.
struct Instance {
    /// Where we connect to it; `None` when it connected to us.
    url: Option<url::Url>,
    /// Our reconnect loop; `None` when it connected to us.
    task: Option<JoinHandle<()>>,
    advertised: bool,
    link: Option<Link>,
    /// The player's IPv4 address: its URL's host, or the peer that connected to us.
    address: Option<Ipv4Addr>,
}

impl Instance {
    fn stop(&self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

pub struct Group {
    store: StateStore,
    clock: Arc<dyn Clock>,
    lead_us: i64,
    input_buffer_us: i64,
    connect: Connector,
    instances: HashMap<String, Instance>,
    pacer: Pacer,
    playing: bool,
    /// Sentences to say over (or without) the music.
    mixer: SpeechMixer,
    /// Satellites whose music is held while someone talks to the house.
    holds: Holds,
    /// Sentences to report as played once the clock reaches their last sample: (id, at).
    reports: Vec<(String, i64)>,
    /// Without music: the next speech chunk's timestamp.
    speech_clock: Option<i64>,
    /// With music: end of the last music chunk sent.
    music_sent_until: i64,
    to_group: mpsc::Sender<FromPlayer>,
    from_players: mpsc::Receiver<FromPlayer>,
}

impl Group {
    pub fn new(store: StateStore, clock: Arc<dyn Clock>, lead_us: i64, connect: Connector) -> Self {
        let (to_group, from_players) = mpsc::channel(1024);
        Self {
            store,
            clock,
            lead_us,
            input_buffer_us: 0,
            connect,
            instances: HashMap::new(),
            pacer: Pacer::new(lead_us),
            playing: false,
            mixer: SpeechMixer::new(),
            holds: Holds::default(),
            reports: Vec::new(),
            speech_clock: None,
            music_sent_until: i64::MIN,
            to_group,
            from_players,
        }
    }

    /// Phone PCM to gather before each new timeline (see [`Pacer`]).
    pub fn with_input_buffer(mut self, buffer_us: i64) -> Self {
        self.input_buffer_us = buffer_us;
        self
    }

    /// Where connections report to the group (players that connect to us
    /// send [`FromPlayer::Accepted`] here first).
    pub fn players(&self) -> mpsc::Sender<FromPlayer> {
        self.to_group.clone()
    }

    pub async fn run(
        mut self,
        mut discovery: mpsc::Receiver<DiscoveryEvent>,
        mut input: mpsc::Receiver<InputEvent>,
        mut control: mpsc::Receiver<Control>,
        mut speech: mpsc::Receiver<Speech>,
        mut holds: mpsc::Receiver<HoldRequest>,
        publish: mpsc::Sender<Published>,
    ) -> anyhow::Result<()> {
        let to_group = self.to_group.clone();
        let known: Vec<PlayerPrefs> = self.store.all().cloned().collect();
        for prefs in known {
            let msg = Published::Player {
                short_id: short_id(&prefs.client_id),
                prefs,
                connected: false,
                address: None,
            };
            publish.send(msg).await.ok();
        }
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                Some(d) = discovery.recv() => self.on_discovery(d, &to_group, &publish).await,
                Some(i) = input.recv() => self.on_input(i).await,
                Some(c) = control.recv() => self.on_control(c, &publish).await,
                Some(m) = self.from_players.recv() => self.on_player(m, &publish).await,
                Some(s) = speech.recv() => self.on_speech(s, &publish).await,
                Some(h) = holds.recv() => self.holds.apply(h, self.clock.now_us()),
                _ = tick.tick(), if self.playing
                    || self.mixer.is_playing()
                    || self.mixer.has_queued()
                    || !self.reports.is_empty()
                    || self.links().any(|l| l.pending_volume.is_some()) => {
                    self.on_tick(&publish).await
                }
            }
        }
    }

    /// Ends every stream (on shutdown).
    pub async fn end_streams(&mut self) {
        for link in self.links_mut() {
            if link.streaming {
                link.send(PlayerCmd::StreamEnd);
                link.streaming = false;
            }
        }
    }

    fn links(&self) -> impl Iterator<Item = &Link> {
        self.instances.values().filter_map(|i| i.link.as_ref())
    }

    fn links_mut(&mut self) -> impl Iterator<Item = &mut Link> {
        self.instances.values_mut().filter_map(|i| i.link.as_mut())
    }

    fn link_for(&mut self, client_id: &str) -> Option<&mut Link> {
        self.links_mut()
            .find(|l| l.connected && l.client_id.as_deref() == Some(client_id))
    }

    fn is_connected(&self, client_id: &str) -> bool {
        self.links()
            .any(|l| l.connected && l.client_id.as_deref() == Some(client_id))
    }

    async fn publish(&self, publish: &mpsc::Sender<Published>, client_id: &str) {
        let Some(prefs) = self.store.get(client_id).cloned() else {
            return;
        };
        let connected = self.is_connected(client_id);
        let address = self
            .instances
            .values()
            .filter(|i| {
                i.link
                    .as_ref()
                    .is_some_and(|l| l.connected && l.client_id.as_deref() == Some(client_id))
            })
            .find_map(|i| i.address);
        let msg = Published::Player {
            short_id: short_id(client_id),
            prefs,
            connected,
            address,
        };
        publish.send(msg).await.ok();
    }

    async fn on_discovery(
        &mut self,
        event: DiscoveryEvent,
        to_group: &mpsc::Sender<FromPlayer>,
        publish: &mpsc::Sender<Published>,
    ) {
        match event {
            DiscoveryEvent::Up(d) => {
                if let Some(i) = self.instances.get_mut(&d.instance) {
                    i.advertised = true;
                    let running = i.task.as_ref().is_some_and(|t| !t.is_finished());
                    if i.url.as_ref() == Some(&d.url) && running {
                        return;
                    }
                    i.stop();
                }
                tracing::info!(instance = %d.instance, url = %d.url, "satellite found");
                let instance = d.instance.clone();
                let url = d.url.clone();
                let task = (self.connect)(d, to_group.clone());
                let old = self.instances.insert(
                    instance,
                    Instance {
                        address: url.host_str().and_then(|h| h.parse().ok()),
                        url: Some(url),
                        task: Some(task),
                        advertised: true,
                        link: None,
                    },
                );
                if let Some(client_id) = old.and_then(|o| o.link).and_then(|l| l.client_id) {
                    self.publish(publish, &client_id).await;
                }
            }
            DiscoveryEvent::Down { instance } => {
                let Some(i) = self.instances.get_mut(&instance) else {
                    return;
                };
                i.advertised = false;
                // A live connection is kept; the reconnect loop stops with it.
                if !i.link.as_ref().is_some_and(|l| l.connected) {
                    i.stop();
                    self.instances.remove(&instance);
                }
            }
        }
    }

    async fn on_player(&mut self, msg: FromPlayer, publish: &mpsc::Sender<Published>) {
        match msg {
            FromPlayer::Link { instance, cmds } => {
                if let Some(i) = self.instances.get_mut(&instance) {
                    i.link = Some(Link::new(cmds));
                }
            }
            FromPlayer::Accepted {
                instance,
                cmds,
                peer,
            } => {
                let address = match peer {
                    Some(IpAddr::V4(ip)) => Some(ip),
                    Some(IpAddr::V6(ip)) => ip.to_ipv4_mapped(),
                    None => None,
                };
                let accepted = Instance {
                    url: None,
                    task: None,
                    advertised: false,
                    link: Some(Link::new(cmds)),
                    address,
                };
                if let Some(old) = self.instances.insert(instance, accepted) {
                    old.stop();
                }
            }
            FromPlayer::Event { instance, event } => self.on_event(&instance, event, publish).await,
        }
    }

    async fn on_event(
        &mut self,
        instance: &str,
        event: PlayerEvent,
        publish: &mpsc::Sender<Published>,
    ) {
        match event {
            PlayerEvent::Connected(info) => {
                self.close_older(instance, &info.client_id).await;
                let prefs = self.store.observe(&info.client_id, &info.name).clone();
                let playing = self.playing;
                let Some(link) = self
                    .instances
                    .get_mut(instance)
                    .and_then(|i| i.link.as_mut())
                else {
                    return;
                };
                tracing::info!(%instance, client_id = %info.client_id, name = %info.name, allowed = prefs.allowed, "satellite connected");
                link.client_id = Some(info.client_id.clone());
                link.connected = true;
                link.send(PlayerCmd::Activate {
                    playback: prefs.allowed,
                });
                link.set_levels(&prefs, true).await;
                link.send(PlayerCmd::Mute(prefs.muted));
                link.expect_mute = Some(prefs.muted);
                if playing && prefs.allowed {
                    link.send(PlayerCmd::StreamStart(PCM_48K_STEREO_16));
                    link.streaming = true;
                }
                self.publish(publish, &info.client_id).await;
            }
            PlayerEvent::State {
                client_id,
                volume,
                muted,
                supported_commands,
                min_buffer_ms,
                output_delay_ms,
                ..
            } => {
                let Some(saved) = self.store.get(&client_id).cloned() else {
                    return;
                };
                let Some(link) = self.link_for(&client_id) else {
                    return;
                };
                link.min_buffer_us = i64::from(min_buffer_ms.unwrap_or(0)) * 1_000;
                link.output_delay_us = i64::from(output_delay_ms.unwrap_or(0)) * 1_000;
                let settable = |c: &str| supported_commands.iter().any(|s| s == c);
                let device = split(saved.volume, saved.speech_volume).device;
                let volume = fresh(&mut link.expect_volume, volume, settable("volume"))
                    .filter(|&v| v != device);
                let muted = fresh(&mut link.expect_mute, muted, settable("mute"))
                    .filter(|&m| m != saved.muted);
                if volume.is_none() && muted.is_none() {
                    return;
                }
                // The device now has `v`: both settings move with it, keeping their gap.
                let updated = self.store.update(&short_id(&client_id), |p| {
                    if let Some(v) = volume {
                        let delta = i16::from(v) - i16::from(device);
                        let shift = |x: u8| (i16::from(x) + delta).clamp(0, 100) as u8;
                        p.volume = shift(p.volume);
                        p.speech_volume = shift(p.speech_volume);
                    }
                    if let Some(m) = muted {
                        p.muted = m;
                    }
                });
                match updated {
                    Ok(Some(p)) => {
                        if let Some(link) = self.link_for(&client_id) {
                            link.levels = split(p.volume, p.speech_volume);
                            if let Some(v) = volume {
                                link.sent_volume = Some(v);
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::error!(error = %format!("{e:#}"), "saving player prefs");
                    }
                }
                self.publish(publish, &client_id).await;
            }
            PlayerEvent::Gone { client_id } => {
                tracing::info!(%instance, %client_id, "satellite disconnected");
                let Some(i) = self.instances.get_mut(instance) else {
                    return;
                };
                i.link = None;
                if !i.advertised {
                    i.stop();
                    self.instances.remove(instance);
                }
                self.publish(publish, &client_id).await;
            }
        }
    }

    /// A player that connects again (a browser after a network hiccup) may
    /// leave its older connection open: close it, the new one wins.
    async fn close_older(&mut self, instance: &str, client_id: &str) {
        for (name, i) in &mut self.instances {
            let Some(link) = i.link.as_mut() else {
                continue;
            };
            if name == instance || !link.connected || link.client_id.as_deref() != Some(client_id) {
                continue;
            }
            tracing::info!(old = %name, new = %instance, %client_id, "player connected again, closing its older connection");
            link.connected = false;
            link.streaming = false;
            link.send(PlayerCmd::Close);
        }
    }

    async fn on_control(&mut self, control: Control, publish: &mpsc::Sender<Published>) {
        let short = control.short_id().to_owned();
        if matches!(control, Control::Remove { .. }) {
            self.remove(&short, publish).await;
            return;
        }
        let prefs = match self.store.update(&short, |p| control.apply(p)) {
            Ok(Some(p)) => p,
            Ok(None) => {
                tracing::warn!(short_id = %short, "command for an unknown satellite");
                return;
            }
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "saving player prefs");
                return;
            }
        };
        let playing = self.playing;
        if let Some(link) = self.link_for(&prefs.client_id) {
            match control {
                Control::SetVolume { .. } | Control::SetSpeech { .. } => {
                    link.set_levels(&prefs, false).await;
                }
                Control::SetMute { .. } => {
                    link.send(PlayerCmd::Mute(prefs.muted));
                    link.expect_mute = Some(prefs.muted);
                }
                Control::SetAllowed { allowed: true, .. } => {
                    link.send(PlayerCmd::Activate { playback: true });
                    if playing && !link.streaming {
                        link.send(PlayerCmd::StreamStart(PCM_48K_STEREO_16));
                        link.streaming = true;
                    }
                }
                Control::SetAllowed { allowed: false, .. } => {
                    if link.streaming {
                        link.send(PlayerCmd::StreamEnd);
                        link.streaming = false;
                    }
                    link.send(PlayerCmd::Activate { playback: false });
                }
                Control::Remove { .. } => {}
            }
        }
        self.publish(publish, &prefs.client_id).await;
    }

    /// Forgets satellite `short`: drops its prefs, closes its connection (its later events
    /// find no prefs and publish nothing) and has its retained messages cleared. A board still
    /// on the network reconnects as a new satellite, waiting for approval.
    async fn remove(&mut self, short: &str, publish: &mpsc::Sender<Published>) {
        let prefs = match self.store.remove(short) {
            Ok(Some(p)) => p,
            Ok(None) => {
                tracing::warn!(short_id = %short, "remove for an unknown satellite");
                return;
            }
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "saving player prefs");
                return;
            }
        };
        tracing::info!(client_id = %prefs.client_id, name = %prefs.name, "satellite removed");
        for link in self.links_mut() {
            if !link.connected || link.client_id.as_deref() != Some(prefs.client_id.as_str()) {
                continue;
            }
            if link.streaming {
                link.send(PlayerCmd::StreamEnd);
                link.streaming = false;
            }
            link.connected = false;
            link.send(PlayerCmd::Close);
        }
        publish.send(Published::Removed { prefs }).await.ok();
    }

    async fn on_input(&mut self, event: InputEvent) {
        match event {
            InputEvent::Started => {
                let lead = self.stream_lead();
                tracing::info!(lead_ms = lead / 1_000, "music started");
                self.pacer = Pacer::new(lead).with_input_buffer(self.input_buffer_us);
                self.pacer.start(self.clock.now_us());
                self.playing = true;
                self.speech_clock = None;
                self.music_sent_until = i64::MIN;
                let allowed: Vec<String> = self
                    .store
                    .all()
                    .filter(|p| p.allowed)
                    .map(|p| p.client_id.clone())
                    .collect();
                for link in self.links_mut() {
                    let is_allowed = link.client_id.as_ref().is_some_and(|c| allowed.contains(c));
                    if link.connected && is_allowed && !link.streaming {
                        link.send(PlayerCmd::StreamStart(PCM_48K_STEREO_16));
                        link.streaming = true;
                    }
                }
            }
            InputEvent::Pcm(pcm) => self.pacer.push(&pcm),
            InputEvent::Stopped => {
                tracing::info!("music stopped");
                self.pacer.stop();
                self.playing = false;
                if self.mixer.is_playing() {
                    self.keep_speech_streams().await;
                } else {
                    self.end_streams().await;
                }
            }
        }
    }

    /// The configured lead, raised to cover every allowed player's
    /// `min_buffer_ms + output_delay_ms` (the spec's minimum send-ahead).
    fn stream_lead(&self) -> i64 {
        let needed = self
            .links()
            .filter(|l| l.connected)
            .filter(|l| {
                l.client_id
                    .as_deref()
                    .and_then(|c| self.store.get(c))
                    .is_some_and(|p| p.allowed)
            })
            .map(|l| l.min_buffer_us + l.output_delay_us)
            .max()
            .unwrap_or(0);
        self.lead_us.max(needed)
    }

    /// A sentence to say: kept for the satellites that are connected and
    /// allowed right now.
    async fn on_speech(&mut self, mut speech: Speech, publish: &mpsc::Sender<Published>) {
        speech.players.retain(|sid| {
            self.store
                .all()
                .find(|p| short_id(&p.client_id) == *sid)
                .is_some_and(|p| p.allowed && self.is_connected(&p.client_id))
        });
        if speech.players.is_empty() {
            tracing::info!("no allowed, connected satellite for this sentence");
            let id = speech.id;
            publish
                .send(Published::Spoken { id, played: false })
                .await
                .ok();
            return;
        }
        let id = speech.id.clone();
        if !self.mixer.push(speech) {
            tracing::warn!("too many sentences waiting, this one is dropped");
            publish
                .send(Published::Spoken { id, played: false })
                .await
                .ok();
        }
    }

    /// Music stopped mid-sentence: the sentence's satellites keep their
    /// stream until it ends, from where the music left off; the others stop.
    async fn keep_speech_streams(&mut self) {
        let targets = self.mixer.targets().to_vec();
        let mut next = self.clock.now_us() + self.stream_lead();
        for link in self.links_mut() {
            if !link.streaming {
                continue;
            }
            if link.short_id().is_some_and(|s| targets.contains(&s)) {
                if let Some(n) = link.next_ts() {
                    next = next.max(n);
                }
            } else {
                link.send(PlayerCmd::StreamEnd);
                link.streaming = false;
            }
        }
        self.speech_clock = Some(next);
    }

    async fn on_tick(&mut self, publish: &mpsc::Sender<Published>) {
        let now = self.clock.now_us();
        if !self.mixer.is_playing() && self.mixer.has_queued() {
            self.start_sentence(now).await;
        }
        for link in self.links_mut() {
            link.flush_volume(now).await;
        }
        let targets = self.mixer.targets().to_vec();
        let is_target = |link: &Link| link.short_id().is_some_and(|s| targets.contains(&s));

        let chunks = if self.playing {
            self.pacer.due_chunks(now)
        } else {
            Vec::new()
        };
        self.holds.expire(now);
        if let Some(last) = chunks.last() {
            let links = self.instances.values_mut().filter_map(|i| i.link.as_mut());
            for link in links.filter(|l| l.streaming) {
                let target = is_target(link);
                let held = link
                    .short_id()
                    .is_some_and(|sid| self.holds.is_held(&sid, now));
                for c in &chunks {
                    let data = music_for(link, c.timestamp_us, &c.data, &self.mixer, target, held);
                    link.audio(c.timestamp_us, data);
                }
            }
            self.music_sent_until = last.timestamp_us + CHUNK_US;
        }

        if !self.playing && self.mixer.is_playing() {
            let lead = self.stream_lead();
            let end = self.mixer.end_us().unwrap_or(i64::MIN);
            let mut ts = self.speech_clock.unwrap_or(now + lead);
            while ts - lead <= now && ts < end {
                // One mix per distinct pair of gains.
                let mut mixed: Vec<(u32, bytes::Bytes)> = Vec::new();
                for link in self.links().filter(|l| l.streaming && is_target(l)) {
                    // No music here: only the Speech gain matters.
                    let gains = (link.levels.music_gain, link.levels.speech_gain);
                    let key = gains.1.to_bits();
                    let data = match mixed.iter().find(|(k, _)| *k == key) {
                        Some((_, d)) => d.clone(),
                        None => {
                            let d = self
                                .mixer
                                .mix_chunk(ts, None, (1.0, gains.1))
                                .unwrap_or_else(|| bytes::Bytes::from(vec![0; CHUNK_BYTES]));
                            mixed.push((key, d.clone()));
                            d
                        }
                    };
                    link.audio(ts, data);
                }
                ts += CHUNK_US;
            }
            self.speech_clock = Some(ts);
        }

        // With music the sentence is done once mixed into the chunks sent: the music stream goes
        // on. Without music its stream ends with it, so wait until the satellites have played it
        // (a stream end drops what they hold), not just until its last chunk went out `lead` ahead.
        let done_at = if self.playing {
            self.music_sent_until
        } else {
            now - SPEECH_END_MARGIN_US
        };
        if self.mixer.is_playing() && self.mixer.finish_if_done(done_at) && !self.playing {
            self.speech_clock = None;
            self.end_streams().await;
        }
        // On the shared clock a sample's timestamp is when it plays: the sentence was heard.
        let (played, waiting): (Vec<_>, Vec<_>) =
            self.reports.drain(..).partition(|(_, at)| *at <= now);
        self.reports = waiting;
        for (id, _) in played {
            publish
                .send(Published::Spoken { id, played: true })
                .await
                .ok();
        }
    }

    /// Starts the next sentence `lead` from now and opens a stream on its
    /// satellites that have none. With music, never before the music already sent: the music's
    /// lead was set when it started and stays longer than ours once a slow player leaves.
    async fn start_sentence(&mut self, now: i64) {
        let mut start = now + self.stream_lead();
        if self.playing {
            start = start.max(self.music_sent_until);
        }
        let Some(targets) = self.mixer.start_next(start).map(<[String]>::to_vec) else {
            return;
        };
        if let (Some(id), Some(at)) = (self.mixer.current_id(), self.mixer.speech_end_us()) {
            self.reports.push((id.to_owned(), at));
        }
        tracing::info!(satellites = targets.len(), "speaking");
        for link in self.links_mut() {
            let target = link.short_id().is_some_and(|s| targets.contains(&s));
            if target && link.connected && !link.streaming {
                link.send(PlayerCmd::StreamStart(PCM_48K_STEREO_16));
                link.streaming = true;
            }
        }
        if !self.playing {
            self.speech_clock = Some(start);
        }
    }
}

/// Silence, for a held satellite's music.
static SILENCE: [u8; CHUNK_BYTES] = [0; CHUNK_BYTES];

/// The chunk `link` plays at `ts`: the music (silence while held, fading back in after a
/// hold), with the sentence mixed in when `link` is one of its satellites. The same inputs
/// give the same bytes, so the satellites of one sentence stay identical.
fn music_for(
    link: &mut Link,
    ts: i64,
    music: &bytes::Bytes,
    mixer: &SpeechMixer,
    target: bool,
    held: bool,
) -> bytes::Bytes {
    let base = if held {
        link.held = true;
        link.fade_from = None;
        bytes::Bytes::from_static(&SILENCE)
    } else {
        if std::mem::take(&mut link.held) {
            link.fade_from = Some(0);
        }
        match link.fade_from {
            Some(from) => {
                let next = from + crate::pacer::CHUNK_FRAMES;
                link.fade_from = (next < FADE_FRAMES).then_some(next);
                fade_in(music, from)
            }
            None => music.clone(),
        }
    };
    let gains = (link.levels.music_gain, link.levels.speech_gain);
    let mixed = if target {
        mixer.mix_chunk(ts, Some(&base), gains)
    } else {
        None
    };
    match mixed {
        Some(chunk) => chunk,
        // Not mixed (or no sentence here): the music alone, at its own level.
        None if gains.0 == 1.0 || held => base,
        None => scale(&base, gains.0),
    }
}

/// A reported value that is not stale: while we wait for the echo of a value
/// we sent (`expect`), other reports are ignored; the echo clears the wait.
/// A player that cannot be commanded never has anything to wait for.
fn fresh<T: PartialEq + Copy>(
    expect: &mut Option<T>,
    reported: Option<T>,
    settable: bool,
) -> Option<T> {
    let reported = reported?;
    if !settable {
        *expect = None;
        return Some(reported);
    }
    match *expect {
        Some(e) if e == reported => {
            *expect = None;
            None
        }
        Some(_) => None,
        None => Some(reported),
    }
}

#[cfg(test)]
mod tests;
