//! The setup API under `/api/v1/setup`: location, MQTT, database, progress.
//!
//! Every write goes through [`SetupCtx::update_file`] and answers `503` once a restart is
//! pending. No response carries a password.

// The `Err` side is an axum `Response`, built at most once per request.
#![allow(clippy::result_large_err)]

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use nidoo_api::{
    DatabaseInput, DatabaseView, GeoPlace, Location, MobileCode, MqttInput, MqttView, Secret,
    SecretView, SetupStatus, SetupStep, TestResult,
};
use serde::Deserialize;
use serde_json::json;
use url::Url;

use crate::config::MqttConfig;
use crate::setup::bundled::{self, Bundled};
use crate::setup::hub_file::RestoreState;
use crate::setup::{HubFile, Mode, SetupCtx, Written, probe, secret, settings};
use crate::weather::open_meteo::{self, Endpoints};

pub(crate) const RESTART_DELAY: Duration = Duration::from_millis(500);
const GEOCODE_LIMIT: u8 = 5;
/// Where the compose file's own broker and database listen (host networking).
const BUNDLED_HOST: &str = "localhost";
const BUNDLED_MQTT_PORT: u16 = 1883;
const BUNDLED_DB_PORT: u16 = 5432;

#[derive(Clone)]
pub(super) struct RouteState {
    pub(super) ctx: Arc<SetupCtx>,
    geocoding: Endpoints,
    client: reqwest::Client,
    /// Where the Solcast key test calls.
    pub(super) solcast_base: String,
}

/// The setup routes, with their own state. `geocoding` is where city lookups go.
pub fn router<S: Clone + Send + Sync + 'static>(
    ctx: Arc<SetupCtx>,
    geocoding: Endpoints,
) -> Router<S> {
    router_with_solcast(ctx, geocoding, nidoo_solcast::BASE_URL)
}

/// [`router`] with the Solcast API address the key test calls (tests point it at a stub).
pub fn router_with_solcast<S: Clone + Send + Sync + 'static>(
    ctx: Arc<SetupCtx>,
    geocoding: Endpoints,
    solcast_base: &str,
) -> Router<S> {
    let client = open_meteo::client().unwrap_or_default();
    Router::new()
        .route("/api/v1/setup", get(status))
        .route("/api/v1/setup/geocode", get(geocode))
        .route("/api/v1/setup/location", put(put_location))
        .route("/api/v1/setup/test/mqtt", post(test_mqtt))
        .route("/api/v1/setup/mqtt", put(put_mqtt))
        .route("/api/v1/setup/test/database", post(test_database))
        .route("/api/v1/setup/database", put(put_database))
        .route("/api/v1/setup/restore", put(put_restore))
        .route("/api/v1/setup/step", put(put_step))
        .route("/api/v1/setup/finish", post(finish))
        .route("/api/v1/setup/mobile-code", get(mobile_code))
        .merge(super::settings::routes())
        .merge(super::restore_routes::routes())
        .with_state(RouteState {
            ctx,
            geocoding,
            client,
            solcast_base: solcast_base.trim_end_matches('/').to_owned(),
        })
}

pub(super) fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

pub(super) fn restarting(ctx: &SetupCtx) -> Option<Response> {
    ctx.restarting
        .load(Ordering::SeqCst)
        .then(|| error(StatusCode::SERVICE_UNAVAILABLE, "restarting"))
}

fn save_failed() -> Response {
    error(StatusCode::INTERNAL_SERVER_ERROR, "cannot save hub.json")
}

pub(super) fn decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .into_owned()
}

fn encode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn mqtt_view(file: &HubFile, bundled: Option<&Bundled>) -> Option<MqttView> {
    if let Some(m) = &file.mqtt {
        return Some(MqttView {
            host: m.host.clone(),
            port: m.port,
            user: m.user.clone(),
            password: SecretView {
                set: m.password.is_some(),
            },
            client_id: Some(m.client_id.clone()),
            z2m_base: file.z2m_base.clone(),
        });
    }
    bundled.map(|b| MqttView {
        host: BUNDLED_HOST.into(),
        port: BUNDLED_MQTT_PORT,
        user: Some(b.mqtt_user.clone()),
        password: SecretView { set: true },
        client_id: None,
        z2m_base: None,
    })
}

