//! The Open-Meteo client: geocode a city, fetch its forecast, and fold the response into a
//! [`WeatherView`]. Times in a response are the place's local time, and are compared as such.

use std::time::Duration;

use chrono::{Duration as Span, NaiveDateTime, NaiveTime, Timelike};
use nidoo_api::{
    WeatherCondition, WeatherCurrent, WeatherDay, WeatherPlace, WeatherToday, WeatherView,
};
use serde::Deserialize;

/// The format of every time Open-Meteo reports (local, minute precision).
const TIME_FORMAT: &str = "%Y-%m-%dT%H:%M";

/// The forecast days requested: today plus the five days the view carries.
const FORECAST_DAYS: &str = "6";

/// Where the two Open-Meteo APIs live. Tests point these at a stub server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    pub geocoding: String,
    pub forecast: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            geocoding: "https://geocoding-api.open-meteo.com/v1/search".into(),
            forecast: "https://api.open-meteo.com/v1/forecast".into(),
        }
    }
}

/// A geocoded place.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub name: String,
    /// The first-level administrative area (a region or a state).
    pub admin: Option<String>,
    /// The ISO country code.
    pub country: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    /// The IANA time zone, such as `Europe/Paris`.
    pub timezone: Option<String>,
}

impl Place {
    /// The place as the weather view names it.
    pub fn view(&self) -> WeatherPlace {
        WeatherPlace {
            name: self.name.clone(),
            admin: self.admin.clone(),
            country: self.country.clone(),
        }
    }
}

/// Why a request to Open-Meteo failed.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// The request failed or the server answered with an error status.
    #[error("weather request failed: {0}")]
    Http(String),
    /// The response was not the JSON that was expected.
    #[error("weather response not understood: {0}")]
    Decode(String),
}

/// An HTTP client for Open-Meteo: 5 s to connect, 15 s in all.
pub fn client() -> Result<reqwest::Client, FetchError> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("nidoo/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| FetchError::Http(e.without_url().to_string()))
}

#[derive(Deserialize)]
struct Geocoding {
    #[serde(default)]
    results: Vec<GeoResult>,
}

#[derive(Deserialize)]
struct GeoResult {
    name: String,
    latitude: f64,
    longitude: f64,
    country_code: Option<String>,
    admin1: Option<String>,
    timezone: Option<String>,
}

/// Looks `city` up, optionally within an ISO `country` code. `Ok(None)` means nothing matched.
pub async fn geocode(
    client: &reqwest::Client,
    ep: &Endpoints,
    city: &str,
    country: Option<&str>,
) -> Result<Option<Place>, FetchError> {
    Ok(search(client, ep, city, country, 10)
        .await?
        .into_iter()
        .next())
}

/// Looks `city` up and returns up to `count` places, best match first.
pub async fn geocode_many(
    client: &reqwest::Client,
    ep: &Endpoints,
    city: &str,
    country: Option<&str>,
    count: u8,
) -> Result<Vec<Place>, FetchError> {
    let mut places = search(client, ep, city, country, count).await?;
    places.truncate(usize::from(count));
    Ok(places)
}

async fn search(
    client: &reqwest::Client,
    ep: &Endpoints,
    city: &str,
    country: Option<&str>,
    count: u8,
) -> Result<Vec<Place>, FetchError> {
    let count = count.to_string();
    let mut query = vec![
        ("name", city),
        ("count", count.as_str()),
        ("format", "json"),
    ];
    if let Some(code) = country {
        query.push(("countryCode", code));
    }
    let found: Geocoding = get_json(client.get(&ep.geocoding).query(&query)).await?;
    Ok(found
        .results
        .into_iter()
        .map(|r| Place {
            name: r.name,
            admin: r.admin1,
            country: r.country_code,
            latitude: r.latitude,
            longitude: r.longitude,
            timezone: r.timezone,
        })
        .collect())
}

