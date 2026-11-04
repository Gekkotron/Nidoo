//! The group: every satellite in the house plays the one input, in sync.
//!
//! A single actor owns the player connections, the pacer and the saved
//! prefs. Only **allowed** players get audio; a new player is not allowed
//! until the operator switches it on. Volume and mute are saved, re-applied
//! on every (re)connect and published to the MQTT link on every change.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::clock::Clock;
use crate::discover::{Discovered, DiscoveryEvent};
use crate::hold::{HoldRequest, Holds};
use crate::input::InputEvent;
use crate::mixer::{FADE_FRAMES, SpeechMixer, fade_in};
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

/// Changes asked for from Nidoo (MQTT commands).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    SetVolume { short_id: String, volume: u8 },
    SetMute { short_id: String, muted: bool },
    SetAllowed { short_id: String, allowed: bool },
}

impl Control {
    pub fn short_id(&self) -> &str {
        match self {
            Control::SetVolume { short_id, .. }
            | Control::SetMute { short_id, .. }
            | Control::SetAllowed { short_id, .. } => short_id,
        }
    }

    fn apply(&self, p: &mut PlayerPrefs) {
        match *self {
            Control::SetVolume { volume, .. } => p.volume = volume.min(100),
            Control::SetMute { muted, .. } => p.muted = muted,
            Control::SetAllowed { allowed, .. } => p.allowed = allowed,
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
    expect_mute: Option<bool>,
    /// Its music was held for the last chunk sent.
    held: bool,
    /// Its music is coming back after a hold: the fade's next frame.
    fade_from: Option<usize>,
    /// Earliest timestamp the next audio chunk may carry in this stream:
    /// timestamps only go up, even when music and speech take turns.
    /// `NO_TS` when the stream has sent nothing yet.
    next_ts: AtomicI64,
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
            expect_mute: None,
            held: false,
            fade_from: None,
            next_ts: AtomicI64::new(NO_TS),
        }
    }

    /// This player's short id, once it said hello.
    fn short_id(&self) -> Option<String> {
        self.client_id.as_deref().map(short_id)
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
        if self
            .cmds
            .try_send(PlayerCmd::Audio { timestamp_us, data })
            .is_err()
        {
            tracing::debug!(client_id = ?self.client_id, "player queue full, chunk dropped");
        }
    }

