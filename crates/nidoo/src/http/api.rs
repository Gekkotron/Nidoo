//! The JSON API: `/health` and the `/api/v1` routes, with hub errors mapped to status codes.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Body as HttpBody;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, RawQuery, State};
use axum::http::StatusCode;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use nidoo_alarm::doc::AlarmSettings;
use nidoo_alarm::view::AlarmView;
use nidoo_api::{
    AlertView, Ambiance, AmbianceView, ApiError, ApplyReport, AreaView, CameraInfo, CamerasView,
    CommandResponse, ConnectionStatus, Dashboard, DashboardReorder, DashboardView, EnergyKind,
    EntityView, FlowView, FlowWriteOutcome, FlowsConfigView, GridDay, GroupSpec, GroupView,
    HistoryDaily, HistoryRange, HistorySeries, HistoryToday, HomeSettings, IgnoredView, LastChange,
    NetworkWatchSpec, NetworkWatchView, OverridePatch, RunView, SchedulePreview, Snapshot,
    VirtualSpec, WeatherView,
};
use nidoo_climate::doc::{HouseSettings, Rules, ZoneSettings};
use nidoo_climate::view::{ClimateView, DecisionLog, LearnStatus, OverrideRequest, RoomEcoRequest};
use nidoo_core::flow::{Flow, Schedule};
use nidoo_core::model::{AreasDocument, Command};
use serde::Serialize;

use crate::history::{HistoryError, query as history_query};
use crate::hub::{Hub, HubError};
use crate::media::{CameraFile, MediaError, Upstream, is_camera_id};

type HubState = State<Arc<Hub>>;

/// A JSON body, or the rejection that becomes a JSON error reply.
type Body<T> = Result<Json<T>, JsonRejection>;

pub(super) fn routes() -> Router<Arc<Hub>> {
    Router::new()
        .route("/snapshot", get(snapshot))
        .route("/entities/{id}/command", post(command))
        .route(
            "/entities/{id}/override",
            patch(set_override).delete(clear_override),
        )
        .route("/entities/{id}/picture", get(entity_picture))
        .route("/areas", put(set_areas))
        .route("/areas/{id}", patch(rename_area))
        .route("/ignored", get(ignored))
        .route("/alerts", get(alerts))
        .route("/alerts/{id}/actions/{action}", post(alert_action))
        .route("/push", get(push_status))
        .route("/media", get(media))
        .route("/cameras", get(cameras))
        .route("/cameras/{id}/latest.jpg", get(camera_latest))
        .route("/cameras/{id}/stream", get(camera_stream))
        .route("/flows", get(flows))
        .route("/schedule", get(schedule_preview))
        .route("/config/flows", get(flows_config))
        .route(
            "/flows/{id}",
            get(get_flow).put(put_flow).delete(delete_flow),
        )
        .route("/flows/{id}/enabled", post(set_flow_enabled))
        .route("/flows/{id}/area", post(set_flow_area))
        .route("/flows/{id}/run", post(run_flow))
        .route("/groups", get(list_groups))
        .route("/groups/{id}", put(put_group).delete(delete_group))
        .route("/network", get(list_network_watches))
        .route(
            "/network/{id}",
            put(put_network_watch).delete(delete_network_watch),
        )
        .route("/ambiances", get(list_ambiances))
        .route("/ambiances/{id}", put(put_ambiance).delete(delete_ambiance))
        .route("/ambiances/{id}/apply", post(apply_ambiance))
        .route("/weather", get(weather))
        .route("/solar", get(solar))
        .route("/home/settings", get(home_settings).put(put_home_settings))
        .route("/alarm", get(alarm))
        .route("/alarm/settings", put(put_alarm_settings))
        .route("/alarm/test-chirp", post(alarm_test_chirp))
        .route("/climate", get(climate))
        .route("/climate/house", put(put_climate_house))
        .route("/climate/zones/{id}", put(put_climate_zone))
        .route(
            "/climate/zones/{id}/override",
            post(set_climate_override).delete(clear_climate_override),
        )
        .route(
            "/climate/zones/{id}/eco",
            put(set_climate_eco).delete(clear_climate_eco),
        )
        .route(
            "/climate/zones/{id}/proposal/approve",
            post(approve_climate),
        )
        .route("/climate/zones/{id}/proposal/reject", post(reject_climate))
        .route("/climate/zones/{id}/learn", post(learn_climate))
        .route("/climate/zones/{id}/decisions", get(climate_decisions))
        .route("/dashboards", get(list_dashboards))
        .route("/dashboards/order", post(reorder_dashboards))
        .route(
            "/dashboards/{id}",
            get(get_dashboard)
                .put(put_dashboard)
                .delete(delete_dashboard),
        )
        .route("/virtual/{id}", put(put_virtual).delete(delete_virtual))
        .route("/history/series", get(history_series))
        .route("/history/daily", get(history_daily))
        .route("/history/today", get(history_today))
        .route("/history/last_change", get(history_last_change))
        .route("/history/grid_day", get(history_grid_day))
        .fallback(not_found)
}

