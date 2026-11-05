//! HTTP API: lists the approved satellites and sets their volume and mute.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
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

/// The satellites known to the group, kept up to date from its `Published` events.
#[derive(Clone, Default)]
pub struct Satellites {
    inner: Arc<RwLock<BTreeMap<String, (PlayerPrefs, bool)>>>,
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

    /// The approved satellites, sorted by name and then id.
    pub fn list(&self) -> Vec<SatelliteView> {
        let map = self.inner.read().unwrap();
        let mut out: Vec<SatelliteView> = map
            .iter()
            .filter(|(_, (prefs, _))| prefs.allowed)
            .map(|(id, (prefs, connected))| SatelliteView {
                id: id.clone(),
                name: prefs.name.clone(),
                volume: prefs.volume,
                muted: prefs.muted,
                connected: *connected,
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        out
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

/// `GET /players` and `PUT /players/{id}`. A PUT is turned into the same
/// `Control` commands the MQTT link sends.
pub fn router(satellites: Satellites, control: mpsc::Sender<Control>) -> Router {
    Router::new()
        .route("/players", get(list_players))
        .route("/players/{id}", put(change_player))
        .with_state((satellites, control))
}

/// Serves the API on `0.0.0.0:port` until the process ends. A bind error is
/// logged and the function returns: audio playback does not depend on it.
pub async fn serve(port: u16, satellites: Satellites, control: mpsc::Sender<Control>) {
    let listener = match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
        Ok(listener) => listener,
        Err(e) => {
            tracing::warn!(port, error = %e, "HTTP API cannot listen, running without it");
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
mod tests {
    use super::*;

    fn prefs(client_id: &str, name: &str, allowed: bool) -> PlayerPrefs {
        PlayerPrefs {
            client_id: client_id.to_owned(),
            name: name.to_owned(),
            volume: 45,
            muted: false,
            allowed,
        }
    }

    fn player(prefs: PlayerPrefs, connected: bool) -> Published {
        Published::Player {
            short_id: short_id(&prefs.client_id),
            prefs,
            connected,
            address: None,
        }
    }

    #[test]
    fn list_keeps_only_approved_sorted_by_name() {
        let s = Satellites::default();
        s.apply(&player(prefs("living-id", "Living room", true), true));
        s.apply(&player(prefs("kitchen-id", "Kitchen", true), true));
        s.apply(&player(prefs("garage-id", "Garage", false), true));
        let list = s.list();
        let names: Vec<&str> = list.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["Kitchen", "Living room"]);
        assert_eq!(list[0].id, short_id("kitchen-id"));
    }

    #[test]
    fn player_update_replaces_the_entry() {
        let s = Satellites::default();
        s.apply(&player(prefs("kitchen-id", "Kitchen", true), true));
        let mut p = prefs("kitchen-id", "Kitchen", true);
        p.volume = 70;
        p.muted = true;
        s.apply(&player(p, false));
        let list = s.list();
        assert_eq!(list.len(), 1);
        assert_eq!(
            (list[0].volume, list[0].muted, list[0].connected),
            (70, true, false)
        );
    }

    #[test]
    fn removed_and_unapproved_leave_the_list() {
        let s = Satellites::default();
        let kitchen = prefs("kitchen-id", "Kitchen", true);
        s.apply(&player(kitchen.clone(), true));
        s.apply(&Published::Removed {
            prefs: kitchen.clone(),
        });
        assert!(s.list().is_empty());

        s.apply(&player(kitchen.clone(), true));
        assert!(s.is_approved(&short_id("kitchen-id")));
        let mut off = kitchen;
        off.allowed = false;
        s.apply(&player(off, true));
        assert!(s.list().is_empty());
        assert!(!s.is_approved(&short_id("kitchen-id")));
    }

    async fn serve(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    /// Kitchen approved, Garage not; returns the base URL and the control receiver.
    async fn fixture() -> (String, mpsc::Receiver<Control>) {
        let s = Satellites::default();
        s.apply(&player(prefs("kitchen-id", "Kitchen", true), true));
        s.apply(&player(prefs("garage-id", "Garage", false), true));
        let (tx, rx) = mpsc::channel(8);
        (serve(router(s, tx)).await, rx)
    }

    #[tokio::test]
    async fn get_players_lists_approved() {
        let (base, _rx) = fixture().await;
        let res = reqwest::get(format!("{base}/players")).await.unwrap();
        assert_eq!(res.status(), 200);
        let body: serde_json::Value = res.json().await.unwrap();
        assert_eq!(body.as_array().unwrap().len(), 1);
        assert_eq!(body[0]["name"], "Kitchen");
        assert_eq!(body[0]["id"], short_id("kitchen-id"));
        assert_eq!(body[0]["volume"], 45);
        assert_eq!(body[0]["muted"], false);
        assert_eq!(body[0]["connected"], true);
    }

    async fn put(base: &str, id: &str, body: &str) -> reqwest::Response {
        reqwest::Client::new()
            .put(format!("{base}/players/{id}"))
            .header("content-type", "application/json")
            .body(body.to_owned())
            .send()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn put_volume_sends_control() {
        let (base, mut rx) = fixture().await;
        let id = short_id("kitchen-id");
        assert_eq!(put(&base, &id, r#"{"volume":100}"#).await.status(), 204);
        assert_eq!(
            rx.try_recv(),
            Ok(Control::SetVolume {
                short_id: id,
                volume: 100
            })
        );
    }

    #[tokio::test]
    async fn put_both_sends_two_controls() {
        let (base, mut rx) = fixture().await;
        let id = short_id("kitchen-id");
        let res = put(&base, &id, r#"{"volume":20,"muted":true}"#).await;
        assert_eq!(res.status(), 204);
        assert_eq!(
            rx.try_recv(),
            Ok(Control::SetVolume {
                short_id: id.clone(),
                volume: 20
            })
        );
        assert_eq!(
            rx.try_recv(),
            Ok(Control::SetMute {
                short_id: id,
                muted: true
            })
        );
    }

    #[tokio::test]
    async fn put_rejects_bad_bodies() {
        let (base, mut rx) = fixture().await;
        let id = short_id("kitchen-id");
        for body in [r#"{"volume":101}"#, "{}", r#"{"volume":"loud"}"#] {
            let res = put(&base, &id, body).await;
            assert_eq!(res.status(), 400, "{body}");
            let json: serde_json::Value = res.json().await.unwrap();
            assert!(json["error"].is_string(), "{body}");
        }
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn put_unknown_or_unapproved_is_404() {
        let (base, mut rx) = fixture().await;
        let res = put(&base, "ffffffffffff", r#"{"volume":10}"#).await;
        assert_eq!(res.status(), 404);
        let garage = short_id("garage-id");
        assert_eq!(put(&base, &garage, r#"{"volume":10}"#).await.status(), 404);
        assert!(rx.try_recv().is_err());
    }
}
