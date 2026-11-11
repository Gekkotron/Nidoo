//! `GET`/`PUT /api/v1/settings/{section}` and the Solcast and Frigate connection tests.
//!
//! A `PUT` is Run mode only. It builds the candidate sections, checks them the way the boot
//! will (`assemble` then `validate_all`), saves them to the database, then to the cache, and
//! restarts the hub so every service picks the change up. No body, log line or error from here
//! carries a secret: reads show `{"set": bool}` and failures use fixed text.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use nidoo_api::{
    FrigateInput, FrigateView, PushInput, PushView, Secret, SecretView, SolcastInput, SolcastView,
    TestResult,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::backup;
use crate::config::FrigateConfig;
use crate::setup::SetupCtx;
use crate::setup::routes::{RESTART_DELAY, RouteState, error};
use crate::setup::sections::{
    AdvancedSection, PushSection, SECTION_KEYS, Sections, SolcastSection, WeatherSection, assemble,
};
use crate::setup::store::{save_live, save_section, write_cache};
use crate::setup::{HubFile, Mode, mobile, probe, restart, secret};

pub(super) fn routes() -> Router<RouteState> {
    Router::new()
        .route(
            "/api/v1/settings/{section}",
            get(get_section).put(put_section),
        )
        .route("/api/v1/setup/test/solcast", post(test_solcast))
        .route("/api/v1/setup/test/frigate", post(test_frigate))
}

/// A refusal with a fixed message, shared by the HTTP handlers and the MCP tools. The message
/// never carries a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: &str) -> ApiError {
        ApiError {
            status,
            message: message.to_owned(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        error(self.status, &self.message)
    }
}

fn unprocessable(message: &str) -> ApiError {
    ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, message)
}

/// The sections the generic routes edit (and restart for). "system" (`Hub::set_system_notify`)
/// and "mobile" (`save_live`) are stored without a restart, so they are not among them.
fn known(section: &str) -> bool {
    !matches!(section, "system" | "mobile" | "backup") && SECTION_KEYS.contains(&section)
}

/// The project id a service-account key JSON names. The JSON and any parse error stay unlogged.
fn project_id(key: &str) -> Option<String> {
    serde_json::from_str::<Value>(key)
        .ok()?
        .get("project_id")?
        .as_str()
        .map(str::to_owned)
}

fn view(section: &str, s: &Sections) -> Value {
    fn to<T: serde::Serialize>(v: &T) -> Value {
        serde_json::to_value(v).expect("a view serializes")
    }
    let set = |v: &Option<String>| SecretView { set: v.is_some() };
    match section {
        "weather" => to(&s.weather),
        "solcast" => to(&SolcastView {
            enabled: s.solcast.enabled,
            sites: s.solcast.sites.clone(),
            daily_limit: s.solcast.daily_limit,
            api_key: set(&s.solcast.api_key),
            notify_unreachable: s.solcast.notify_unreachable,
        }),
        "frigate" => {
            let f = &s.frigate;
            // A login written into an address (from an import) never shows.
            let address = |u: &Option<String>| u.as_deref().map(secret::without_userinfo);
            to(&FrigateView {
                url: address(&f.url),
                user: f.user.clone(),
                password: set(&f.password),
                media_url: address(&f.media_url),
                bridge: f.bridge,
                labels: f.labels.clone(),
                cameras: f.cameras.clone(),
                zones: f.zones.clone(),
                cooldown: f.cooldown,
                severity: f.severity.clone(),
                ttl: f.ttl,
                topic_prefix: f.topic_prefix.clone(),
                live: f.live,
            })
        }
        "push" => {
            let p = &s.push;
            to(&PushView {
                service_account: set(&p.service_account),
                project_id: p.service_account.as_deref().and_then(project_id),
                prefix: p.prefix.clone(),
                retry_interval: p.retry_interval,
                max_retries: p.max_retries,
                history_size: p.history_size,
                history_days: p.history_days,
            })
        }
        "climate" => to(&s.climate),
        "advanced" => to(&s.advanced),
        _ => Value::Null,
    }
}