fn database_view(file: &HubFile, bundled: Option<&Bundled>) -> Option<DatabaseView> {
    if let Some(url) = &file.database_url {
        let url = Url::parse(url).ok()?;
        return Some(DatabaseView {
            host: url.host_str()?.to_owned(),
            port: url.port().unwrap_or(BUNDLED_DB_PORT),
            database: decode(url.path().trim_start_matches('/')),
            user: decode(url.username()),
            password: SecretView {
                set: url.password().is_some(),
            },
        });
    }
    bundled.map(|b| DatabaseView {
        host: BUNDLED_HOST.into(),
        port: BUNDLED_DB_PORT,
        database: b.db_name.clone(),
        user: b.db_user.clone(),
        password: SecretView { set: true },
    })
}

/// The text the phone setup QR encodes. It holds the broker login and the gateway key: it is
/// never logged, and only served in Run mode.
async fn mobile_code(State(s): State<RouteState>, headers: HeaderMap) -> Response {
    if s.ctx.mode != Mode::Run {
        return error(StatusCode::CONFLICT, "finish MQTT and database setup first");
    }
    let Some(mqtt) = s.ctx.file.lock().await.mqtt.clone() else {
        return error(StatusCode::CONFLICT, "finish MQTT and database setup first");
    };
    let sections = s.ctx.sections.lock().await.clone();
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
    Json(MobileCode {
        payload: super::mobile::build_payload(&mqtt, &sections, host),
    })
    .into_response()
}

async fn status(State(s): State<RouteState>) -> Json<SetupStatus> {
    let bundled = bundled::load(&s.ctx.data_dir);
    let file = s.ctx.file.lock().await.clone();
    Json(SetupStatus {
        mode: match s.ctx.mode {
            Mode::Setup => "setup",
            Mode::Restore => "restore",
            Mode::Run => "running",
        }
        .into(),
        step: file.step,
        finished: file.finished,
        bundled: bundled.is_some(),
        location: file.location.clone(),
        mqtt: mqtt_view(&file, bundled.as_ref()),
        database: database_view(&file, bundled.as_ref()),
        restarting: s.ctx.restarting.load(Ordering::SeqCst),
        restore_pending: file.restore.is_some(),
    })
}

#[derive(Deserialize)]
struct GeocodeQuery {
    #[serde(default)]
    city: String,
    #[serde(default)]
    country: Option<String>,
}

async fn geocode(State(s): State<RouteState>, Query(q): Query<GeocodeQuery>) -> Response {
    let city = q.city.trim();
    if city.is_empty() {
        return Json(Vec::<GeoPlace>::new()).into_response();
    }
    let country = q
        .country
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    match open_meteo::geocode_many(&s.client, &s.geocoding, city, country, GEOCODE_LIMIT).await {
        Ok(places) => Json(
            places
                .into_iter()
                .map(|p| GeoPlace {
                    name: p.name,
                    admin: p.admin,
                    country: p.country,
                    latitude: p.latitude,
                    longitude: p.longitude,
                    timezone: p.timezone,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(_) => error(StatusCode::BAD_GATEWAY, "the city lookup failed"),
    }
}

/// Saves under the file lock, then answers with the new status. A restart is scheduled (still
/// under the lock) when `restarts` and the hub runs, or when the file just became complete. A
/// restart already pending answers 503 without saving. On a running hub, a write that restarts
/// it must leave a config the hub can boot with (`main` checks it before anything starts), or
/// it answers 422 with the reason and saves nothing.
async fn finish_write(s: &RouteState, restarts: bool, edit: impl FnOnce(&mut HubFile)) -> Response {
    let in_run = s.ctx.mode == Mode::Run;
    // The settings lock comes first (the lock order) and is held through the write, so the
    // check sees the settings the restart boots with. Setup mode is not checked: its settings
    // can only be edited once the hub runs.
    let sections = if restarts && in_run {
        if let Some(r) = restarting(&s.ctx) {
            return r;
        }
        Some(s.ctx.sections.lock().await)
    } else {
        None
    };
    let saved = s
        .ctx
        .write_unless_restarting(
            RESTART_DELAY,
            edit,
            |before, after| {
                restarts
                    && (s.ctx.mode != Mode::Setup || (!before.is_complete() && after.is_complete()))
            },
            |after| match &sections {
                Some(held) => settings::check_boot(after, held),
                None => Ok(()),
            },
        )
        .await;
    drop(sections);
    match saved {
        Err(_) => save_failed(),
        Ok(Written::Restarting) => error(StatusCode::SERVICE_UNAVAILABLE, "restarting"),
        Ok(Written::Refused(message)) => error(StatusCode::UNPROCESSABLE_ENTITY, &message),
        Ok(Written::Saved) => status(State(s.clone())).await.into_response(),
    }
}

async fn put_location(State(s): State<RouteState>, Json(loc): Json<Location>) -> Response {
    if let Some(r) = restarting(&s.ctx) {
        return r;
    }
    if loc.city.trim().is_empty()
        || !(-90.0..=90.0).contains(&loc.latitude)
        || !(-180.0..=180.0).contains(&loc.longitude)
        || loc.timezone.parse::<chrono_tz::Tz>().is_err()
    {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "The location is not valid",
        );
    }
    // The weather check wants two letters; an empty field is no country.
    let country = match loc.country.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(c) if c.len() == 2 && c.chars().all(|c| c.is_ascii_alphabetic()) => {
            Some(c.to_ascii_uppercase())
        }
        Some(_) => {
            return error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "The country must be two letters, such as FR",
            );
        }
    };
    let loc = Location { country, ..loc };
    finish_write(&s, true, |f| {
        f.location = Some(loc);
        // The chosen place supersedes anything the import left as a fallback.
        f.pending_city = None;
        f.timezone = None;
        f.step = f.step.max(SetupStep::Mqtt);
    })
    .await
}

