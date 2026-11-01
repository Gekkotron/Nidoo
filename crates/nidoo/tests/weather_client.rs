//! The Open-Meteo client against a local stub server, and the view built from its responses.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use nidoo::weather::open_meteo::{
    Endpoints, FetchError, Forecast, Place, build_view, client, forecast, geocode, geocode_many,
};
use nidoo_api::WeatherCondition;

const GEOCODING: &str = include_str!("fixtures/weather/geocoding_paris.json");
const FORECAST: &str = include_str!("fixtures/weather/forecast_paris.json");
const NULLS: &str = include_str!("fixtures/weather/forecast_nulls.json");
const LATE: &str = include_str!("fixtures/weather/forecast_late_evening.json");

#[derive(Clone)]
struct Stub {
    status: u16,
    body: String,
    seen: Arc<Mutex<Vec<HashMap<String, String>>>>,
}

async fn answer(State(s): State<Stub>, Query(q): Query<HashMap<String, String>>) -> Response {
    s.seen.lock().unwrap().push(q);
    (
        StatusCode::from_u16(s.status).unwrap(),
        [("content-type", "application/json")],
        s.body.clone(),
    )
        .into_response()
}

/// A stub answering every request on both routes with `body`; returns the endpoints and the
/// decoded query parameters of each request received.
async fn stub(status: u16, body: &str) -> (Endpoints, Arc<Mutex<Vec<HashMap<String, String>>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let state = Stub {
        status,
        body: body.to_string(),
        seen: seen.clone(),
    };
    let app = Router::new()
        .route("/geocode", get(answer))
        .route("/forecast", get(answer))
        .with_state(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (
        Endpoints {
            geocoding: format!("{base}/geocode"),
            forecast: format!("{base}/forecast"),
        },
        seen,
    )
}

fn paris() -> Place {
    Place {
        name: "Paris".into(),
        admin: Some("Île-de-France".into()),
        country: Some("FR".into()),
        latitude: 48.85341,
        longitude: 2.3488,
        timezone: Some("Europe/Paris".into()),
    }
}

fn parse(json: &str) -> Forecast {
    serde_json::from_str(json).unwrap()
}

fn close(a: Option<f64>, b: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() < 1e-9)
}