/// The `section` as the API shows it: secrets are `{"set": bool}`.
pub async fn read_section(ctx: &SetupCtx, section: &str) -> Result<Value, ApiError> {
    if section == "mobile" {
        let held = ctx.sections.lock().await;
        return Ok(serde_json::to_value(mobile::view(&held.mobile)).expect("a view serializes"));
    }
    if section == "backup" {
        let held = ctx.sections.lock().await;
        return Ok(
            serde_json::to_value(backup::settings::view(&held.backup)).expect("a view serializes")
        );
    }
    if !known(section) {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "unknown section"));
    }
    let sections = ctx.sections.lock().await.clone();
    Ok(view(section, &sections))
}

async fn get_section(State(s): State<RouteState>, Path(section): Path<String>) -> Response {
    match read_section(&s.ctx, &section).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

/// The body as `T`, or a fixed refusal (a serde error could quote the value).
fn parse<T: DeserializeOwned>(body: Value, section: &str) -> Result<T, ApiError> {
    serde_json::from_value(body)
        .map_err(|_| unprocessable(&format!("The {section} settings are not valid")))
}

/// `scheme://host[:port]` of a Frigate address; a path or a default port never counts.
fn address_of(url: &str) -> String {
    crate::media::origin_of(url).unwrap_or_else(|| url.trim().to_lowercase())
}

/// The Frigate config a request would save, on top of the stored one.
fn frigate_candidate(old: &FrigateConfig, input: FrigateInput) -> Result<FrigateConfig, ApiError> {
    let clean = |v: Option<String>| v.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    let url = clean(input.url).map(|u| u.trim_end_matches('/').to_owned());
    let media_url = clean(input.media_url);
    // A login inside an address would be shown back by every read.
    if [&url, &media_url]
        .into_iter()
        .flatten()
        .any(|u| secret::split_userinfo(u).is_some())
    {
        return Err(unprocessable(
            "Put the user and password in their own fields",
        ));
    }
    if let Some(url) = &url {
        if crate::media::origin_of(url).is_none() {
            return Err(unprocessable("The Frigate address is not valid"));
        }
        let old_address = old.url.as_deref().map(address_of);
        secret::require_for_new_address(
            old_address.as_deref(),
            &address_of(url),
            &input.password,
            old.password.is_some(),
        )
        .map_err(unprocessable)?;
    }
    // The whole section is replaced: what is left out takes its default, except the connection
    // timeout, which no form sets.
    let mut cfg = FrigateConfig {
        connect_timeout_ms: old.connect_timeout_ms,
        ..FrigateConfig::default()
    };
    cfg.url = url;
    cfg.user = clean(input.user);
    cfg.password = secret::apply(old.password.clone(), input.password);
    cfg.media_url = media_url;
    cfg.bridge = input.bridge;
    cfg.labels = input.labels;
    cfg.cameras = input.cameras;
    cfg.zones = input.zones;
    cfg.live = input.live;
    if let Some(v) = input.cooldown {
        cfg.cooldown = v;
    }
    if let Some(v) = input.severity {
        cfg.severity = v;
    }
    if let Some(v) = input.ttl {
        cfg.ttl = v;
    }
    if let Some(v) = input.topic_prefix {
        cfg.topic_prefix = v;
    }
    Ok(cfg)
}

/// Applies `body` to the `section` of `next`.
fn merge(section: &str, body: Value, next: &mut Sections, file: &HubFile) -> Result<(), ApiError> {
    match section {
        "weather" => {
            let weather: WeatherSection = parse(body, section)?;
            // An imported city waiting for its lookup counts: the poller geocodes it.
            if weather.enabled && file.location.is_none() && file.pending_city.is_none() {
                return Err(unprocessable("Set the home location first"));
            }
            next.weather = weather;
        }
        "solcast" => {
            let input: SolcastInput = parse(body, section)?;
            let default = SolcastSection::default();
            next.solcast = SolcastSection {
                enabled: input.enabled,
                sites: input.sites.iter().map(|s| s.trim().to_owned()).collect(),
                daily_limit: input.daily_limit.unwrap_or(default.daily_limit),
                api_key: secret::apply(next.solcast.api_key.clone(), input.api_key),
                notify_unreachable: input.notify_unreachable,
            };
        }
        "frigate" => next.frigate = frigate_candidate(&next.frigate, parse(body, section)?)?,
        "push" => {
            let input: PushInput = parse(body, section)?;
            if let Secret::Set(key) = &input.service_account
                && project_id(key).is_none()
            {
                return Err(unprocessable("The service account key is not valid"));
            }
            let default = PushSection::default();
            next.push = PushSection {
                service_account: secret::apply(
                    next.push.service_account.clone(),
                    input.service_account,
                ),
                prefix: input.prefix.unwrap_or(default.prefix),
                retry_interval: input.retry_interval.unwrap_or(default.retry_interval),
                max_retries: input.max_retries.unwrap_or(default.max_retries),
                history_size: input.history_size.unwrap_or(default.history_size),
                history_days: input.history_days.unwrap_or(default.history_days),
            };
        }
        "climate" => next.climate = parse(body, section)?,
        "advanced" => next.advanced = parse::<AdvancedSection>(body, section)?,
        _ => return Err(ApiError::new(StatusCode::NOT_FOUND, "unknown section")),
    }
    Ok(())
}

/// Whether the hub could boot with `file` and `sections`: the check `main` runs before
/// anything starts. Every write that leads to a restart runs it first, so a saved change can
/// never leave a hub that refuses to start.
pub(super) fn check_boot(file: &HubFile, sections: &Sections) -> Result<(), String> {
    assemble(file, sections).validate_all()
}

/// Replaces the `section` with `body`: validates, saves its row to the database and the
/// whole settings to the cache, then schedules the restart. Returns `{"restarting": true}`.
pub async fn write_section(
    ctx: &Arc<SetupCtx>,
    section: &str,
    body: Value,
) -> Result<Value, ApiError> {
    let unavailable = |m: &str| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, m);
    if section == "mobile" {
        return write_mobile(ctx, body).await;
    }
    if section == "backup" {
        return write_backup(ctx, body).await;
    }
    if !known(section) {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "unknown section"));
    }
    if ctx.restarting.load(Ordering::SeqCst) {
        return Err(unavailable("restarting"));
    }
    if ctx.mode != Mode::Run {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "finish MQTT and database setup first",
        ));
    }
    let Some(db) = ctx.db.as_ref() else {
        return Err(unavailable("database unreachable"));
    };
    // Held from the snapshot to the replacement, so this save and the import copy never
    // overwrite each other with a stale copy.
    let mut held = ctx.sections.lock().await;
    if ctx.restarting.load(Ordering::SeqCst) {
        return Err(unavailable("restarting"));
    }
    let file = ctx.file.lock().await.clone();
    let mut next = held.clone();
    merge(section, body, &mut next, &file)?;
    check_boot(&file, &next).map_err(|message| unprocessable(&message))?;
    // Only the edited row: the others in memory may be defaults the hub fell back to while
    // the database was down, and must not overwrite the stored ones.
    if let Err(error) = save_section(db, section, &next).await {
        tracing::warn!(%error, "settings: cannot save to the database");
        return Err(unavailable("database unreachable"));
    }
    if let Err(error) = write_cache(&ctx.data_dir, &next) {
        // Boot reads the cache until the import copy is done, so a change the cache lacks
        // could come back reverted: report the failure, keep the old settings, and put the
        // table back as it was (best effort).
        tracing::error!(error = %error.kind(), "settings: cannot write the settings cache");
        if let Err(error) = save_section(db, section, &held).await {
            tracing::warn!(%error, "settings: cannot restore the database after a cache failure");
        }
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not save the settings cache",
        ));
    }
    *held = next;
    restart::schedule_restart(ctx, RESTART_DELAY);
    Ok(json!({ "restarting": true }))
}