/// The broker address and password the stored settings (or the bundled ones) hold.
async fn stored_mqtt(ctx: &SetupCtx) -> (Option<MqttConfig>, Option<String>, Option<String>) {
    let file = ctx.file.lock().await.clone();
    if let Some(m) = file.mqtt {
        let (addr, pw) = (format!("{}:{}", m.host, m.port), m.password.clone());
        return (Some(m), Some(addr), pw);
    }
    match bundled::load(&ctx.data_dir) {
        Some(b) => (
            None,
            Some(format!("{BUNDLED_HOST}:{BUNDLED_MQTT_PORT}")),
            Some(b.mqtt_password),
        ),
        None => (None, None, None),
    }
}

/// The config a request would save, or the refusal.
async fn mqtt_candidate(ctx: &SetupCtx, input: &MqttInput) -> Result<MqttConfig, Response> {
    let (stored, old_addr, stored_pw) = stored_mqtt(ctx).await;
    let new_addr = format!("{}:{}", input.host.trim(), input.port);
    secret::require_for_new_address(
        old_addr.as_deref(),
        &new_addr,
        &input.password,
        stored_pw.is_some(),
    )
    .map_err(|m| error(StatusCode::UNPROCESSABLE_ENTITY, m))?;
    let mut cfg = stored.unwrap_or_default();
    cfg.host = input.host.trim().to_owned();
    cfg.port = input.port;
    cfg.user = input.user.clone().filter(|u| !u.is_empty());
    cfg.password = secret::apply(stored_pw, input.password.clone());
    if let Some(id) = input.client_id.as_deref().filter(|c| !c.is_empty()) {
        cfg.client_id = id.to_owned();
    }
    // The broker login needs a user: refuse rather than save a password nothing would send.
    if cfg.user.is_none() && cfg.password.is_some() {
        return Err(error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Enter a user name for the password",
        ));
    }
    Ok(cfg)
}

async fn test_mqtt(State(s): State<RouteState>, Json(input): Json<MqttInput>) -> Response {
    match mqtt_candidate(&s.ctx, &input).await {
        Ok(cfg) => Json(probe::mqtt(&cfg).await).into_response(),
        Err(r) => r,
    }
}

async fn put_mqtt(State(s): State<RouteState>, Json(input): Json<MqttInput>) -> Response {
    if let Some(r) = restarting(&s.ctx) {
        return r;
    }
    let cfg = match mqtt_candidate(&s.ctx, &input).await {
        Ok(cfg) => cfg,
        Err(r) => return r,
    };
    // Probe first: wrong credentials must never reach hub.json, or a running house would
    // restart into a broker it cannot reach.
    let result = probe::mqtt(&cfg).await;
    if !result.ok {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(result)).into_response();
    }
    let z2m = input.z2m_base.clone().filter(|z| !z.is_empty());
    finish_write(&s, true, |f| {
        // Another broker has not been checked for data: a pending restore starts over.
        if f.restore.is_some() && f.mqtt.as_ref() != Some(&cfg) {
            f.restore = Some(RestoreState::default());
        }
        f.mqtt = Some(cfg);
        if z2m.is_some() {
            f.z2m_base = z2m;
        }
        f.step = f.step.max(SetupStep::Database);
    })
    .await
}