    async fn send(&self, cmd: PlayerCmd) {
        if matches!(cmd, PlayerCmd::StreamStart(_) | PlayerCmd::StreamEnd) {
            self.next_ts.store(NO_TS, Ordering::Relaxed);
        }
        if self.cmds.send(cmd).await.is_err() {
            tracing::debug!(client_id = ?self.client_id, "player connection gone");
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
                    || !self.reports.is_empty() => {
                    self.on_tick(&publish).await
                }
            }
        }
    }

    /// Ends every stream (on shutdown).
    pub async fn end_streams(&mut self) {
        for link in self.links_mut() {
            if link.streaming {
                link.send(PlayerCmd::StreamEnd).await;
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
                })
                .await;
                link.send(PlayerCmd::Volume(prefs.volume)).await;
                link.expect_volume = Some(prefs.volume);
                link.send(PlayerCmd::Mute(prefs.muted)).await;
                link.expect_mute = Some(prefs.muted);
                if playing && prefs.allowed {
                    link.send(PlayerCmd::StreamStart(PCM_48K_STEREO_16)).await;
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
                let volume = fresh(&mut link.expect_volume, volume, settable("volume"))
                    .filter(|&v| v != saved.volume);
                let muted = fresh(&mut link.expect_mute, muted, settable("mute"))
                    .filter(|&m| m != saved.muted);
                if volume.is_none() && muted.is_none() {
                    return;
                }
                let updated = self.store.update(&short_id(&client_id), |p| {
                    if let Some(v) = volume {
                        p.volume = v;
                    }
                    if let Some(m) = muted {
                        p.muted = m;
                    }
                });
                if let Err(e) = updated {
                    tracing::error!(error = %format!("{e:#}"), "saving player prefs");
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
            link.send(PlayerCmd::Close).await;
        }
    }

    async fn on_control(&mut self, control: Control, publish: &mpsc::Sender<Published>) {
        let short = control.short_id().to_owned();
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
                Control::SetVolume { .. } => {
                    link.send(PlayerCmd::Volume(prefs.volume)).await;
                    link.expect_volume = Some(prefs.volume);
                }
                Control::SetMute { .. } => {
                    link.send(PlayerCmd::Mute(prefs.muted)).await;
                    link.expect_mute = Some(prefs.muted);
                }
                Control::SetAllowed { allowed: true, .. } => {
                    link.send(PlayerCmd::Activate { playback: true }).await;
                    if playing && !link.streaming {
                        link.send(PlayerCmd::StreamStart(PCM_48K_STEREO_16)).await;
                        link.streaming = true;
                    }
                }
                Control::SetAllowed { allowed: false, .. } => {
                    if link.streaming {
                        link.send(PlayerCmd::StreamEnd).await;
                        link.streaming = false;
                    }
                    link.send(PlayerCmd::Activate { playback: false }).await;
                }
            }
        }
        self.publish(publish, &prefs.client_id).await;
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
                        link.send(PlayerCmd::StreamStart(PCM_48K_STEREO_16)).await;
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
                link.send(PlayerCmd::StreamEnd).await;
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
                let data = self
                    .mixer
                    .mix_chunk(ts, None)
                    .unwrap_or_else(|| bytes::Bytes::from(vec![0; CHUNK_BYTES]));
                for link in self.links().filter(|l| l.streaming && is_target(l)) {
                    link.audio(ts, data.clone());
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
                link.send(PlayerCmd::StreamStart(PCM_48K_STEREO_16)).await;
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
    if target {
        mixer.mix_chunk(ts, Some(&base)).unwrap_or(base)
    } else {
        base
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
mod tests {
    use super::*;
    use crate::clock::FakeClock;
    use crate::pacer::{CHUNK_BYTES, CHUNK_US};
    use crate::player::PlayerInfo;
    use crate::proto::messages::PCM_48K_STEREO_16;
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(2);
    const LEAD: i64 = 500_000;

    /// One simulated player connection, as handed out by the test connector.
    struct Fake {
        instance: String,
        cmds: mpsc::Receiver<PlayerCmd>,
        to_group: mpsc::Sender<FromPlayer>,
    }

    impl Fake {
        async fn event(&self, event: PlayerEvent) {
            self.to_group
                .send(FromPlayer::Event {
                    instance: self.instance.clone(),
                    event,
                })
                .await
                .unwrap();
        }

        async fn connected(&self, client_id: &str) {
            self.event(PlayerEvent::Connected(PlayerInfo {
                client_id: client_id.into(),
                name: format!("Satellite {client_id}"),
                supported_formats: vec![PCM_48K_STEREO_16],
                buffer_capacity: 600_000,
                supported_commands: vec![],
            }))
            .await;
        }

        async fn state(&self, client_id: &str, volume: u8, muted: bool) {
            self.event(PlayerEvent::State {
                client_id: client_id.into(),
                available: true,
                volume: Some(volume),
                muted: Some(muted),
                supported_commands: vec!["volume".into(), "mute".into()],
                min_buffer_ms: Some(500),
                output_delay_ms: Some(0),
            })
            .await;
        }

        async fn cmd(&mut self) -> PlayerCmd {
            tokio::time::timeout(WAIT, self.cmds.recv())
                .await
                .expect("no command")
                .expect("closed")
        }

        /// Next command that is not audio.
        async fn control_cmd(&mut self) -> PlayerCmd {
            loop {
                match self.cmd().await {
                    PlayerCmd::Audio { .. } => continue,
                    c => return c,
                }
            }
        }

        async fn audio(&mut self) -> i64 {
            loop {
                if let PlayerCmd::Audio { timestamp_us, data } = self.cmd().await {
                    assert_eq!(data.len(), CHUNK_BYTES);
                    return timestamp_us;
                }
            }
        }

        /// Commands received within a short while.
        async fn drain(&mut self) -> Vec<PlayerCmd> {
            let mut got = Vec::new();
            while let Ok(Some(c)) =
                tokio::time::timeout(Duration::from_millis(100), self.cmds.recv()).await
            {
                got.push(c);
            }
            got
        }
    }

    struct Harness {
        discovery: mpsc::Sender<DiscoveryEvent>,
        input: mpsc::Sender<InputEvent>,
        control: mpsc::Sender<Control>,
        speech: mpsc::Sender<crate::speech::Speech>,
        holds: mpsc::Sender<crate::hold::HoldRequest>,
        published: mpsc::Receiver<Published>,
        links: mpsc::Receiver<Fake>,
        /// Where an inbound (client-initiated) connection reports.
        to_group: mpsc::Sender<FromPlayer>,
        clock: Arc<FakeClock>,
        _dir: tempfile::TempDir,
    }

    impl Harness {
        async fn start_with(prefs: &[(&str, u8, bool, bool)]) -> Harness {
            Self::start_buffered(prefs, 0).await
        }

        async fn start_buffered(prefs: &[(&str, u8, bool, bool)], buffer_us: i64) -> Harness {
            let dir = tempfile::tempdir().unwrap();
            let mut store = StateStore::load(dir.path().join("players.json")).unwrap();
            for &(id, volume, muted, allowed) in prefs {
                store.observe(id, "saved");
                store
                    .update(&short_id(id), |p| {
                        p.volume = volume;
                        p.muted = muted;
                        p.allowed = allowed;
                    })
                    .unwrap();
            }
            let clock = Arc::new(FakeClock::default());
            let (links_tx, links) = mpsc::channel(8);
            let connect: Connector = Arc::new(move |d: Discovered, to_group| {
                let links_tx = links_tx.clone();
                tokio::spawn(async move {
                    let (tx, rx) = mpsc::channel(4096);
                    to_group
                        .send(FromPlayer::Link {
                            instance: d.instance.clone(),
                            cmds: tx,
                        })
                        .await
                        .unwrap();
                    links_tx
                        .send(Fake {
                            instance: d.instance,
                            cmds: rx,
                            to_group,
                        })
                        .await
                        .unwrap();
                })
            });
            let group =
                Group::new(store, clock.clone(), LEAD, connect).with_input_buffer(buffer_us);
            let to_group = group.players();
            let (discovery, d_rx) = mpsc::channel(8);
            let (input, i_rx) = mpsc::channel(64);
            let (control, c_rx) = mpsc::channel(8);
            let (p_tx, published) = mpsc::channel(64);
            let (speech, s_rx) = mpsc::channel(8);
            let (holds, h_rx) = mpsc::channel(8);
            tokio::spawn(group.run(d_rx, i_rx, c_rx, s_rx, h_rx, p_tx));
            Harness {
                discovery,
                input,
                control,
                speech,
                holds,
                published,
                links,
                to_group,
                clock,
                _dir: dir,
            }
        }

        async fn start() -> Harness {
            Self::start_with(&[]).await
        }

        /// Discovers a player and returns its connection.
        async fn player(&mut self, instance: &str) -> Fake {
            let url = url::Url::parse(&format!("ws://10.0.0.9:8928/{instance}")).unwrap();
            self.discovery
                .send(DiscoveryEvent::Up(Discovered {
                    instance: instance.into(),
                    url,
                }))
                .await
                .unwrap();
            tokio::time::timeout(WAIT, self.links.recv())
                .await
                .unwrap()
                .unwrap()
        }

        /// A player that connected to us and finished its handshake.
        async fn accepted(&mut self, instance: &str) -> Fake {
            let (tx, cmds) = mpsc::channel(4096);
            self.to_group
                .send(FromPlayer::Accepted {
                    instance: instance.into(),
                    cmds: tx,
                    peer: Some("192.0.2.20".parse().unwrap()),
                })
                .await
                .unwrap();
            Fake {
                instance: instance.into(),
                cmds,
                to_group: self.to_group.clone(),
            }
        }

        /// Discovered, connected, first state reported: returns after Activate.
        async fn ready_player(&mut self, instance: &str, client_id: &str) -> (Fake, bool) {
            let mut p = self.player(instance).await;
            p.connected(client_id).await;
            let PlayerCmd::Activate { playback } = p.control_cmd().await else {
                panic!("expected Activate")
            };
            // Saved (or default) prefs are applied right after activation.
            assert!(matches!(p.control_cmd().await, PlayerCmd::Volume(_)));
            assert!(matches!(p.control_cmd().await, PlayerCmd::Mute(_)));
            p.state(client_id, 30, false).await;
            (p, playback)
        }

        /// Starts the music and lets the group pick it up at the current time, before a test
        /// moves the clock (the group's loop may otherwise take a tick first).
        async fn play(&self) {
            self.input.send(InputEvent::Started).await.unwrap();
            self.input
                .send(InputEvent::Pcm(bytes::Bytes::from(vec![
                    1u8;
                    4 * CHUNK_BYTES
                ])))
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(30)).await;
        }

        async fn published_for(&mut self, short: &str) -> (PlayerPrefs, bool) {
            let (prefs, connected, _) = self.published_with_address(short).await;
            (prefs, connected)
        }

        async fn published_with_address(
            &mut self,
            short: &str,
        ) -> (PlayerPrefs, bool, Option<std::net::Ipv4Addr>) {
            loop {
                let msg = tokio::time::timeout(WAIT, self.published.recv())
                    .await
                    .unwrap()
                    .unwrap();
                if let Published::Player {
                    short_id: s,
                    prefs,
                    connected,
                    address,
                } = msg
                    && s == short
                {
                    return (prefs, connected, address);
                }
            }
        }

        /// The next "sentence played" report, if one comes within a short while.
        async fn spoken(&mut self) -> Option<(String, bool)> {
            loop {
                match tokio::time::timeout(Duration::from_millis(200), self.published.recv()).await
                {
                    Ok(Some(Published::Spoken { id, played })) => return Some((id, played)),
                    Ok(Some(_)) => continue,
                    _ => return None,
                }
            }
        }
    }

    #[tokio::test]
    async fn unapproved_player_gets_no_playback_activity() {
        let mut h = Harness::start().await;
        let (mut p, playback) = h.ready_player("a", "A").await;
        assert!(!playback);
        h.play().await;
        h.clock.advance(100_000);
        let got = p.drain().await;
        assert!(
            got.iter()
                .all(|c| !matches!(c, PlayerCmd::Audio { .. } | PlayerCmd::StreamStart(_))),
            "{got:?}"
        );
        let (prefs, connected) = h.published_for(&short_id("A")).await;
        assert!(!prefs.allowed && connected);
    }

    #[tokio::test]
    async fn prefs_applied_on_connect() {
        let mut h = Harness::start_with(&[("A", 70, true, true)]).await;
        let mut p = h.player("a").await;
        p.connected("A").await;
        assert_eq!(
            p.control_cmd().await,
            PlayerCmd::Activate { playback: true }
        );
        assert_eq!(p.control_cmd().await, PlayerCmd::Volume(70));
        assert_eq!(p.control_cmd().await, PlayerCmd::Mute(true));
    }

    #[tokio::test]
    async fn stale_first_state_does_not_overwrite_saved_volume() {
        let mut h = Harness::start_with(&[("A", 70, false, true)]).await;
        let mut p = h.player("a").await;
        p.connected("A").await;
        p.drain().await;
        // The board reports its own old volume before our command reaches it.
        p.state("A", 50, false).await;
        p.state("A", 70, false).await;
        // A later change on the board itself is taken.
        p.state("A", 20, false).await;
        loop {
            let (prefs, _) = h.published_for(&short_id("A")).await;
            assert_ne!(prefs.volume, 50, "stale report must not be saved");
            if prefs.volume == 20 {
                break;
            }
        }
    }

    #[tokio::test]
    async fn approved_players_get_identical_timestamps() {
        let mut h = Harness::start_with(&[("A", 30, false, true), ("B", 30, false, true)]).await;
        let (mut a, _) = h.ready_player("a", "A").await;
        let (mut b, _) = h.ready_player("b", "B").await;
        h.play().await;
        assert!(
            matches!(a.control_cmd().await, PlayerCmd::StreamStart(f) if f == PCM_48K_STEREO_16)
        );
        assert!(matches!(b.control_cmd().await, PlayerCmd::StreamStart(_)));
        h.clock.advance(60_000);
        let ta: Vec<i64> = [a.audio().await, a.audio().await, a.audio().await].into();
        let tb: Vec<i64> = [b.audio().await, b.audio().await, b.audio().await].into();
        assert_eq!(ta, tb);
        assert_eq!(ta[1] - ta[0], CHUNK_US);
    }

    #[tokio::test]
    async fn late_joiner_gets_stream_start_and_current_timeline() {
        let mut h = Harness::start_with(&[("A", 30, false, true), ("B", 30, false, true)]).await;
        let (mut a, _) = h.ready_player("a", "A").await;
        h.play().await;
        let first = a.audio().await;
        h.clock.advance(1_000_000);
        // Let the pacer emit the second of chunks that just came due.
        while a.audio().await < first + 1_000_000 {}
        let (mut b, playback) = h.ready_player("b", "B").await;
        assert!(playback);
        assert!(matches!(b.control_cmd().await, PlayerCmd::StreamStart(_)));
        h.clock.advance(CHUNK_US);
        let ts = b.audio().await;
        assert!(
            ts >= h.clock.now_us() + LEAD - CHUNK_US,
            "future timestamps only: {ts}"
        );
        assert_eq!((ts - first) % CHUNK_US, 0, "on the shared 20 ms grid");
    }

    #[tokio::test]
    async fn input_stopped_sends_stream_end() {
        let mut h = Harness::start_with(&[("A", 30, false, true)]).await;
        let (mut a, _) = h.ready_player("a", "A").await;
        h.play().await;
        assert!(matches!(a.control_cmd().await, PlayerCmd::StreamStart(_)));
        h.input.send(InputEvent::Stopped).await.unwrap();
        assert_eq!(a.control_cmd().await, PlayerCmd::StreamEnd);
        h.clock.advance(200_000);
        assert!(
            !a.drain()
                .await
                .iter()
                .any(|c| matches!(c, PlayerCmd::Audio { .. }))
        );
    }

    #[tokio::test]
    async fn allowing_mid_stream_starts_that_player() {
        let mut h = Harness::start().await;
        let (mut a, playback) = h.ready_player("a", "A").await;
        assert!(!playback);
        h.play().await;
        h.control
            .send(Control::SetAllowed {
                short_id: short_id("A"),
                allowed: true,
            })
            .await
            .unwrap();
        assert_eq!(
            a.control_cmd().await,
            PlayerCmd::Activate { playback: true }
        );
        assert!(matches!(a.control_cmd().await, PlayerCmd::StreamStart(_)));
        h.clock.advance(CHUNK_US);
        a.audio().await;
        while !h.published_for(&short_id("A")).await.0.allowed {}
        // And disallowing stops it.
        h.control
            .send(Control::SetAllowed {
                short_id: short_id("A"),
                allowed: false,
            })
            .await
            .unwrap();
        assert_eq!(a.control_cmd().await, PlayerCmd::StreamEnd);
        assert_eq!(
            a.control_cmd().await,
            PlayerCmd::Activate { playback: false }
        );
    }

    #[tokio::test]
    async fn lead_covers_the_players_min_buffer() {
        let mut h = Harness::start_with(&[("A", 30, false, true)]).await;
        let mut a = h.player("a").await;
        a.connected("A").await;
        a.drain().await;
        a.event(PlayerEvent::State {
            client_id: "A".into(),
            available: true,
            volume: Some(30),
            muted: Some(false),
            supported_commands: vec![],
            min_buffer_ms: Some(700),
            output_delay_ms: Some(100),
        })
        .await;
        a.drain().await;
        h.clock.set(10_000_000);
        h.play().await;
        let ts = a.audio().await;
        assert!(
            ts >= 10_000_000 + 800_000,
            "lead >= min_buffer + output_delay: {ts}"
        );
    }

    #[tokio::test]
    async fn command_for_offline_player_is_stored_and_echoed() {
        let mut h = Harness::start_with(&[("A", 30, false, true)]).await;
        h.control
            .send(Control::SetVolume {
                short_id: short_id("A"),
                volume: 64,
            })
            .await
            .unwrap();
        loop {
            let (prefs, connected) = h.published_for(&short_id("A")).await;
            assert!(!connected);
            if prefs.volume == 64 {
                break;
            }
        }
        // Applied when the board comes back.
        let mut p = h.player("a").await;
        p.connected("A").await;
        assert_eq!(
            p.control_cmd().await,
            PlayerCmd::Activate { playback: true }
        );
        assert_eq!(p.control_cmd().await, PlayerCmd::Volume(64));
    }

    #[tokio::test]
    async fn client_state_change_publishes_new_volume() {
        let mut h = Harness::start().await;
        let (p, _) = h.ready_player("a", "A").await;
        // The echo of the default volume clears the wait; then a change on the board.
        p.state("A", 12, false).await;
        loop {
            let (prefs, _) = h.published_for(&short_id("A")).await;
            if prefs.volume == 12 {
                break;
            }
        }
        let saved = StateStore::load(h._dir.path().join("players.json")).unwrap();
        assert_eq!(saved.get("A").unwrap().volume, 12);
    }

    #[tokio::test]
    async fn gone_publishes_disconnected() {
        let mut h = Harness::start().await;
        let (p, _) = h.ready_player("a", "A").await;
        p.event(PlayerEvent::Gone {
            client_id: "A".into(),
        })
        .await;
        loop {
            let (_, connected) = h.published_for(&short_id("A")).await;
            if !connected {
                break;
            }
        }
    }

    #[tokio::test]
    async fn accepted_player_is_served_like_a_discovered_one() {
        let mut h = Harness::start_with(&[("A", 40, true, true)]).await;
        let mut p = h.accepted("browser-1").await;
        p.connected("A").await;
        assert_eq!(
            p.control_cmd().await,
            PlayerCmd::Activate { playback: true }
        );
        assert_eq!(p.control_cmd().await, PlayerCmd::Volume(40));
        assert_eq!(p.control_cmd().await, PlayerCmd::Mute(true));
        p.state("A", 40, true).await;
        // The saved prefs are first published as offline at start.
        while !h.published_for(&short_id("A")).await.1 {}
        h.play().await;
        assert!(matches!(p.control_cmd().await, PlayerCmd::StreamStart(_)));
        h.clock.advance(60_000);
        p.audio().await;
    }

    #[tokio::test]
    async fn accepted_player_is_forgotten_when_gone() {
        let mut h = Harness::start().await;
        let mut p = h.accepted("browser-1").await;
        p.connected("A").await;
        p.drain().await;
        p.event(PlayerEvent::Gone {
            client_id: "A".into(),
        })
        .await;
        loop {
            let (_, connected) = h.published_for(&short_id("A")).await;
            if !connected {
                break;
            }
        }
        // The reload of a browser tab: a new connection under a new name.
        let mut again = h.accepted("browser-2").await;
        again.connected("A").await;
        assert!(matches!(
            again.control_cmd().await,
            PlayerCmd::Activate { .. }
        ));
        while !h.published_for(&short_id("A")).await.1 {}
    }

    #[tokio::test]
    async fn reconnecting_player_closes_its_older_connection() {
        let mut h = Harness::start_with(&[("A", 40, false, true)]).await;
        let mut old = h.accepted("browser-1").await;
        old.connected("A").await;
        old.drain().await;
        // The same player opens a second connection without closing the first.
        let mut new = h.accepted("browser-2").await;
        new.connected("A").await;
        assert!(
            old.drain().await.contains(&PlayerCmd::Close),
            "the older connection must be closed"
        );
        new.drain().await;
        h.control
            .send(Control::SetVolume {
                short_id: short_id("A"),
                volume: 55,
            })
            .await
            .unwrap();
        assert_eq!(new.control_cmd().await, PlayerCmd::Volume(55));
        assert!(old.drain().await.is_empty());
    }

    #[tokio::test]
    async fn input_buffer_delays_the_first_audio() {
        let mut h = Harness::start_buffered(&[("A", 30, false, true)], 100_000).await;
        let (mut a, _) = h.ready_player("a", "A").await;
        h.play().await;
        assert!(matches!(a.control_cmd().await, PlayerCmd::StreamStart(_)));
        h.clock.advance(60_000);
        assert!(
            a.drain().await.is_empty(),
            "no audio before the buffer fills"
        );
        h.clock.advance(40_000);
        assert_eq!(a.audio().await, 100_000 + LEAD);
    }

    // ---- speech ----

    use crate::pacer::CHUNK_FRAMES;
    use crate::speech::Speech;

    /// `frames` of a constant sample for these client ids.
    fn speech_for(ids: &[&str], value: i16, frames: usize) -> Speech {
        Speech {
            id: "sentence".into(),
            players: ids.iter().map(|id| short_id(id)).collect(),
            pcm: bytes::Bytes::from(
                std::iter::repeat_n(value.to_le_bytes(), frames * 2)
                    .flatten()
                    .collect::<Vec<u8>>(),
            ),
        }
    }

    fn has_sample(data: &[u8], v: i16) -> bool {
        data.as_chunks::<2>()
            .0
            .iter()
            .any(|b| i16::from_le_bytes(*b) == v)
    }

    fn audio_of(cmds: &[PlayerCmd]) -> Vec<(i64, bytes::Bytes)> {
        cmds.iter()
            .filter_map(|c| match c {
                PlayerCmd::Audio { timestamp_us, data } => Some((*timestamp_us, data.clone())),
                _ => None,
            })
            .collect()
    }

    impl Harness {
        /// Hands a sentence to the group and lets it pick it up at the current time.
        async fn say(&self, speech: Speech) {
            self.speech.send(speech).await.unwrap();
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }

    #[tokio::test]
    async fn speech_with_music_reaches_only_its_targets() {
        let mut h = Harness::start_with(&[("A", 30, false, true), ("B", 30, false, true)]).await;
        let (mut a, _) = h.ready_player("a", "A").await;
        let (mut b, _) = h.ready_player("b", "B").await;
        h.play().await;
        assert!(matches!(a.control_cmd().await, PlayerCmd::StreamStart(_)));
        assert!(matches!(b.control_cmd().await, PlayerCmd::StreamStart(_)));
        h.say(speech_for(&["A"], 8_000, CHUNK_FRAMES)).await;
        h.clock.advance(1_000_000);
        let got_a = audio_of(&a.drain().await);
        let got_b = audio_of(&b.drain().await);
        assert!(got_a.iter().any(|(_, d)| has_sample(d, 8_000)), "A says it");
        // B keeps the shared music chunks, untouched: only the music's
        // samples (0x0101 bytes, then silence), never speech or a ducked value.
        assert!(!got_b.is_empty());
        assert!(got_b.iter().all(|(_, d)| {
            d.as_chunks::<2>()
                .0
                .iter()
                .all(|b| matches!(i16::from_le_bytes(*b), 0 | 257))
        }));
    }

    #[tokio::test]
    async fn speech_without_music_starts_and_ends_a_stream() {
        let mut h = Harness::start_with(&[("A", 30, false, true), ("B", 30, false, true)]).await;
        let (mut a, _) = h.ready_player("a", "A").await;
        let (mut b, _) = h.ready_player("b", "B").await;
        h.say(speech_for(&["A"], 8_000, CHUNK_FRAMES)).await;
        assert!(matches!(a.control_cmd().await, PlayerCmd::StreamStart(_)));
        h.clock.advance(1_000_000);
        let cmds = a.drain().await;
        assert!(audio_of(&cmds).iter().any(|(_, d)| has_sample(d, 8_000)));
        assert_eq!(
            cmds.last(),
            Some(&PlayerCmd::StreamEnd),
            "ends after the sentence"
        );
        assert!(b.drain().await.is_empty(), "B hears nothing");
    }

    #[tokio::test]
    async fn speech_skips_players_not_allowed_or_offline() {
        let mut h = Harness::start_with(&[("A", 30, false, false)]).await;
        let (mut a, _) = h.ready_player("a", "A").await;
        h.say(speech_for(&["A", "Offline"], 8_000, CHUNK_FRAMES))
            .await;
        h.clock.advance(1_000_000);
        let cmds = a.drain().await;
        assert!(
            !cmds
                .iter()
                .any(|c| matches!(c, PlayerCmd::StreamStart(_) | PlayerCmd::Audio { .. })),
            "{cmds:?}"
        );
    }

    #[tokio::test]
    async fn two_sentences_play_one_after_the_other() {
        let mut h = Harness::start_with(&[("A", 30, false, true)]).await;
        let (mut a, _) = h.ready_player("a", "A").await;
        h.speech
            .send(speech_for(&["A"], 8_000, CHUNK_FRAMES))
            .await
            .unwrap();
        h.say(speech_for(&["A"], 4_000, CHUNK_FRAMES)).await;
        let mut cmds = Vec::new();
        for _ in 0..3 {
            h.clock.advance(1_000_000);
            cmds.extend(a.drain().await);
        }
        let audio = audio_of(&cmds);
        let first_end = audio
            .iter()
            .filter(|(_, d)| has_sample(d, 8_000))
            .map(|(t, _)| *t)
            .max()
            .expect("first sentence");
        let second_start = audio
            .iter()
            .filter(|(_, d)| has_sample(d, 4_000))
            .map(|(t, _)| *t)
            .min()
            .expect("second sentence");
        assert!(second_start > first_end, "{second_start} after {first_end}");
    }

    #[tokio::test]
    async fn music_stopping_mid_sentence_keeps_the_stream() {
        let mut h = Harness::start_with(&[("A", 30, false, true)]).await;
        let (mut a, _) = h.ready_player("a", "A").await;
        h.play().await;
        assert!(matches!(a.control_cmd().await, PlayerCmd::StreamStart(_)));
        // Half a second of speech: the window lasts 0.8 s.
        h.say(speech_for(&["A"], 8_000, 25 * CHUNK_FRAMES)).await;
        h.clock.advance(600_000);
        a.drain().await;
        h.input.send(InputEvent::Stopped).await.unwrap();
        let mid = a.drain().await;
        assert!(!mid.contains(&PlayerCmd::StreamEnd), "{mid:?}");
        h.clock.advance(1_000_000);
        let rest = a.drain().await;
        assert!(audio_of(&rest).iter().any(|(_, d)| has_sample(d, 8_000)));
        assert_eq!(rest.last(), Some(&PlayerCmd::StreamEnd));
    }

    #[tokio::test]
    async fn a_sentence_without_music_ends_its_stream_only_once_played() {
        // Window: two 150 ms fades + 500 ms of speech = 800 ms, from now + lead (500 ms): it
        // plays until 1.3 s. Every chunk is sent by 0.8 s (lead ahead), but ending the stream
        // then would make the satellite drop the half second it has not played yet.
        let mut h = Harness::start_with(&[("A", 30, false, true)]).await;
        let (mut a, _) = h.ready_player("a", "A").await;
        h.say(speech_for(&["A"], 8_000, 25 * CHUNK_FRAMES)).await;
        h.clock.advance(1_000_000);
        let sent = a.drain().await;
        assert!(audio_of(&sent).iter().any(|(_, d)| has_sample(d, 8_000)));
        assert!(
            !sent.contains(&PlayerCmd::StreamEnd),
            "the stream ended before the sentence played: {:?}",
            sent.iter()
                .filter(|c| !matches!(c, PlayerCmd::Audio { .. }))
                .collect::<Vec<_>>()
        );
        h.clock.advance(600_000);
        assert_eq!(a.drain().await.last(), Some(&PlayerCmd::StreamEnd));
    }

    #[tokio::test]
    async fn music_starting_mid_sentence_never_overlaps() {
        let mut h = Harness::start_with(&[("A", 30, false, true)]).await;
        let (mut a, _) = h.ready_player("a", "A").await;
        h.say(speech_for(&["A"], 8_000, 25 * CHUNK_FRAMES)).await;
        let mut cmds = Vec::new();
        h.clock.advance(300_000);
        cmds.extend(a.drain().await);
        h.play().await;
        h.clock.advance(1_000_000);
        cmds.extend(a.drain().await);
        let starts = cmds
            .iter()
            .filter(|c| matches!(c, PlayerCmd::StreamStart(_)))
            .count();
        assert_eq!(starts, 1, "one stream: {cmds:?}");
        let ts: Vec<i64> = audio_of(&cmds).iter().map(|(t, _)| *t).collect();
        assert!(ts.len() > 10);
        assert!(ts.windows(2).all(|w| w[1] > w[0]), "{ts:?}");
    }

    #[tokio::test]
    async fn a_target_leaving_mid_sentence_does_not_stop_the_others() {
        let mut h = Harness::start_with(&[("A", 30, false, true), ("B", 30, false, true)]).await;
        let (a, _) = h.ready_player("a", "A").await;
        let (mut b, _) = h.ready_player("b", "B").await;
        h.say(speech_for(&["A", "B"], 8_000, 25 * CHUNK_FRAMES))
            .await;
        h.clock.advance(200_000);
        a.event(PlayerEvent::Gone {
            client_id: "A".into(),
        })
        .await;
        drop(a);
        // Past the sentence's end (1.3 s) and the end margin.
        h.clock.advance(1_300_000);
        let cmds = b.drain().await;
        assert!(audio_of(&cmds).iter().any(|(_, d)| has_sample(d, 8_000)));
        assert_eq!(cmds.last(), Some(&PlayerCmd::StreamEnd));
    }

    #[tokio::test]
    async fn a_sentence_over_music_starts_after_the_music_already_sent() {
        // The music started with a slow player's 1.5 s lead; once it leaves, ours is 0.5 s, but
        // the music is already sent 1.5 s ahead: a sentence starting 0.5 s ahead would land on
        // chunks gone out without it.
        let mut h = Harness::start_with(&[("A", 30, false, true), ("S", 30, false, true)]).await;
        let (mut a, _) = h.ready_player("a", "A").await;
        let (mut s, _) = h.ready_player("s", "S").await;
        s.event(PlayerEvent::State {
            client_id: "S".into(),
            available: true,
            volume: Some(30),
            muted: Some(false),
            supported_commands: vec![],
            min_buffer_ms: Some(1_500),
            output_delay_ms: Some(0),
        })
        .await;
        s.drain().await;
        h.play().await;
        h.clock.advance(100_000);
        s.event(PlayerEvent::Gone {
            client_id: "S".into(),
        })
        .await;
        drop(s);
        a.drain().await;
        // Half a second of speech: 25 chunks.
        h.say(speech_for(&["A"], 8_000, 25 * CHUNK_FRAMES)).await;
        let mut cmds = Vec::new();
        for _ in 0..3 {
            h.clock.advance(1_000_000);
            cmds.extend(a.drain().await);
        }
        let spoken = audio_of(&cmds)
            .iter()
            .filter(|(_, d)| has_sample(d, 8_000))
            .count();
        assert!(spoken >= 25, "only {spoken} chunks carry the sentence");
    }

    #[tokio::test]
    async fn targets_of_one_sentence_get_identical_chunks() {
        let mut h = Harness::start_with(&[("A", 30, false, true), ("B", 30, false, true)]).await;
        let (mut a, _) = h.ready_player("a", "A").await;
        let (mut b, _) = h.ready_player("b", "B").await;
        h.say(speech_for(&["A", "B"], 8_000, CHUNK_FRAMES)).await;
        h.clock.advance(1_000_000);
        let got_a = audio_of(&a.drain().await);
        assert!(!got_a.is_empty());
        assert_eq!(got_a, audio_of(&b.drain().await));
    }

    // ---- hold ----

    use crate::hold::HoldRequest;

    impl Harness {
        /// Holds (or, with `for_ms: 0`, releases) these client ids' music.
        async fn hold(&self, ids: &[&str], for_ms: u32) {
            self.holds
                .send(HoldRequest {
                    id: "exchange".into(),
                    players: ids.iter().map(|id| short_id(id)).collect(),
                    for_ms,
                })
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(30)).await;
        }

        /// Starts `chunks` of music (samples 257) at the current time.
        async fn play_long(&self, chunks: usize) {
            self.input.send(InputEvent::Started).await.unwrap();
            self.input
                .send(InputEvent::Pcm(bytes::Bytes::from(vec![
                    1u8;
                    chunks
                        * CHUNK_BYTES
                ])))
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }

    fn samples(data: &[u8]) -> impl Iterator<Item = i16> + '_ {
        data.as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
    }

    #[tokio::test]
    async fn held_player_gets_silence_others_keep_music() {
        let mut h = Harness::start_with(&[("A", 30, false, true), ("B", 30, false, true)]).await;
        let (mut a, _) = h.ready_player("a", "A").await;
        let (mut b, _) = h.ready_player("b", "B").await;
        h.hold(&["A"], 30_000).await;
        h.play_long(50).await;
        h.clock.advance(500_000);
        let got_a = audio_of(&a.drain().await);
        let got_b = audio_of(&b.drain().await);
        assert!(!got_a.is_empty());
        assert!(
            got_a.iter().all(|(_, d)| samples(d).all(|v| v == 0)),
            "A is held"
        );
        assert!(
            got_b.iter().any(|(_, d)| has_sample(d, 257)),
            "B keeps the music"
        );
    }

    #[tokio::test]
    async fn held_player_still_hears_speech() {
        let mut h = Harness::start_with(&[("A", 30, false, true)]).await;
        let (mut a, _) = h.ready_player("a", "A").await;
        h.hold(&["A"], 30_000).await;
        h.play_long(50).await;
        h.say(speech_for(&["A"], 8_000, CHUNK_FRAMES)).await;
        h.clock.advance(1_000_000);
        let got = audio_of(&a.drain().await);
        assert!(
            got.iter().any(|(_, d)| has_sample(d, 8_000)),
            "the sentence is heard"
        );
        assert!(
            got.iter()
                .all(|(_, d)| samples(d).all(|v| v == 0 || v == 8_000)),
            "over silence, not over ducked music"
        );
    }

    #[tokio::test]
    async fn released_player_fades_back_in() {
        let mut h = Harness::start_with(&[("A", 30, false, true)]).await;
        let (mut a, _) = h.ready_player("a", "A").await;
        h.hold(&["A"], 30_000).await;
        h.play_long(100).await;
        h.clock.advance(200_000);
        tokio::time::sleep(Duration::from_millis(30)).await;
        h.hold(&["A"], 0).await;
        h.clock.advance(800_000);
        let got = audio_of(&a.drain().await);
        let first = got
            .iter()
            .position(|(_, d)| samples(d).any(|v| v != 0))
            .expect("the music comes back");
        assert!(
            samples(&got[first].1).all(|v| v < 257),
            "the first chunk back is faded"
        );
        assert!(
            got[first..]
                .iter()
                .any(|(_, d)| samples(d).all(|v| v == 257))
        );
    }

    #[tokio::test]
    async fn hold_on_unknown_player_is_harmless() {
        let mut h = Harness::start_with(&[("A", 30, false, true)]).await;
        let (mut a, _) = h.ready_player("a", "A").await;
        h.hold(&["nobody"], 30_000).await;
        h.play_long(50).await;
        h.clock.advance(500_000);
        assert!(
            audio_of(&a.drain().await)
                .iter()
                .any(|(_, d)| has_sample(d, 257))
        );
    }

    // ---- reports ----

    #[tokio::test]
    async fn spoken_is_reported_when_the_last_sample_plays() {
        let mut h = Harness::start_with(&[("A", 30, false, true)]).await;
        let (_a, _) = h.ready_player("a", "A").await;
        h.say(speech_for(&["A"], 8_000, 48_000)).await;
        h.clock.advance(1_000_000);
        assert_eq!(h.spoken().await, None, "1 s of speech after a 150 ms fade");
        h.clock.advance(1_000_000 + LEAD);
        assert_eq!(h.spoken().await, Some(("sentence".into(), true)));
        h.clock.advance(1_000_000);
        assert_eq!(h.spoken().await, None, "once");
    }

    #[tokio::test]
    async fn spoken_is_reported_over_music_too() {
        let mut h = Harness::start_with(&[("A", 30, false, true)]).await;
        let (_a, _) = h.ready_player("a", "A").await;
        h.play_long(200).await;
        h.say(speech_for(&["A"], 8_000, 48_000)).await;
        h.clock.advance(1_000_000);
        assert_eq!(h.spoken().await, None);
        h.clock.advance(1_000_000 + LEAD);
        assert_eq!(h.spoken().await, Some(("sentence".into(), true)));
    }

    #[tokio::test]
    async fn dropped_sentence_reports_not_played() {
        let mut h = Harness::start_with(&[("A", 30, false, false)]).await;
        let (_a, _) = h.ready_player("a", "A").await;
        h.say(speech_for(&["A"], 8_000, CHUNK_FRAMES)).await;
        assert_eq!(h.spoken().await, Some(("sentence".into(), false)));
    }

    #[tokio::test]
    async fn connected_player_publishes_its_address() {
        let mut h = Harness::start_with(&[("A", 30, false, true)]).await;
        let a = h.player("a").await;
        a.connected("A").await;
        let address = loop {
            let (_, connected, address) = h.published_with_address(&short_id("A")).await;
            if connected {
                break address;
            }
            assert_eq!(address, None, "no address while not connected");
        };
        assert_eq!(address, Some("10.0.0.9".parse().unwrap()));
    }

    #[tokio::test]
    async fn accepted_player_publishes_its_peer_address() {
        let mut h = Harness::start_with(&[("A", 30, false, true)]).await;
        let a = h.accepted("in").await;
        a.connected("A").await;
        let address = loop {
            let (_, connected, address) = h.published_with_address(&short_id("A")).await;
            if connected {
                break address;
            }
            assert_eq!(address, None, "no address while not connected");
        };
        assert_eq!(address, Some("192.0.2.20".parse().unwrap()));
    }
}
