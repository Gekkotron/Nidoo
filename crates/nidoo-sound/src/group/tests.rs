use super::*;
use crate::clock::FakeClock;
use crate::pacer::{CHUNK_BYTES, CHUNK_US};
use crate::player::PlayerInfo;
use crate::proto::messages::PCM_48K_STEREO_16;
use crate::state::DEFAULT_VOLUME;
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
        Self::start_full(prefs, &[], buffer_us).await
    }

    /// Players saved with their own Speech volume: `(id, music, speech)`.
    async fn start_levels(levels: &[(&str, u8, u8)]) -> Harness {
        let prefs: Vec<_> = levels
            .iter()
            .map(|&(id, m, _)| (id, m, false, true))
            .collect();
        let speech: Vec<_> = levels.iter().map(|&(id, _, sp)| (id, sp)).collect();
        Self::start_full(&prefs, &speech, 0).await
    }

    /// `speech` overrides a player's Speech volume (default: its volume).
    async fn start_full(
        prefs: &[(&str, u8, bool, bool)],
        speech: &[(&str, u8)],
        buffer_us: i64,
    ) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let mut store = StateStore::load(dir.path().join("players.json")).unwrap();
        for &(id, volume, muted, allowed) in prefs {
            store.observe(id, "saved");
            store
                .update(&short_id(id), |p| {
                    p.volume = volume;
                    p.speech_volume = speech
                        .iter()
                        .find(|(i, _)| *i == id)
                        .map_or(volume, |&(_, v)| v);
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
        let group = Group::new(store, clock.clone(), LEAD, connect).with_input_buffer(buffer_us);
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
        self.accepted_with(instance, 4096).await
    }

    /// [`Harness::accepted`] with a command queue of `capacity`.
    async fn accepted_with(&mut self, instance: &str, capacity: usize) -> Fake {
        let (tx, cmds) = mpsc::channel(capacity);
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

    /// The prefs of the next satellite forgotten.
    async fn removed(&mut self) -> PlayerPrefs {
        loop {
            let msg = tokio::time::timeout(WAIT, self.published.recv())
                .await
                .unwrap()
                .unwrap();
            if let Published::Removed { prefs } = msg {
                return prefs;
            }
        }
    }

    /// The next "sentence played" report, if one comes within a short while.
    async fn spoken(&mut self) -> Option<(String, bool)> {
        loop {
            match tokio::time::timeout(Duration::from_millis(200), self.published.recv()).await {
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
    assert!(matches!(a.control_cmd().await, PlayerCmd::StreamStart(f) if f == PCM_48K_STEREO_16));
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
async fn removing_an_offline_player_forgets_it() {
    let mut h = Harness::start_with(&[("A", 64, true, true), ("B", 30, false, true)]).await;
    h.control
        .send(Control::Remove {
            short_id: short_id("A"),
        })
        .await
        .unwrap();
    assert_eq!(h.removed().await.client_id, "A");
    let saved = StateStore::load(h._dir.path().join("players.json")).unwrap();
    assert_eq!(saved.get("A"), None);
    assert!(saved.get("B").is_some());
}

#[tokio::test]
async fn removing_a_connected_player_closes_it_and_it_comes_back_as_new() {
    let mut h = Harness::start_with(&[("A", 64, false, true)]).await;
    let (mut p, playback) = h.ready_player("a", "A").await;
    assert!(playback);
    h.play().await;
    assert!(matches!(p.control_cmd().await, PlayerCmd::StreamStart(_)));
    h.control
        .send(Control::Remove {
            short_id: short_id("A"),
        })
        .await
        .unwrap();
    assert_eq!(h.removed().await.client_id, "A");
    let got = p.drain().await;
    assert!(got.contains(&PlayerCmd::StreamEnd), "{got:?}");
    assert!(got.contains(&PlayerCmd::Close), "{got:?}");
    // Its reconnect loop brings it back: a new satellite, waiting for approval.
    let mut again = h.accepted("a-again").await;
    again.connected("A").await;
    assert_eq!(
        again.control_cmd().await,
        PlayerCmd::Activate { playback: false }
    );
    assert_eq!(again.control_cmd().await, PlayerCmd::Volume(DEFAULT_VOLUME));
    loop {
        let (prefs, connected) = h.published_for(&short_id("A")).await;
        if connected {
            assert!(!prefs.allowed);
            break;
        }
    }
}

#[tokio::test]
async fn removing_an_unknown_player_publishes_nothing() {
    let mut h = Harness::start().await;
    h.control
        .send(Control::Remove {
            short_id: "000000000000".into(),
        })
        .await
        .unwrap();
    let got = tokio::time::timeout(Duration::from_millis(200), h.published.recv()).await;
    assert!(got.is_err(), "{got:?}");
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

/// Connects `client_id` and consumes the connect commands; returns the Volume sent.
async fn connect_levels(h: &mut Harness, instance: &str, client_id: &str) -> (Fake, u8) {
    let mut p = h.player(instance).await;
    p.connected(client_id).await;
    assert!(matches!(p.control_cmd().await, PlayerCmd::Activate { .. }));
    let PlayerCmd::Volume(v) = p.control_cmd().await else {
        panic!("expected Volume")
    };
    assert!(matches!(p.control_cmd().await, PlayerCmd::Mute(_)));
    loop {
        if h.published_for(&short_id(client_id)).await.1 {
            break;
        }
    }
    (p, v)
}

fn volumes_of(cmds: &[PlayerCmd]) -> Vec<u8> {
    cmds.iter()
        .filter_map(|c| match c {
            PlayerCmd::Volume(v) => Some(*v),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn connect_sends_the_louder_setting() {
    let mut h = Harness::start_levels(&[("A", 30, 80)]).await;
    let (_p, v) = connect_levels(&mut h, "a", "A").await;
    assert_eq!(v, 80);
}

#[tokio::test]
async fn quieter_change_sends_no_volume() {
    let mut h = Harness::start_levels(&[("A", 30, 80)]).await;
    let (mut p, _) = connect_levels(&mut h, "a", "A").await;
    h.control
        .send(Control::SetVolume {
            short_id: short_id("A"),
            volume: 20,
        })
        .await
        .unwrap();
    loop {
        let (prefs, _) = h.published_for(&short_id("A")).await;
        if prefs.volume == 20 {
            assert_eq!(prefs.speech_volume, 80);
            break;
        }
    }
    assert_eq!(volumes_of(&p.drain().await), Vec::<u8>::new());
}

#[tokio::test]
async fn louder_change_sends_volume() {
    let mut h = Harness::start_levels(&[("A", 30, 80)]).await;
    let (mut p, _) = connect_levels(&mut h, "a", "A").await;
    h.control
        .send(Control::SetSpeech {
            short_id: short_id("A"),
            volume: 90,
        })
        .await
        .unwrap();
    assert_eq!(p.control_cmd().await, PlayerCmd::Volume(90));
}

/// Sets Speech to 80 on a streaming player at (30, 30) and returns it with
/// the timestamp of the first chunk that carries the new gains.
async fn streaming_then_speech_80(h: &mut Harness) -> (Fake, i64) {
    let (mut a, _) = h.ready_player("a", "A").await;
    h.play_long(50).await;
    h.clock.advance(200_000);
    let sent = audio_of(&a.drain().await);
    let next = sent.last().expect("music is streaming").0 + CHUNK_US;
    h.control
        .send(Control::SetSpeech {
            short_id: short_id("A"),
            volume: 80,
        })
        .await
        .unwrap();
    loop {
        if h.published_for(&short_id("A")).await.0.speech_volume == 80 {
            break;
        }
    }
    (a, next)
}

#[tokio::test]
async fn volume_waits_for_the_audio_while_streaming() {
    let mut h = Harness::start_levels(&[("A", 30, 30)]).await;
    let (mut a, next) = streaming_then_speech_80(&mut h).await;
    assert!(next > h.clock.now_us());
    assert_eq!(volumes_of(&a.drain().await), Vec::<u8>::new());
    h.clock.set(next - 1);
    assert_eq!(volumes_of(&a.drain().await), Vec::<u8>::new());
    h.clock.set(next);
    assert_eq!(volumes_of(&a.drain().await), vec![80]);
    // Sent once.
    h.clock.advance(100_000);
    assert_eq!(volumes_of(&a.drain().await), Vec::<u8>::new());
}

#[tokio::test]
async fn volume_is_immediate_when_not_streaming() {
    let mut h = Harness::start_levels(&[("A", 30, 30)]).await;
    let (mut a, _) = connect_levels(&mut h, "a", "A").await;
    h.control
        .send(Control::SetSpeech {
            short_id: short_id("A"),
            volume: 80,
        })
        .await
        .unwrap();
    assert_eq!(a.control_cmd().await, PlayerCmd::Volume(80));
}

#[tokio::test]
async fn old_value_report_while_pending_is_ignored() {
    let mut h = Harness::start_levels(&[("A", 30, 30)]).await;
    let (a, _next) = streaming_then_speech_80(&mut h).await;
    a.state("A", 30, false).await;
    let more = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            if let Some(Published::Player { .. }) = h.published.recv().await {
                return;
            }
        }
    })
    .await;
    assert!(more.is_err(), "nothing published");
    let saved = StateStore::load(h._dir.path().join("players.json")).unwrap();
    let prefs = saved.get("A").unwrap();
    assert_eq!((prefs.volume, prefs.speech_volume), (30, 80));
}

#[tokio::test]
async fn pending_volume_is_sent_at_once_when_the_stream_ends() {
    let mut h = Harness::start_levels(&[("A", 30, 30)]).await;
    let (mut a, _next) = streaming_then_speech_80(&mut h).await;
    h.input.send(InputEvent::Stopped).await.unwrap();
    let got = a.drain().await;
    assert!(got.contains(&PlayerCmd::StreamEnd));
    assert_eq!(volumes_of(&got), vec![80]);
}

#[tokio::test]
async fn echo_of_max_is_not_a_change() {
    let mut h = Harness::start_levels(&[("A", 30, 80)]).await;
    let (p, _) = connect_levels(&mut h, "a", "A").await;
    p.state("A", 80, false).await;
    let more = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            if let Some(Published::Player { .. }) = h.published.recv().await {
                return;
            }
        }
    })
    .await;
    assert!(more.is_err(), "nothing published");
    let saved = StateStore::load(h._dir.path().join("players.json")).unwrap();
    let prefs = saved.get("A").unwrap();
    assert_eq!((prefs.volume, prefs.speech_volume), (30, 80));
}

#[tokio::test]
async fn device_change_keeps_the_gap() {
    // (music, speech, reported device volume) -> (music, speech)
    let cases = [
        ((30, 80, 70), (20, 70)),
        ((50, 50, 60), (60, 60)),
        ((10, 40, 100), (70, 100)),
        ((10, 40, 5), (0, 5)),
    ];
    for ((m, sp, report), (want_m, want_sp)) in cases {
        let mut h = Harness::start_levels(&[("A", m, sp)]).await;
        let (p, sent) = connect_levels(&mut h, "a", "A").await;
        p.state("A", sent, false).await;
        p.state("A", report, false).await;
        loop {
            let (prefs, _) = h.published_for(&short_id("A")).await;
            if prefs.volume != m || prefs.speech_volume != sp {
                assert_eq!(
                    (prefs.volume, prefs.speech_volume),
                    (want_m, want_sp),
                    "from {m}/{sp} reported {report}"
                );
                break;
            }
        }
    }
}

#[tokio::test]
async fn music_is_scaled_for_a_speech_louder_player() {
    let mut h = Harness::start_levels(&[("A", 30, 80)]).await;
    let (mut a, _) = connect_levels(&mut h, "a", "A").await;
    a.drain().await;
    h.play().await;
    assert!(matches!(a.control_cmd().await, PlayerCmd::StreamStart(_)));
    h.clock.advance(1_000_000);
    let got = audio_of(&a.drain().await);
    let want = (257.0 * crate::level::split(30, 80).music_gain).round() as i16;
    assert_ne!(want, 257);
    assert!(!got.is_empty());
    let mut scaled = 0;
    for (_, d) in &got {
        for b in d.as_chunks::<2>().0 {
            let v = i16::from_le_bytes(*b);
            assert!(v == 0 || v == want, "{v} vs {want}");
            scaled += usize::from(v == want);
        }
    }
    assert!(scaled > 0);
}

#[tokio::test]
async fn speech_only_targets_get_their_own_gain() {
    let mut h = Harness::start_levels(&[("A", 50, 50), ("B", 90, 40), ("C", 50, 50)]).await;
    let (mut a, _) = connect_levels(&mut h, "a", "A").await;
    let (mut b, _) = connect_levels(&mut h, "b", "B").await;
    let (mut c, _) = connect_levels(&mut h, "c", "C").await;
    h.say(speech_for(&["A", "B", "C"], 8_000, CHUNK_FRAMES))
        .await;
    h.clock.advance(1_000_000);
    let got_a = audio_of(&a.drain().await);
    let got_b = audio_of(&b.drain().await);
    let got_c = audio_of(&c.drain().await);
    assert!(got_a.iter().any(|(_, d)| has_sample(d, 8_000)));
    let quiet = (8_000.0 * crate::level::split(90, 40).speech_gain).round() as i16;
    assert!(got_b.iter().any(|(_, d)| has_sample(d, quiet)));
    assert!(!got_b.iter().any(|(_, d)| has_sample(d, 8_000)));
    assert_eq!(got_a, got_c, "equal pairs get identical bytes");
}

#[tokio::test]
async fn a_stuck_player_does_not_hold_up_the_others() {
    let mut h = Harness::start_with(&[("A", 30, false, true), ("B", 30, false, true)]).await;
    let (mut a, _) = h.ready_player("a", "A").await;
    // B's connection stopped taking commands: its small queue fills up.
    let mut b = h.accepted_with("b", 32).await;
    b.connected("B").await;
    b.state("B", 30, false).await;
    h.play().await;
    assert!(matches!(a.control_cmd().await, PlayerCmd::StreamStart(_)));
    h.input
        .send(InputEvent::Pcm(bytes::Bytes::from(vec![
            1u8;
            200 * CHUNK_BYTES
        ])))
        .await
        .unwrap();
    for _ in 0..100 {
        h.clock.advance(CHUNK_US);
        a.audio().await;
    }

    // Stopping and restarting sends B control commands while its queue is full.
    h.input.send(InputEvent::Stopped).await.unwrap();
    assert!(matches!(a.control_cmd().await, PlayerCmd::StreamEnd));
    h.play().await;
    assert!(matches!(a.control_cmd().await, PlayerCmd::StreamStart(_)));

    // Room was kept for B's control commands: it ends and restarts its stream.
    let got = b.drain().await;
    let controls: Vec<_> = got
        .iter()
        .filter(|c| !matches!(c, PlayerCmd::Audio { .. }))
        .collect();
    assert!(
        matches!(
            controls[controls.len() - 2..],
            [PlayerCmd::StreamEnd, PlayerCmd::StreamStart(_)]
        ),
        "{controls:?}"
    );
}
