//! One connection to one Sendspin player: we connect to the WebSocket it
//! advertises ([`run_player`]) or it connected to us ([`serve_player`]); either
//! way we run the handshake (cleartext init, Noise KKpsk2 with the Sentinel
//! PSK, the server being the Noise initiator), then answer its clock requests
//! and forward commands, the stream lifecycle and audio chunks from the group.
//!
//! Rules this module enforces for the group:
//! - nothing role-specific (commands, `stream/start`, audio) is sent before
//!   the player's first `client/state`; volume and mute wait, the latest wins;
//! - `stream/start` only while the player reports `available: true`;
//! - commands only when listed in the player's `supported_commands` (from
//!   `client/hello` or `client/state`);
//! - a `client/state` field that is absent keeps its last value (states after
//!   the first may carry only what changed);
//! - audio never exceeds the player's `buffer_capacity` (13-byte header +
//!   payload per chunk, released once the chunk has finished playing).

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::{Instant, timeout};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::clock::Clock;
use crate::proto::audio::{CHUNK_HEADER_LEN, MSG_JSON, Reassembler, encode_chunk};
use crate::proto::messages::{
    ACTIVITY_PLAYBACK, AudioFormat, ClientMsg, NoiseHandshake, PROTOCOL_VERSION, PlayerCommand,
    ROLE_PLAYER_V1, ServerActivate, ServerCommand, ServerError, ServerHello, ServerInit, ServerMsg,
    ServerTime, StreamEnd, StreamStart,
};
use crate::proto::noise::{
    NoiseServer, NoiseTransport, SENTINEL_CATEGORY, SENTINEL_PSK, ServerIdentity, Suite,
};

/// The name the server announces in `server/hello`.
pub const SERVER_NAME: &str = "Nidoo";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Per expected message during the handshake (the spec suggests 30 s).
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// How a connection is judged alive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Liveness {
    /// This long without any frame from the player ends the connection (a
    /// board that rebooted leaves a half-open TCP connection).
    pub silence: Duration,
    /// Send a WebSocket ping this often; the pong counts as a frame.
    pub ping_every: Option<Duration>,
}

impl Liveness {
    /// Satellites ask for the time every ~10 s; no pings, so nothing new
    /// reaches their firmware.
    pub const SATELLITE: Liveness = Liveness {
        silence: Duration::from_secs(35),
        ping_every: None,
    };
    /// Players that connect to us: a browser tab in the background may send
    /// nothing for a minute (throttled timers) but its network stack still
    /// answers pings.
    pub const INBOUND: Liveness = Liveness {
        silence: Duration::from_secs(35),
        ping_every: Some(Duration::from_secs(10)),
    };
}
/// How often queued audio is re-checked against the player's buffer.
const PUMP_INTERVAL: Duration = Duration::from_millis(5);

