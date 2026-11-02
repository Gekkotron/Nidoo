//! The Solcast poller against a local stub of the API, with the hub served on an ephemeral
//! port. No test reaches the real network.

mod common;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chrono::{DateTime, TimeZone, Utc};
use nidoo::config::Config;
use nidoo::http::router;
use nidoo::hub::Hub;
use nidoo::solar::poller::spawn_with;
use nidoo_api::{ConnectionStatus, SolarView};
use nidoo_core::command::Publish;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// What the stub answers for a site.
#[derive(Clone, Copy)]
enum Answer {
    Ok,
    /// 200 with no period at all.
    Empty,
    Status(u16),
}

/// What the stub serves and how often it was asked, per site.
struct Stub {
    answers: Mutex<BTreeMap<String, Answer>>,
    calls: Mutex<BTreeMap<String, usize>>,
    total: AtomicUsize,
    body: String,
}

impl Stub {
    fn new(answers: &[(&str, Answer)], scale: f64) -> Arc<Stub> {
        Self::with_body(answers, solcast_body(scale, None))
    }

    /// Like [`Stub::new`], but serving only the periods that start at or after `from`, as
    /// Solcast does once part of the day is over.
    fn from_instant(scale: f64, from: DateTime<Utc>) -> Arc<Stub> {
        Self::with_body(&[], solcast_body(scale, Some(from)))
    }

    fn with_body(answers: &[(&str, Answer)], body: String) -> Arc<Stub> {
        Arc::new(Stub {
            answers: Mutex::new(
                answers
                    .iter()
                    .map(|(site, a)| ((*site).to_owned(), *a))
                    .collect(),
            ),
            calls: Mutex::new(BTreeMap::new()),
            total: AtomicUsize::new(0),
            body,
        })
    }

    fn total(&self) -> usize {
        self.total.load(Ordering::SeqCst)
    }

    fn calls(&self, site: &str) -> usize {
        self.calls.lock().unwrap().get(site).copied().unwrap_or(0)
    }
}

/// The shifted fixture, serialised back to Solcast's JSON shape, every estimate times `scale`.
fn solcast_body(scale: f64, from: Option<DateTime<Utc>>) -> String {
    let forecast = common::solar::forecast_today();
    let forecasts: Vec<_> = forecast
        .periods
        .iter()
        .filter(|p| from.is_none_or(|from| p.start() >= from))
        .map(|p| {
            json!({
                "pv_estimate": p.estimate * scale,
                "pv_estimate10": p.low * scale,
                "pv_estimate90": p.high * scale,
                "period_end": p.end.to_rfc3339(),
                "period": "PT30M",
            })
        })
        .collect();
    json!({ "forecasts": forecasts }).to_string()
}

async fn site_forecasts(State(s): State<Arc<Stub>>, Path(site): Path<String>) -> Response {
    s.total.fetch_add(1, Ordering::SeqCst);
    *s.calls.lock().unwrap().entry(site.clone()).or_insert(0) += 1;
    let answer = s
        .answers
        .lock()
        .unwrap()
        .get(&site)
        .copied()
        .unwrap_or(Answer::Ok);
    match answer {
        Answer::Ok => ([("content-type", "application/json")], s.body.clone()).into_response(),
        Answer::Empty => (
            [("content-type", "application/json")],
            json!({ "forecasts": [] }).to_string(),
        )
            .into_response(),
        Answer::Status(code) => StatusCode::from_u16(code).unwrap().into_response(),
    }
}

async fn serve_stub(stub: Arc<Stub>) -> String {
    let app = Router::new()
        .route("/rooftop_sites/{site}/forecasts", get(site_forecasts))
        .with_state(stub);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

struct App {
    hub: Arc<Hub>,
    addr: SocketAddr,
    /// What the hub published, in order, as `(topic, payload)`; also fed back into the hub.
    published: Arc<Mutex<Vec<(String, String)>>>,
}

/// A key file in the temp dir, unique to this call.
fn key_file() -> std::path::PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "nidoo-solar-poller-{}-{}.key",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::write(&path, "test-key\n").unwrap();
    path
}

async fn app(enabled: bool, sites: &[&str], limit: u32) -> App {
    let app = app_unsubscribed(enabled, sites, limit).await;
    // The broker acknowledged the subscription.
    app.hub.on_suback();
    app
}