/// The URL a request would save, built on the stored URL (so a kept password stays as it is).
async fn database_candidate(ctx: &SetupCtx, input: &DatabaseInput) -> Result<String, Response> {
    let file = ctx.file.lock().await.clone();
    let stored = file
        .database_url
        .as_deref()
        .and_then(|u| Url::parse(u).ok());
    let bundled = bundled::load(&ctx.data_dir);
    let old_addr = match (&stored, &bundled) {
        (Some(u), _) => u
            .host_str()
            .map(|h| format!("{h}:{}", u.port().unwrap_or(BUNDLED_DB_PORT))),
        (None, Some(_)) => Some(format!("{BUNDLED_HOST}:{BUNDLED_DB_PORT}")),
        (None, None) => None,
    };
    let new_addr = format!("{}:{}", input.host.trim(), input.port);
    let stored_secret = match (&stored, &bundled) {
        (Some(u), _) => u.password().is_some(),
        (None, b) => b.is_some(),
    };
    secret::require_for_new_address(
        old_addr.as_deref(),
        &new_addr,
        &input.password,
        stored_secret,
    )
    .map_err(|m| error(StatusCode::UNPROCESSABLE_ENTITY, m))?;
    build_database_url(
        stored,
        bundled.as_ref().map(|b| b.db_password.as_str()),
        input,
    )
    .ok_or_else(|| error(StatusCode::UNPROCESSABLE_ENTITY, "The address is not valid"))
}

/// Whether two database URLs reach the same database as the same user: host, port, database,
/// user and password, decoded (a re-saved URL may encode them differently).
fn same_database(a: &str, b: &str) -> bool {
    let target = |u: &str| {
        let u = Url::parse(u).ok()?;
        Some((
            u.host_str()?.to_ascii_lowercase(),
            u.port().unwrap_or(BUNDLED_DB_PORT),
            decode(u.path().trim_start_matches('/')),
            decode(u.username()),
            u.password().map(decode),
        ))
    };
    matches!((target(a), target(b)), (Some(x), Some(y)) if x == y)
}

/// The URL to save: the stored one with the address, user and database replaced (a kept password
/// stays as stored), or a new one carrying the bundled password. User, password and database
/// are percent-encoded up front, because `Url` leaves `%` alone and sqlx decodes it.
fn build_database_url(
    stored: Option<Url>,
    bundled_password: Option<&str>,
    input: &DatabaseInput,
) -> Option<String> {
    let mut url = match stored {
        Some(u) => u,
        None => {
            let mut u = Url::parse("postgres://placeholder/").ok()?;
            if let Some(p) = bundled_password {
                u.set_password(Some(&encode(p))).ok()?;
            }
            u
        }
    };
    url.set_host(Some(input.host.trim())).ok()?;
    url.set_port(Some(input.port)).ok()?;
    url.set_username(&encode(&input.user)).ok()?;
    url.set_path(&format!("/{}", encode(&input.database)));
    match &input.password {
        Secret::Keep => {}
        Secret::Clear => url.set_password(None).ok()?,
        Secret::Set(p) => url.set_password(Some(&encode(p))).ok()?,
    }
    Some(url.into())
}

async fn test_database(State(s): State<RouteState>, Json(input): Json<DatabaseInput>) -> Response {
    match database_candidate(&s.ctx, &input).await {
        Ok(url) => Json(probe::database(&url).await).into_response(),
        Err(r) => r,
    }
}

async fn put_database(State(s): State<RouteState>, Json(input): Json<DatabaseInput>) -> Response {
    if let Some(r) = restarting(&s.ctx) {
        return r;
    }
    let url = match database_candidate(&s.ctx, &input).await {
        Ok(url) => url,
        Err(r) => return r,
    };
    let result: TestResult = probe::database(&url).await;
    if !result.ok {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(result)).into_response();
    }
    finish_write(&s, true, |f| {
        // Another database has not been checked for data: a pending restore starts over.
        if f.restore.is_some()
            && !f
                .database_url
                .as_deref()
                .is_some_and(|old| same_database(old, &url))
        {
            f.restore = Some(RestoreState::default());
        }
        f.database_url = Some(url);
        f.step = if f.restore.is_some() {
            SetupStep::Restore
        } else {
            f.step.max(SetupStep::Devices)
        };
    })
    .await
}

