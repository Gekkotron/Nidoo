//! The climate engine's REST routes. A `503` means the engine is disabled on the hub.

use gloo_net::http::Request;
use nidoo_api::ApiError;
use nidoo_climate::doc::{HouseSettings, Rules, ZoneSettings};
pub use nidoo_climate::view::OverrideRequest as OverrideBody;
use nidoo_climate::view::{ClimateView, DecisionLog, LearnStatus, RoomEcoRequest};

use super::core::{WebError, check, encode_id, read_json};

/// What a 503 means here: `[climate] enabled` is off on the hub. The nav hides its link on it.
pub const CLIMATE_DISABLED: WebError = WebError::ClimateDisabled;

/// True when `e` is the 503 above, whichever language its text was built in.
pub fn is_climate_disabled(e: &ApiError) -> bool {
    use crate::i18n::{Lang, errors::web_error_text};
    [Lang::En, Lang::Fr]
        .iter()
        .any(|&l| e.error == web_error_text(l, &CLIMATE_DISABLED))
}

fn zone_url(zone: &str, tail: &str) -> String {
    format!("/api/v1/climate/zones/{}{tail}", encode_id(zone))
}

/// The whole engine: house settings, zones with their readings, decisions and proposals.
pub async fn get_climate() -> Result<ClimateView, ApiError> {
    let response = check(
        Request::get("/api/v1/climate").build(),
        Some(CLIMATE_DISABLED),
    )
    .await?;
    read_json(response).await
}

pub async fn put_house(settings: &HouseSettings) -> Result<(), ApiError> {
    check(
        Request::put("/api/v1/climate/house").json(settings),
        Some(CLIMATE_DISABLED),
    )
    .await?;
    Ok(())
}

pub async fn put_zone(zone: &str, settings: &ZoneSettings) -> Result<(), ApiError> {
    check(
        Request::put(&zone_url(zone, "")).json(settings),
        Some(CLIMATE_DISABLED),
    )
    .await?;
    Ok(())
}

pub async fn set_override(zone: &str, body: OverrideBody) -> Result<(), ApiError> {
    check(
        Request::post(&zone_url(zone, "/override")).json(&body),
        Some(CLIMATE_DISABLED),
    )
    .await?;
    Ok(())
}

pub async fn clear_override(zone: &str) -> Result<(), ApiError> {
    check(
        Request::delete(&zone_url(zone, "/override")).build(),
        Some(CLIMATE_DISABLED),
    )
    .await?;
    Ok(())
}

/// Puts the zone in eco until `until` (Unix seconds).
pub async fn set_eco(zone: &str, until: u64) -> Result<(), ApiError> {
    check(
        Request::put(&zone_url(zone, "/eco")).json(&RoomEcoRequest { until }),
        Some(CLIMATE_DISABLED),
    )
    .await?;
    Ok(())
}

/// Ends the zone's eco.
pub async fn clear_eco(zone: &str) -> Result<(), ApiError> {
    check(
        Request::delete(&zone_url(zone, "/eco")).build(),
        Some(CLIMATE_DISABLED),
    )
    .await?;
    Ok(())
}

/// Approves the zone's proposal, as learned or, with `rules`, as edited.
pub async fn approve(zone: &str, rules: Option<&Rules>) -> Result<(), ApiError> {
    let url = zone_url(zone, "/proposal/approve");
    let request = match rules {
        Some(rules) => Request::post(&url).json(rules),
        None => Request::post(&url).build(),
    };
    check(request, Some(CLIMATE_DISABLED)).await?;
    Ok(())
}

pub async fn reject(zone: &str) -> Result<(), ApiError> {
    check(
        Request::post(&zone_url(zone, "/proposal/reject")).build(),
        Some(CLIMATE_DISABLED),
    )
    .await?;
    Ok(())
}

/// Runs the learner for the zone now.
pub async fn learn(zone: &str) -> Result<LearnStatus, ApiError> {
    let response = check(
        Request::post(&zone_url(zone, "/learn")).build(),
        Some(CLIMATE_DISABLED),
    )
    .await?;
    read_json(response).await
}

/// The zone's decisions: the recent ones, then older rows from the history database.
pub async fn decisions(zone: &str, limit: u32) -> Result<DecisionLog, ApiError> {
    let url = zone_url(zone, &format!("/decisions?limit={limit}"));
    let response = check(Request::get(&url).build(), Some(CLIMATE_DISABLED)).await?;
    read_json(response).await
}
