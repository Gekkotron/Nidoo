//! The WebSocket client: feeds [`House`] from the hub's `/ws` and reconnects with back-off.

use futures_util::StreamExt;
use gloo_net::websocket::{Message, futures::WebSocket};
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::Set;
use nidoo_api::{ConnectionStatus, WsEvent};
use wasm_bindgen_futures::spawn_local;

use crate::house::House;
use crate::i18n::current_lang;
use crate::i18n::errors::{ErrorText, error_text};

const FIRST_BACKOFF_MS: u32 = 1_000;
const MAX_BACKOFF_MS: u32 = 10_000;

/// Keeps a WebSocket to `path` (e.g. `/ws`, on the page's own host) open, applying every event
/// to `house`.
pub fn connect(path: &str, house: House) {
    let path = path.to_owned();
    spawn_local(async move {
        let mut backoff = FIRST_BACKOFF_MS;
        // The shell fetches the dashboards at start-up; only a reconnect refetches them here.
        let mut connected_before = false;
        loop {
            house.connection.set(ConnectionStatus::Connecting);
            let reason = match absolute_url(&path)
                .and_then(|u| WebSocket::open(&u).map_err(|e| e.to_string()))
            {
                Ok(mut socket) => {
                    let mut closed = None;
                    while let Some(frame) = socket.next().await {
                        match frame {
                            Ok(Message::Text(text)) => match serde_json::from_str::<WsEvent>(&text)
                            {
                                Ok(event) => {
                                    let connected = matches!(event, WsEvent::Snapshot(_));
                                    if connected {
                                        backoff = FIRST_BACKOFF_MS;
                                    }
                                    house.apply(event);
                                    // Weather, home settings, the solar forecast and the ambiances are
                                    // not in the snapshot: refetch them on every (re)connect.
                                    if connected {
                                        house.refresh_weather();
                                        house.refresh_solar();
                                        house.refresh_ambiances();
                                        house.refresh_home_settings();
                                        if connected_before {
                                            house.refresh_dashboards(|_| ());
                                        }
                                        connected_before = true;
                                    }
                                }
                                Err(e) => {
                                    web_sys::console::warn_1(&format!("bad ws event: {e}").into())
                                }
                            },
                            Ok(Message::Bytes(_)) => {}
                            Err(e) => {
                                closed = Some(e.to_string());
                                break;
                            }
                        }
                    }
                    // Built now, so it follows a toggle made while the socket was open.
                    closed.unwrap_or_else(|| {
                        error_text(current_lang(), ErrorText::ConnectionClosed).to_owned()
                    })
                }
                Err(e) => e,
            };
            house.connection.set(ConnectionStatus::Offline { reason });
            TimeoutFuture::new(backoff).await;
            backoff = (backoff * 2).min(MAX_BACKOFF_MS);
        }
    });
}

/// `ws://host/path` or `wss://host/path`, following the page's protocol.
fn absolute_url(path: &str) -> Result<String, String> {
    let no_address = || error_text(current_lang(), ErrorText::NoAddress).to_owned();
    let location = web_sys::window().ok_or_else(no_address)?.location();
    let protocol = location.protocol().map_err(|_| no_address())?;
    let host = location.host().map_err(|_| no_address())?;
    let scheme = if protocol == "https:" { "wss" } else { "ws" };
    Ok(format!("{scheme}://{host}{path}"))
}
