//! The whole runtime over loopback: a fake hub on a UDP socket speaking `nidoo_voice`,
//! the real `nidoo-sound` player code on the Sendspin port, fake mic and detector.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nidoo_satellite::brain::{IconState, Trainer, UiState, WakeDetector};
use nidoo_satellite::run::{Config, Control, Handles, MicSwitch, start_with_greeting_ttl};
use nidoo_satellite::store::Store;
use nidoo_satellite::voice::GREETING_TTL;
use nidoo_satellite::wake_head::TrainProgress;
use nidoo_sound::clock::MonotonicClock;
use nidoo_sound::player::{PlayerEvent, run_player};
use nidoo_sound::proto::noise::ServerIdentity;
use nidoo_voice::{BoardMsg, HubMsg, Key, WakeKind, decode_board, encode_hub};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(5);
const WAKE: i16 = 12345;
const KEY_HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn key() -> Key {
    Key::from_bytes(std::array::from_fn(|i| i as u8))
}

struct FakeDetector;

impl WakeDetector for FakeDetector {
    fn push(&mut self, samples_16k: &[i16]) -> bool {
        samples_16k.contains(&WAKE)
    }
    fn reset(&mut self) {}
}

struct NoTrainer;

impl Trainer for NoTrainer {
    fn start(&mut self) {}
    fn push(&mut self, _samples_16k: &[i16]) -> TrainProgress {
        TrainProgress::Training
    }
    fn cancel(&mut self) {}
}

#[derive(Clone, Default)]
struct RecordingMic(Arc<Mutex<Vec<bool>>>);

impl MicSwitch for RecordingMic {
    fn set(&mut self, on: bool) -> anyhow::Result<()> {
        self.0.lock().unwrap().push(on);
        Ok(())
    }
}

impl RecordingMic {
    fn last(&self) -> Option<bool> {
        self.0.lock().unwrap().last().copied()
    }
}

struct Rig {
    handles: Handles,
    mic: RecordingMic,
    mic_tx: mpsc::Sender<Vec<i16>>,
    hub: UdpSocket,
    sid: String,
    ctr: u64,
    _player: JoinHandle<anyhow::Result<()>>,
    _player_events: mpsc::Receiver<PlayerEvent>,
    _player_cmds: mpsc::Sender<nidoo_sound::player::PlayerCmd>,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

async fn rig(greeting_ttl: Duration) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let sid = Store::open(dir.path()).unwrap().identity().sid.clone();
    let mic = RecordingMic::default();
    let (mic_tx, mic_rx) = mpsc::channel(64);
    let cfg = Config {
        store_dir: dir.path().to_path_buf(),
        name: "Test Mac".into(),
        voice_port: 0,
        sendspin_port: 0,
        advertise: false,
    };
    let handles = start_with_greeting_ttl(
        cfg,
        greeting_ttl,
        Box::new(mic.clone()),
        mic_rx,
        Box::new(FakeDetector),
        Box::new(NoTrainer),
    )
    .await
    .unwrap();

    // The real nidoo-sound player code connects to our Sendspin port.
    let server_dir = tempfile::tempdir().unwrap();
    let identity =
        Arc::new(ServerIdentity::load_or_create(&server_dir.path().join("server.key")).unwrap());
    let url = url::Url::parse(&format!(
        "ws://127.0.0.1:{}/sendspin",
        handles.sendspin_port
    ))
    .unwrap();
    let (ev_tx, mut player_events) = mpsc::channel(64);
    let (player_cmds, cmd_rx) = mpsc::channel(64);
    let player = tokio::spawn(run_player(
        url,
        identity,
        Arc::new(MonotonicClock::new()),
        ev_tx,
        cmd_rx,
    ));
    timeout(WAIT, async {
        while let Some(ev) = player_events.recv().await {
            if matches!(ev, PlayerEvent::Connected(_)) {
                return;
            }
        }
        panic!("player events closed");
    })
    .await
    .expect("nidoo-sound connects");

    let hub = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut rig = Rig {
        handles,
        mic,
        mic_tx,
        hub,
        sid,
        ctr: 0,
        _player: player,
        _player_events: player_events,
        _player_cmds: player_cmds,
        _dirs: (dir, server_dir),
    };
    rig.control(Control::SetVoiceKey(format!("voice_key: \"{KEY_HEX}\"")))
        .await;
    rig.ui_until("waiting for the key to be taken", |ui| {
        ui.status.contains("Allow this Mac")
    })
    .await;
    rig
}

impl Rig {
    async fn control(&self, c: Control) {
        self.handles.control.send(c).await.unwrap();
    }

