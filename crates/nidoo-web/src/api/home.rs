//! The home header's REST routes: the weather, the solar forecast and the home settings.

use gloo_net::http::Request;
use nidoo_api::{ApiError, HomeSettings, SolarView, WeatherView};

use super::core::{check, check_unless_503, read_json};

/// The current weather; `Ok(None)` while the hub has none (weather off, or not fetched yet: 503).
pub async fn get_weather() -> Result<Option<WeatherView>, ApiError> {
    match check_unless_503(Request::get("/api/v1/weather").build()).await? {
        Some(response) => read_json(response).await.map(Some),
        None => Ok(None),
    }
}

/// The solar forecast; `Ok(None)` while the hub has none (solar off, or not fetched yet: 503).
pub async fn get_solar() -> Result<Option<SolarView>, ApiError> {
    match check_unless_503(Request::get("/api/v1/solar").build()).await? {
        Some(response) => read_json(response).await.map(Some),
        None => Ok(None),
    }
}

/// The home settings document.
pub async fn get_home_settings() -> Result<HomeSettings, ApiError> {
    let response = check(Request::get("/api/v1/home/settings").build(), None).await?;
    read_json(response).await
}

/// Replaces the whole settings document; the reply is it as stored. A 422 comes back as an
/// `ApiError` whose text names the field.
pub async fn put_home_settings(settings: &HomeSettings) -> Result<HomeSettings, ApiError> {
    let response = check(Request::put("/api/v1/home/settings").json(settings), None).await?;
    read_json(response).await
}
