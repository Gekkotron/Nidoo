//! The group: every satellite in the house plays the one input, in sync.
//!
//! A single actor owns the player connections, the pacer and the saved
//! prefs. Only **allowed** players get audio; a new player is not allowed
//! until the operator switches it on. Volume and mute are saved, re-applied
//! on every (re)connect and published to the MQTT link on every change.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::clock::Clock;
use crate::discover::{Discovered, DiscoveryEvent};
use crate::input::InputEvent;
use crate::pacer::Pacer;
use crate::player::{PlayerCmd, PlayerEvent, run_player};
use crate::proto::messages::PCM_48K_STEREO_16;
use crate::proto::noise::ServerIdentity;
use crate::state::{PlayerPrefs, StateStore, short_id};

/// How often the pacer is polled while playing.
const TICK: Duration = Duration::from_millis(5);
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
    },
}

/// From a connection task to the group.
#[derive(Debug)]
pub enum FromPlayer {
    /// A new connection attempt to `instance`; commands go to `cmds`.
    Link {
        instance: String,
        cmds: mpsc::Sender<PlayerCmd>,
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
}

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
        }
    }

    async fn send(&self, cmd: PlayerCmd) {
        if self.cmds.send(cmd).await.is_err() {
            tracing::debug!(client_id = ?self.client_id, "player connection gone");
        }
    }
}

/// An advertised (or recently advertised) player.
struct Instance {
    url: url::Url,
    task: JoinHandle<()>,
    advertised: bool,
    link: Option<Link>,
}

pub struct Group {
    store: StateStore,
    clock: Arc<dyn Clock>,
    lead_us: i64,
    connect: Connector,
    instances: HashMap<String, Instance>,
    pacer: Pacer,
    playing: bool,
}

impl Group {
    pub fn new(store: StateStore, clock: Arc<dyn Clock>, lead_us: i64, connect: Connector) -> Self {
        Self {
            store,
            clock,
            lead_us,
            connect,
            instances: HashMap::new(),
            pacer: Pacer::new(lead_us),
            playing: false,
        }
    }

    pub async fn run(
        mut self,
        mut discovery: mpsc::Receiver<DiscoveryEvent>,
        mut input: mpsc::Receiver<InputEvent>,
        mut control: mpsc::Receiver<Control>,
        publish: mpsc::Sender<Published>,
    ) -> anyhow::Result<()> {
        let (to_group, mut from_players) = mpsc::channel(1024);
        let known: Vec<PlayerPrefs> = self.store.all().cloned().collect();
        for prefs in known {
            let msg = Published::Player {
                short_id: short_id(&prefs.client_id),
                prefs,
                connected: false,
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
                Some(m) = from_players.recv() => self.on_player(m, &publish).await,
                _ = tick.tick(), if self.playing => self.on_tick(),
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
        let msg = Published::Player {
            short_id: short_id(client_id),
            prefs,
            connected,
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
                    if i.url == d.url && !i.task.is_finished() {
                        return;
                    }
                    i.task.abort();
                }
                tracing::info!(instance = %d.instance, url = %d.url, "satellite found");
                let instance = d.instance.clone();
                let url = d.url.clone();
                let task = (self.connect)(d, to_group.clone());
                let old = self.instances.insert(
                    instance,
                    Instance {
                        url,
                        task,
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
                    i.task.abort();
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
                    i.task.abort();
                    self.instances.remove(instance);
                }
                self.publish(publish, &client_id).await;
            }
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
                self.pacer = Pacer::new(lead);
                self.pacer.start(self.clock.now_us());
                self.playing = true;
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
                self.end_streams().await;
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

    fn on_tick(&mut self) {
        let chunks = self.pacer.due_chunks(self.clock.now_us());
        if chunks.is_empty() {
            return;
        }
        for link in self.links().filter(|l| l.streaming) {
            for c in &chunks {
                let cmd = PlayerCmd::Audio {
                    timestamp_us: c.timestamp_us,
                    data: c.data.clone(),
                };
                if link.cmds.try_send(cmd).is_err() {
                    tracing::debug!(client_id = ?link.client_id, "player queue full, chunk dropped");
                }
            }
        }
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
        published: mpsc::Receiver<Published>,
        links: mpsc::Receiver<Fake>,
        clock: Arc<FakeClock>,
        _dir: tempfile::TempDir,
    }

    impl Harness {
        async fn start_with(prefs: &[(&str, u8, bool, bool)]) -> Harness {
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
            let group = Group::new(store, clock.clone(), LEAD, connect);
            let (discovery, d_rx) = mpsc::channel(8);
            let (input, i_rx) = mpsc::channel(64);
            let (control, c_rx) = mpsc::channel(8);
            let (p_tx, published) = mpsc::channel(64);
            tokio::spawn(group.run(d_rx, i_rx, c_rx, p_tx));
            Harness {
                discovery,
                input,
                control,
                published,
                links,
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

        async fn play(&self) {
            self.input.send(InputEvent::Started).await.unwrap();
            self.input
                .send(InputEvent::Pcm(bytes::Bytes::from(vec![
                    1u8;
                    4 * CHUNK_BYTES
                ])))
                .await
                .unwrap();
        }

        async fn published_for(&mut self, short: &str) -> (PlayerPrefs, bool) {
            loop {
                let Published::Player {
                    short_id: s,
                    prefs,
                    connected,
                } = tokio::time::timeout(WAIT, self.published.recv())
                    .await
                    .unwrap()
                    .unwrap();
                if s == short {
                    return (prefs, connected);
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
}
