//! The ambiance routes. A `422` (the hub's text names what it refused), `503` (the publish
//! failed) or `404` comes back as an `ApiError` carrying the hub's own message.

use gloo_net::http::Request;
use nidoo_api::{Ambiance, AmbianceView, ApiError, ApplyReport};

use super::core::{check, encode_id, read_json};

/// Every stored ambiance, the unreadable ones included with their reason.
pub async fn list_ambiances() -> Result<Vec<AmbianceView>, ApiError> {
    let response = check(Request::get("/api/v1/ambiances").build(), None).await?;
    read_json(response).await
}

/// Creates or replaces ambiance `a.id`; the reply is it as the hub now has it.
pub async fn put_ambiance(a: &Ambiance) -> Result<AmbianceView, ApiError> {
    let url = format!("/api/v1/ambiances/{}", encode_id(&a.id));
    let response = check(Request::put(&url).json(a), None).await?;
    read_json(response).await
}

/// Deletes ambiance `id` on the hub.
pub async fn delete_ambiance(id: &str) -> Result<(), ApiError> {
    let url = format!("/api/v1/ambiances/{}", encode_id(id));
    check(Request::delete(&url).build(), None).await?;
    Ok(())
}

/// Applies the saved version of ambiance `id` and returns what each action did.
pub async fn apply_ambiance(id: &str) -> Result<ApplyReport, ApiError> {
    let url = format!("/api/v1/ambiances/{}/apply", encode_id(id));
    let response = check(Request::post(&url).build(), None).await?;
    read_json(response).await
}
