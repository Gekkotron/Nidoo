//! The weather poller against a local stub of Open-Meteo, with the hub served on an ephemeral
//! port. No test reaches the real network.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use nidoo::config::Config;
use nidoo::http::router;
use nidoo::hub::Hub;
use nidoo::weather::open_meteo::Endpoints;
use nidoo::weather::poller::spawn_with;
use nidoo_api::{ConnectionStatus, WeatherView};
use nidoo_core::command::Publish;
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::StreamableHttpClientTransport;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

const GEOCODING: &str = include_str!("fixtures/weather/geocoding_paris.json");
const FORECAST: &str = include_str!("fixtures/weather/forecast_paris.json");
const NO_RESULTS: &str = "{}";

/// What the stub serves and how often it was asked.
struct Stub {
    geocoding: &'static str,
    /// The forecast requests answered with 200 before the 500s start (`usize::MAX`: never).
    good_forecasts: usize,
    geocoding_calls: AtomicUsize,
    forecast_calls: AtomicUsize,
    /// The query parameters of each forecast request.
    forecast_queries: std::sync::Mutex<Vec<HashMap<String, String>>>,
}

impl Stub {
    fn new(geocoding: &'static str, good_forecasts: usize) -> Arc<Stub> {
        Arc::new(Stub {
            geocoding,
            good_forecasts,
            geocoding_calls: AtomicUsize::new(0),
            forecast_calls: AtomicUsize::new(0),
            forecast_queries: std::sync::Mutex::default(),
        })
    }

    fn requests(&self) -> usize {
        self.geocoding_calls.load(Ordering::SeqCst) + self.forecast_calls.load(Ordering::SeqCst)
    }
}

fn json(body: &'static str) -> Response {
    ([("content-type", "application/json")], body).into_response()
}

async fn geocode(State(s): State<Arc<Stub>>) -> Response {
    s.geocoding_calls.fetch_add(1, Ordering::SeqCst);
    json(s.geocoding)
}

async fn forecast(
    State(s): State<Arc<Stub>>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    s.forecast_queries.lock().unwrap().push(q);
    let n = s.forecast_calls.fetch_add(1, Ordering::SeqCst);
    if n < s.good_forecasts {
        json(FORECAST)
    } else {
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    }
}

async fn serve_stub(stub: Arc<Stub>) -> Endpoints {
    let app = Router::new()
        .route("/geocode", get(geocode))
        .route("/forecast", get(forecast))
        .with_state(stub);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Endpoints {
        geocoding: format!("{base}/geocode"),
        forecast: format!("{base}/forecast"),
    }
}

struct App {
    hub: Arc<Hub>,
    addr: SocketAddr,
    /// What the hub published, in order, as `(topic, payload)`; also fed back into the hub.
    published: Arc<std::sync::Mutex<Vec<(String, String)>>>,
}

async fn app(enabled: bool) -> App {
    app_at(enabled, None).await
}