#[derive(Deserialize)]
struct StepBody {
    step: SetupStep,
}

async fn put_step(State(s): State<RouteState>, Json(body): Json<StepBody>) -> Response {
    if let Some(r) = restarting(&s.ctx) {
        return r;
    }
    finish_write(&s, false, |f| f.step = body.step).await
}

#[derive(Deserialize)]
struct RestoreBody {
    pending: bool,
}

/// Turns the restore path on or off. On only while setting up; off also while a restore is
/// pending, which sends the hub back to the devices step and restarts it into Run.
async fn put_restore(State(s): State<RouteState>, Json(body): Json<RestoreBody>) -> Response {
    if let Some(r) = restarting(&s.ctx) {
        return r;
    }
    match (s.ctx.mode, body.pending) {
        (Mode::Run, _) => error(StatusCode::CONFLICT, "the hub is already running"),
        (Mode::Restore, true) => error(StatusCode::CONFLICT, "a restore is already pending"),
        (Mode::Setup, true) => {
            finish_write(&s, false, |f| f.restore = Some(RestoreState::default())).await
        }
        (Mode::Setup, false) => {
            finish_write(&s, false, |f| {
                f.restore = None;
                if f.step == SetupStep::Restore {
                    f.step = SetupStep::Devices;
                }
            })
            .await
        }
        (Mode::Restore, false) => {
            finish_write(&s, true, |f| {
                f.restore = None;
                f.step = SetupStep::Devices;
            })
            .await
        }
    }
}

async fn finish(State(s): State<RouteState>) -> Response {
    if let Some(r) = restarting(&s.ctx) {
        return r;
    }
    // Nothing clears MQTT or the database once set, so this cannot go stale before the write.
    if !s.ctx.file.lock().await.is_complete() {
        return error(StatusCode::CONFLICT, "finish MQTT and database setup first");
    }
    finish_write(&s, false, |f| {
        f.finished = true;
        f.step = SetupStep::Done;
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_url_round_trips_awkward_characters() {
        let input = DatabaseInput {
            host: "192.0.2.7".into(),
            port: 5433,
            database: "my db".into(),
            user: "me@home".into(),
            password: Secret::Set("ab%41:c/d@e f".into()),
        };
        let url = build_database_url(None, None, &input).unwrap();
        // What sqlx will see once it decodes the URL.
        let parsed = Url::parse(&url).unwrap();
        assert_eq!(decode(parsed.password().unwrap()), "ab%41:c/d@e f");
        assert_eq!(decode(parsed.username()), "me@home");
        let file = HubFile {
            database_url: Some(url),
            ..HubFile::default()
        };
        let view = database_view(&file, None).unwrap();
        assert_eq!(view.host, "192.0.2.7");
        assert_eq!(view.port, 5433);
        assert_eq!(view.user, "me@home");
        assert_eq!(view.database, "my db");
        assert!(view.password.set);
        // sqlx itself reads the same values back.
        let opts: sqlx::postgres::PgConnectOptions = file.database_url.unwrap().parse().unwrap();
        assert_eq!(opts.get_username(), "me@home");
        assert_eq!(opts.get_database(), Some("my db"));
    }

    #[test]
    fn same_database_ignores_the_encoding() {
        let stored = "postgres://nidoo:pw@db.local:5432/nidoo_test_1";
        assert!(same_database(
            stored,
            "postgres://nidoo:pw@DB.local/nidoo%5Ftest%5F1"
        ));
        for other in [
            "postgres://nidoo:pw@db.local:5432/nidoo_test_2",
            "postgres://nidoo:pw@db2.local:5432/nidoo_test_1",
            "postgres://nidoo:pw@db.local:5433/nidoo_test_1",
            "postgres://other:pw@db.local:5432/nidoo_test_1",
            "postgres://nidoo:pw2@db.local:5432/nidoo_test_1",
            "not a url",
        ] {
            assert!(!same_database(stored, other), "{other}");
        }
    }

    #[test]
    fn bundled_password_is_encoded_too() {
        let input = DatabaseInput {
            host: "localhost".into(),
            port: 5432,
            database: "nidoo".into(),
            user: "nidoo".into(),
            password: Secret::Keep,
        };
        let url = build_database_url(None, Some("p%41ss"), &input).unwrap();
        assert_eq!(
            decode(Url::parse(&url).unwrap().password().unwrap()),
            "p%41ss"
        );
    }
}
