//! Players that connect to us (client-initiated, e.g. sendspin-js in a
//! browser): a WebSocket endpoint on [`PATH`]. After the WebSocket opens the
//! protocol is the same as when we connect: client/init, server/init, then
//! Noise with the server as initiator. We cannot reconnect to such a player;
//! it reconnects itself.

use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;

use crate::clock::Clock;
use crate::group::FromPlayer;
use crate::player::{Liveness, PlayerEvent, serve_player};
use crate::proto::noise::ServerIdentity;

/// The WebSocket path players connect to (the spec's recommended one).
pub const PATH: &str = "/sendspin";
/// The spec's recommended server port.
pub const DEFAULT_PORT: u16 = 8927;

const UPGRADE_TIMEOUT: Duration = Duration::from_secs(10);

/// Accepts players forever. Each one reaches the group only once its
/// handshake and `client/hello` are done: [`FromPlayer::Accepted`] (under a
/// fresh `inbound-N` instance) then its events.
pub async fn accept_players(
    listener: TcpListener,
    identity: Arc<ServerIdentity>,
    clock: Arc<dyn Clock>,
    to_group: mpsc::Sender<FromPlayer>,
    liveness: Liveness,
) {
    let mut count = 0u64;
    loop {
        let (tcp, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                tracing::warn!(error = %e, "accepting a player connection");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        count += 1;
        let instance = format!("inbound-{count}");
        let (identity, clock, to_group) = (identity.clone(), clock.clone(), to_group.clone());
        tokio::spawn(async move {
            if let Err(e) = serve_one(tcp, &instance, identity, clock, to_group, liveness).await {
                tracing::info!(%instance, %peer, error = %format!("{e:#}"), "player connection ended");
            }
        });
    }
}

async fn serve_one(
    tcp: TcpStream,
    instance: &str,
    identity: Arc<ServerIdentity>,
    clock: Arc<dyn Clock>,
    to_group: mpsc::Sender<FromPlayer>,
    liveness: Liveness,
) -> anyhow::Result<()> {
    let ws = timeout(
        UPGRADE_TIMEOUT,
        tokio_tungstenite::accept_hdr_async(MaybeTlsStream::Plain(tcp), check_path),
    )
    .await??;

    let peer = ws.get_ref().get_ref().peer_addr().ok().map(|a| a.ip());
    let (cmds, cmd_rx) = mpsc::channel(256);
    let (ev_tx, mut ev_rx) = mpsc::channel(64);
    let forward = {
        let instance = instance.to_owned();
        tokio::spawn(async move {
            // Held until the player is connected, then handed to the group.
            let mut cmds = Some(cmds);
            while let Some(event) = ev_rx.recv().await {
                if matches!(event, PlayerEvent::Connected(_))
                    && let Some(cmds) = cmds.take()
                {
                    let accepted = FromPlayer::Accepted {
                        instance: instance.clone(),
                        cmds,
                        peer,
                    };
                    if to_group.send(accepted).await.is_err() {
                        break;
                    }
                }
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
    let result = serve_player(ws, identity, clock, ev_tx, cmd_rx, liveness).await;
    let _ = forward.await;
    result
}

// The error type is fixed by tungstenite.
#[allow(clippy::result_large_err)]
fn check_path(req: &Request, resp: Response) -> Result<Response, ErrorResponse> {
    if req.uri().path() == PATH {
        return Ok(resp);
    }
    let mut refused = ErrorResponse::new(Some("not found".into()));
    *refused.status_mut() = StatusCode::NOT_FOUND;
    Err(refused)
}