    async fn send_hub(&mut self, msg: HubMsg) {
        let to = SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.handles.voice_port);
        self.hub
            .send_to(&encode_hub(&key(), &self.sid, &msg), to)
            .await
            .unwrap();
    }

    async fn hello(&mut self) {
        self.ctr += 1;
        let std::net::SocketAddr::V4(hub) = self.hub.local_addr().unwrap() else {
            unreachable!()
        };
        self.send_hub(HubMsg::Hello {
            nonce: 42,
            hub_ctr: self.ctr,
            hub,
        })
        .await;
    }

    async fn next_board(&self) -> BoardMsg {
        let mut buf = [0u8; 2048];
        let (n, _) = timeout(WAIT, self.hub.recv_from(&mut buf))
            .await
            .expect("a datagram from the satellite")
            .unwrap();
        let (sid, msg) = decode_board(&key(), &buf[..n]).unwrap();
        assert_eq!(sid, self.sid);
        msg
    }

    async fn nothing_next_board(&self) {
        let mut buf = [0u8; 2048];
        let got = timeout(Duration::from_millis(300), self.hub.recv_from(&mut buf)).await;
        assert!(got.is_err(), "unexpected datagram");
    }

    async fn ui_until(&mut self, what: &str, pick: impl Fn(&UiState) -> bool) -> UiState {
        let ui: &mut watch::Receiver<UiState> = &mut self.handles.ui;
        let got = timeout(WAIT, async {
            ui.wait_for(|u| pick(u)).await.map(|u| u.clone())
        })
        .await;
        match got {
            Ok(u) => u.unwrap(),
            Err(_) => panic!("timed out waiting for {what}: {:?}", ui.borrow()),
        }
    }
}

#[tokio::test]
async fn wake_streams_until_stop() {
    let mut r = rig(GREETING_TTL).await;
    r.hello().await;
    r.ui_until("Ready", |ui| ui.icon == IconState::Ready).await;
    assert_eq!(r.mic.last(), Some(true), "mic switched on");

    r.mic_tx.send(vec![0, WAKE, 0]).await.unwrap();
    assert_eq!(
        r.next_board().await,
        BoardMsg::Wake {
            nonce: 42,
            session: 1,
            kind: WakeKind::WakeWord,
            listen_ctr: 0
        }
    );
    r.ui_until("Listening", |ui| ui.icon == IconState::Listening)
        .await;
    r.mic_tx.send(vec![3; 320]).await.unwrap();
    let BoardMsg::Audio {
        session, samples, ..
    } = r.next_board().await
    else {
        panic!("expected Audio");
    };
    assert_eq!((session, samples.len()), (1, 320));

    r.ctr += 1;
    let hub_ctr = r.ctr;
    r.send_hub(HubMsg::Stop {
        hub_ctr,
        session: 1,
    })
    .await;
    r.ui_until("Ready again", |ui| ui.icon == IconState::Ready)
        .await;
    r.mic_tx.send(vec![3; 320]).await.unwrap();
    r.nothing_next_board().await;
}

#[tokio::test]
async fn follow_up_after_listen() {
    let mut r = rig(GREETING_TTL).await;
    r.hello().await;
    r.ui_until("Ready", |ui| ui.icon == IconState::Ready).await;
    r.ctr += 1;
    let hub_ctr = r.ctr;
    r.send_hub(HubMsg::Listen { hub_ctr }).await;
    assert_eq!(
        r.next_board().await,
        BoardMsg::Wake {
            nonce: 42,
            session: 1,
            kind: WakeKind::FollowUp,
            listen_ctr: hub_ctr
        }
    );
    r.mic_tx.send(vec![5; 100]).await.unwrap();
    assert!(matches!(
        r.next_board().await,
        BoardMsg::Audio { session: 1, .. }
    ));
}

#[tokio::test]
async fn gray_without_hello_for_15s() {
    let mut r = rig(Duration::from_millis(300)).await;
    r.hello().await;
    r.ui_until("Ready", |ui| ui.icon == IconState::Ready).await;
    r.ui_until("Gray after the greeting lifetime", |ui| {
        ui.icon == IconState::Gray
    })
    .await;
    assert_eq!(r.mic.last(), Some(false), "mic closed while gray");
    r.hello().await;
    r.ui_until("Ready again on the next Hello", |ui| {
        ui.icon == IconState::Ready
    })
    .await;
    assert_eq!(r.mic.last(), Some(true));
}

#[tokio::test]
async fn junk_voice_key_is_refused_and_old_key_kept() {
    let mut r = rig(GREETING_TTL).await;
    r.control(Control::SetVoiceKey("hello there".into())).await;
    r.ui_until("refusal", |ui| ui.status.contains("not a voice key"))
        .await;
    r.hello().await;
    r.ui_until("Ready under the old key", |ui| ui.icon == IconState::Ready)
        .await;
}

#[tokio::test]
async fn listening_choice_survives_a_restart() {
    let mut r = rig(GREETING_TTL).await;
    r.control(Control::SetListening(false)).await;
    r.ui_until("listening off", |ui| !ui.listening).await;
    let dir = r._dirs.0.path().to_path_buf();
    assert!(!Store::open(&dir).unwrap().settings().listening);
}

#[tokio::test]
async fn quit_leaves_the_house() {
    let mut r = rig(GREETING_TTL).await;
    r.hello().await;
    r.ui_until("Ready", |ui| ui.icon == IconState::Ready).await;
    r.mic_tx.send(vec![WAKE]).await.unwrap();
    assert!(matches!(r.next_board().await, BoardMsg::Wake { .. }));
    r.control(Control::Quit).await;
    assert_eq!(r.next_board().await, BoardMsg::End { session: 1 });
    timeout(WAIT, &mut r.handles.done)
        .await
        .expect("the runtime stops")
        .unwrap();
    assert_eq!(r.mic.last(), Some(false));
    let port = r.handles.sendspin_port;
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_err(),
        "the Sendspin port is closed"
    );
}
