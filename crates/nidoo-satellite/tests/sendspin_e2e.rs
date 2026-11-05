//! The real `nidoo-sound` server side against our player, over loopback.

use std::sync::Arc;
use std::time::Duration;

use nidoo_satellite::sendspin::{
    LocalPlayerState, PlayerConfig, SendspinEvent, clock::now_us, serve,
};
use nidoo_satellite::store::Store;
use nidoo_sound::clock::{Clock, MonotonicClock};
use nidoo_sound::player::{PlayerCmd, PlayerEvent, run_player};
use nidoo_sound::proto::messages::PCM_48K_STEREO_16;
use nidoo_sound::proto::noise::ServerIdentity;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

const WAIT: Duration = Duration::from_secs(5);

struct Mac {
    url: url::Url,
    events: mpsc::Receiver<SendspinEvent>,
    local: watch::Sender<LocalPlayerState>,
    _dir: tempfile::TempDir,
    _serve: JoinHandle<anyhow::Result<()>>,
}

async fn mac() -> Mac {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url =
        url::Url::parse(&format!("ws://{}/sendspin", listener.local_addr().unwrap())).unwrap();
    let (ev_tx, events) = mpsc::channel(64);
    let (local, local_rx) = watch::channel(LocalPlayerState {
        volume: 100,
        muted: false,
    });
    let cfg = PlayerConfig {
        name: "Test Mac".into(),
        buffer_capacity: 1_048_576,
    };
    let serve = tokio::spawn(serve(
        listener,
        Arc::new(store.identity().clone()),
        cfg,
        ev_tx,
        local_rx,
    ));
    Mac {
        url,
        events,
        local,
        _dir: dir,
        _serve: serve,
    }
}

struct Server {
    clock: Arc<MonotonicClock>,
    events: mpsc::Receiver<PlayerEvent>,
    cmds: mpsc::Sender<PlayerCmd>,
    task: JoinHandle<anyhow::Result<()>>,
    _dir: tempfile::TempDir,
}

fn server(url: &url::Url) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let identity =
        Arc::new(ServerIdentity::load_or_create(&dir.path().join("server.key")).unwrap());
    let clock = Arc::new(MonotonicClock::new());
    let (ev_tx, events) = mpsc::channel(64);
    let (cmds, cmd_rx) = mpsc::channel(64);
    let task = tokio::spawn(run_player(
        url.clone(),
        identity,
        clock.clone(),
        ev_tx,
        cmd_rx,
    ));
    Server {
        clock,
        events,
        cmds,
        task,
        _dir: dir,
    }
}

async fn next<T: std::fmt::Debug>(
    rx: &mut mpsc::Receiver<T>,
    what: &str,
    pick: impl Fn(&T) -> bool,
) -> T {
    tokio::time::timeout(WAIT, async {
        loop {
            let ev = rx
                .recv()
                .await
                .unwrap_or_else(|| panic!("channel closed waiting for {what}"));
            if pick(&ev) {
                return ev;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

async fn connected(mac: &mut Mac, srv: &mut Server) {
    let ev = next(&mut srv.events, "server Connected", |e| {
        matches!(e, PlayerEvent::Connected(_))
    })
    .await;
    let PlayerEvent::Connected(info) = ev else {
        unreachable!()
    };
    assert_eq!(info.name, "Test Mac");
    assert!(info.supported_formats.contains(&PCM_48K_STEREO_16));
    next(&mut srv.events, "server State", |e| {
        matches!(
            e,
            PlayerEvent::State {
                available: true,
                ..
            }
        )
    })
    .await;
    next(&mut mac.events, "our Connected", |e| {
        matches!(e, SendspinEvent::Connected)
    })
    .await;
}

#[tokio::test]
async fn nidoo_sound_connects_and_streams() {
    let mut mac = mac().await;
    let mut srv = server(&mac.url);
    connected(&mut mac, &mut srv).await;

    srv.cmds
        .send(PlayerCmd::Activate { playback: true })
        .await
        .unwrap();
    srv.cmds
        .send(PlayerCmd::StreamStart(PCM_48K_STEREO_16))
        .await
        .unwrap();
    let start = next(&mut mac.events, "StreamStart", |e| {
        matches!(e, SendspinEvent::StreamStart(_))
    })
    .await;
    assert!(matches!(start, SendspinEvent::StreamStart(f) if f == PCM_48K_STEREO_16));

    let pcm = bytes::Bytes::from((0..1920u32).map(|i| i as u8).collect::<Vec<u8>>());
    let expected = now_us() + 500_000;
    let ts = srv.clock.now_us() + 500_000;
    srv.cmds
        .send(PlayerCmd::Audio {
            timestamp_us: ts,
            data: pcm.clone(),
        })
        .await
        .unwrap();
    let chunk = next(&mut mac.events, "Chunk", |e| {
        matches!(e, SendspinEvent::Chunk { .. })
    })
    .await;
    let SendspinEvent::Chunk {
        local_play_at_us,
        pcm: got,
    } = chunk
    else {
        unreachable!()
    };
    assert_eq!(got, pcm);
    assert!(
        (local_play_at_us - expected).abs() < 50_000,
        "off by {} us",
        local_play_at_us - expected
    );

    srv.cmds.send(PlayerCmd::StreamEnd).await.unwrap();
    next(&mut mac.events, "StreamEnd", |e| {
        matches!(e, SendspinEvent::StreamEnd)
    })
    .await;
}

#[tokio::test]
async fn volume_and_mute_round_trip() {
    let mut mac = mac().await;
    let mut srv = server(&mac.url);
    connected(&mut mac, &mut srv).await;

    srv.cmds.send(PlayerCmd::Volume(40)).await.unwrap();
    next(&mut mac.events, "Volume(40)", |e| {
        matches!(e, SendspinEvent::Volume(40))
    })
    .await;
    srv.cmds.send(PlayerCmd::Mute(true)).await.unwrap();
    next(&mut mac.events, "Mute(true)", |e| {
        matches!(e, SendspinEvent::Mute(true))
    })
    .await;

    mac.local
        .send(LocalPlayerState {
            volume: 40,
            muted: true,
        })
        .unwrap();
    next(&mut srv.events, "server sees volume 40", |e| {
        matches!(
            e,
            PlayerEvent::State {
                volume: Some(40),
                muted: Some(true),
                ..
            }
        )
    })
    .await;
}

#[tokio::test]
async fn accepts_a_new_connection_after_the_first_drops() {
    let mut mac = mac().await;
    let mut first = server(&mac.url);
    connected(&mut mac, &mut first).await;
    first.task.abort();
    next(&mut mac.events, "Disconnected", |e| {
        matches!(e, SendspinEvent::Disconnected)
    })
    .await;

    let mut second = server(&mac.url);
    connected(&mut mac, &mut second).await;
}

#[tokio::test]
async fn a_newer_connection_replaces_the_live_one() {
    let mut mac = mac().await;
    let mut first = server(&mac.url);
    connected(&mut mac, &mut first).await;

    let mut second = server(&mac.url);
    next(&mut mac.events, "Disconnected", |e| {
        matches!(e, SendspinEvent::Disconnected)
    })
    .await;
    connected(&mut mac, &mut second).await;
    next(&mut first.events, "first Gone", |e| {
        matches!(e, PlayerEvent::Gone { .. })
    })
    .await;
}
