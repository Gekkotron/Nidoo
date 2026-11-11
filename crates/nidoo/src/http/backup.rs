//! The backup routes under `/api/v1/backup`: back up now, follow and cancel the job, download a
//! backup, the last backup and the FTP connection test.

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use nidoo_api::{BackupProgress, FtpCheck, FtpInput};

use super::api::HttpError;
use crate::backup::BackupError;
use crate::hub::Hub;

/// How long a "back up now" may run.
const TIME_LIMIT: Duration = Duration::from_secs(120 * 60);

pub(super) fn routes() -> Router<Arc<Hub>> {
    Router::new()
        .route("/backup/last", get(last))
        .route("/backup/test", post(test_ftp))
        .route("/backup/runs", post(start))
        .route("/backup/current", get(current).delete(cancel))
        .route("/backup/download", get(download))
}

impl From<BackupError> for HttpError {
    fn from(err: BackupError) -> Self {
        let status = match err {
            BackupError::Busy | BackupError::NoPassword | BackupError::FtpNotConfigured => {
                StatusCode::CONFLICT
            }
            _ => StatusCode::SERVICE_UNAVAILABLE,
        };
        HttpError::new(status, err.to_string())
    }
}

async fn last(State(hub): State<Arc<Hub>>) -> Response {
    match hub.last_backup() {
        Some(last) => Json(last).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

/// The body is optional: none tests the stored settings.
async fn test_ftp(
    State(hub): State<Arc<Hub>>,
    body: Bytes,
) -> Result<Json<Vec<FtpCheck>>, HttpError> {
    let input: Option<FtpInput> = if body.iter().all(u8::is_ascii_whitespace) {
        None
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| HttpError::new(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?
    };
    Ok(Json(hub.test_ftp(input).await))
}

async fn start(
    State(hub): State<Arc<Hub>>,
) -> Result<(StatusCode, Json<BackupProgress>), HttpError> {
    Ok((StatusCode::ACCEPTED, Json(hub.start_backup(TIME_LIMIT)?)))
}

async fn current(State(hub): State<Arc<Hub>>) -> Response {
    match hub.backup_progress() {
        Some(progress) => Json(progress).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

async fn cancel(State(hub): State<Arc<Hub>>) -> Result<StatusCode, HttpError> {
    if hub.cancel_backup() {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(HttpError::new(
            StatusCode::NOT_FOUND,
            "no backup is running",
        ))
    }
}

async fn download(State(hub): State<Arc<Hub>>) -> Result<Response, HttpError> {
    let (file, stream) = hub.backup_download()?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{file}\""),
            ),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}
