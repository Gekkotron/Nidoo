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
use axum::routing::{delete, get, post};
use nidoo_api::{BackupProgress, FtpCheck, FtpInput};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::api::HttpError;
use crate::backup::BackupError;
use crate::backup::restore::{
    RESTORED_Z2M_FILE, delete_restored_z2m, restored_z2m_info as read_info,
};
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
        .route("/backup/restored-z2m", delete(delete_z2m))
        .route("/backup/restored-z2m/download", post(restored_z2m))
        .route("/backup/restored-z2m/info", get(restored_z2m_info))
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

fn not_found() -> HttpError {
    HttpError::new(StatusCode::NOT_FOUND, "no restored Zigbee2MQTT data")
}

async fn restored_z2m_info(
    State(hub): State<Arc<Hub>>,
) -> Result<Json<nidoo_api::RestoredZ2m>, HttpError> {
    let dir = hub.data_dir().ok_or_else(not_found)?;
    read_info(&dir).map(Json).ok_or_else(not_found)
}

#[derive(Deserialize)]
struct PasswordBody {
    #[serde(default)]
    password: String,
}

/// Whether `given` is `stored`, compared in constant time (over the digests, so the length
/// does not leak either).
fn password_matches(stored: &str, given: &str) -> bool {
    let a = Sha256::digest(stored.as_bytes());
    let b = Sha256::digest(given.as_bytes());
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// The restored zip holds the Zigbee network key: it is served only to whoever gives the
/// backup password. The password and the zip are never logged.
async fn restored_z2m(State(hub): State<Arc<Hub>>, body: Bytes) -> Result<Response, HttpError> {
    let given: PasswordBody = serde_json::from_slice(&body).unwrap_or(PasswordBody {
        password: String::new(),
    });
    let Some(stored) = hub.stored_backup_password().await else {
        return Err(HttpError::new(
            StatusCode::CONFLICT,
            "no backup password set",
        ));
    };
    if !password_matches(&stored, &given.password) {
        return Err(HttpError::new(StatusCode::FORBIDDEN, "Wrong password"));
    }
    let dir = hub.data_dir().ok_or_else(not_found)?;
    let bytes = match tokio::fs::read(dir.join(RESTORED_Z2M_FILE)).await {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(not_found()),
        Err(e) => {
            return Err(HttpError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                e.to_string(),
            ));
        }
    };
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"zigbee2mqtt-backup.zip\"",
            ),
        ],
        bytes,
    )
        .into_response())
}

async fn delete_z2m(State(hub): State<Arc<Hub>>) -> Result<StatusCode, HttpError> {
    if let Some(dir) = hub.data_dir() {
        delete_restored_z2m(&dir)
            .map_err(|e| HttpError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::password_matches;

    #[test]
    fn the_password_must_match_exactly() {
        assert!(password_matches("s3cret", "s3cret"));
        assert!(!password_matches("s3cret", "s3cre"));
        assert!(!password_matches("s3cret", ""));
        assert!(!password_matches("s3cret", "S3cret"));
    }
}