/// The picture of a camera or image entity: stored bytes, or the URL fetched now.
async fn entity_picture(
    State(hub): HubState,
    Path(id): Path<String>,
) -> Result<Response, HttpError> {
    use crate::pictures::Picture;
    let picture = hub.pictures().get(&id).ok_or_else(|| HttpError {
        status: StatusCode::NOT_FOUND,
        error: "no picture for this entity".to_owned(),
    })?;
    match picture {
        Picture::Bytes { content_type, body } => Ok(picture_response(&content_type, body)),
        Picture::Url(url) => {
            let (content_type, body) = fetch_picture(&url).await.map_err(|err| {
                tracing::warn!(%id, %err, "could not fetch picture URL");
                HttpError {
                    status: StatusCode::BAD_GATEWAY,
                    error: "could not fetch picture URL".to_owned(),
                }
            })?;
            Ok(picture_response(&content_type, body))
        }
    }
}

/// A picture reply that can only be shown as an image: anything that is not `image/*` is
/// served as an opaque download, and sniffing is off, so a broker-supplied page cannot run
/// from the hub's origin.
fn picture_response(content_type: &str, body: impl Into<axum::body::Body>) -> Response {
    let content_type = if content_type
        .trim()
        .get(..6)
        .is_some_and(|p| p.eq_ignore_ascii_case("image/"))
    {
        content_type
    } else {
        "application/octet-stream"
    };
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        body.into(),
    )
        .into_response()
}

/// Fetches `url` (no redirects, 10 s, 2xx, at most 5 MiB).
async fn fetch_picture(url: &str) -> Result<(String, bytes::Bytes), String> {
    use crate::pictures::MAX_PICTURE_BYTES;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|err| err.to_string())?;
    let mut response = client
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|err| err.to_string())?;
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .to_owned();
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|err| err.to_string())? {
        if body.len() + chunk.len() > MAX_PICTURE_BYTES {
            return Err("picture over 5 MiB".to_owned());
        }
        body.extend_from_slice(&chunk);
    }
    Ok((content_type, bytes::Bytes::from(body)))
}

/// An error reply: the status and an [`ApiError`] body.
pub(super) struct HttpError {
    status: StatusCode,
    error: String,
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        (self.status, Json(ApiError { error: self.error })).into_response()
    }
}

impl From<HubError> for HttpError {
    fn from(err: HubError) -> Self {
        let status = match &err {
            HubError::UnknownEntity(_)
            | HubError::UnknownAlert(_)
            | HubError::UnknownAction(_)
            | HubError::UnknownFlow(_)
            | HubError::UnknownArea(_)
            | HubError::UnknownDashboard(_)
            | HubError::UnknownAmbiance(_) => StatusCode::NOT_FOUND,
            HubError::Rejected(_) | HubError::Command(_) | HubError::InvalidFlow(_) => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            HubError::Patch(_) => StatusCode::BAD_REQUEST,
            HubError::Publish(_) => StatusCode::SERVICE_UNAVAILABLE,
        };
        Self {
            status,
            error: err.to_string(),
        }
    }
}

impl From<JsonRejection> for HttpError {
    fn from(rejection: JsonRejection) -> Self {
        Self {
            status: rejection.status(),
            error: rejection.body_text(),
        }
    }
}

impl From<HistoryError> for HttpError {
    fn from(err: HistoryError) -> Self {
        let status = match &err {
            HistoryError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            HistoryError::NotRecorded => StatusCode::NOT_FOUND,
            HistoryError::BadRequest(_) => StatusCode::BAD_REQUEST,
            HistoryError::NoGridMeter => StatusCode::NOT_FOUND,
        };
        Self {
            status,
            error: err.code().to_owned(),
        }
    }
}

/// A `400 bad_request` reply built from a user-facing reason — the body stays stable.
fn bad_request() -> HttpError {
    HttpError {
        status: StatusCode::BAD_REQUEST,
        error: "bad_request".to_owned(),
    }
}

/// A `503 history_unavailable` reply, used when `[history] url` is not configured.
fn history_unavailable() -> HttpError {
    HttpError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        error: "history_unavailable".to_owned(),
    }
}

/// Parses a `?a=b&c=d` string with repeating keys. Values are percent-decoded.
fn parse_query(q: Option<&str>) -> Vec<(String, String)> {
    let Some(q) = q else {
        return Vec::new();
    };
    url::form_urlencoded::parse(q.as_bytes())
        .into_owned()
        .collect()
}

