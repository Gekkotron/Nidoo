//! The native alarm's REST routes. The alarm always answers; it just idles while disabled.

use gloo_net::http::Request;
use nidoo_alarm::doc::AlarmSettings;
use nidoo_alarm::view::AlarmView;
use nidoo_api::ApiError;

use super::core::{check, read_json};

/// The alarm as one document: settings, state and warnings.
pub async fn get_alarm() -> Result<AlarmView, ApiError> {
    let response = check(Request::get("/api/v1/alarm").build(), None).await?;
    read_json(response).await
}

/// Replaces the settings. A `422` carries the reason the hub refused them.
pub async fn put_alarm_settings(settings: &AlarmSettings) -> Result<(), ApiError> {
    check(Request::put("/api/v1/alarm/settings").json(settings), None).await?;
    Ok(())
}

/// Sounds the chirp of the saved settings' siren number `siren`.
pub async fn test_chirp(siren: usize) -> Result<(), ApiError> {
    check(
        Request::post("/api/v1/alarm/test-chirp").json(&serde_json::json!({ "siren": siren })),
        None,
    )
    .await?;
    Ok(())
}
