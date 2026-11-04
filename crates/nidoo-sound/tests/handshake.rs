//! End to end: `run_player` against a fake Sendspin player on localhost that
//! does the real handshake (KKpsk2 responder, Sentinel PSK) like sendspin-cpp,
//! and the same fake player connecting to `accept_players` (client-initiated,
//! like sendspin-js).

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures_util::{SinkExt, StreamExt};
use nidoo_sound::clock::{Clock, FakeClock};
use nidoo_sound::group::FromPlayer;
use nidoo_sound::listen::accept_players;
use nidoo_sound::player::{PlayerCmd, PlayerEvent, run_player};
use nidoo_sound::proto::messages::PCM_48K_STEREO_16;
use nidoo_sound::proto::noise::{SENTINEL_PSK, ServerIdentity, psk_id};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

const WAIT: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_millis(150);

/// What the fake player received, decrypted.
#[derive(Debug)]
enum Got {
    Json(Value),
    Binary(u8, Vec<u8>),
}

struct FakePlayer {
    ws: WebSocketStream<TcpStream>,
    noise: snow::TransportState,
    client_id: String,
    origin_seen: bool,
}

impl FakePlayer {
    async fn send_json(&mut self, v: Value) {
        let mut plain = vec![0u8];
        plain.extend(serde_json::to_vec(&v).unwrap());
        let mut out = vec![0u8; plain.len() + 16];
        let n = self.noise.write_message(&plain, &mut out).unwrap();
        out.truncate(n);
        self.ws.send(Message::Binary(out.into())).await.unwrap();
    }

    async fn recv(&mut self) -> Got {
        loop {
            let msg = tokio::time::timeout(WAIT, self.ws.next())
                .await
                .expect("timed out waiting for a server message")
                .expect("connection ended")
                .unwrap();
            match msg {
                Message::Binary(b) => {
                    let mut plain = vec![0u8; b.len()];
                    let n = self.noise.read_message(&b, &mut plain).unwrap();
                    plain.truncate(n);
                    return if plain[0] == 0 {
                        Got::Json(serde_json::from_slice(&plain[1..]).unwrap())
                    } else {
                        Got::Binary(plain[0], plain[1..].to_vec())
                    };
                }
                Message::Ping(_) | Message::Pong(_) => continue,
                other => panic!("unexpected frame in transport mode: {other:?}"),
            }
        }
    }

    async fn recv_json(&mut self) -> Value {
        match self.recv().await {
            Got::Json(v) => v,
            other => panic!("expected JSON, got {other:?}"),
        }
    }

    /// Asserts nothing arrives for a short while.
    async fn expect_quiet(&mut self) {
        if let Ok(Some(Ok(m))) = tokio::time::timeout(QUIET, self.ws.next()).await {
            let mut plain = vec![0u8; m.len()];
            let n = self.noise.read_message(&m.into_data(), &mut plain).unwrap();
            panic!("expected silence, got {:?}", &plain[..n.min(80)]);
        }
    }

    fn hello(buffer_capacity: u32) -> Value {
        json!({"type":"client/hello","payload":{
            "name":"Test satellite",
            "device_info":{"product_name":"XIAO","manufacturer":"Seeed"},
            "supported_roles":["player@v1"],
            "player@v1_support":{
                "supported_formats":[{"codec":"pcm","channels":2,"sample_rate":48000,"bit_depth":16}],
                "buffer_capacity":buffer_capacity
            },
            "supported_pair_methods":{"pairing_psk":{}},
            "unpaired_access":{"enabled":true}
        }})
    }

    fn state(commands: &[&str]) -> Value {
        json!({"type":"client/state","payload":{"available":true,"player":{
            "volume":30,"muted":false,"output_delay_ms":0,"required_lead_time_ms":150,
            "min_buffer_ms":500,"supported_commands":commands
        }}})
    }
}

/// Accepts one connection and runs the player side of the handshake up to
/// receiving `server/hello`.
// The header callback's error type is fixed by tungstenite.
#[allow(clippy::result_large_err)]
async fn accept_player(listener: TcpListener) -> FakePlayer {
    let (tcp, _) = listener.accept().await.unwrap();
    let mut origin_seen = false;
    let mut path_ok = false;
    let ws = tokio_tungstenite::accept_hdr_async(tcp, |req: &Request, resp: Response| {
        origin_seen = req.headers().contains_key("origin");
        path_ok = req.uri().path() == "/sendspin";
        Ok(resp)
    })
    .await
    .unwrap();
    assert!(path_ok, "server must connect to the advertised path");
    player_handshake(ws, origin_seen).await
}

