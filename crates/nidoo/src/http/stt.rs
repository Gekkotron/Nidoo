//! The speech input routes under `/api/v1/assist/stt`.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::routing::{delete, get, put};
use nidoo_api::{DownloadView, SttKey, SttSettings, SttStatus};

use super::api::HttpError;
use crate::assist::stt::{Stt, SttError};
use crate::hub::Hub;

type HubState = State<Arc<Hub>>;

pub(super) fn routes() -> Router<Arc<Hub>> {
    Router::new()
        .route("/assist/stt/status", get(status))
        .route("/assist/stt/settings", put(set_settings))
        .route(
            "/assist/stt/download",
            delete(cancel_download).post(start_download),
        )
        .route("/assist/stt/model", delete(remove_model))
        .route("/assist/stt/key", get(key).post(new_key))
}

fn stt(hub: &Hub) -> Result<Arc<Stt>, HttpError> {
    hub.stt().cloned().ok_or_else(|| {
        HttpError::new(
            StatusCode::CONFLICT,
            "speech input is not set up on this hub",
        )
    })
}

async fn status(State(hub): HubState) -> Result<Json<SttStatus>, HttpError> {
    Ok(Json(stt(&hub)?.status()))
}

async fn set_settings(
    State(hub): HubState,
    body: Result<Json<SttSettings>, JsonRejection>,
) -> Result<Json<SttStatus>, HttpError> {
    let Json(settings) = body?;
    Ok(Json(stt(&hub)?.set_settings(settings)?))
}

async fn start_download(
    State(hub): HubState,
) -> Result<(StatusCode, Json<DownloadView>), HttpError> {
    let view = stt(&hub)?.start_download()?;
    Ok((StatusCode::ACCEPTED, Json(view)))
}

async fn cancel_download(State(hub): HubState) -> Result<StatusCode, HttpError> {
    stt(&hub)?.cancel_download()?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_model(State(hub): HubState) -> Result<StatusCode, HttpError> {
    stt(&hub)?.remove_model()?;
    Ok(StatusCode::NO_CONTENT)
}

async fn key(State(hub): HubState) -> Result<Json<SttKey>, HttpError> {
    let key = stt(&hub)?
        .key()
        .ok_or_else(|| SttError::NotFound("no key yet".into()))?;
    Ok(Json(SttKey { key: key.to_hex() }))
}

async fn new_key(State(hub): HubState) -> Result<Json<SttKey>, HttpError> {
    let key = stt(&hub)?.new_key()?;
    Ok(Json(SttKey { key: key.to_hex() }))
}