/// What a player told us about itself in `client/hello`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInfo {
    pub client_id: String,
    pub name: String,
    pub supported_formats: Vec<AudioFormat>,
    pub buffer_capacity: u32,
    pub supported_commands: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerEvent {
    /// Handshake and `client/hello` done.
    Connected(PlayerInfo),
    /// A `client/state` from the player.
    State {
        client_id: String,
        available: bool,
        volume: Option<u8>,
        muted: Option<bool>,
        supported_commands: Vec<String>,
        min_buffer_ms: Option<u32>,
        output_delay_ms: Option<u32>,
    },
    /// The connection ended (sent only after `Connected`).
    Gone { client_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerCmd {
    /// Sends `server/activate` with the player role; `playback` declares the
    /// `playback` activity.
    Activate {
        playback: bool,
    },
    Volume(u8),
    Mute(bool),
    StreamStart(AudioFormat),
    Audio {
        timestamp_us: i64,
        data: bytes::Bytes,
    },
    StreamEnd,
    Close,
}

/// An open WebSocket to a player, whichever side opened it.
pub type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Connects to an advertised player and runs the connection until it ends.
/// Sends [`PlayerEvent::Gone`] on every exit after [`PlayerEvent::Connected`].
pub async fn run_player(
    url: url::Url,
    identity: Arc<ServerIdentity>,
    clock: Arc<dyn Clock>,
    events: mpsc::Sender<PlayerEvent>,
    cmds: mpsc::Receiver<PlayerCmd>,
) -> anyhow::Result<()> {
    // No Origin header: tungstenite only sends what the URL implies.
    let (ws, _) = timeout(
        CONNECT_TIMEOUT,
        tokio_tungstenite::connect_async(url.as_str()),
    )
    .await
    .context("connect timed out")?
    .with_context(|| format!("connecting to {url}"))?;
    serve_player(ws, identity, clock, events, cmds, Liveness::SATELLITE).await
}

/// Runs the protocol on an open WebSocket until the connection ends (the
/// player may have opened it). Sends [`PlayerEvent::Gone`] on every exit after
/// [`PlayerEvent::Connected`].
pub async fn serve_player(
    ws: Ws,
    identity: Arc<ServerIdentity>,
    clock: Arc<dyn Clock>,
    events: mpsc::Sender<PlayerEvent>,
    mut cmds: mpsc::Receiver<PlayerCmd>,
    liveness: Liveness,
) -> anyhow::Result<()> {
    let mut connected_id = None;
    let result = session(
        ws,
        &identity,
        clock,
        &events,
        &mut cmds,
        &mut connected_id,
        liveness,
    )
    .await;
    if let Some(client_id) = connected_id {
        let _ = events.send(PlayerEvent::Gone { client_id }).await;
    }
    result
}

async fn session(
    mut ws: Ws,
    identity: &ServerIdentity,
    clock: Arc<dyn Clock>,
    events: &mpsc::Sender<PlayerEvent>,
    cmds: &mut mpsc::Receiver<PlayerCmd>,
    connected_id: &mut Option<String>,
    liveness: Liveness,
) -> anyhow::Result<()> {
    let (transport, client_id) = handshake(&mut ws, identity).await?;
    let mut conn = Conn {
        ws,
        noise: transport,
        reassembler: Reassembler::default(),
        clock,
        last_rx: Instant::now(),
    };
    conn.send(&ServerMsg::Hello(ServerHello {
        name: SERVER_NAME.into(),
    }))
    .await?;

    let info = loop {
        let (msg, _) = timeout(HANDSHAKE_TIMEOUT, conn.recv())
            .await
            .context("no client/hello")??
            .context("closed before client/hello")?;
        match msg {
            ClientMsg::Hello(h) => {
                let support = h
                    .player_support()
                    .context("the client does not offer the player@v1 role")?;
                break PlayerInfo {
                    client_id: client_id.clone(),
                    name: h.name.clone(),
                    supported_formats: support.supported_formats.clone(),
                    buffer_capacity: support.buffer_capacity,
                    supported_commands: support.supported_commands.clone(),
                };
            }
            other => tracing::debug!(?other, "ignored before client/hello"),
        }
    };
    let mut s = State::new(info.buffer_capacity as usize, &info.supported_commands);
    events.send(PlayerEvent::Connected(info)).await.ok();
    *connected_id = Some(client_id.clone());

    let ping_every = liveness.ping_every.unwrap_or(Duration::MAX);
    let mut next_ping = Instant::now() + ping_every.min(Duration::from_secs(86_400));
    loop {
        let pumping = !s.outbox.is_empty();
        let silent_until = conn.last_rx + liveness.silence;
        tokio::select! {
            msg = conn.recv() => {
                let Some((msg, arrived)) = msg? else { return Ok(()) };
                if !s.on_client_msg(&mut conn, events, &client_id, msg, arrived).await? {
                    return Ok(());
                }
            }
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { break };
                if !s.on_cmd(&mut conn, cmd).await? {
                    break;
                }
            }
            _ = tokio::time::sleep(PUMP_INTERVAL), if pumping => {}
            _ = tokio::time::sleep_until(next_ping), if liveness.ping_every.is_some() => {
                conn.ws.send(Message::Ping(Vec::new().into())).await?;
                next_ping = Instant::now() + ping_every;
            }
            // Pongs are read inside `recv`, so look again before giving up.
            _ = tokio::time::sleep_until(silent_until) => {
                if conn.last_rx + liveness.silence <= Instant::now() {
                    bail!("no message from the player for {:?}", liveness.silence);
                }
            }
        }
        s.pump(&mut conn).await?;
    }
    let _ = conn.ws.close(None).await;
    Ok(())
}

/// Cleartext init exchange then Noise KKpsk2 (server = initiator).
async fn handshake(
    ws: &mut Ws,
    identity: &ServerIdentity,
) -> anyhow::Result<(NoiseTransport, String)> {
    let init_text = recv_text(ws).await.context("waiting for client/init")?;
    let init = match serde_json::from_str::<ClientMsg>(&init_text) {
        Ok(ClientMsg::Init(init)) => init,
        _ => return init_failure(ws, "malformed").await,
    };
    if init.version != PROTOCOL_VERSION {
        return init_failure(ws, "unsupported_version").await;
    }
    let Some(suite) = Suite::from_wire(&init.suite) else {
        return init_failure(ws, "unsupported_suite").await;
    };
    if crate::proto::noise::decode_client_id(&init.client_id).is_err() {
        return init_failure(ws, "malformed").await;
    }

    let server_init = serde_json::to_string(&ServerMsg::Init(ServerInit {
        server_id: identity.server_id.clone(),
        version: PROTOCOL_VERSION,
    }))?;
    ws.send(Message::Text(server_init.clone().into())).await?;

    let prologue = [init_text.as_bytes(), server_init.as_bytes()].concat();
    let (noise, msg1) = NoiseServer::start(
        identity,
        &init.client_id,
        suite,
        &prologue,
        &SENTINEL_PSK,
        SENTINEL_CATEGORY,
    )?;
    let msg1 = serde_json::to_string(&ServerMsg::NoiseHandshake(NoiseHandshake {
        data: URL_SAFE_NO_PAD.encode(msg1),
    }))?;
    ws.send(Message::Text(msg1.into())).await?;

    let msg2 = recv_text(ws).await.context("waiting for noise message 2")?;
    let Ok(ClientMsg::NoiseHandshake(msg2)) = serde_json::from_str::<ClientMsg>(&msg2) else {
        bail!("expected noise/handshake, got {msg2:.80}");
    };
    let msg2 = URL_SAFE_NO_PAD
        .decode(&msg2.data)
        .context("noise message 2 is not base64url")?;
    let transport = noise.finish(&msg2).context("noise message 2 rejected")?;
    tracing::debug!(client_id = %init.client_id, "noise handshake done");
    Ok((transport, init.client_id))
}

async fn init_failure<T>(ws: &mut Ws, reason: &str) -> anyhow::Result<T> {
    let msg = serde_json::to_string(&ServerMsg::Error(ServerError {
        reason: reason.into(),
    }))?;
    let _ = ws.send(Message::Text(msg.into())).await;
    let _ = ws.close(None).await;
    bail!("client/init rejected: {reason}")
}

/// One cleartext text message (handshake phase).
async fn recv_text(ws: &mut Ws) -> anyhow::Result<String> {
    loop {
        let msg = timeout(HANDSHAKE_TIMEOUT, ws.next())
            .await
            .context("handshake timed out")?
            .context("connection closed during the handshake")??;
        match msg {
            Message::Text(t) => return Ok(t.to_string()),
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
            other => bail!("unexpected {other:?} during the handshake"),
        }
    }
}

/// The encrypted channel.
struct Conn {
    ws: Ws,
    noise: NoiseTransport,
    reassembler: Reassembler,
    clock: Arc<dyn Clock>,
    /// When the player last sent any frame (a pong included).
    last_rx: Instant,
}

impl Conn {
    /// Encrypts and sends one JSON message (binary message type 0).
    async fn send(&mut self, msg: &ServerMsg) -> anyhow::Result<()> {
        let mut plain = vec![MSG_JSON];
        serde_json::to_writer(&mut plain, msg)?;
        self.send_plain(&plain).await
    }

    async fn send_plain(&mut self, plain: &[u8]) -> anyhow::Result<()> {
        let cipher = self.noise.encrypt(plain)?;
        self.ws.send(Message::Binary(cipher.into())).await?;
        Ok(())
    }

    /// Next JSON message from the player with its arrival time; `None` when
    /// the connection closed. Other binary message types are ignored.
    async fn recv(&mut self) -> anyhow::Result<Option<(ClientMsg, i64)>> {
        loop {
            let Some(msg) = self.ws.next().await else {
                return Ok(None);
            };
            let arrived = self.clock.now_us();
            self.last_rx = Instant::now();
            match msg? {
                Message::Binary(b) => {
                    let plain = self.noise.decrypt(&b).context("decrypting")?;
                    let Some((kind, body)) = self.reassembler.push(&plain)? else {
                        continue;
                    };
                    if kind != MSG_JSON {
                        continue;
                    }
                    match serde_json::from_slice::<ClientMsg>(&body) {
                        Ok(msg) => return Ok(Some((msg, arrived))),
                        Err(e) => tracing::debug!(error = %e, "ignoring an unreadable message"),
                    }
                }
                Message::Text(_) => bail!("cleartext message after the handshake"),
                Message::Close(_) => return Ok(None),
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    }
}

/// Per-connection protocol state after `client/hello`.
struct State {
    /// A `client/state` with a player object has arrived.
    ready: bool,
    available: bool,
    /// Listed in `client/hello`.
    hello_commands: Vec<String>,
    /// Listed in the latest `client/state` that had the field.
    state_commands: Vec<String>,
    /// Both lists.
    supported_commands: Vec<String>,
    min_buffer_ms: Option<u32>,
    output_delay_ms: Option<u32>,
    output_delay_us: i64,
    pending_volume: Option<u8>,
    pending_mute: Option<bool>,
    /// The group wants a stream in this format.
    wanted: Option<AudioFormat>,
    /// The stream the player was told about with `stream/start`.
    active: Option<AudioFormat>,
    /// Chunks waiting for room in the player's buffer: (timestamp, pcm).
    outbox: VecDeque<(i64, bytes::Bytes)>,
    /// Chunks counted against `buffer_capacity`: (completion time, bytes).
    in_flight: VecDeque<(i64, usize)>,
    in_flight_bytes: usize,
    capacity: usize,
}

impl State {
    fn new(capacity: usize, hello_commands: &[String]) -> Self {
        Self {
            ready: false,
            available: false,
            hello_commands: hello_commands.to_vec(),
            state_commands: Vec::new(),
            supported_commands: hello_commands.to_vec(),
            min_buffer_ms: None,
            output_delay_ms: None,
            output_delay_us: 0,
            pending_volume: None,
            pending_mute: None,
            wanted: None,
            active: None,
            outbox: VecDeque::new(),
            in_flight: VecDeque::new(),
            in_flight_bytes: 0,
            capacity,
        }
    }

    /// Handles one message from the player; `false` ends the connection.
    async fn on_client_msg(
        &mut self,
        conn: &mut Conn,
        events: &mpsc::Sender<PlayerEvent>,
        client_id: &str,
        msg: ClientMsg,
        arrived: i64,
    ) -> anyhow::Result<bool> {
        match msg {
            ClientMsg::Time(t) => {
                let reply = ServerMsg::Time(ServerTime {
                    client_transmitted: t.client_transmitted,
                    server_received: arrived,
                    server_transmitted: conn.clock.now_us(),
                });
                conn.send(&reply).await?;
            }
            ClientMsg::State(st) => {
                self.available = st.available;
                let (mut volume, mut muted) = (None, None);
                if let Some(p) = st.player {
                    self.ready = true;
                    if let Some(commands) = p.supported_commands {
                        self.state_commands = commands;
                        let mut all = self.hello_commands.clone();
                        all.extend(
                            self.state_commands
                                .iter()
                                .filter(|c| !self.hello_commands.contains(c))
                                .cloned(),
                        );
                        self.supported_commands = all;
                    }
                    self.min_buffer_ms = p.min_buffer_ms.or(self.min_buffer_ms);
                    self.output_delay_ms = p.output_delay_ms.or(self.output_delay_ms);
                    self.output_delay_us = i64::from(self.output_delay_ms.unwrap_or(0)) * 1_000;
                    (volume, muted) = (p.volume, p.muted);
                }
                events
                    .send(PlayerEvent::State {
                        client_id: client_id.to_owned(),
                        available: st.available,
                        volume,
                        muted,
                        supported_commands: self.supported_commands.clone(),
                        min_buffer_ms: self.min_buffer_ms,
                        output_delay_ms: self.output_delay_ms,
                    })
                    .await
                    .ok();
                if self.ready {
                    self.flush_commands(conn).await?;
                }
                self.sync_stream(conn).await?;
            }
            ClientMsg::Goodbye(g) => {
                tracing::info!(client_id, reason = %g.reason, "player said goodbye");
                return Ok(false);
            }
            other => tracing::debug!(?other, "ignored"),
        }
        Ok(true)
    }

    /// Handles one command from the group; `false` ends the connection.
    async fn on_cmd(&mut self, conn: &mut Conn, cmd: PlayerCmd) -> anyhow::Result<bool> {
        match cmd {
            PlayerCmd::Activate { playback } => {
                let activities = if playback {
                    vec![ACTIVITY_PLAYBACK.into()]
                } else {
                    Vec::new()
                };
                let msg = ServerMsg::Activate(ServerActivate {
                    activities,
                    active_roles: Some(vec![ROLE_PLAYER_V1.into()]),
                });
                conn.send(&msg).await?;
            }
            PlayerCmd::Volume(v) => {
                self.pending_volume = Some(v);
                if self.ready {
                    self.flush_commands(conn).await?;
                }
            }
            PlayerCmd::Mute(m) => {
                self.pending_mute = Some(m);
                if self.ready {
                    self.flush_commands(conn).await?;
                }
            }
            PlayerCmd::StreamStart(format) => {
                self.wanted = Some(format);
                self.sync_stream(conn).await?;
            }
            PlayerCmd::Audio { timestamp_us, data } => {
                if self.active.is_some() {
                    self.outbox.push_back((timestamp_us, data));
                }
            }
            PlayerCmd::StreamEnd => {
                self.wanted = None;
                self.sync_stream(conn).await?;
            }
            PlayerCmd::Close => return Ok(false),
        }
        Ok(true)
    }

    /// Sends pending volume/mute commands the player accepts. A command the
    /// player does not list stays pending (latest wins) until it does.
    async fn flush_commands(&mut self, conn: &mut Conn) -> anyhow::Result<()> {
        let supports = |c: &str| self.supported_commands.iter().any(|s| s == c);
        let volume = if supports("volume") {
            self.pending_volume.take()
        } else {
            None
        };
        let mute = if supports("mute") {
            self.pending_mute.take()
        } else {
            None
        };
        if let Some(v) = volume {
            conn.send(&ServerMsg::Command(ServerCommand {
                player: PlayerCommand::volume(v),
            }))
            .await?;
        }
        if let Some(m) = mute {
            conn.send(&ServerMsg::Command(ServerCommand {
                player: PlayerCommand::mute(m),
            }))
            .await?;
        }
        Ok(())
    }

    /// Starts or ends the player's stream so it matches what the group wants
    /// and what the player can take (ready and available).
    async fn sync_stream(&mut self, conn: &mut Conn) -> anyhow::Result<()> {
        let can_play = self.ready && self.available;
        let target = self.wanted.clone().filter(|_| can_play);
        if self.active == target {
            return Ok(());
        }
        if self.active.is_some() {
            conn.send(&ServerMsg::StreamEnd(StreamEnd::default()))
                .await?;
            self.active = None;
            self.outbox.clear();
            self.in_flight.clear();
            self.in_flight_bytes = 0;
        }
        if let Some(format) = target {
            let msg = ServerMsg::StreamStart(StreamStart {
                server_transmitted: conn.clock.now_us(),
                player: format.clone(),
            });
            conn.send(&msg).await?;
            self.active = Some(format);
        }
        Ok(())
    }

    /// Releases finished chunks and sends queued ones while they fit.
    async fn pump(&mut self, conn: &mut Conn) -> anyhow::Result<()> {
        let Some(format) = self.active.clone() else {
            return Ok(());
        };
        let now = conn.clock.now_us();
        while let Some(&(done, size)) = self.in_flight.front() {
            if done > now {
                break;
            }
            self.in_flight.pop_front();
            self.in_flight_bytes -= size;
        }
        // Chunks that would finish before now are useless to the player.
        while let Some((ts, data)) = self.outbox.front() {
            if ts + duration_us(&format, data.len()) - self.output_delay_us > now {
                break;
            }
            self.outbox.pop_front();
        }
        while let Some((ts, data)) = self.outbox.front() {
            let size = CHUNK_HEADER_LEN + data.len();
            if self.in_flight_bytes + size > self.capacity {
                break;
            }
            let (ts, data) = (*ts, data.clone());
            self.outbox.pop_front();
            let now = conn.clock.now_us();
            let send_ahead = (ts - now).clamp(0, i64::from(u32::MAX)) as u32;
            conn.send_plain(&encode_chunk(ts, send_ahead, &data))
                .await?;
            let done = ts + duration_us(&format, data.len()) - self.output_delay_us;
            self.in_flight.push_back((done, size));
            self.in_flight_bytes += size;
        }
        Ok(())
    }
}

/// Playing time of `bytes` of PCM in `format`, in microseconds.
fn duration_us(format: &AudioFormat, bytes: usize) -> i64 {
    let frame = usize::from(format.channels) * usize::from(format.bit_depth / 8);
    if frame == 0 || format.sample_rate == 0 {
        return 0;
    }
    (bytes / frame) as i64 * 1_000_000 / i64::from(format.sample_rate)
}