/// The player side of the handshake on an open WebSocket, whichever side
/// opened it, up to receiving `server/hello`.
async fn player_handshake(mut ws: WebSocketStream<TcpStream>, origin_seen: bool) -> FakePlayer {
    let params: snow::params::NoiseParams = "Noise_KKpsk2_25519_ChaChaPoly_SHA256".parse().unwrap();
    let keys = snow::Builder::new(params.clone())
        .generate_keypair()
        .unwrap();
    let client_id = URL_SAFE_NO_PAD.encode(&keys.public);

    let init = json!({"type":"client/init","payload":{"client_id":client_id,"version":1,"suite":"25519_ChaChaPoly_SHA256"}})
        .to_string();
    ws.send(Message::Text(init.clone().into())).await.unwrap();

    let server_init = match ws.next().await.unwrap().unwrap() {
        Message::Text(t) => t.to_string(),
        other => panic!("server/init must be a text frame, got {other:?}"),
    };
    let si: Value = serde_json::from_str(&server_init).unwrap();
    assert_eq!(si["type"], "server/init");
    assert_eq!(si["payload"]["version"], 1);
    let server_pub = URL_SAFE_NO_PAD
        .decode(si["payload"]["server_id"].as_str().unwrap())
        .unwrap();

    let msg1 = match ws.next().await.unwrap().unwrap() {
        Message::Text(t) => serde_json::from_str::<Value>(&t).unwrap(),
        other => panic!("noise message 1 must be a text frame, got {other:?}"),
    };
    assert_eq!(msg1["type"], "noise/handshake");
    let msg1 = URL_SAFE_NO_PAD
        .decode(msg1["payload"]["data"].as_str().unwrap())
        .unwrap();

    let prologue = [init.as_bytes(), server_init.as_bytes()].concat();
    let mut hs = snow::Builder::new(params)
        .local_private_key(&keys.private)
        .unwrap()
        .remote_public_key(&server_pub)
        .unwrap()
        .psk(2, &SENTINEL_PSK)
        .unwrap()
        .prologue(&prologue)
        .unwrap()
        .build_responder()
        .unwrap();
    let mut buf = vec![0u8; 1024];
    let n = hs.read_message(&msg1, &mut buf).unwrap();
    let p1: Value = serde_json::from_slice(&buf[..n]).unwrap();
    assert_eq!(p1["psk_id"], psk_id(&SENTINEL_PSK));
    assert_eq!(p1["psk_category"], "sn");
    let n = hs.write_message(b"{}", &mut buf).unwrap();
    let msg2 =
        json!({"type":"noise/handshake","payload":{"data":URL_SAFE_NO_PAD.encode(&buf[..n])}});
    ws.send(Message::Text(msg2.to_string().into()))
        .await
        .unwrap();

    let mut p = FakePlayer {
        ws,
        noise: hs.into_transport_mode().unwrap(),
        client_id,
        origin_seen,
    };
    let hello = p.recv_json().await;
    assert_eq!(hello["type"], "server/hello");
    assert!(hello["payload"]["name"].is_string());
    p
}

struct Harness {
    events: mpsc::Receiver<PlayerEvent>,
    cmds: mpsc::Sender<PlayerCmd>,
    clock: Arc<FakeClock>,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn event(&mut self) -> PlayerEvent {
        tokio::time::timeout(WAIT, self.events.recv())
            .await
            .expect("timed out waiting for a player event")
            .expect("event channel closed")
    }
}

/// Starts `run_player` against a fresh fake player; returns once the fake
/// has received `server/hello`.
async fn start() -> (Harness, FakePlayer) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url =
        url::Url::parse(&format!("ws://{}/sendspin", listener.local_addr().unwrap())).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let identity =
        Arc::new(ServerIdentity::load_or_create(&dir.path().join("identity.key")).unwrap());
    let clock = Arc::new(FakeClock::default());
    let (ev_tx, events) = mpsc::channel(64);
    let (cmds, cmd_rx) = mpsc::channel(64);
    let task = tokio::spawn(run_player(
        url,
        identity,
        clock.clone() as Arc<dyn Clock>,
        ev_tx,
        cmd_rx,
    ));
    let fake = accept_player(listener).await;
    (
        Harness {
            events,
            cmds,
            clock,
            task,
            _dir: dir,
        },
        fake,
    )
}

