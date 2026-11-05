//! `POST /api/v1/sound/speak`: say a sentence on the speaker satellites.
//! `DELETE /api/v1/sound/satellites/{id}`: forget a satellite (by short id).

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, post};
use nidoo_api::{SoundSpeakReply, SoundSpeakRequest};

use super::api::HttpError;
use crate::hub::Hub;
use crate::sound::{RemoveError, SpeakError, SpeakTarget};

pub(super) fn routes() -> Router<Arc<Hub>> {
    Router::new()
        .route("/sound/speak", post(speak))
        .route("/sound/satellites/{id}", delete(remove))
}

impl From<SpeakError> for HttpError {
    fn from(err: SpeakError) -> Self {
        match err {
            SpeakError::UnknownRoom(_) => {
                HttpError::new(StatusCode::UNPROCESSABLE_ENTITY, err.to_string())
            }
            SpeakError::NoSatellite
            | SpeakError::NoSatelliteInRoom
            | SpeakError::SatelliteCannotSpeak
            | SpeakError::NotSetUp => HttpError::new(StatusCode::CONFLICT, err.to_string()),
            SpeakError::Tts(err) => err.into(),
            SpeakError::Publish(err) => err.into(),
        }
    }
}

impl From<RemoveError> for HttpError {
    fn from(err: RemoveError) -> Self {
        match err {
            RemoveError::UnknownSatellite(_) => {
                HttpError::new(StatusCode::NOT_FOUND, err.to_string())
            }
            RemoveError::ServiceOffline => HttpError::new(StatusCode::CONFLICT, err.to_string()),
            RemoveError::Publish(err) => err.into(),
        }
    }
}

/// 202: `nidoo-sound` clears the satellite's retained messages, and the hub drops its
/// entities as they go.
async fn remove(
    State(hub): State<Arc<Hub>>,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    hub.remove_satellite(&id).await?;
    Ok(StatusCode::ACCEPTED)
}

async fn speak(
    State(hub): State<Arc<Hub>>,
    body: Result<Json<SoundSpeakRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<SoundSpeakReply>), HttpError> {
    let Json(req) = body?;
    let target = match (req.room, req.satellite) {
        (Some(_), Some(_)) => {
            return Err(HttpError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "give a room or a satellite, not both",
            ));
        }
        (Some(room), None) => SpeakTarget::Room(room),
        (None, Some(satellite)) => SpeakTarget::Satellite(satellite),
        (None, None) => SpeakTarget::All,
    };
    let players = hub
        .speak_in(target, &req.text, req.language, req.voice)
        .await?;
    Ok((StatusCode::ACCEPTED, Json(SoundSpeakReply { players })))
}
