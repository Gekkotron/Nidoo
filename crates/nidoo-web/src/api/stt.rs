//! The speech input routes: status, settings, the model download and the satellites' key.

use gloo_net::http::Request;
use nidoo_api::{ApiError, DownloadView, SttKey, SttSettings, SttStatus};

use super::core::{check, read_json};

const BASE: &str = "/api/v1/assist/stt";

pub async fn get_status() -> Result<SttStatus, ApiError> {
    let response = check(Request::get(&format!("{BASE}/status")).build(), None).await?;
    read_json(response).await
}

pub async fn put_settings(settings: &SttSettings) -> Result<SttStatus, ApiError> {
    let response = check(
        Request::put(&format!("{BASE}/settings")).json(settings),
        None,
    )
    .await?;
    read_json(response).await
}

pub async fn start_download() -> Result<DownloadView, ApiError> {
    let response = check(Request::post(&format!("{BASE}/download")).build(), None).await?;
    read_json(response).await
}

pub async fn cancel_download() -> Result<(), ApiError> {
    check(Request::delete(&format!("{BASE}/download")).build(), None).await?;
    Ok(())
}

pub async fn remove_model() -> Result<(), ApiError> {
    check(Request::delete(&format!("{BASE}/model")).build(), None).await?;
    Ok(())
}

/// A new key; the old one stops working.
pub async fn new_key() -> Result<SttKey, ApiError> {
    let response = check(Request::post(&format!("{BASE}/key")).build(), None).await?;
    read_json(response).await
}