/// `mobile` is stored live: no restart, and nothing else reads it at boot.
async fn write_mobile(ctx: &Arc<SetupCtx>, body: Value) -> Result<Value, ApiError> {
    if ctx.restarting.load(Ordering::SeqCst) {
        return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "restarting"));
    }
    if ctx.mode != Mode::Run {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "finish MQTT and database setup first",
        ));
    }
    let input: nidoo_api::MobileInput = parse(body, "mobile")?;
    let stored = ctx.sections.lock().await.mobile.clone();
    let next = mobile::apply_input(&stored, input).map_err(unprocessable)?;
    let saved = save_live(ctx, "mobile", |s| s.mobile = next)
        .await
        .map_err(|why| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, why))?;
    Ok(serde_json::to_value(mobile::view(&saved.mobile)).expect("a view serializes"))
}

/// `backup` is stored live like `mobile`: the job reads it from the live sections.
async fn write_backup(ctx: &Arc<SetupCtx>, body: Value) -> Result<Value, ApiError> {
    if ctx.restarting.load(Ordering::SeqCst) {
        return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "restarting"));
    }
    if ctx.mode != Mode::Run {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "finish MQTT and database setup first",
        ));
    }
    let input: nidoo_api::BackupInput = parse(body, "backup")?;
    let stored = ctx.sections.lock().await.backup.clone();
    let next = backup::settings::apply_input(&stored, input).map_err(|m| unprocessable(&m))?;
    let saved = save_live(ctx, "backup", |s| s.backup = next)
        .await
        .map_err(|why| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, why))?;
    Ok(serde_json::to_value(backup::settings::view(&saved.backup)).expect("a view serializes"))
}

