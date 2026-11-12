//! The hub's REST API. Every failure comes back as an [`ApiError`] whose text is fit for a toast.

use gloo_net::http::{Request, Response};
use nidoo_api::{
    ApiError, CamerasView, CommandResponse, Dashboard, DashboardReorder, DashboardView, EntityView,
    FlowView, FlowWriteOutcome, FlowsConfigView, GroupSpec, GroupView, NetworkWatchSpec,
    NetworkWatchView, NotifyInput, OverridePatch, RunView, SchedulePreview, SystemView,
};
use nidoo_core::flow::Flow;
use nidoo_core::model::{AreasDocument, Command};
use serde::de::DeserializeOwned;
use serde_json::Value;

pub use crate::i18n::errors::WebError;
use crate::i18n::{current_lang, errors::web_error_text};

/// A web failure as an `ApiError` whose text is built now, in the language active now. The hub's
/// own messages never come through here: they stay as received.
impl From<WebError> for ApiError {
    fn from(e: WebError) -> Self {
        ApiError {
            error: web_error_text(current_lang(), &e),
        }
    }
}

fn network_error(e: impl std::fmt::Display) -> ApiError {
    WebError::Network(e.to_string()).into()
}

/// The error for a non-2xx reply: the hub's own `ApiError` body, or "The hub answered HTTP <status>".
fn decode_error(status: u16, body: &str) -> ApiError {
    serde_json::from_str::<ApiError>(body).unwrap_or_else(|_| WebError::Http(status).into())
}

async fn send(request: Result<Request, gloo_net::Error>) -> Result<Response, ApiError> {
    request
        .map_err(network_error)?
        .send()
        .await
        .map_err(network_error)
}

/// The error for a reply with a status of 400 and up.
async fn reject(response: Response) -> ApiError {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    decode_error(status, &body)
}

/// Sends `request` and turns a transport failure or a status of 400 and up into an `ApiError`.
/// A 503 becomes `on_503` when there is one.
pub(crate) async fn check(
    request: Result<Request, gloo_net::Error>,
    on_503: Option<WebError>,
) -> Result<Response, ApiError> {
    let response = send(request).await?;
    let status = response.status();
    if status == 503
        && let Some(e) = on_503
    {
        return Err(e.into());
    }
    if status >= 400 {
        return Err(reject(response).await);
    }
    Ok(response)
}

/// Like [`check`], but a 503 is `Ok(None)`: the hub has nothing to show yet.
pub(crate) async fn check_unless_503(
    request: Result<Request, gloo_net::Error>,
) -> Result<Option<Response>, ApiError> {
    let response = send(request).await?;
    match response.status() {
        503 => Ok(None),
        status if status >= 400 => Err(reject(response).await),
        _ => Ok(Some(response)),
    }
}

/// Like [`check`], but a 502 is `Ok(None)`: the hub could not reach what it fronts.
pub(crate) async fn check_unless_502(
    request: Result<Request, gloo_net::Error>,
) -> Result<Option<Response>, ApiError> {
    let response = send(request).await?;
    match response.status() {
        502 => Ok(None),
        status if status >= 400 => Err(reject(response).await),
        _ => Ok(Some(response)),
    }
}

/// Like [`check`], but a 404 is `Ok(None)`: the hub has nothing of that kind.
pub(crate) async fn check_unless_404(
    request: Result<Request, gloo_net::Error>,
) -> Result<Option<Response>, ApiError> {
    let response = send(request).await?;
    match response.status() {
        404 => Ok(None),
        status if status >= 400 => Err(reject(response).await),
        _ => Ok(Some(response)),
    }
}

pub(crate) async fn read_json<T: DeserializeOwned>(response: Response) -> Result<T, ApiError> {
    response
        .json::<T>()
        .await
        .map_err(|e| WebError::Decode(e.to_string()).into())
}

/// Like [`read_json`], but a `204 No Content` reply is `Ok(None)`: the hub has nothing to show.
pub(crate) async fn read_json_or_none<T: DeserializeOwned>(
    response: Response,
) -> Result<Option<T>, ApiError> {
    if response.status() == 204 {
        return Ok(None);
    }
    read_json(response).await.map(Some)
}