/// Like [`app`], but the broker has not acknowledged the subscription yet.
async fn app_unsubscribed(enabled: bool, sites: &[&str], limit: u32) -> App {
    let mut cfg = Config::default();
    cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
    cfg.flows.timezone = Some("Europe/Paris".into());
    cfg.solcast.enabled = enabled;
    cfg.solcast.sites = sites.iter().map(|s| (*s).to_owned()).collect();
    cfg.solcast.daily_limit = limit;
    cfg.solcast.api_key_file = Some(key_file());
    let (tx, mut rx) = mpsc::channel::<Publish>(1024);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    let published = Arc::new(Mutex::new(Vec::new()));
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

async fn get_solar(app: &App) -> reqwest::Response {
    reqwest::get(format!("http://{}/api/v1/solar", app.addr))
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

/// Feeds the hub a retained cache message, as the broker would replay it.
fn feed_cache(
    hub: &Hub,
    forecast: Option<&nidoo_solcast::Forecast>,
    ledger: &nidoo_solcast::Ledger,
) {
    let payload = json!({ "forecast": forecast, "ledger": ledger }).to_string();
    hub.ingest_message("nidoo/solar/cache", payload.as_bytes(), true);
}

#[tokio::test]
async fn first_start_fetches_once_per_site_and_serves_view() {
    let app = app(true, &["a", "b"], 10).await;
    let stub = Stub::new(&[], 1.0);
    let base = serve_stub(Arc::clone(&stub)).await;
    let task = spawn_with(&app.hub, base, Duration::ZERO);

    let hub = Arc::clone(&app.hub);
    until("the first view", || hub.solar_view().is_ok()).await;
    // Over a few seconds the poller makes no second round. (The decision logic is unit-tested
    // in the poller module: a loop turn waits for the next half hour.)
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(stub.calls("a"), 1);
    assert_eq!(stub.calls("b"), 1);
    let response = get_solar(&app).await;
    assert_eq!(response.status(), 200);
    let view: SolarView = response.json().await.unwrap();
    assert_eq!(view.calls_today, 2);
    let single = hub_estimate();
    let today = view.today.estimate.expect("today estimate");
    assert!((today - 2.0 * single).abs() < 0.01, "{today} vs {single}");
    assert!(
        app.published
            .lock()
            .unwrap()
            .iter()
            .any(|(t, p)| t == "nidoo/solar/availability" && p == "online")
    );
    task.abort();
}

#[tokio::test]
async fn no_call_before_the_suback() {
    let app = app_unsubscribed(true, &["a"], 10).await;
    let stub = Stub::new(&[], 1.0);
    let base = serve_stub(Arc::clone(&stub)).await;
    let task = spawn_with(&app.hub, base, Duration::ZERO);

    // The entities are published, yet nothing is asked while the cache may still be coming.
    let published = Arc::clone(&app.published);
    until("the configs", || {
        published
            .lock()
            .unwrap()
            .iter()
            .any(|(t, _)| t.ends_with("/config"))
    })
    .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(stub.total(), 0, "called before the SubAck");

    app.hub.on_suback();
    until("the call after the SubAck", || stub.total() >= 1).await;
    task.abort();
}

#[tokio::test]
async fn first_site_server_error_skips_second_site() {
    let app = app(true, &["a", "b"], 10).await;
    let stub = Stub::new(&[("a", Answer::Status(500))], 1.0);
    let base = serve_stub(Arc::clone(&stub)).await;
    let task = spawn_with(&app.hub, base, Duration::ZERO);

    until("the first call", || stub.total() >= 1).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(stub.calls("a"), 1);
    assert_eq!(stub.calls("b"), 0);
    assert_eq!(app.hub.solar_ledger().calls_on(Utc::now()), 1);
    task.abort();
}

/// One site's energy for today, from the fixture.
fn hub_estimate() -> f64 {
    let forecast = common::solar::forecast_today();
    forecast.periods.iter().map(|p| p.kwh()).sum()
}

#[tokio::test]
async fn restart_reads_cache_and_makes_no_call() {
    let first = app(true, &["a"], 10).await;
    let stub = Stub::new(&[], 1.0);
    let base = serve_stub(Arc::clone(&stub)).await;
    let task = spawn_with(&first.hub, base.clone(), Duration::ZERO);
    let hub = Arc::clone(&first.hub);
    until("the first view", || hub.solar_view().is_ok()).await;
    task.abort();
    assert_eq!(stub.total(), 1);

    // A second hub, fed a cache whose last attempt is now, starts without calling.
    let second = app(true, &["a"], 10).await;
    let ledger = nidoo_solcast::Ledger {
        utc_day: Some(Utc::now().date_naive()),
        calls: 1,
        last_attempt: Some(Utc::now().timestamp()),
        blocked_until: None,
    };
    feed_cache(&second.hub, Some(&common::solar::forecast_today()), &ledger);
    let task = spawn_with(&second.hub, base, Duration::ZERO);
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(stub.total(), 1, "the restart must not call Solcast");
    assert_eq!(get_solar(&second).await.status(), 200);
    {
        let published = second.published.lock().unwrap();
        assert!(published.iter().any(|(t, _)| t.ends_with("/config")));
        assert!(
            published
                .iter()
                .any(|(t, p)| t == "nidoo/solar/availability" && p == "online")
        );
    }
    task.abort();
}

#[tokio::test]
async fn one_failing_site_keeps_previous_forecast() {
    let app = app(true, &["a", "b"], 10).await;
    let stub = Stub::new(&[("b", Answer::Status(500))], 2.0);
    let base = serve_stub(Arc::clone(&stub)).await;
    // A held forecast and no recorded attempt: a slot is due, so the poller calls.
    let before = common::solar::forecast_today();
    feed_cache(&app.hub, Some(&before), &nidoo_solcast::Ledger::default());
    let task = spawn_with(&app.hub, base, Duration::ZERO);

    until("both sites asked", || stub.calls("b") >= 1).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(stub.calls("a"), 1);
    assert_eq!(stub.calls("b"), 1);
    let ledger = app.hub.solar_ledger();
    assert_eq!(ledger.calls_on(Utc::now()), 2);
    // Nothing from site A's answer (twice the fixture) reached the view.
    let view = app.hub.solar_view().unwrap();
    let today = view.today.estimate.unwrap();
    assert!((today - hub_estimate()).abs() < 0.01, "{today}");
    assert_eq!(view.fetched_at, u64::try_from(before.fetched_at).unwrap());
    task.abort();
}

#[tokio::test]
async fn empty_site_answer_keeps_previous_forecast() {
    let app = app(true, &["a", "b"], 10).await;
    let stub = Stub::new(&[("b", Answer::Empty)], 2.0);
    let base = serve_stub(Arc::clone(&stub)).await;
    let before = common::solar::forecast_today();
    feed_cache(&app.hub, Some(&before), &nidoo_solcast::Ledger::default());
    let task = spawn_with(&app.hub, base, Duration::ZERO);

    until("both sites asked", || stub.calls("b") >= 1).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(stub.calls("a"), 1);
    assert_eq!(stub.calls("b"), 1);
    assert_eq!(app.hub.solar_ledger().calls_on(Utc::now()), 2);
    // Site A's doubled answer was not applied.
    let view = app.hub.solar_view().unwrap();
    let today = view.today.estimate.unwrap();
    assert!((today - hub_estimate()).abs() < 0.01, "{today}");
    assert_eq!(view.fetched_at, u64::try_from(before.fetched_at).unwrap());
    task.abort();
}

#[tokio::test]
async fn second_refresh_keeps_morning_periods() {
    let app = app(true, &["a"], 10).await;
    let before = common::solar::forecast_today();
    let noon = chrono_tz::Europe::Paris
        .from_local_datetime(
            &Utc::now()
                .with_timezone(&chrono_tz::Europe::Paris)
                .date_naive()
                .and_hms_opt(12, 0, 0)
                .unwrap(),
        )
        .earliest()
        .unwrap()
        .with_timezone(&Utc);
    let stub = Stub::from_instant(2.0, noon);
    let base = serve_stub(Arc::clone(&stub)).await;
    // A held full-day forecast and no recorded attempt: a slot is due, so the poller calls.
    feed_cache(&app.hub, Some(&before), &nidoo_solcast::Ledger::default());
    let task = spawn_with(&app.hub, base, Duration::ZERO);

    until("the call", || stub.total() >= 1).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let morning: f64 = before
        .periods
        .iter()
        .filter(|p| p.start() < noon)
        .map(|p| p.kwh())
        .sum();
    let afternoon: f64 = before
        .periods
        .iter()
        .filter(|p| p.start() >= noon)
        .map(|p| 2.0 * p.kwh())
        .sum();
    let view: SolarView = get_solar(&app).await.json().await.unwrap();
    let today = view.today.estimate.expect("today estimate");
    assert!(
        (today - (morning + afternoon)).abs() < 0.01,
        "{today} vs {morning} + {afternoon}"
    );
    task.abort();
}

#[tokio::test]
async fn server_error_does_not_retry_immediately() {
    let app = app(true, &["a"], 10).await;
    let stub = Stub::new(&[("a", Answer::Status(500))], 1.0);
    let base = serve_stub(Arc::clone(&stub)).await;
    let task = spawn_with(&app.hub, base, Duration::ZERO);

    until("the first call", || stub.total() >= 1).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(stub.total(), 1);
    task.abort();
}

#[tokio::test]
async fn unauthorized_stops_all_calls() {
    let app = app(true, &["a", "b"], 10).await;
    let stub = Stub::new(
        &[("a", Answer::Status(401)), ("b", Answer::Status(401))],
        1.0,
    );
    let base = serve_stub(Arc::clone(&stub)).await;
    let task = spawn_with(&app.hub, base, Duration::ZERO);

    until("the first call", || stub.total() >= 1).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(stub.total(), 1);
    assert!(!task.is_finished());
    task.abort();
}

#[tokio::test]
async fn quota_exceeded_spends_the_day() {
    let app = app(true, &["a"], 10).await;
    let stub = Stub::new(&[("a", Answer::Status(429))], 1.0);
    let base = serve_stub(Arc::clone(&stub)).await;
    let task = spawn_with(&app.hub, base, Duration::ZERO);

    until("the first call", || stub.total() >= 1).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(stub.total(), 1);
    let ledger = app.hub.solar_ledger();
    assert_eq!(ledger.calls, 10);
    assert!(
        ledger
            .blocked_until
            .is_some_and(|t| t > Utc::now().timestamp())
    );
    task.abort();
}

#[tokio::test]
async fn disabled_makes_no_request() {
    let app = app(false, &[], 10).await;
    let stub = Stub::new(&[], 1.0);
    let _base = serve_stub(Arc::clone(&stub)).await;
    assert!(app.hub.start_solar().is_none());
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(stub.total(), 0);
    assert_eq!(get_solar(&app).await.status(), 503);
}