async fn put_section(
    State(s): State<RouteState>,
    Path(section): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    match write_section(&s.ctx, &section, body).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

fn failed(message: &str) -> Response {
    Json(TestResult {
        ok: false,
        message: message.to_owned(),
        detail: None,
    })
    .into_response()
}

async fn test_solcast(State(s): State<RouteState>, Json(body): Json<Value>) -> Response {
    let input: SolcastInput = match parse(body, "solcast") {
        Ok(i) => i,
        Err(e) => return e.into_response(),
    };
    let stored = s.ctx.sections.lock().await.solcast.api_key.clone();
    let key = secret::apply(stored, input.api_key).filter(|k| !k.trim().is_empty());
    let Some(key) = key else {
        return failed("Enter the API key");
    };
    Json(probe::solcast(&s.solcast_base, &key).await).into_response()
}

async fn test_frigate(State(s): State<RouteState>, Json(body): Json<Value>) -> Response {
    let input: FrigateInput = match parse(body, "frigate") {
        Ok(i) => i,
        Err(e) => return e.into_response(),
    };
    let old = s.ctx.sections.lock().await.frigate.clone();
    let cfg = match frigate_candidate(&old, input) {
        Ok(cfg) => cfg,
        Err(e) => return e.into_response(),
    };
    if cfg.url.is_none() {
        return failed("Enter the Frigate address");
    }
    Json(probe::frigate(&cfg).await).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_ignores_the_path_and_default_ports() {
        assert_eq!(
            address_of("http://Frigate.example.com:80/a/b"),
            address_of("http://frigate.example.com")
        );
        assert_ne!(
            address_of("http://frigate.example.com:5000"),
            address_of("https://frigate.example.com:5000")
        );
    }

    #[test]
    fn project_id_never_fails_loudly() {
        assert_eq!(
            project_id(r#"{"project_id":"p-1"}"#).as_deref(),
            Some("p-1")
        );
        assert_eq!(project_id("{not json"), None);
        assert_eq!(project_id(r#"{"project_id":3}"#), None);
    }

    #[tokio::test]
    async fn the_system_section_is_not_a_generic_section() {
        let ctx = SetupCtx::inert();
        let before = ctx.sections.lock().await.clone();
        let read = read_section(&ctx, "system").await.unwrap_err();
        assert_eq!(read.status, StatusCode::NOT_FOUND);
        let write = write_section(&ctx, "system", json!({})).await.unwrap_err();
        assert_eq!(write.status, StatusCode::NOT_FOUND);
        assert_eq!(*ctx.sections.lock().await, before);
        assert!(!ctx.restarting.load(Ordering::SeqCst));
        let mut next = before.clone();
        let file = HubFile::default();
        assert_eq!(
            merge("system", json!({}), &mut next, &file)
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(next, before);
    }
}