/// Hello, `Connected`, activation for playback and the first `client/state`.
async fn start_ready(buffer_capacity: u32, commands: &[&str]) -> (Harness, FakePlayer) {
    let (mut h, mut p) = start().await;
    p.send_json(FakePlayer::hello(buffer_capacity)).await;
    assert!(matches!(h.event().await, PlayerEvent::Connected(_)));
    h.cmds
        .send(PlayerCmd::Activate { playback: true })
        .await
        .unwrap();
    assert_eq!(p.recv_json().await["type"], "server/activate");
    p.send_json(FakePlayer::state(commands)).await;
    assert!(matches!(h.event().await, PlayerEvent::State { .. }));
    (h, p)
}

#[tokio::test]
async fn handshake_then_hello_then_activate() {
    let (mut h, mut p) = start().await;
    p.send_json(FakePlayer::hello(400_000)).await;
    let PlayerEvent::Connected(info) = h.event().await else {
        panic!("expected Connected")
    };
    assert_eq!(info.client_id, p.client_id);
    assert_eq!(info.name, "Test satellite");
    assert_eq!(info.buffer_capacity, 400_000);
    assert!(info.supported_formats.contains(&PCM_48K_STEREO_16));

    h.cmds
        .send(PlayerCmd::Activate { playback: true })
        .await
        .unwrap();
    let act = p.recv_json().await;
    assert_eq!(act["type"], "server/activate");
    assert!(
        act["payload"]["activities"]
            .as_array()
            .unwrap()
            .contains(&json!("playback"))
    );
    assert_eq!(act["payload"]["active_roles"], json!(["player@v1"]));

    h.cmds
        .send(PlayerCmd::Activate { playback: false })
        .await
        .unwrap();
    let act = p.recv_json().await;
    assert_eq!(act["payload"]["activities"], json!([]));
}

#[tokio::test]
async fn time_requests_are_answered() {
    let (mut h, mut p) = start().await;
    p.send_json(FakePlayer::hello(400_000)).await;
    h.event().await;
    h.clock.set(5_000);
    p.send_json(json!({"type":"client/time","payload":{"client_transmitted":42}}))
        .await;
    let t = p.recv_json().await;
    assert_eq!(t["type"], "server/time");
    assert_eq!(t["payload"]["client_transmitted"], 42);
    let rx = t["payload"]["server_received"].as_i64().unwrap();
    let tx = t["payload"]["server_transmitted"].as_i64().unwrap();
    assert_eq!(rx, 5_000);
    assert!(rx <= tx);
}

#[tokio::test]
async fn commands_wait_for_the_first_client_state() {
    let (mut h, mut p) = start().await;
    p.send_json(FakePlayer::hello(400_000)).await;
    h.event().await;
    h.cmds
        .send(PlayerCmd::Activate { playback: true })
        .await
        .unwrap();
    p.recv_json().await;
    h.cmds.send(PlayerCmd::Volume(70)).await.unwrap();
    h.cmds.send(PlayerCmd::Mute(true)).await.unwrap();
    p.expect_quiet().await;

    p.send_json(FakePlayer::state(&["volume", "mute"])).await;
    let a = p.recv_json().await;
    let b = p.recv_json().await;
    assert_eq!(
        a["payload"]["player"],
        json!({"command":"volume","volume":70})
    );
    assert_eq!(
        b["payload"]["player"],
        json!({"command":"mute","mute":true})
    );
}

#[tokio::test]
async fn volume_command_only_when_supported() {
    let (h, mut p) = start_ready(400_000, &["mute"]).await;
    h.cmds.send(PlayerCmd::Volume(10)).await.unwrap();
    h.cmds.send(PlayerCmd::Mute(true)).await.unwrap();
    // The mute command arrives first: the volume command was never sent.
    let m = p.recv_json().await;
    assert_eq!(
        m["payload"]["player"],
        json!({"command":"mute","mute":true})
    );

    p.send_json(FakePlayer::state(&["volume"])).await;
    h.cmds.send(PlayerCmd::Volume(10)).await.unwrap();
    let v = p.recv_json().await;
    assert_eq!(v["type"], "server/command");
    assert_eq!(
        v["payload"]["player"],
        json!({"command":"volume","volume":10})
    );
}