fn get_one<'a>(params: &'a [(String, String)], key: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn get_many<'a>(params: &'a [(String, String)], key: &str) -> Vec<&'a str> {
    params
        .iter()
        .filter_map(|(k, v)| (k == key).then_some(v.as_str()))
        .collect()
}

fn now_micros() -> i64 {
    crate::clock::now_micros()
}

fn require_history(hub: &Hub) -> Result<&std::sync::Arc<crate::history::History>, HttpError> {
    hub.history().ok_or_else(history_unavailable)
}

/// The `/health` body.
#[derive(Serialize)]
pub(super) struct Health {
    status: &'static str,
    connection: ConnectionStatus,
    entities: usize,
}

pub(super) async fn health(State(hub): HubState) -> Json<Health> {
    let connection = hub.connection().borrow().clone();
    let (entities, _) = hub.counts();
    Json(Health {
        status: "ok",
        connection,
        entities,
    })
}

pub(super) async fn not_found() -> HttpError {
    HttpError {
        status: StatusCode::NOT_FOUND,
        error: "no such route".to_owned(),
    }
}

async fn snapshot(State(hub): HubState) -> Json<Snapshot> {
    Json(hub.snapshot())
}

async fn command(
    State(hub): HubState,
    Path(id): Path<String>,
    body: Body<Command>,
) -> Result<Json<CommandResponse>, HttpError> {
    let Json(cmd) = body?;
    Ok(Json(hub.send_command(&id, cmd).await?))
}

async fn set_override(
    State(hub): HubState,
    Path(id): Path<String>,
    body: Body<OverridePatch>,
) -> Result<Json<EntityView>, HttpError> {
    let Json(patch) = body?;
    Ok(Json(hub.set_override(&id, &patch).await?))
}

