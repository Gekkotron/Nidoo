//! The player's WebSocket server: one live `nidoo-sound` connection at a time.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures_util::{SinkExt, StreamExt};
use nidoo_sound::proto::audio::{CHUNK_HEADER_LEN, MSG_JSON, MSG_PLAYER_AUDIO, Reassembler};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;

use super::clock::{ClockSync, now_us};
use super::messages::{
    ServerMsg, client_hello, client_init, client_state, client_time, noise_handshake,
};
use super::noise::{NoiseClient, NoiseTransport};
use super::{LocalPlayerState, PATH, PlayerConfig, SendspinEvent};
use crate::store::PlayerIdentity;

type Ws = WebSocketStream<TcpStream>;

/// The WebSocket upgrade and each handshake message.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// This long without any frame from the server ends the connection (the server
/// answers our `client/time` at least every 10 s).
const SILENCE: Duration = Duration::from_secs(35);

/// Aborts the task when dropped, so nothing outlives `serve`.
struct Owned(JoinHandle<()>);

impl Drop for Owned {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Accepts WebSocket upgrades on `/sendspin` and plays the player role on the newest
/// one. A newer connection replaces the live one once its upgrade succeeds, so a stray
/// TCP probe cannot cut the music.
pub async fn serve(
    listener: TcpListener,
    identity: Arc<PlayerIdentity>,
    cfg: PlayerConfig,
    events: mpsc::Sender<SendspinEvent>,
    local: watch::Receiver<LocalPlayerState>,
) -> anyhow::Result<()> {
    let (up_tx, mut upgraded) = mpsc::channel::<Ws>(4);
    let mut upgrades: Vec<Owned> = Vec::new();
    let mut current: Option<(oneshot::Sender<()>, Owned)> = None;
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (tcp, peer) = accepted.context("accepting")?;
                let up_tx = up_tx.clone();
                upgrades.retain(|u| !u.0.is_finished());
                upgrades.push(Owned(tokio::spawn(async move {
                    match timeout(HANDSHAKE_TIMEOUT, tokio_tungstenite::accept_hdr_async(tcp, only_our_path)).await {
                        Ok(Ok(ws)) => {
                            let _ = up_tx.send(ws).await;
                        }
                        Ok(Err(e)) => tracing::debug!(%peer, error = %e, "not a Sendspin upgrade"),
                        Err(_) => tracing::debug!(%peer, "upgrade timed out"),
                    }
                })));
            }
            Some(ws) = upgraded.recv() => {
                if let Some((stop, mut task)) = current.take() {
                    let _ = stop.send(());
                    let _ = (&mut task.0).await;
                }
                let (stop_tx, stop_rx) = oneshot::channel();
                let task = tokio::spawn(connection(
                    ws,
                    identity.clone(),
                    cfg.clone(),
                    events.clone(),
                    local.clone(),
                    stop_rx,
                ));
                current = Some((stop_tx, Owned(task)));
            }
        }
    }
}

#[allow(clippy::result_large_err)] // the signature tungstenite asks for
fn only_our_path(req: &Request, resp: Response) -> Result<Response, ErrorResponse> {
    if req.uri().path() == PATH {
        Ok(resp)
    } else {
        let mut refused = ErrorResponse::new(None);
        *refused.status_mut() = StatusCode::NOT_FOUND;
        Err(refused)
    }
}

/// Runs one connection until it ends or a newer one takes over; sends `Disconnected`
/// after a `Connected`.
async fn connection(
    ws: Ws,
    identity: Arc<PlayerIdentity>,
    cfg: PlayerConfig,
    events: mpsc::Sender<SendspinEvent>,
    local: watch::Receiver<LocalPlayerState>,
    stop: oneshot::Receiver<()>,
) {
    let mut connected = false;
    let result = tokio::select! {
        r = session(ws, &identity, &cfg, &events, local, &mut connected) => r,
        _ = stop => Ok(()),
    };
    if let Err(e) = result {
        tracing::info!(error = %e, "Sendspin connection ended");
    }
    if connected {
        let _ = events.send(SendspinEvent::Disconnected).await;
    }
}

async fn session(
    mut ws: Ws,
    identity: &PlayerIdentity,
    cfg: &PlayerConfig,
    events: &mpsc::Sender<SendspinEvent>,
    mut local: watch::Receiver<LocalPlayerState>,
    connected: &mut bool,
) -> anyhow::Result<()> {
    let noise = handshake(&mut ws, identity).await?;
    let mut conn = Conn {
        ws,
        noise,
        reassembler: Reassembler::default(),
    };
    let mut clock = ClockSync::default();
    conn.send(&client_hello(
        &identity.client_id,
        &cfg.name,
        cfg.buffer_capacity,
    ))
    .await?;
    conn.send(&client_time(now_us())).await?;
    let state = *local.borrow_and_update();
    conn.send(&client_state(state.volume, state.muted)).await?;
    *connected = true;
    events.send(SendspinEvent::Connected).await?;

    let mut next_time = Instant::now() + clock.interval();
    let mut last_rx = Instant::now();
    loop {
        tokio::select! {
            got = conn.recv() => {
                let Some((incoming, arrived)) = got? else { return Ok(()) };
                last_rx = Instant::now();
                let event = match incoming {
                    Incoming::Json(msg) => on_server_msg(msg, arrived, &mut clock),
                    Incoming::Chunk { timestamp_us, pcm } => match clock.to_local(timestamp_us) {
                        Some(local_play_at_us) => Some(SendspinEvent::Chunk { local_play_at_us, pcm }),
                        None => {
                            tracing::debug!("audio before the first clock sample, dropped");
                            None
                        }
                    },
                    Incoming::Frame => None,
                };
                if let Some(event) = event {
                    events.send(event).await?;
                }
            }
            changed = local.changed() => {
                if changed.is_err() {
                    return Ok(());
                }
                let state = *local.borrow_and_update();
                conn.send(&client_state(state.volume, state.muted)).await?;
            }
            _ = tokio::time::sleep_until(next_time) => {
                conn.send(&client_time(now_us())).await?;
                next_time = Instant::now() + clock.interval();
            }
            _ = tokio::time::sleep_until(last_rx + SILENCE) => {
                bail!("no message from the server for {SILENCE:?}");
            }
        }
    }
}