#[tokio::test]
async fn client_state_is_forwarded() {
    let (mut h, mut p) = start_ready(400_000, &["volume", "mute"]).await;
    p.send_json(
        json!({"type":"client/state","payload":{"available":true,"player":{
        "volume":12,"muted":true,"output_delay_ms":40,"required_lead_time_ms":150,
        "min_buffer_ms":300,"supported_commands":["volume","mute"]}}}),
    )
    .await;
    let PlayerEvent::State {
        client_id,
        volume,
        muted,
        min_buffer_ms,
        output_delay_ms,
        available,
        ..
    } = h.event().await
    else {
        panic!("expected State")
    };
    assert_eq!(client_id, p.client_id);
    assert_eq!(
        (volume, muted, min_buffer_ms, output_delay_ms, available),
        (Some(12), Some(true), Some(300), Some(40), true)
    );
}

#[tokio::test]
async fn buffer_capacity_is_respected() {
    let chunk = 13 + 3840;
    let (h, mut p) = start_ready(3 * chunk, &["volume", "mute"]).await;
    h.cmds
        .send(PlayerCmd::StreamStart(PCM_48K_STEREO_16))
        .await
        .unwrap();
    let s = p.recv_json().await;
    assert_eq!(s["type"], "stream/start");
    assert_eq!(s["payload"]["player"]["codec"], "pcm");

    for i in 0..5 {
        let ts = 1_000_000 + i * 20_000;
        h.cmds
            .send(PlayerCmd::Audio {
                timestamp_us: ts,
                data: bytes::Bytes::from(vec![i as u8; 3840]),
            })
            .await
            .unwrap();
    }
    for i in 0..3i64 {
        let Got::Binary(4, b) = p.recv().await else {
            panic!("expected an audio chunk")
        };
        assert_eq!(
            i64::from_be_bytes(b[0..8].try_into().unwrap()),
            1_000_000 + i * 20_000
        );
        // send_ahead = timestamp - now, now = 0.
        assert_eq!(
            u32::from_be_bytes(b[8..12].try_into().unwrap()) as i64,
            1_000_000 + i * 20_000
        );
        assert_eq!(b.len() - 12, 3840);
    }
    p.expect_quiet().await;

    // The first chunk has finished playing at 1_020_000: room for one more.
    h.clock.set(1_020_000);
    let Got::Binary(4, b) = p.recv().await else {
        panic!("expected an audio chunk")
    };
    assert_eq!(i64::from_be_bytes(b[0..8].try_into().unwrap()), 1_060_000);
    p.expect_quiet().await;
}

#[tokio::test]
async fn audio_waits_for_stream_start_and_stream_end_clears() {
    let (h, mut p) = start_ready(400_000, &[]).await;
    // Audio outside a stream is never sent.
    h.cmds
        .send(PlayerCmd::Audio {
            timestamp_us: 500_000,
            data: bytes::Bytes::from(vec![0; 3840]),
        })
        .await
        .unwrap();
    p.expect_quiet().await;
    h.cmds
        .send(PlayerCmd::StreamStart(PCM_48K_STEREO_16))
        .await
        .unwrap();
    assert_eq!(p.recv_json().await["type"], "stream/start");
    h.cmds.send(PlayerCmd::StreamEnd).await.unwrap();
    assert_eq!(p.recv_json().await["type"], "stream/end");
    h.cmds.send(PlayerCmd::StreamEnd).await.unwrap();
    p.expect_quiet().await;
}

