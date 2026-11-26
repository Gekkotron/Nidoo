//! HTTP API: lists the approved satellites and sets their volume and mute,
//! lists the speakers waiting for approval, and allows or forgets one.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::group::{Control, Published};
use crate::state::{PlayerPrefs, short_id};

/// One approved satellite, as the API shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SatelliteView {
    pub id: String,
    pub name: String,
    pub volume: u8,
    pub muted: bool,
    pub connected: bool,
}

/// One connected speaker that is not allowed yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WaitingView {
    pub id: String,
    pub name: String,
    pub connected: bool,
}

/// The satellites known to the group, kept up to date from its `Published` events.
#[derive(Clone, Default)]
pub struct Satellites {
    inner: Arc<RwLock<BTreeMap<String, (PlayerPrefs, bool)>>>,
    /// Short id -> the name given in Nidoo, which wins over the satellite's own.
    names: Arc<RwLock<BTreeMap<String, String>>>,
}

impl Satellites {
    pub fn apply(&self, msg: &Published) {
        let mut map = self.inner.write().unwrap();
        match msg {
            Published::Player {
                short_id,
                prefs,
                connected,
                ..
            } => {
                map.insert(short_id.clone(), (prefs.clone(), *connected));
            }
            Published::Removed { prefs } => {
                map.remove(&short_id(&prefs.client_id));
            }
            Published::Spoken { .. } => {}
        }
    }

    /// Records the name given in Nidoo to satellite `short_id`; `None` goes back to the
    /// name the satellite reports. It may come before the satellite does.
    pub fn set_name(&self, short_id: &str, name: Option<String>) {
        let mut names = self.names.write().unwrap();
        match name {
            Some(name) => names.insert(short_id.to_owned(), name),
            None => names.remove(short_id),
        };
    }

    /// The approved satellites, sorted by name and then id.
    pub fn list(&self) -> Vec<SatelliteView> {
        let map = self.inner.read().unwrap();
        let names = self.names.read().unwrap();
        let mut out: Vec<SatelliteView> = map
            .iter()
            .filter(|(_, (prefs, _))| prefs.allowed)
            .map(|(id, (prefs, connected))| SatelliteView {
                id: id.clone(),
                name: names.get(id).unwrap_or(&prefs.name).clone(),
                volume: prefs.volume,
                muted: prefs.muted,
                connected: *connected,
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        out
    }

    /// The connected speakers still waiting for approval, sorted by name and then id.
    pub fn waiting(&self) -> Vec<WaitingView> {
        let map = self.inner.read().unwrap();
        let names = self.names.read().unwrap();
        let mut out: Vec<WaitingView> = map
            .iter()
            .filter(|(_, (prefs, connected))| !prefs.allowed && *connected)
            .map(|(id, (prefs, connected))| WaitingView {
                id: id.clone(),
                name: names.get(id).unwrap_or(&prefs.name).clone(),
                connected: *connected,
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        out
    }

    /// Whether the group has this speaker, allowed or not.
    pub fn is_known(&self, id: &str) -> bool {
        self.inner.read().unwrap().contains_key(id)
    }

    pub fn is_approved(&self, id: &str) -> bool {
        self.inner
            .read()
            .unwrap()
            .get(id)
            .is_some_and(|(prefs, _)| prefs.allowed)
    }
}

/// Body of `PUT /players/{id}`: either field, or both.
#[derive(Deserialize)]
struct Change {
    volume: Option<u8>,
    muted: Option<bool>,
}

type ApiState = (Satellites, mpsc::Sender<Control>);

fn error(status: StatusCode, text: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": text.into() }))).into_response()
}

/// `GET /players`, `GET /players/waiting`, `PUT /players/{id}`,
/// `POST /players/{id}/allow` and `DELETE /players/{id}`. Each change is turned
/// into the same `Control` commands the MQTT link sends.
pub fn router(satellites: Satellites, control: mpsc::Sender<Control>) -> Router {
    Router::new()
        .route("/players", get(list_players))
        .route("/players/waiting", get(list_waiting))
        .route("/players/{id}", put(change_player).delete(forget_player))
        .route("/players/{id}/allow", post(allow_player))
        .with_state((satellites, control))
}

/// Serves the API on `bind:port` until the process ends. A bind error is
/// logged and the function returns: audio playback does not depend on it.
pub async fn serve(
    bind: std::net::IpAddr,
    port: u16,
    satellites: Satellites,
    control: mpsc::Sender<Control>,
) {
    let listener = match tokio::net::TcpListener::bind((bind, port)).await {
        Ok(listener) => listener,
        Err(e) => {
            tracing::warn!(%bind, port, error = %e, "HTTP API cannot listen, running without it");
            return;
        }
    };
    if let Err(e) = axum::serve(listener, router(satellites, control)).await {
        tracing::warn!(error = %e, "HTTP API stopped");
    }
}

async fn list_players(State((satellites, _)): State<ApiState>) -> Json<Vec<SatelliteView>> {
    Json(satellites.list())
}

async fn list_waiting(State((satellites, _)): State<ApiState>) -> Json<Vec<WaitingView>> {
    Json(satellites.waiting())
}

/// Sends `command` for a known speaker: 204, 404 or 503.
async fn control_known(
    satellites: &Satellites,
    control: &mpsc::Sender<Control>,
    id: &str,
    command: Control,
) -> Response {
    if !satellites.is_known(id) {
        return error(StatusCode::NOT_FOUND, format!("no speaker {id}"));
    }
    if control.send(command).await.is_err() {
        return error(StatusCode::SERVICE_UNAVAILABLE, "nidoo-sound is stopping");
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn allow_player(
    State((satellites, control)): State<ApiState>,
    Path(id): Path<String>,
) -> Response {
    let command = Control::SetAllowed {
        short_id: id.clone(),
        allowed: true,
    };
    control_known(&satellites, &control, &id, command).await
}

async fn forget_player(
    State((satellites, control)): State<ApiState>,
    Path(id): Path<String>,
) -> Response {
    let command = Control::Remove {
        short_id: id.clone(),
    };
    control_known(&satellites, &control, &id, command).await
}

async fn change_player(
    State((satellites, control)): State<ApiState>,
    Path(id): Path<String>,
    body: Result<Json<Change>, JsonRejection>,
) -> Response {
    if !satellites.is_approved(&id) {
        return error(StatusCode::NOT_FOUND, format!("no approved speaker {id}"));
    }
    let change = match body {
        Ok(Json(change)) => change,
        Err(rejection) => return error(StatusCode::BAD_REQUEST, rejection.body_text()),
    };
    if change.volume.is_none() && change.muted.is_none() {
        return error(StatusCode::BAD_REQUEST, "nothing to change");
    }
    if change.volume.is_some_and(|v| v > 100) {
        return error(StatusCode::BAD_REQUEST, "volume must be 0-100");
    }
    let mut commands = Vec::with_capacity(2);
    if let Some(volume) = change.volume {
        commands.push(Control::SetVolume {
            short_id: id.clone(),
            volume,
        });
    }
    if let Some(muted) = change.muted {
        commands.push(Control::SetMute {
            short_id: id,
            muted,
        });
    }
    for command in commands {
        if control.send(command).await.is_err() {
            return error(StatusCode::SERVICE_UNAVAILABLE, "nidoo-sound is stopping");
        }
    }
    StatusCode::NO_CONTENT.into_response()
}

#[cfg(test)]
mod tests;