fn on_server_msg(msg: ServerMsg, arrived: i64, clock: &mut ClockSync) -> Option<SendspinEvent> {
    match msg {
        ServerMsg::Time {
            client_transmitted,
            server_received,
            server_transmitted,
        } => {
            clock.on_sample(
                client_transmitted,
                server_received,
                server_transmitted,
                arrived,
            );
            None
        }
        ServerMsg::Command {
            volume: Some(v), ..
        } => Some(SendspinEvent::Volume(v.min(100))),
        ServerMsg::Command { mute: Some(m), .. } => Some(SendspinEvent::Mute(m)),
        ServerMsg::StreamStart(Some(format)) => Some(SendspinEvent::StreamStart(format)),
        ServerMsg::StreamEnd => Some(SendspinEvent::StreamEnd),
        other => {
            tracing::debug!(?other, "ignored");
            None
        }
    }
}

/// Cleartext init exchange, then Noise KKpsk2 as the responder.
async fn handshake(ws: &mut Ws, identity: &PlayerIdentity) -> anyhow::Result<NoiseTransport> {
    let init = client_init(&identity.client_id);
    ws.send(Message::Text(init.clone().into())).await?;
    let server_init = recv_text(ws).await.context("waiting for server/init")?;
    let server_id = match serde_json::from_str::<ServerMsg>(&server_init) {
        Ok(ServerMsg::Init { server_id, .. }) => server_id,
        Ok(ServerMsg::Error { reason }) => bail!("the server refused client/init: {reason}"),
        _ => bail!("expected server/init, got {server_init:.80}"),
    };
    let prologue = [init.as_bytes(), server_init.as_bytes()].concat();
    let noise = NoiseClient::new(&identity.private_key(), &server_id, &prologue)?;

    let msg1 = recv_text(ws).await.context("waiting for noise message 1")?;
    let Ok(ServerMsg::NoiseHandshake { data }) = serde_json::from_str::<ServerMsg>(&msg1) else {
        bail!("expected noise/handshake, got {msg1:.80}");
    };
    let msg1 = URL_SAFE_NO_PAD
        .decode(&data)
        .context("noise message 1 is not base64url")?;
    let (msg2, transport) = noise.respond(&msg1).context("noise message 1 rejected")?;
    ws.send(Message::Text(
        noise_handshake(&URL_SAFE_NO_PAD.encode(msg2)).into(),
    ))
    .await?;
    Ok(transport)
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

enum Incoming {
    Json(ServerMsg),
    Chunk {
        timestamp_us: i64,
        pcm: bytes::Bytes,
    },
    /// A ping, pong or ignored message: proof of life only.
    Frame,
}

/// The encrypted channel.
struct Conn {
    ws: Ws,
    noise: NoiseTransport,
    reassembler: Reassembler,
}

impl Conn {
    async fn send(&mut self, json: &str) -> anyhow::Result<()> {
        let mut plain = Vec::with_capacity(1 + json.len());
        plain.push(MSG_JSON);
        plain.extend_from_slice(json.as_bytes());
        let cipher = self.noise.encrypt(&plain)?;
        self.ws.send(Message::Binary(cipher.into())).await?;
        Ok(())
    }

    /// The next frame with its arrival time on our clock; `None` when the connection
    /// closed.
    async fn recv(&mut self) -> anyhow::Result<Option<(Incoming, i64)>> {
        let Some(msg) = self.ws.next().await else {
            return Ok(None);
        };
        let arrived = now_us();
        let b = match msg? {
            Message::Binary(b) => b,
            Message::Text(_) => bail!("cleartext message after the handshake"),
            Message::Close(_) => return Ok(None),
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {
                return Ok(Some((Incoming::Frame, arrived)));
            }
        };
        let plain = self.noise.decrypt(&b).context("decrypting")?;
        let Some((kind, body)) = self.reassembler.push(&plain)? else {
            return Ok(Some((Incoming::Frame, arrived)));
        };
        let incoming = match kind {
            MSG_JSON => match serde_json::from_slice::<ServerMsg>(&body) {
                Ok(msg) => Incoming::Json(msg),
                Err(e) => {
                    tracing::debug!(error = %e, "ignoring an unreadable message");
                    Incoming::Frame
                }
            },
            MSG_PLAYER_AUDIO if body.len() >= CHUNK_HEADER_LEN - 1 => {
                let timestamp_us = i64::from_be_bytes(body[..8].try_into().expect("8 bytes"));
                // body[8..12] is send_ahead: not needed, the timestamp says it all.
                Incoming::Chunk {
                    timestamp_us,
                    pcm: bytes::Bytes::copy_from_slice(&body[CHUNK_HEADER_LEN - 1..]),
                }
            }
            _ => Incoming::Frame,
        };
        Ok(Some((incoming, arrived)))
    }
}