#[tokio::test]
async fn geocode_picks_the_first_result_and_sends_country() {
    let (ep, seen) = stub(200, GEOCODING).await;
    let place = geocode(&client().unwrap(), &ep, "Paris", Some("FR"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(place.name, "Paris");
    assert_eq!(place.country.as_deref(), Some("FR"));
    assert_eq!(place.admin.as_deref(), Some("Île-de-France"));
    assert_eq!(place.latitude, 48.85341);
    let q = &seen.lock().unwrap()[0];
    assert_eq!(q["name"], "Paris");
    assert_eq!(q["countryCode"], "FR");
    assert_eq!(q["count"], "10");
    assert_eq!(q["format"], "json");
}

#[tokio::test]
async fn geocode_reads_the_timezone() {
    let (ep, _) = stub(200, GEOCODING).await;
    let place = geocode(&client().unwrap(), &ep, "Paris", None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(place.timezone.as_deref(), Some("Europe/Paris"));
    assert_eq!(place, paris());
}

#[tokio::test]
async fn geocode_many_sends_the_count_and_returns_every_place() {
    let two = r#"{"results":[
      {"name":"Lyon","latitude":45.74,"longitude":4.84,"country_code":"FR","admin1":"Auvergne-Rhone-Alpes","timezone":"Europe/Paris"},
      {"name":"Lyon","latitude":42.0,"longitude":-71.0,"country_code":"US"}]}"#;
    let (ep, seen) = stub(200, two).await;
    let places = geocode_many(&client().unwrap(), &ep, "Lyon", None, 5)
        .await
        .unwrap();
    assert_eq!(places.len(), 2);
    assert_eq!(places[0].timezone.as_deref(), Some("Europe/Paris"));
    assert_eq!(places[1].timezone, None);
    assert_eq!(places[1].country.as_deref(), Some("US"));
    assert_eq!(seen.lock().unwrap()[0]["count"], "5");
}

#[tokio::test]
async fn geocode_without_results_is_none() {
    let (ep, _) = stub(200, r#"{"generationtime_ms":0.1}"#).await;
    assert_eq!(
        geocode(&client().unwrap(), &ep, "Nowhereville", None)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn geocode_sends_the_city_url_encoded() {
    let (ep, seen) = stub(200, GEOCODING).await;
    geocode(&client().unwrap(), &ep, "Saint-Étienne", None)
        .await
        .unwrap();
    let q = &seen.lock().unwrap()[0];
    assert_eq!(q["name"], "Saint-Étienne");
    assert!(!q.contains_key("countryCode"));
}

#[tokio::test]
async fn http_error_is_fetch_error() {
    let (ep, _) = stub(500, "{}").await;
    let err = geocode(&client().unwrap(), &ep, "Paris", None)
        .await
        .unwrap_err();
    assert!(matches!(err, FetchError::Http(_)), "{err:?}");
    let err = forecast(&client().unwrap(), &ep, &paris())
        .await
        .unwrap_err();
    assert!(matches!(err, FetchError::Http(_)), "{err:?}");
}

#[tokio::test]
async fn undecodable_body_is_a_decode_error() {
    let (ep, _) = stub(200, "not json").await;
    let err = forecast(&client().unwrap(), &ep, &paris())
        .await
        .unwrap_err();
    assert!(matches!(err, FetchError::Decode(_)), "{err:?}");
}

#[tokio::test]
async fn forecast_requests_the_needed_variables() {
    let (ep, seen) = stub(200, FORECAST).await;
    let f = forecast(&client().unwrap(), &ep, &paris()).await.unwrap();
    assert_eq!(f.daily.time.len(), 6);
    let q = &seen.lock().unwrap()[0];
    assert_eq!(q["latitude"], "48.85341");
    assert_eq!(q["longitude"], "2.3488");
    assert_eq!(q["timezone"], "auto");
    assert_eq!(q["forecast_days"], "6");
    let has = |key: &str, names: &str| {
        let got: Vec<&str> = q[key].split(',').collect();
        for n in names.split(',') {
            assert!(got.contains(&n), "{key} lacks {n}: {got:?}");
        }
    };
    has(
        "current",
        "temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,is_day,cloud_cover,precipitation,wind_speed_10m,wind_gusts_10m,wind_direction_10m,pressure_msl",
    );
    has(
        "hourly",
        "temperature_2m,uv_index,relative_humidity_2m,cloud_cover,pressure_msl",
    );
    has("minutely_15", "precipitation");
    has(
        "daily",
        "weather_code,temperature_2m_max,temperature_2m_min,precipitation_sum,precipitation_probability_max,wind_gusts_10m_max,apparent_temperature_max,wind_speed_10m_max,wind_direction_10m_dominant,uv_index_max",
    );
}

#[test]
fn build_view_maps_current_today_and_days() {
    let v = build_view(&parse(FORECAST), &paris(), 1_792_000_000);
    assert_eq!(v.place.name, "Paris");
    assert_eq!(v.place.admin.as_deref(), Some("Île-de-France"));
    assert_eq!(v.place.country.as_deref(), Some("FR"));
    assert_eq!(v.fetched_at, 1_792_000_000);
    assert!(!v.stale);
    let c = &v.current;
    assert_eq!(c.temperature, Some(14.2));
    assert_eq!(c.apparent_temperature, Some(12.6));
    assert_eq!(c.humidity, Some(71.0));
    assert_eq!(c.cloud_cover, Some(100.0));
    assert_eq!(c.precipitation, Some(0.0));
    assert_eq!(c.wind_speed, Some(14.4));
    assert_eq!(c.wind_gusts, Some(28.1));
    assert_eq!(c.wind_direction, Some(230.0));
    assert_eq!(c.pressure, Some(1012.4));
    assert_eq!(c.condition, Some(WeatherCondition::Cloudy));
    assert_eq!(c.daylight, Some(true));
    assert_eq!(v.today.min, Some(9.7));
    assert_eq!(v.today.max, Some(15.8));
    assert_eq!(v.today.precipitation, Some(0.2));
    assert_eq!(v.today.precipitation_probability, Some(20.0));
    assert_eq!(v.today.wind_gusts_max, Some(34.6));
    assert_eq!(v.days.len(), 5);
    assert_eq!(v.days[0].date, "2026-10-24");
    assert_eq!(v.days[0].condition, Some(WeatherCondition::Rain));
    assert_eq!(v.days[0].min, Some(10.2));
    assert_eq!(v.days[0].max, Some(14.1));
    assert_eq!(v.days[0].precipitation_probability, Some(85.0));
    assert_eq!(v.days[0].apparent_max, Some(12.5));
    assert_eq!(v.days[0].precipitation, Some(4.6));
    assert_eq!(v.days[0].wind_speed_max, Some(25.4));
    assert_eq!(v.days[0].wind_gusts_max, Some(41.0));
    assert_eq!(v.days[0].wind_direction, Some(250.0));
    assert_eq!(v.days[0].uv_index_max, Some(1.8));
    assert_eq!(v.days[4].date, "2026-10-28");
    assert_eq!(v.days[4].wind_gusts_max, Some(19.8));
}

#[test]
fn day_means_cover_the_hours_of_that_date() {
    let v = build_view(&parse(FORECAST), &paris(), 0);
    // 2026-10-24: the humidity of 05:00 is missing, so the mean runs over 23 values.
    assert_eq!(v.days[0].humidity, Some(71.8));
    assert_eq!(v.days[0].cloud_cover, Some(75.0));
    assert_eq!(v.days[0].pressure, Some(1005.8));
    // The fixture has no hourly values for 2026-10-25 onward.
    assert_eq!(v.days[1].humidity, None);
    assert_eq!(v.days[1].cloud_cover, None);
    assert_eq!(v.days[1].pressure, None);
}

#[test]
fn rain_next_hour_sums_four_slots_after_now() {
    // `current.time` is 14:15 and a slot holds the precipitation of the 15 minutes before its
    // label, so the 14:15 slot (0.1) is past. 14:30 .. 15:15 are 0.2 + 0.3 + 0.4 + 0.0.
    let v = build_view(&parse(FORECAST), &paris(), 0);
    assert!(close(v.rain_next_hour, 0.9), "{:?}", v.rain_next_hour);
}

#[test]
fn uv_index_comes_from_the_current_hour() {
    let f = parse(FORECAST);
    let slot = f.hourly.time.iter().position(|t| t == "2026-10-23T14:00");
    let want = f.hourly.uv_index.as_ref().unwrap()[slot.unwrap()];
    assert!(want.is_some());
    let v = build_view(&f, &paris(), 0);
    assert_eq!(v.current.uv_index, want);
}

#[test]
fn build_view_tolerates_nulls() {
    let v = build_view(&parse(NULLS), &paris(), 0);
    assert_eq!(v.rain_next_hour, None);
    assert_eq!(v.current.uv_index, None);
    assert_eq!(v.current.temperature, Some(14.2));
    assert_eq!(v.today.max, Some(15.8));
    assert_eq!(v.days[0].humidity, None);
    assert_eq!(v.days[0].uv_index_max, None);
    assert!(v.tonight_min.is_some());
}

#[test]
fn tonight_min_near_midnight_uses_local_time() {
    // Now is 23:30; the window runs to 09:00 tomorrow. The 06:00 value (4.1) is the lowest in
    // it; the 10:00 value (2.0) lies outside.
    let v = build_view(&parse(LATE), &paris(), 0);
    assert_eq!(v.tonight_min, Some(4.1));
}

#[test]
fn tonight_min_in_the_afternoon_spans_to_the_next_morning() {
    let v = build_view(&parse(FORECAST), &paris(), 0);
    // 14:00 today .. 08:00 tomorrow; the fixture curve bottoms out at hour 0.
    assert_eq!(v.tonight_min, Some(10.0));
}

#[test]
fn unknown_weather_code_falls_back_to_cloudy() {
    let mut f = parse(FORECAST);
    f.current.weather_code = Some(42.0);
    let v = build_view(&f, &paris(), 0);
    assert_eq!(v.current.condition, Some(WeatherCondition::Cloudy));
}
