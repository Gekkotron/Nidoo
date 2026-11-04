//! `POST /api/v1/sound/speak`: say a sentence on the speaker satellites.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::routing::post;
use nidoo_api::{SoundSpeakReply, SoundSpeakRequest};

use super::api::HttpError;
use crate::hub::Hub;
use crate::sound::{SpeakError, SpeakTarget};

pub(super) fn routes() -> Router<Arc<Hub>> {
    Router::new().route("/sound/speak", post(speak))
}

impl From<SpeakError> for HttpError {
    fn from(err: SpeakError) -> Self {
        match err {
            SpeakError::UnknownRoom(_) => {
                HttpError::new(StatusCode::UNPROCESSABLE_ENTITY, err.to_string())
            }
            SpeakError::NoSatellite | SpeakError::NotSetUp => {
                HttpError::new(StatusCode::CONFLICT, err.to_string())
            }
            SpeakError::Tts(err) => err.into(),
            SpeakError::Publish(err) => err.into(),
        }
    }
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
