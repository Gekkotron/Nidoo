//! The speech management routes under `/api/v1/assist/tts`.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use nidoo_api::{
    ActiveRequest, DownloadRequest, DownloadView, InspectRequest, ModelView, RepoListing,
    SpeechRequest, SpeechTicket, TokenInput, TtsStatus, UploadSession, VoiceRequest, VoicesView,
};

use super::api::HttpError;
use crate::assist::tts::Tts;
use crate::hub::Hub;

type HubState = State<Arc<Hub>>;
type Json_<T> = Result<Json<T>, JsonRejection>;

pub(super) fn routes() -> Router<Arc<Hub>> {
    Router::new()
        .route("/assist/tts/status", get(status))
        .route("/assist/tts/models", get(models))
        .route("/assist/tts/models/{id}", delete(remove_model))
        .route("/assist/tts/active", put(set_active))
        .route("/assist/tts/inspect", post(inspect))
        .route("/assist/tts/downloads", post(start_download))
        .route(
            "/assist/tts/downloads/current",
            get(current_download).delete(cancel_download),
        )
        .route("/assist/tts/uploads", post(open_upload))
        .route(
            "/assist/tts/uploads/{id}/files/{name}",
            put(upload_file).layer(DefaultBodyLimit::disable()),
        )
        .route("/assist/tts/uploads/{id}/finish", post(finish_upload))
        .route("/assist/tts/uploads/{id}", delete(abort_upload))
        .route("/assist/tts/voices", get(voices))
        .route("/assist/tts/voice", put(set_voice))
        .route("/assist/tts/token", put(set_token))
        .route("/assist/speech", post(speak))
        .route("/assist/speech/{file}", get(speech_file))
}

fn tts(hub: &Hub) -> Result<Arc<Tts>, HttpError> {
    hub.tts()
        .cloned()
        .ok_or_else(|| HttpError::new(StatusCode::CONFLICT, "speech is not set up on this hub"))
}

async fn status(State(hub): HubState) -> Result<Json<TtsStatus>, HttpError> {
    Ok(Json(tts(&hub)?.status()))
}

async fn models(State(hub): HubState) -> Result<Json<Vec<ModelView>>, HttpError> {
    Ok(Json(tts(&hub)?.models()))
}

async fn remove_model(
    State(hub): HubState,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    tts(&hub)?.remove(&id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_active(
    State(hub): HubState,
    body: Json_<ActiveRequest>,
) -> Result<Json<TtsStatus>, HttpError> {
    let Json(req) = body?;
    Ok(Json(tts(&hub)?.set_active(req.model)?))
}

async fn inspect(
    State(hub): HubState,
    body: Json_<InspectRequest>,
) -> Result<Json<RepoListing>, HttpError> {
    let Json(req) = body?;
    Ok(Json(tts(&hub)?.inspect(&req.url).await?))
}

async fn start_download(
    State(hub): HubState,
    body: Json_<DownloadRequest>,
) -> Result<(StatusCode, Json<DownloadView>), HttpError> {
    let Json(req) = body?;
    Ok((StatusCode::ACCEPTED, Json(tts(&hub)?.start_download(req)?)))
}

async fn current_download(State(hub): HubState) -> Result<Json<DownloadView>, HttpError> {
    tts(&hub)?
        .download()
        .map(Json)
        .ok_or_else(|| HttpError::new(StatusCode::NOT_FOUND, "no download"))
}

async fn cancel_download(State(hub): HubState) -> Result<StatusCode, HttpError> {
    tts(&hub)?.cancel_download()?;
    Ok(StatusCode::NO_CONTENT)
}

async fn open_upload(State(hub): HubState) -> Result<Json<UploadSession>, HttpError> {
    Ok(Json(tts(&hub)?.upload_open()?))
}

async fn upload_file(
    State(hub): HubState,
    Path((id, name)): Path<(String, String)>,
    body: Body,
) -> Result<StatusCode, HttpError> {
    tts(&hub)?.upload_file(&id, &name, body).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn finish_upload(
    State(hub): HubState,
    Path(id): Path<String>,
) -> Result<Json<ModelView>, HttpError> {
    Ok(Json(tts(&hub)?.upload_finish(&id)?))
}

async fn abort_upload(
    State(hub): HubState,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    tts(&hub)?.upload_abort(&id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn voices(State(hub): HubState) -> Result<Json<VoicesView>, HttpError> {
    Ok(Json(tts(&hub)?.voices().await))
}

async fn set_voice(
    State(hub): HubState,
    body: Json_<VoiceRequest>,
) -> Result<Json<VoicesView>, HttpError> {
    let Json(req) = body?;
    Ok(Json(tts(&hub)?.set_voice(&req.voice).await?))
}

async fn set_token(State(hub): HubState, body: Json_<TokenInput>) -> Result<StatusCode, HttpError> {
    let Json(req) = body?;
    tts(&hub)?.set_token(req.token)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn speak(
    State(hub): HubState,
    body: Json_<SpeechRequest>,
) -> Result<(StatusCode, Json<SpeechTicket>), HttpError> {
    let Json(req) = body?;
    Ok((StatusCode::CREATED, Json(tts(&hub)?.speak(req)?)))
}

async fn speech_file(
    State(hub): HubState,
    Path(file): Path<String>,
) -> Result<Response, HttpError> {
    let not_found = || HttpError::new(StatusCode::NOT_FOUND, "no such speech");
    let tts = tts(&hub)?;
    let id = file.strip_suffix(".wav").ok_or_else(not_found)?;
    let clip = tts.clip(id).ok_or_else(not_found)?;
    let rate = tts.engine().sample_rate().unwrap_or(24_000);
    Ok((
        [
            (header::CONTENT_TYPE, "audio/wav"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        clip.body(rate),
    )
        .into_response())
}