async fn clear_override(
    State(hub): HubState,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    hub.clear_override(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_areas(
    State(hub): HubState,
    body: Body<AreasDocument>,
) -> Result<Json<Vec<AreaView>>, HttpError> {
    let Json(doc) = body?;
    Ok(Json(hub.set_areas(doc).await?))
}

/// The `PATCH /areas/{id}` body.
#[derive(serde::Deserialize)]
struct RenameArea {
    name: String,
}

async fn rename_area(
    State(hub): HubState,
    Path(id): Path<String>,
    body: Body<RenameArea>,
) -> Result<Json<Vec<AreaView>>, HttpError> {
    let Json(RenameArea { name }) = body?;
    Ok(Json(hub.rename_area(&id, &name).await?))
}

async fn ignored(State(hub): HubState) -> Json<Vec<IgnoredView>> {
    Json(hub.ignored())
}

/// Whether push sends (`off` without credentials, `syncing`, `ready`), the registered installs
/// and the alerts waiting in the retry queue.
async fn push_status(State(hub): HubState) -> Json<nidoo_push::PushStatus> {
    Json(hub.push().status())
}

async fn alerts(State(hub): HubState) -> Json<Vec<AlertView>> {
    Json(hub.alerts())
}

#[derive(serde::Deserialize)]
pub(super) struct MediaQuery {
    url: Option<String>,
}

/// Serves a Frigate snapshot or clip through the hub: the upstream status, the headers a
/// `<img>` or `<video>` needs and the streamed body. `400` without a valid absolute `url`,
/// `403` off the Frigate origin or without a `[frigate]` section, `502` when Frigate is out
/// of reach or refuses the login.
async fn media(
    State(hub): HubState,
    Query(query): Query<MediaQuery>,
    headers: HeaderMap,
) -> Result<Response, HttpError> {
    let url = query
        .url
        .filter(|u| crate::media::origin_of(u).is_some())
        .ok_or(HttpError {
            status: StatusCode::BAD_REQUEST,
            error: "missing or invalid `url`".to_owned(),
        })?;
    let proxy = hub.media().ok_or(HttpError {
        status: StatusCode::FORBIDDEN,
        error: "no Frigate configured".to_owned(),
    })?;
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let upstream = proxy.fetch(&url, range).await.map_err(media_error)?;
    Ok(upstream_response(upstream))
}

fn media_error(err: MediaError) -> HttpError {
    HttpError {
        status: match err {
            MediaError::Forbidden => StatusCode::FORBIDDEN,
            MediaError::Unavailable(_) => StatusCode::BAD_GATEWAY,
        },
        error: err.to_string(),
    }
}

/// Frigate's status, the headers an `<img>` or `<video>` needs and the streamed body.
fn upstream_response(upstream: Upstream) -> Response {
    let mut response = Response::new(HttpBody::empty());
    *response.status_mut() =
        StatusCode::from_u16(upstream.status).unwrap_or(StatusCode::BAD_GATEWAY);
    let pairs = [
        (header::CONTENT_TYPE, upstream.content_type),
        (header::CONTENT_LENGTH, upstream.content_length),
        (header::CONTENT_RANGE, upstream.content_range),
        (header::ACCEPT_RANGES, upstream.accept_ranges),
    ];
    for (name, value) in pairs {
        if let Some(value) = value.as_deref().and_then(|v| HeaderValue::from_str(v).ok()) {
            response.headers_mut().insert(name, value);
        }
    }
    *response.body_mut() = upstream.body;
    response
}

/// The Frigate cameras (names from the bridge's `pretty` rule) and the `[frigate] live`
/// switch. An empty list without a `[frigate]` section or when Frigate is out of reach.
async fn cameras(State(hub): HubState) -> Json<CamerasView> {
    let Some(proxy) = hub.media() else {
        return Json(CamerasView::default());
    };
    let cameras = proxy
        .camera_ids()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|id| CameraInfo {
            name: nidoo_frigate::pretty(&id),
            id,
        })
        .collect();
    Json(CamerasView {
        live: proxy.live(),
        cameras,
    })
}

#[derive(serde::Deserialize)]
pub(super) struct FrameQuery {
    h: Option<u32>,
}

/// The frame height when the browser names none.
const DEFAULT_FRAME_HEIGHT: u32 = 360;
const MAX_FRAME_HEIGHT: u32 = 1080;

/// One live frame of camera `id`; see [`camera_frame`].
async fn camera_latest(
    State(hub): HubState,
    Path(id): Path<String>,
    Query(query): Query<FrameQuery>,
) -> Result<Response, HttpError> {
    camera_frame(&hub, &id, CameraFile::Latest, query.h).await
}

/// The MJPEG stream of camera `id`, with no total timeout; see [`camera_frame`].
async fn camera_stream(
    State(hub): HubState,
    Path(id): Path<String>,
    Query(query): Query<FrameQuery>,
) -> Result<Response, HttpError> {
    camera_frame(&hub, &id, CameraFile::Stream, query.h).await
}

/// `403` while `[frigate] live` is off or without Frigate, `404` for an id that is not one of
/// Frigate's enabled cameras (nothing is fetched for it), `502` when Frigate is out of reach,
/// otherwise Frigate's reply, never cached. `h` is clamped to `1..=1080`.
async fn camera_frame(
    hub: &Hub,
    id: &str,
    file: CameraFile,
    height: Option<u32>,
) -> Result<Response, HttpError> {
    let proxy = hub.media().filter(|proxy| proxy.live()).ok_or(HttpError {
        status: StatusCode::FORBIDDEN,
        error: "live camera frames are off".to_owned(),
    })?;
    let not_found = || HttpError {
        status: StatusCode::NOT_FOUND,
        error: format!("no camera {id}"),
    };
    if !is_camera_id(id) {
        return Err(not_found());
    }
    let known = proxy.has_camera(id).await.map_err(|err| HttpError {
        status: StatusCode::BAD_GATEWAY,
        error: err.to_string(),
    })?;
    if !known {
        return Err(not_found());
    }
    let height = height
        .unwrap_or(DEFAULT_FRAME_HEIGHT)
        .clamp(1, MAX_FRAME_HEIGHT);
    let upstream = proxy
        .fetch_camera(id, file, height)
        .await
        .map_err(media_error)?;
    let mut response = upstream_response(upstream);
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

/// Publishes the reply for a tapped alert button; `202` once it is handed to the broker task.
async fn alert_action(
    State(hub): HubState,
    Path((id, action)): Path<(String, String)>,
) -> Result<StatusCode, HttpError> {
    hub.reply(&id, &action)?;
    Ok(StatusCode::ACCEPTED)
}

/// Every flow with its engine state.
async fn flows(State(hub): HubState) -> Json<Vec<FlowView>> {
    Json(hub.flows())
}

/// One flow; `404` for an unknown id.
async fn get_flow(
    State(hub): HubState,
    Path(id): Path<String>,
) -> Result<Json<FlowView>, HttpError> {
    hub.flow(&id)
        .map(Json)
        .ok_or_else(|| HubError::UnknownFlow(id).into())
}

/// Creates or replaces a flow. The path id wins: a body without an `id` takes it, a body with
/// another one is `422 id mismatch`. `422` for a flow that does not validate, `503` while the
/// broker link is down.
async fn put_flow(
    State(hub): HubState,
    Path(id): Path<String>,
    body: Body<serde_json::Value>,
) -> Result<Json<FlowWriteOutcome>, HttpError> {
    let Json(mut value) = body?;
    if let Some(object) = value.as_object_mut() {
        match object.get("id").and_then(|v| v.as_str()) {
            None | Some("") => {
                object.insert("id".to_owned(), serde_json::Value::String(id.clone()));
            }
            Some(body_id) if body_id != id => return Err(id_mismatch(&id, body_id)),
            Some(_) => {}
        }
    }
    let flow = Flow::decode(value.to_string().as_bytes()).map_err(|error| HttpError {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        error,
    })?;
    Ok(Json(hub.set_flow(flow).await?))
}

/// The `[flows]` settings: the configured house point, the zone and whether the Paris fallback
/// is active.
async fn flows_config(State(hub): HubState) -> Json<FlowsConfigView> {
    Json(hub.flows_config_view())
}

/// Removes a flow (204); `404` for an unknown id.
async fn delete_flow(
    State(hub): HubState,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    hub.delete_flow(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Every valid dashboard, House first.
async fn list_dashboards(State(hub): HubState) -> Json<Vec<DashboardView>> {
    Json(hub.dashboards())
}

/// One dashboard by id; `404` for an unknown or invalid one.
async fn get_dashboard(
    State(hub): HubState,
    Path(id): Path<String>,
) -> Result<Json<DashboardView>, HttpError> {
    hub.dashboard(&id)
        .map(Json)
        .ok_or_else(|| HttpError::from(HubError::UnknownDashboard(id)))
}

/// `403` for the reserved `house` id, `422` when the path and body ids differ or a widget
/// does not validate, `503` while the broker link is down.
async fn put_dashboard(
    State(hub): HubState,
    Path(id): Path<String>,
    body: Body<Dashboard>,
) -> Result<Json<Dashboard>, HttpError> {
    reject_house(&id)?;
    let Json(dashboard) = body?;
    if dashboard.id != id {
        return Err(id_mismatch(&id, &dashboard.id));
    }
    Ok(Json(hub.set_dashboard(dashboard).await?))
}

/// `204` once the dashboard is cleared (an invalid one too); `403` for the reserved `house` id.
async fn delete_dashboard(
    State(hub): HubState,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    reject_house(&id)?;
    hub.delete_dashboard(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `204` after reassigning orders; `422` when the list does not match the current dashboards.
async fn reorder_dashboards(
    State(hub): HubState,
    body: Body<DashboardReorder>,
) -> Result<StatusCode, HttpError> {
    let Json(body) = body?;
    hub.reorder_dashboards(&body.order).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `422 id mismatch` for a PUT whose body names another id than its path, the same on every
/// route.
fn id_mismatch(path: &str, body: &str) -> HttpError {
    HttpError {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        error: format!("id mismatch: the path says `{path}`, the body `{body}`"),
    }
}

fn reject_house(id: &str) -> Result<(), HttpError> {
    if id == nidoo_api::HOUSE_ID {
        return Err(HttpError {
            status: StatusCode::FORBIDDEN,
            error: format!("`{id}` is reserved"),
        });
    }
    Ok(())
}

/// The `POST /flows/{id}/enabled` body.
#[derive(serde::Deserialize)]
struct EnabledBody {
    enabled: bool,
}

/// Enables or disables a flow and returns it as it now stands.
async fn set_flow_enabled(
    State(hub): HubState,
    Path(id): Path<String>,
    body: Body<EnabledBody>,
) -> Result<Json<FlowView>, HttpError> {
    let Json(EnabledBody { enabled }) = body?;
    Ok(Json(hub.set_flow_enabled(&id, enabled).await?))
}

/// The `POST /flows/{id}/area` body; `null` clears the room. The key is required (`{}` or a
/// misspelt key is a `422`, not a silent clear): `deserialize_with` stops serde from defaulting
/// a missing `Option` field to `None`.
#[derive(serde::Deserialize)]
struct AreaBody {
    #[serde(deserialize_with = "serde::Deserialize::deserialize")]
    area: Option<String>,
}

/// Files a flow under a room (or clears it) and returns it as it now stands.
async fn set_flow_area(
    State(hub): HubState,
    Path(id): Path<String>,
    body: Body<AreaBody>,
) -> Result<Json<FlowView>, HttpError> {
    let Json(AreaBody { area }) = body?;
    Ok(Json(hub.set_flow_area(&id, area).await?))
}

/// The optional body of a run request: the `trigger.manual` card to start from.
#[derive(serde::Deserialize)]
struct RunBody {
    #[serde(default)]
    node: Option<String>,
}

/// Starts a run by hand, from the Manual card named in the body or else the first trigger;
/// `202` with the run, `422` when the flow is disabled, invalid or busy, or the card is not a
/// manual trigger of it.
async fn run_flow(
    State(hub): HubState,
    Path(id): Path<String>,
    body: Option<Json<RunBody>>,
) -> Result<(StatusCode, Json<RunView>), HttpError> {
    let node = body.and_then(|Json(body)| body.node);
    Ok((
        StatusCode::ACCEPTED,
        Json(hub.run_flow_from(&id, node.as_deref())?),
    ))
}

#[derive(serde::Deserialize)]
struct ScheduleQuery {
    cron: String,
    count: Option<usize>,
}

/// The next ticks of a cron expression in the house time zone: `count` of them (default 3,
/// at most 10). `422` for an expression that does not parse.
async fn schedule_preview(
    State(hub): HubState,
    Query(query): Query<ScheduleQuery>,
) -> Result<Json<SchedulePreview>, HttpError> {
    let schedule = Schedule::parse(&query.cron).map_err(|err| HttpError {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        error: err.to_string(),
    })?;
    let zone = hub.zone();
    let mut at = chrono::Utc::now().with_timezone(&zone);
    let mut next = Vec::new();
    for _ in 0..query.count.unwrap_or(3).clamp(1, 10) {
        let Some(tick) = schedule.next_after(&at) else {
            break;
        };
        next.push(tick.to_rfc3339());
        at = tick;
    }
    Ok(Json(SchedulePreview {
        timezone: zone.name().to_owned(),
        next,
    }))
}

/// The stored groups (`200`); `503` while `[groups] enabled` is false.
async fn list_groups(State(hub): HubState) -> Result<Json<Vec<GroupView>>, HttpError> {
    Ok(Json(hub.list_groups()?))
}

/// Creates or replaces a group (`200` with its view); `422` for a bad id or spec, `503` while
/// groups are off or the broker link is down.
async fn put_group(
    State(hub): HubState,
    Path(id): Path<String>,
    body: Body<GroupSpec>,
) -> Result<Json<GroupView>, HttpError> {
    let Json(spec) = body?;
    Ok(Json(hub.set_group(&id, spec).await?))
}

/// Removes a group (204); `404` for an id that is not one.
async fn delete_group(
    State(hub): HubState,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    hub.delete_group(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The stored network watches (`200`); `503` while `[network] enabled` is false.
async fn list_network_watches(
    State(hub): HubState,
) -> Result<Json<Vec<NetworkWatchView>>, HttpError> {
    Ok(Json(hub.list_network_watches()?))
}

/// Creates or replaces a network watch (`200` with its view); `422` for a bad id or spec, `503`
/// while network presence is off or the broker link is down.
async fn put_network_watch(
    State(hub): HubState,
    Path(id): Path<String>,
    body: Body<NetworkWatchSpec>,
) -> Result<Json<NetworkWatchView>, HttpError> {
    let Json(spec) = body?;
    Ok(Json(hub.set_network_watch(&id, spec).await?))
}

/// Removes a network watch (204); `404` for an id that is not one.
async fn delete_network_watch(
    State(hub): HubState,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    hub.delete_network_watch(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The stored ambiances (`200`), the unreadable ones included with their reason.
async fn list_ambiances(State(hub): HubState) -> Json<Vec<AmbianceView>> {
    Json(hub.ambiances())
}

/// Creates or replaces an ambiance (`200` with its view). An empty body id is taken from the
/// path; a different one is `422`, as is an ambiance the house refuses; `503` while the broker
/// link is down.
async fn put_ambiance(
    State(hub): HubState,
    Path(id): Path<String>,
    body: Body<Ambiance>,
) -> Result<Json<AmbianceView>, HttpError> {
    let Json(mut ambiance) = body?;
    if ambiance.id.is_empty() {
        ambiance.id.clone_from(&id);
    } else if ambiance.id != id {
        return Err(id_mismatch(&id, &ambiance.id));
    }
    Ok(Json(hub.set_ambiance(ambiance).await?))
}

/// Removes an ambiance (204); `404` for an id that is not one.
async fn delete_ambiance(
    State(hub): HubState,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    hub.delete_ambiance(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Runs an ambiance's actions once, in order, and returns the report (`200`); `404` for an
/// unknown id, `422` for an invalid or already running one, `503` while the broker is offline.
async fn apply_ambiance(
    State(hub): HubState,
    Path(id): Path<String>,
) -> Result<Json<ApplyReport>, HttpError> {
    Ok(Json(hub.apply_ambiance(&id).await?))
}

/// The outdoor weather (`200`); `503` while `[weather]` is off or before the first fetch.
async fn weather(State(hub): HubState) -> Result<Json<WeatherView>, HttpError> {
    Ok(Json(hub.weather_view()?))
}

/// The Solcast solar forecast (`200`); `503` while `[solcast]` is off or before the first fetch.
async fn solar(State(hub): HubState) -> Result<Json<nidoo_api::SolarView>, HttpError> {
    Ok(Json(hub.solar_view_with_history().await?))
}

/// The home settings document (`200`), defaults until one is saved.
async fn home_settings(State(hub): HubState) -> Json<HomeSettings> {
    Json(hub.home_settings())
}

/// Replaces the home settings (`200` with the stored document); `422` when they do not
/// validate, `503` while the broker link is down.
async fn put_home_settings(
    State(hub): HubState,
    body: Body<HomeSettings>,
) -> Result<Json<HomeSettings>, HttpError> {
    let Json(settings) = body?;
    Ok(Json(hub.set_home_settings(settings)?))
}

/// The alarm as one document (`200`): settings, state and warnings.
async fn alarm(State(hub): HubState) -> Json<AlarmView> {
    Json(hub.alarm_view())
}

/// Replaces the alarm settings (204); `422` when they do not validate or name an entity the
/// house does not have, `503` while the broker link is down.
async fn put_alarm_settings(
    State(hub): HubState,
    body: Body<AlarmSettings>,
) -> Result<StatusCode, HttpError> {
    let Json(settings) = body?;
    hub.set_alarm_settings(settings).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Which siren of the saved settings to chirp.
#[derive(serde::Deserialize)]
struct TestChirp {
    siren: usize,
}

/// Sounds one siren's chirp (204); `422` when there is no such siren, it has no chirp or the alarm rings.
async fn alarm_test_chirp(
    State(hub): HubState,
    body: Body<TestChirp>,
) -> Result<StatusCode, HttpError> {
    let Json(request) = body?;
    hub.alarm_test_chirp(request.siren)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The climate engine as one document (`200`); `503` while `[climate] enabled` is false.
async fn climate(State(hub): HubState) -> Result<Json<ClimateView>, HttpError> {
    Ok(Json(hub.climate_view()?))
}

/// Replaces the house settings (204); `422` when they do not validate, `503` while climate is
/// off or the broker link is down.
async fn put_climate_house(
    State(hub): HubState,
    body: Body<HouseSettings>,
) -> Result<StatusCode, HttpError> {
    let Json(house) = body?;
    hub.set_climate_house(house)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Replaces the settings of a zone (204); `404` for an area that is not a zone, `422` for
/// settings that do not validate.
async fn put_climate_zone(
    State(hub): HubState,
    Path(id): Path<String>,
    body: Body<ZoneSettings>,
) -> Result<StatusCode, HttpError> {
    let Json(settings) = body?;
    hub.set_climate_zone(&id, settings)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Holds a target and/or mode on the zone (204); exactly one of `minutes` and `until`, else
/// `422`.
async fn set_climate_override(
    State(hub): HubState,
    Path(id): Path<String>,
    body: Body<OverrideRequest>,
) -> Result<StatusCode, HttpError> {
    let Json(request) = body?;
    let ov = request
        .into_override(crate::clock::now())
        .map_err(HubError::Rejected)?;
    hub.set_climate_override(&id, ov).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Removes every override of the zone (204).
async fn clear_climate_override(
    State(hub): HubState,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    hub.clear_climate_override(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Holds the zone in eco until `until` (Unix seconds, 204); a past `until` is `422`.
async fn set_climate_eco(
    State(hub): HubState,
    Path(id): Path<String>,
    body: Body<RoomEcoRequest>,
) -> Result<StatusCode, HttpError> {
    let Json(request) = body?;
    let eco = request
        .into_room_eco(crate::clock::now())
        .map_err(HubError::Rejected)?;
    hub.set_climate_eco(&id, eco).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Ends the zone's eco (204).
async fn clear_climate_eco(
    State(hub): HubState,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    hub.clear_climate_eco(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Approves the pending proposal of a zone (204), with optional edited rules as the body;
/// `422` when there is none. The body is optional, so it is read as bytes, then parsed like
/// every other JSON body: `400` for broken JSON, `422` for rules of the wrong shape.
async fn approve_climate(
    State(hub): HubState,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Result<StatusCode, HttpError> {
    let rules: Option<Rules> = if body.iter().all(u8::is_ascii_whitespace) {
        None
    } else {
        let Json(rules) = Json::<Rules>::from_bytes(&body)?;
        Some(rules)
    };
    hub.approve_climate_proposal(&id, rules).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Rejects the pending proposal of a zone (204); `422` when there is none.
async fn reject_climate(
    State(hub): HubState,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    hub.reject_climate_proposal(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Learns a zone now (`200` with where the learner stands).
async fn learn_climate(
    State(hub): HubState,
    Path(id): Path<String>,
) -> Result<Json<LearnStatus>, HttpError> {
    Ok(Json(hub.learn_climate_zone(&id).await?))
}

/// `GET /climate/zones/{id}/decisions?limit=` — the recent decisions from memory, then older
/// rows from history; `limit` is 50 by default and 500 at most.
async fn climate_decisions(
    State(hub): HubState,
    Path(id): Path<String>,
    RawQuery(q): RawQuery,
) -> Result<Json<DecisionLog>, HttpError> {
    let params = parse_query(q.as_deref());
    let limit = match get_one(&params, "limit") {
        Some(text) => text.parse::<usize>().map_err(|_| bad_request())?,
        None => 50,
    };
    Ok(Json(hub.climate_decision_log(&id, limit.min(500)).await?))
}

/// Creates or replaces a virtual switch, number or button (`200` with the entity); `422` for
/// a bad id or spec, `503` while the broker link is down.
async fn put_virtual(
    State(hub): HubState,
    Path(id): Path<String>,
    body: Body<VirtualSpec>,
) -> Result<Json<EntityView>, HttpError> {
    let Json(spec) = body?;
    Ok(Json(hub.set_virtual(&id, spec).await?))
}

/// Removes a virtual entity (204); `404` for an id that is not one.
async fn delete_virtual(
    State(hub): HubState,
    Path(id): Path<String>,
) -> Result<StatusCode, HttpError> {
    hub.delete_virtual(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /history/series?entity=&range=` — 400 for a missing or unknown range, 503 without
/// history, 404 for an entity that was never recorded.
async fn history_series(
    State(hub): HubState,
    RawQuery(q): RawQuery,
) -> Result<Json<HistorySeries>, HttpError> {
    let params = parse_query(q.as_deref());
    let entity = get_one(&params, "entity").ok_or_else(bad_request)?;
    let range = get_one(&params, "range")
        .and_then(HistoryRange::parse)
        .ok_or_else(bad_request)?;
    let history = require_history(&hub)?;
    Ok(Json(
        history_query::series(history, entity, range, now_micros()).await?,
    ))
}

/// `GET /history/daily?entity=&days=&kind=` — `days` must be 7, 30 or 365; `kind` is
/// counter or power.
async fn history_daily(
    State(hub): HubState,
    RawQuery(q): RawQuery,
) -> Result<Json<HistoryDaily>, HttpError> {
    let params = parse_query(q.as_deref());
    let entity = get_one(&params, "entity").ok_or_else(bad_request)?;
    let days: u32 = get_one(&params, "days")
        .and_then(|s| s.parse().ok())
        .ok_or_else(bad_request)?;
    if !matches!(days, 7 | 30 | 365) {
        return Err(bad_request());
    }
    let kind = get_one(&params, "kind")
        .and_then(EnergyKind::parse)
        .ok_or_else(bad_request)?;
    let history = require_history(&hub)?;
    Ok(Json(
        history_query::daily(history, entity, days, kind, now_micros()).await?,
    ))
}

/// `GET /history/today?entity=&kind=` — kWh since local midnight for the current day.
async fn history_today(
    State(hub): HubState,
    RawQuery(q): RawQuery,
) -> Result<Json<HistoryToday>, HttpError> {
    let params = parse_query(q.as_deref());
    let entity = get_one(&params, "entity").ok_or_else(bad_request)?;
    let kind = get_one(&params, "kind")
        .and_then(EnergyKind::parse)
        .ok_or_else(bad_request)?;
    let history = require_history(&hub)?;
    Ok(Json(
        history_query::today(history, entity, kind, now_micros()).await?,
    ))
}

/// `GET /history/grid_day?date=YYYY-MM-DD` — what crossed the grid meter that house-local day,
/// today when `date` is absent.
async fn history_grid_day(
    State(hub): HubState,
    RawQuery(q): RawQuery,
) -> Result<Json<GridDay>, HttpError> {
    let params = parse_query(q.as_deref());
    let date = match get_one(&params, "date") {
        None => None,
        Some(d) => {
            Some(chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|_| bad_request())?)
        }
    };
    let history = require_history(&hub)?;
    Ok(Json(
        history_query::grid_day(
            history,
            &hub.grid_counters(),
            &hub.off_peak_history(),
            hub.home_settings().off_peak_device.as_ref(),
            date,
            now_micros(),
        )
        .await?,
    ))
}

/// `GET /history/last_change?entity=...&entity=...` — 1 to 50 ids, in request order.
async fn history_last_change(
    State(hub): HubState,
    RawQuery(q): RawQuery,
) -> Result<Json<LastChange>, HttpError> {
    let params = parse_query(q.as_deref());
    let ids: Vec<String> = get_many(&params, "entity")
        .into_iter()
        .map(str::to_owned)
        .collect();
    if ids.is_empty() {
        return Err(bad_request());
    }
    let history = require_history(&hub)?;
    Ok(Json(history_query::last_change(history, &ids).await?))
}