/// The forecast response, as far as the view needs it. Missing numbers are `None`.
#[derive(Clone, Debug, Deserialize)]
pub struct Forecast {
    pub current: ForecastCurrent,
    #[serde(default)]
    pub minutely_15: Minutely,
    #[serde(default)]
    pub hourly: Hourly,
    #[serde(default)]
    pub daily: Daily,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ForecastCurrent {
    /// Local time, `YYYY-MM-DDTHH:MM`.
    pub time: String,
    pub temperature_2m: Option<f64>,
    pub apparent_temperature: Option<f64>,
    pub relative_humidity_2m: Option<f64>,
    pub weather_code: Option<f64>,
    /// 1 by day, 0 by night.
    pub is_day: Option<f64>,
    pub cloud_cover: Option<f64>,
    pub precipitation: Option<f64>,
    pub wind_speed_10m: Option<f64>,
    pub wind_gusts_10m: Option<f64>,
    pub wind_direction_10m: Option<f64>,
    pub pressure_msl: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Minutely {
    #[serde(default)]
    pub time: Vec<String>,
    #[serde(default)]
    pub precipitation: Option<Vec<Option<f64>>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Hourly {
    #[serde(default)]
    pub time: Vec<String>,
    #[serde(default)]
    pub temperature_2m: Option<Vec<Option<f64>>>,
    #[serde(default)]
    pub uv_index: Option<Vec<Option<f64>>>,
    #[serde(default)]
    pub relative_humidity_2m: Option<Vec<Option<f64>>>,
    #[serde(default)]
    pub cloud_cover: Option<Vec<Option<f64>>>,
    #[serde(default)]
    pub pressure_msl: Option<Vec<Option<f64>>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Daily {
    #[serde(default)]
    pub time: Vec<String>,
    #[serde(default)]
    pub weather_code: Option<Vec<Option<f64>>>,
    #[serde(default)]
    pub temperature_2m_max: Option<Vec<Option<f64>>>,
    #[serde(default)]
    pub temperature_2m_min: Option<Vec<Option<f64>>>,
    #[serde(default)]
    pub precipitation_sum: Option<Vec<Option<f64>>>,
    #[serde(default)]
    pub precipitation_probability_max: Option<Vec<Option<f64>>>,
    #[serde(default)]
    pub wind_gusts_10m_max: Option<Vec<Option<f64>>>,
    #[serde(default)]
    pub apparent_temperature_max: Option<Vec<Option<f64>>>,
    #[serde(default)]
    pub wind_speed_10m_max: Option<Vec<Option<f64>>>,
    #[serde(default)]
    pub wind_direction_10m_dominant: Option<Vec<Option<f64>>>,
    #[serde(default)]
    pub uv_index_max: Option<Vec<Option<f64>>>,
}

/// Fetches the forecast for `place`: now, the next hours, and the next five days (daily figures, plus hourly ones the view averages per day).
pub async fn forecast(
    client: &reqwest::Client,
    ep: &Endpoints,
    place: &Place,
) -> Result<Forecast, FetchError> {
    let latitude = place.latitude.to_string();
    let longitude = place.longitude.to_string();
    let query = [
        ("latitude", latitude.as_str()),
        ("longitude", longitude.as_str()),
        ("timezone", "auto"),
        ("forecast_days", FORECAST_DAYS),
        (
            "current",
            "temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,is_day,cloud_cover,precipitation,wind_speed_10m,wind_gusts_10m,wind_direction_10m,pressure_msl",
        ),
        (
            "hourly",
            "temperature_2m,uv_index,relative_humidity_2m,cloud_cover,pressure_msl",
        ),
        ("minutely_15", "precipitation"),
        (
            "daily",
            "weather_code,temperature_2m_max,temperature_2m_min,precipitation_sum,precipitation_probability_max,wind_gusts_10m_max,apparent_temperature_max,wind_speed_10m_max,wind_direction_10m_dominant,uv_index_max",
        ),
    ];
    get_json(client.get(&ep.forecast).query(&query)).await
}

async fn get_json<T: serde::de::DeserializeOwned>(
    request: reqwest::RequestBuilder,
) -> Result<T, FetchError> {
    let response = request
        .send()
        .await
        .map_err(|e| FetchError::Http(e.without_url().to_string()))?
        .error_for_status()
        .map_err(|e| FetchError::Http(e.without_url().to_string()))?;
    response
        .json()
        .await
        .map_err(|e| FetchError::Decode(e.without_url().to_string()))
}

/// Folds a forecast into the document the hub serves. `fetched_at` is Unix seconds; `stale` is
/// left `false` for the hub to set when it serves old data.
pub fn build_view(forecast: &Forecast, place: &Place, fetched_at: u64) -> WeatherView {
    let cur = &forecast.current;
    let now = parse_time(&cur.time);
    let daily = &forecast.daily;
    let day = |i: usize| {
        let date = daily.time.get(i).cloned().unwrap_or_default();
        let hourly = &forecast.hourly;
        WeatherDay {
            condition: condition(at(&daily.weather_code, i)),
            min: at(&daily.temperature_2m_min, i),
            max: at(&daily.temperature_2m_max, i),
            precipitation_probability: at(&daily.precipitation_probability_max, i),
            apparent_max: at(&daily.apparent_temperature_max, i),
            precipitation: at(&daily.precipitation_sum, i),
            wind_speed_max: at(&daily.wind_speed_10m_max, i),
            wind_gusts_max: at(&daily.wind_gusts_10m_max, i),
            wind_direction: at(&daily.wind_direction_10m_dominant, i),
            humidity: day_mean(hourly, &hourly.relative_humidity_2m, &date),
            pressure: day_mean(hourly, &hourly.pressure_msl, &date),
            cloud_cover: day_mean(hourly, &hourly.cloud_cover, &date),
            uv_index_max: at(&daily.uv_index_max, i),
            date,
        }
    };
    WeatherView {
        place: place.view(),
        fetched_at,
        stale: false,
        current: WeatherCurrent {
            temperature: cur.temperature_2m,
            apparent_temperature: cur.apparent_temperature,
            humidity: cur.relative_humidity_2m,
            cloud_cover: cur.cloud_cover,
            precipitation: cur.precipitation,
            wind_speed: cur.wind_speed_10m,
            wind_gusts: cur.wind_gusts_10m,
            wind_direction: cur.wind_direction_10m,
            pressure: cur.pressure_msl,
            uv_index: now.and_then(|now| uv_index(forecast, now)),
            condition: condition(cur.weather_code),
            daylight: cur.is_day.map(|d| d >= 0.5),
        },
        today: WeatherToday {
            min: at(&daily.temperature_2m_min, 0),
            max: at(&daily.temperature_2m_max, 0),
            precipitation: at(&daily.precipitation_sum, 0),
            precipitation_probability: at(&daily.precipitation_probability_max, 0),
            wind_gusts_max: at(&daily.wind_gusts_10m_max, 0),
        },
        rain_next_hour: now.and_then(|now| rain_next_hour(forecast, now)),
        tonight_min: now.and_then(|now| tonight_min(forecast, now)),
        days: (1..daily.time.len()).map(day).collect(),
    }
}

fn parse_time(t: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(t, TIME_FORMAT).ok()
}

fn at(values: &Option<Vec<Option<f64>>>, i: usize) -> Option<f64> {
    values.as_ref()?.get(i).copied().flatten()
}

/// The mean of the hourly `values` whose local date is `date` (`YYYY-MM-DD`), to one decimal.
/// Missing values are ignored; `None` when none is left.
fn day_mean(hourly: &Hourly, values: &Option<Vec<Option<f64>>>, date: &str) -> Option<f64> {
    let (sum, count) = hourly
        .time
        .iter()
        .enumerate()
        .filter(|(_, t)| t.starts_with(date))
        .filter_map(|(i, _)| at(values, i))
        .fold((0.0, 0u32), |(sum, count), v| (sum + v, count + 1));
    (count > 0).then(|| (sum / f64::from(count) * 10.0).round() / 10.0)
}

/// The condition for a WMO code; a code the table lacks is logged and shown as cloudy.
fn condition(code: Option<f64>) -> Option<WeatherCondition> {
    let code = code? as u16;
    Some(WeatherCondition::try_from_wmo(code).unwrap_or_else(|| {
        tracing::debug!(code, "unknown WMO weather code");
        WeatherCondition::Cloudy
    }))
}

/// The UV index of the hour `now` falls in.
fn uv_index(forecast: &Forecast, now: NaiveDateTime) -> Option<f64> {
    let hour = now.with_minute(0)?;
    let slot = forecast
        .hourly
        .time
        .iter()
        .position(|t| parse_time(t) == Some(hour))?;
    at(&forecast.hourly.uv_index, slot)
}

/// The mm expected over the first four quarter-hour slots after `now`. Open-Meteo labels a
/// slot with its end ("Preceding 15 minutes sum"), so the slot at `now` is already past.
fn rain_next_hour(forecast: &Forecast, now: NaiveDateTime) -> Option<f64> {
    let slots = &forecast.minutely_15;
    let values: Vec<f64> = slots
        .time
        .iter()
        .enumerate()
        .filter(|(_, t)| parse_time(t).is_some_and(|t| t > now))
        .take(4)
        .filter_map(|(i, _)| at(&slots.precipitation, i))
        .collect();
    (!values.is_empty()).then(|| values.iter().sum())
}

/// The lowest hourly temperature from the current hour until the next 09:00.
fn tonight_min(forecast: &Forecast, now: NaiveDateTime) -> Option<f64> {
    let from = now.with_minute(0)?;
    let nine = NaiveTime::from_hms_opt(9, 0, 0)?;
    let mut until = now.date().and_time(nine);
    if until <= now {
        until += Span::days(1);
    }
    forecast
        .hourly
        .time
        .iter()
        .enumerate()
        .filter(|(_, t)| parse_time(t).is_some_and(|t| from <= t && t < until))
        .filter_map(|(i, _)| at(&forecast.hourly.temperature_2m, i))
        .min_by(|a, b| a.total_cmp(b))
}