async fn app_at(enabled: bool, coordinates: Option<(f64, f64)>) -> App {
    let mut cfg = Config::default();
    cfg.weather.latitude = coordinates.map(|c| c.0);
    cfg.weather.longitude = coordinates.map(|c| c.1);
    cfg.topics.discovery_prefixes = vec!["oikos".to_owned()];
    cfg.weather.enabled = enabled;
    cfg.weather.city = "Paris".to_owned();
    // Below the validated floor on purpose: the poller sleeps its one-second minimum.
    cfg.weather.refresh_minutes = 0;
    let (tx, mut rx) = mpsc::channel::<Publish>(256);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    let published = Arc::new(std::sync::Mutex::new(Vec::new()));
    {
        let hub = Arc::clone(&hub);
        let published = Arc::clone(&published);
        tokio::spawn(async move {
            while let Some(p) = rx.recv().await {
                published
                    .lock()
                    .unwrap()
                    .push((p.topic.clone(), p.payload.clone()));
                hub.ingest_message(&p.topic, p.payload.as_bytes(), true);
            }
        });
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(
        axum::serve(
            listener,
            router(Arc::clone(&hub), nidoo::setup::SetupCtx::inert()),
        )
        .into_future(),
    );
    App {
        hub,
        addr,
        published,
    }
}

async fn get_weather(app: &App) -> reqwest::Response {
    reqwest::get(format!("http://{}/api/v1/weather", app.addr))
        .await
        .unwrap()
}

async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    for _ in 0..100 {
        if ready() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test]
async fn poller_publishes_offline_first_then_online_and_states() {
    let app = app(true).await;
    let stub = Stub::new(GEOCODING, usize::MAX);
    let endpoints = serve_stub(stub).await;
    let task = spawn_with(&app.hub, endpoints);

    let hub = Arc::clone(&app.hub);
    until("the first view", || hub.weather_view().is_ok()).await;
    // The publishes reach the registry through the forwarding task, a moment later.
    until("the temperature entity", || {
        hub.entity_view("weather_temperature")
            .is_some_and(|e| e.state.value == nidoo_core::model::StateValue::Number(14.2))
    })
    .await;
    {
        let published = app.published.lock().unwrap();
        let availability: Vec<&str> = published
            .iter()
            .filter(|(t, _)| t == "oikos/weather/availability")
            .map(|(_, p)| p.as_str())
            .collect();
        assert!(availability.len() >= 2, "{availability:?}");
        assert_eq!(availability[..2], ["offline", "online"], "{availability:?}");
        assert!(published[0].0.ends_with("/config"), "{:?}", published[0]);
    }
    let temperature = app.hub.entity_view("weather_temperature").expect("entity");
    assert_eq!(
        temperature.state.value,
        nidoo_core::model::StateValue::Number(14.2)
    );

    let response = get_weather(&app).await;
    assert_eq!(response.status(), 200);
    let view: WeatherView = response.json().await.unwrap();
    assert_eq!(view.place.name, "Paris");
    assert_eq!(view.days.len(), 5);
    task.abort();
}

#[tokio::test]
async fn poller_with_coordinates_skips_geocoding() {
    let app = app_at(true, Some((45.76, 4.84))).await;
    let stub = Stub::new(GEOCODING, usize::MAX);
    let endpoints = serve_stub(Arc::clone(&stub)).await;
    let task = spawn_with(&app.hub, endpoints);

    let hub = Arc::clone(&app.hub);
    until("the first view", || hub.weather_view().is_ok()).await;
    assert_eq!(stub.geocoding_calls.load(Ordering::SeqCst), 0);
    {
        let queries = stub.forecast_queries.lock().unwrap();
        assert_eq!(queries[0]["latitude"], "45.76");
        assert_eq!(queries[0]["longitude"], "4.84");
    }
    // The place is named after the configured city.
    assert_eq!(app.hub.weather_view().unwrap().place.name, "Paris");
    task.abort();
}

#[tokio::test]
async fn unknown_city_stays_unavailable() {
    let app = app(true).await;
    let stub = Stub::new(NO_RESULTS, usize::MAX);
    let endpoints = serve_stub(Arc::clone(&stub)).await;
    let task = spawn_with(&app.hub, endpoints);

    until("the geocoding call", || {
        stub.geocoding_calls.load(Ordering::SeqCst) == 1
    })
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(get_weather(&app).await.status(), 503);
    assert_eq!(stub.forecast_calls.load(Ordering::SeqCst), 0);
    assert!(task.is_finished());
}

#[tokio::test]
async fn disabled_weather_makes_no_request() {
    let app = app(false).await;
    let stub = Stub::new(GEOCODING, usize::MAX);
    let _endpoints = serve_stub(Arc::clone(&stub)).await;
    assert!(app.hub.start_weather().is_none());
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(stub.requests(), 0);
    assert_eq!(get_weather(&app).await.status(), 503);
}

#[tokio::test]
async fn fetch_failure_keeps_last_view_and_goes_stale() {
    let app = app(true).await;
    let stub = Stub::new(GEOCODING, 1);
    let endpoints = serve_stub(Arc::clone(&stub)).await;
    let task = spawn_with(&app.hub, endpoints);

    let hub = Arc::clone(&app.hub);
    until("the first view", || hub.weather_view().is_ok()).await;
    until("a failed fetch", || {
        stub.forecast_calls.load(Ordering::SeqCst) >= 2
    })
    .await;
    let response = get_weather(&app).await;
    assert_eq!(response.status(), 200);
    let view: WeatherView = response.json().await.unwrap();
    assert_eq!(view.place.name, "Paris");
    // (`stale` is not asserted: with refresh_minutes = 0 any age counts as stale.)

    let mut old = app.hub.weather_view().unwrap();
    old.fetched_at = 1_000;
    app.hub.apply_weather(old);
    assert!(app.hub.weather_view().unwrap().stale);
    task.abort();
}

#[tokio::test]
async fn get_weather_mcp_tool_matches_rest() {
    let app = app(true).await;
    let stub = Stub::new(GEOCODING, usize::MAX);
    let endpoints = serve_stub(stub).await;
    let task = spawn_with(&app.hub, endpoints);
    let hub = Arc::clone(&app.hub);
    until("the first view", || hub.weather_view().is_ok()).await;
    // Stop refreshing, then pin `fetched_at` far in the past: `stale` is computed at each read,
    // so with refresh_minutes = 0 both reads are stale for certain, whatever the second.
    task.abort();
    let mut old = app.hub.weather_view().unwrap();
    old.fetched_at = 1_000;
    app.hub.apply_weather(old);

    let rest: Value = get_weather(&app).await.json().await.unwrap();
    let transport = StreamableHttpClientTransport::from_uri(format!("http://{}/mcp", app.addr));
    let client = ().serve(transport).await.expect("mcp initialize");
    let result = client
        .call_tool(CallToolRequestParams::new("get_weather"))
        .await
        .expect("get_weather");
    assert_ne!(result.is_error, Some(true));
    let body = result
        .structured_content
        .clone()
        .expect("structured content");
    assert_eq!(body, rest);
    client.cancel().await.expect("close");
}

#[tokio::test]
async fn lowercase_country_is_sent_uppercase() {
    let mut cfg = Config::default();
    cfg.weather.country = Some("fr".to_owned());
    let (tx, _rx) = mpsc::channel::<Publish>(1);
    let hub = Hub::new(&cfg, tx);
    assert_eq!(hub.weather_country().as_deref(), Some("FR"));
}
