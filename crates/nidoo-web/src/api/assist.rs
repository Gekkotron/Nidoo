//! The assistant's REST routes.

use gloo_net::http::Request;
use nidoo_api::ApiError;
use nidoo_assist::view::{AssistRequest, AssistTurn, AssistTurns};

use super::core::{check, encode_id, read_json};

/// Sends a sentence or the answer to a pending question; the reply is the turn.
pub async fn post_assist(req: &AssistRequest) -> Result<AssistTurn, ApiError> {
    let response = check(Request::post("/api/v1/assist").json(req), None).await?;
    read_json(response).await
}

/// The recent turns, newest first, and whether the hub keeps them.
pub async fn get_turns(limit: u32) -> Result<AssistTurns, ApiError> {
    let url = format!("/api/v1/assist/turns?limit={limit}");
    let response = check(Request::get(&url).build(), None).await?;
    read_json(response).await
}

/// Marks a turn as wrongly understood.
pub async fn mark_wrong(id: &str) -> Result<(), ApiError> {
    let url = format!("/api/v1/assist/turns/{}/wrong", encode_id(id));
    check(Request::post(&url).build(), None).await?;
    Ok(())
}