#[tokio::test]
async fn origin_header_absent_and_goodbye_ends_with_gone() {
    let (mut h, mut p) = start().await;
    assert!(!p.origin_seen, "the server must not send an Origin header");
    p.send_json(FakePlayer::hello(400_000)).await;
    h.event().await;
    p.send_json(json!({"type":"client/goodbye","payload":{"reason":"restart"}}))
        .await;
    let PlayerEvent::Gone { client_id } = h.event().await else {
        panic!("expected Gone")
    };
    assert_eq!(client_id, p.client_id);
    tokio::time::timeout(WAIT, h.task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn close_command_ends_the_connection() {
    let (mut h, mut p) = start().await;
    p.send_json(FakePlayer::hello(400_000)).await;
    h.event().await;
    h.cmds.send(PlayerCmd::Close).await.unwrap();
    assert!(matches!(h.event().await, PlayerEvent::Gone { .. }));
    // The fake sees the socket close.
    let end = tokio::time::timeout(WAIT, p.ws.next()).await.unwrap();
    assert!(matches!(
        end,
        None | Some(Ok(Message::Close(_))) | Some(Err(_))
    ));
}

/// `accept_players` on a fresh port; the test owns the group's end.
struct Listening {
    addr: std::net::SocketAddr,
    to_group: mpsc::Receiver<FromPlayer>,
    _dir: tempfile::TempDir,
}

async fn listen() -> Listening {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let identity =
        Arc::new(ServerIdentity::load_or_create(&dir.path().join("identity.key")).unwrap());
    let clock: Arc<dyn Clock> = Arc::new(FakeClock::default());
    let (tx, to_group) = mpsc::channel(64);
    tokio::spawn(accept_players(listener, identity, clock, tx));
    Listening {
        addr,
        to_group,
        _dir: dir,
    }
}

impl Listening {
    async fn open(&self, path: &str) -> anyhow::Result<WebSocketStream<TcpStream>> {
        let tcp = TcpStream::connect(self.addr).await?;
        let url = format!("ws://{}{path}", self.addr);
        let (ws, _) = tokio_tungstenite::client_async(url, tcp).await?;
        Ok(ws)
    }

    async fn next(&mut self) -> FromPlayer {
        tokio::time::timeout(WAIT, self.to_group.recv())
            .await
            .expect("timed out waiting for the group message")
            .expect("listener ended")
    }

    async fn expect_quiet(&mut self) {
        if let Ok(m) = tokio::time::timeout(QUIET, self.to_group.recv()).await {
            panic!("expected nothing for the group, got {m:?}");
        }
    }
}

#[tokio::test]
async fn client_initiated_player_joins_after_its_hello() {
    let mut l = listen().await;
    let ws = l.open("/sendspin").await.unwrap();
    let mut p = player_handshake(ws, false).await;
    // Nothing reaches the group before client/hello.
    l.expect_quiet().await;
    p.send_json(FakePlayer::hello(400_000)).await;
    let FromPlayer::Accepted { instance, cmds } = l.next().await else {
        panic!("expected Accepted first")
    };
    match l.next().await {
        FromPlayer::Event {
            instance: i,
            event: PlayerEvent::Connected(info),
        } => {
            assert_eq!(i, instance);
            assert_eq!(info.client_id, p.client_id);
        }
        other => panic!("expected Connected, got {other:?}"),
    }
    cmds.send(PlayerCmd::Activate { playback: true })
        .await
        .unwrap();
    assert_eq!(p.recv_json().await["type"], "server/activate");

    p.send_json(json!({"type":"client/goodbye","payload":{"reason":"user_request"}}))
        .await;
    assert!(matches!(
        l.next().await,
        FromPlayer::Event {
            event: PlayerEvent::Gone { .. },
            ..
        }
    ));
}

#[tokio::test]
async fn two_client_initiated_players_get_distinct_instances() {
    let mut l = listen().await;
    let mut a = player_handshake(l.open("/sendspin").await.unwrap(), false).await;
    let mut b = player_handshake(l.open("/sendspin").await.unwrap(), false).await;
    a.send_json(FakePlayer::hello(400_000)).await;
    // Kept: dropping a player's command sender ends its connection.
    let FromPlayer::Accepted {
        instance: ia,
        cmds: _a_cmds,
    } = l.next().await
    else {
        panic!("expected Accepted")
    };
    l.next().await;
    b.send_json(FakePlayer::hello(400_000)).await;
    let FromPlayer::Accepted { instance: ib, .. } = l.next().await else {
        panic!("expected Accepted")
    };
    assert_ne!(ia, ib);
}

#[tokio::test]
async fn other_paths_are_refused() {
    let l = listen().await;
    assert!(l.open("/other").await.is_err());
}

#[tokio::test]
async fn failed_handshake_never_reaches_the_group() {
    let mut l = listen().await;
    let mut ws = l.open("/sendspin").await.unwrap();
    ws.send(Message::Text("not json".into())).await.unwrap();
    // The server answers server/error and closes.
    while let Some(Ok(_)) = tokio::time::timeout(WAIT, ws.next()).await.unwrap() {}
    l.expect_quiet().await;
    // The listener keeps serving.
    let mut p = player_handshake(l.open("/sendspin").await.unwrap(), false).await;
    p.send_json(FakePlayer::hello(400_000)).await;
    assert!(matches!(l.next().await, FromPlayer::Accepted { .. }));
}