/// `id` as one URL path segment: everything but the unreserved characters (`A-Za-z0-9-._~`)
/// becomes `%XX` over its UTF-8 bytes, so `/`, `#`, `?`, `%` and spaces cannot change the route.
///
/// Hand-rolled rather than `js_sys::encode_uri_component` (which also leaves `!'()*` alone and
/// panics off wasm, so it cannot be unit-tested natively) or the `percent-encoding` crate (an
/// extra dependency for ten lines).
pub fn encode_id(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for b in id.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub async fn send_command(id: &str, cmd: Command) -> Result<CommandResponse, ApiError> {
    let url = format!("/api/v1/entities/{}/command", encode_id(id));
    let response = check(
        Request::post(&url).json(&cmd),
        Some(WebError::BrokerOffline),
    )
    .await?;
    read_json(response).await
}

/// Presses alert `id`'s button `action`; the hub forwards it to the producer.
pub async fn send_alert_action(id: &str, action: &str) -> Result<(), ApiError> {
    let url = format!(
        "/api/v1/alerts/{}/actions/{}",
        encode_id(id),
        encode_id(action)
    );
    check(Request::post(&url).build(), Some(WebError::BrokerOffline)).await?;
    Ok(())
}

pub async fn set_override(id: &str, patch: OverridePatch) -> Result<EntityView, ApiError> {
    let url = format!("/api/v1/entities/{}/override", encode_id(id));
    let response = check(Request::patch(&url).json(&Value::Object(patch)), None).await?;
    read_json(response).await
}

pub async fn clear_override(id: &str) -> Result<(), ApiError> {
    let url = format!("/api/v1/entities/{}/override", encode_id(id));
    check(Request::delete(&url).build(), None).await?;
    Ok(())
}

/// Enables or disables flow `id`; the reply is the flow as the hub now has it.
pub async fn set_flow_enabled(id: &str, enabled: bool) -> Result<FlowView, ApiError> {
    let url = format!("/api/v1/flows/{}/enabled", encode_id(id));
    let body = serde_json::json!({ "enabled": enabled });
    let response = check(Request::post(&url).json(&body), None).await?;
    read_json(response).await
}

/// Moves flow `id` to room `area`, or out of any room with `None`; the reply is the flow as
/// the hub now has it.
pub async fn set_flow_area(id: &str, area: Option<&str>) -> Result<FlowView, ApiError> {
    let url = format!("/api/v1/flows/{}/area", encode_id(id));
    let body = serde_json::json!({ "area": area });
    let response = check(Request::post(&url).json(&body), None).await?;
    read_json(response).await
}

/// Replaces flow `id`'s definition on the hub. The id in `flow` must match the path. The reply
/// is the flow as stored and the hub's warning, when it has one (the Paris sun fallback).
pub async fn put_flow(flow: &Flow) -> Result<(FlowView, Option<String>), ApiError> {
    let url = format!("/api/v1/flows/{}", encode_id(&flow.id));
    let response = check(Request::put(&url).json(flow), None).await?;
    let outcome: FlowWriteOutcome = read_json(response).await?;
    Ok((outcome.flow, outcome.warning))
}

/// Removes flow `id` from the hub; the hub's flows push then drops it from the list.
pub async fn delete_flow(id: &str) -> Result<(), ApiError> {
    let url = format!("/api/v1/flows/{}", encode_id(id));
    check(Request::delete(&url).build(), None).await?;
    Ok(())
}

/// The `[flows]` settings: the house point, the time zone and whether the Paris fallback is on.
pub async fn get_flows_config() -> Result<FlowsConfigView, ApiError> {
    let response = check(Request::get("/api/v1/config/flows").build(), None).await?;
    read_json(response).await
}

/// Every hand-made group the hub holds.
pub async fn list_groups() -> Result<Vec<GroupView>, ApiError> {
    let response = check(Request::get("/api/v1/groups").build(), None).await?;
    read_json(response).await
}

/// Creates or replaces group `id` from `spec`; the reply is the group as the hub now has it.
pub async fn set_group(id: &str, spec: &GroupSpec) -> Result<GroupView, ApiError> {
    let url = format!("/api/v1/groups/{}", encode_id(id));
    let response = check(Request::put(&url).json(spec), None).await?;
    read_json(response).await
}

/// Deletes group `id` on the hub.
pub async fn delete_group(id: &str) -> Result<(), ApiError> {
    let url = format!("/api/v1/groups/{}", encode_id(id));
    check(Request::delete(&url).build(), None).await?;
    Ok(())
}

/// Every network watch the hub holds.
pub async fn list_network_watches() -> Result<Vec<NetworkWatchView>, ApiError> {
    let response = check(Request::get("/api/v1/network").build(), None).await?;
    read_json(response).await
}

/// Creates or replaces watch `id` from `spec`; the reply is the watch as the hub now has it.
pub async fn set_network_watch(
    id: &str,
    spec: &NetworkWatchSpec,
) -> Result<NetworkWatchView, ApiError> {
    let url = format!("/api/v1/network/{}", encode_id(id));
    let response = check(Request::put(&url).json(spec), None).await?;
    read_json(response).await
}

/// The system health view: uptime, machine figures, counters and service states.
pub async fn get_system() -> Result<SystemView, ApiError> {
    let response = check(Request::get("/api/v1/system").build(), None).await?;
    read_json(response).await
}

/// Switches outage pushes; the reply is the value the hub saved.
pub async fn set_system_notify(notify: bool) -> Result<NotifyInput, ApiError> {
    let response = check(
        Request::put("/api/v1/system/notify").json(&NotifyInput { notify }),
        None,
    )
    .await?;
    read_json(response).await
}

/// Deletes watch `id` on the hub.
pub async fn delete_network_watch(id: &str) -> Result<(), ApiError> {
    let url = format!("/api/v1/network/{}", encode_id(id));
    check(Request::delete(&url).build(), None).await?;
    Ok(())
}

/// The next three ticks of `cron` in the house time zone; a `422` (the hub's message names what
/// is wrong with the expression) comes back as an `ApiError`.
pub async fn schedule_preview(cron: &str) -> Result<SchedulePreview, ApiError> {
    let url = format!("/api/v1/schedule?cron={}&count=3", encode_id(cron));
    let response = check(Request::get(&url).build(), None).await?;
    read_json(response).await
}

/// Starts flow `id` by hand; the hub answers 202 with the new run.
pub async fn run_flow(id: &str) -> Result<RunView, ApiError> {
    let url = format!("/api/v1/flows/{}/run", encode_id(id));
    let response = check(Request::post(&url).build(), None).await?;
    read_json(response).await
}

/// Starts flow `id` from its manual card `node`; the hub answers 202 with the new run, or 422
/// when the flow cannot start (disabled, or a run is active in `single` mode).
pub async fn run_flow_from(id: &str, node: &str) -> Result<RunView, ApiError> {
    let url = format!("/api/v1/flows/{}/run", encode_id(id));
    let body = serde_json::json!({ "node": node });
    let response = check(Request::post(&url).json(&body), None).await?;
    read_json(response).await
}

/// Renames area `id`; the hub answers with the area list it now has.
pub async fn rename_area(id: &str, name: &str) -> Result<Vec<nidoo_api::AreaView>, ApiError> {
    let url = format!("/api/v1/areas/{}", encode_id(id));
    let body = serde_json::json!({ "name": name });
    let response = check(Request::patch(&url).json(&body), None).await?;
    read_json(response).await
}

/// The body of a room-group edit: absent fields are left out, `Some(None)` clears the name.
// Reserved: a full area editor is not built yet, so nothing calls this.
pub async fn set_areas(doc: AreasDocument) -> Result<Vec<nidoo_api::AreaView>, ApiError> {
    let response = check(Request::put("/api/v1/areas").json(&doc), None).await?;
    read_json(response).await
}

/// Frigate's cameras and the hub's `[frigate] live` switch.
pub async fn cameras() -> Result<CamerasView, ApiError> {
    let response = check(Request::get("/api/v1/cameras").build(), None).await?;
    read_json(response).await
}

/// House first at index 0, then the authored dashboards by `order`.
pub async fn fetch_dashboards() -> Result<Vec<DashboardView>, ApiError> {
    let response = check(Request::get("/api/v1/dashboards").build(), None).await?;
    read_json(response).await
}

/// One dashboard by id; House is at `house`.
pub async fn fetch_dashboard(id: &str) -> Result<DashboardView, ApiError> {
    let url = format!("/api/v1/dashboards/{}", encode_id(id));
    let response = check(Request::get(&url).build(), None).await?;
    read_json(response).await
}

/// Create-or-replace; the reply carries the hub's normalized state (notably `order`).
pub async fn put_dashboard(d: &Dashboard) -> Result<Dashboard, ApiError> {
    let url = format!("/api/v1/dashboards/{}", encode_id(&d.id));
    let response = check(Request::put(&url).json(d), None).await?;
    // Plain serde would reject widget kinds this build does not know; decode_dashboard drops them.
    let value: Value = read_json(response).await?;
    nidoo_api::decode_dashboard(value).map_err(|e| WebError::Decode(e.to_string()).into())
}

/// House cannot be deleted (the hub answers 403), so the client hides its delete UI.
pub async fn delete_dashboard(id: &str) -> Result<(), ApiError> {
    let url = format!("/api/v1/dashboards/{}", encode_id(id));
    check(Request::delete(&url).build(), None).await?;
    Ok(())
}

/// Reorders the authored dashboards; House stays at index 0. A drifted list comes back as a 422.
pub async fn reorder_dashboards(order: &[String]) -> Result<(), ApiError> {
    let body = DashboardReorder {
        order: order.to_vec(),
    };
    check(Request::post("/api/v1/dashboards/order").json(&body), None).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{decode_error, encode_id};

    #[test]
    fn ordinary_ids_pass_through() {
        assert_eq!(
            encode_id("light.kitchen_lamp-1~x"),
            "light.kitchen_lamp-1~x"
        );
    }

    #[test]
    fn reserved_characters_are_escaped() {
        assert_eq!(encode_id("a/b#c?d e%f"), "a%2Fb%23c%3Fd%20e%25f");
        assert_eq!(encode_id("é"), "%C3%A9");
    }

    #[test]
    fn server_message_is_surfaced() {
        let e = decode_error(422, r#"{"error":"light does not support color"}"#);
        assert_eq!(e.error, "light does not support color");
    }

    #[test]
    fn unparsable_body_falls_back_to_status() {
        assert_eq!(
            decode_error(502, "<html>bad gateway</html>").error,
            "The hub answered HTTP 502"
        );
        assert_eq!(decode_error(500, "").error, "The hub answered HTTP 500");
    }
}
