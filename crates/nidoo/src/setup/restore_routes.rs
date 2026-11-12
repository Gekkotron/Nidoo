//! The restore routes under `/api/v1/setup/restore`, served only in restore mode: find a backup
//! on an FTP server, read its summary, restore it from there or from an upload, and follow the
//! restore. No response or error carries a password.

// The `Err` side is an axum `Response`, built at most once per request.
#![allow(clippy::result_large_err)]

use std::io;

use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use futures_util::TryStreamExt;
use nidoo_api::{FtpSource, RestoreFromFtp};
use tokio_util::io::StreamReader;

use crate::backup::BackupError;
use crate::backup::restore::{
    claim, ftp_settings, inspect, inspect_ftp, list_ftp, restore_from_ftp, restore_upload,
    spawn_tracked,
};
use crate::setup::routes::{RouteState, decode, error, restarting};
use crate::setup::{Mode, SetupCtx};

/// The most of a file's head an upload inspect reads.
const INSPECT_LIMIT: usize = 4 * 1024 * 1024;
const PASSWORD_HEADER: &str = "x-backup-password";
const UNREACHABLE: &str = "Can't reach the FTP server. Is it switched on?";

pub(super) fn routes() -> Router<RouteState> {
    Router::new()
        .route("/api/v1/setup/restore/ftp/list", post(ftp_list))
        .route("/api/v1/setup/restore/ftp/inspect", post(ftp_inspect))
        .route("/api/v1/setup/restore/ftp", post(ftp_restore))
        .route(
            "/api/v1/setup/restore/upload/inspect",
            post(upload_inspect).layer(DefaultBodyLimit::max(INSPECT_LIMIT)),
        )
        .route(
            "/api/v1/setup/restore/upload",
            put(upload_restore).layer(DefaultBodyLimit::disable()),
        )
        .route("/api/v1/setup/restore/progress", get(progress))
}

fn waiting(ctx: &SetupCtx) -> Result<(), Response> {
    if ctx.mode != Mode::Restore {
        return Err(error(StatusCode::CONFLICT, "not waiting for a restore"));
    }
    Ok(())
}

/// Restore mode, no restart pending.
fn open(ctx: &SetupCtx) -> Result<(), Response> {
    waiting(ctx)?;
    restarting(ctx).map_or(Ok(()), Err)
}

/// [`open`], with a database to restore into.
fn ready(ctx: &SetupCtx) -> Result<(), Response> {
    open(ctx)?;
    if ctx.db.is_none() {
        return Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "database unreachable",
        ));
    }
    Ok(())
}

fn refused(e: BackupError) -> Response {
    let status = match &e {
        BackupError::Unreachable(_) => return error(StatusCode::BAD_GATEWAY, UNREACHABLE),
        BackupError::Busy | BackupError::HoldsData(_) => StatusCode::CONFLICT,
        BackupError::WrongPassword
        | BackupError::NotABackup
        | BackupError::TooCostly
        | BackupError::TooNew
        | BackupError::Corrupt(_)
        | BackupError::LoginRefused
        | BackupError::Folder(_) => StatusCode::UNPROCESSABLE_ENTITY,
        BackupError::Database(_) | BackupError::Broker(_) => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    error(status, &e.to_string())
}

/// The backup password an upload sends, percent-encoded, beside the file.
fn password(headers: &HeaderMap) -> String {
    headers
        .get(PASSWORD_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(decode)
        .unwrap_or_default()
}

async fn ftp_list(State(s): State<RouteState>, Json(source): Json<FtpSource>) -> Response {
    if let Err(r) = open(&s.ctx) {
        return r;
    }
    match list_ftp(&ftp_settings(&source)).await {
        Ok(list) => Json(list).into_response(),
        Err(e) => refused(e),
    }
}

async fn ftp_inspect(State(s): State<RouteState>, Json(req): Json<RestoreFromFtp>) -> Response {
    if let Err(r) = open(&s.ctx) {
        return r;
    }
    match inspect_ftp(&s.ctx, &ftp_settings(&req.source), &req.file, &req.password).await {
        Ok(summary) => Json(summary).into_response(),
        Err(e) => refused(e),
    }
}

async fn upload_inspect(State(s): State<RouteState>, headers: HeaderMap, head: Bytes) -> Response {
    if let Err(r) = open(&s.ctx) {
        return r;
    }
    match inspect(&s.ctx, &head[..], &password(&headers)).await {
        Ok(summary) => Json(summary).into_response(),
        Err(e) => refused(e),
    }
}

/// Starts the restore and answers at once; `GET …/progress` follows it.
async fn ftp_restore(State(s): State<RouteState>, Json(req): Json<RestoreFromFtp>) -> Response {
    if let Err(r) = ready(&s.ctx) {
        return r;
    }
    let started = match claim(&s.ctx, None).await {
        Ok(started) => started,
        Err(e) => return refused(e),
    };
    let ctx = s.ctx.clone();
    let source = ftp_settings(&req.source);
    spawn_tracked(s.ctx.clone(), async move {
        restore_from_ftp(&ctx, source, &req.file, &req.password).await
    });
    (StatusCode::ACCEPTED, Json(started)).into_response()
}

/// Restores the file the request carries, answering once the restore ended. A client that
/// leaves mid-way ends the body, and with it the restore, which then cleans up.
async fn upload_restore(State(s): State<RouteState>, headers: HeaderMap, body: Body) -> Response {
    if let Err(r) = ready(&s.ctx) {
        return r;
    }
    let total = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    if let Err(e) = claim(&s.ctx, total).await {
        return refused(e);
    }
    let ctx = s.ctx.clone();
    let password = password(&headers);
    let input = StreamReader::new(body.into_data_stream().map_err(io::Error::other));
    let tracked = spawn_tracked(s.ctx.clone(), async move {
        restore_upload(&ctx, input, &password).await
    });
    match tracked.await {
        Ok(Ok(())) => match s.ctx.restore.lock().await.clone() {
            Some(done) => Json(done).into_response(),
            None => StatusCode::NO_CONTENT.into_response(),
        },
        Ok(Err(e)) => refused(e),
        Err(_) => refused(BackupError::Io("the restore stopped".into())),
    }
}

async fn progress(State(s): State<RouteState>) -> Response {
    if let Err(r) = waiting(&s.ctx) {
        return r;
    }
    match s.ctx.restore.lock().await.clone() {
        Some(p) => Json(p).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}
