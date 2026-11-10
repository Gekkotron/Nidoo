//! Flow labels and choices: the catalog, its edits, a flow's labels and the label switch.

use gloo_net::http::Request;
use nidoo_api::{
    ApiError, FlowLabelsView, FlowView, LabelEdit, LabelSwitchBody, LabelSwitchResult,
};
use nidoo_core::flow::LabelAction;

use super::core::{check, encode_id, read_json};

/// The label catalog and how many flows carry each label.
pub async fn get_flow_labels() -> Result<FlowLabelsView, ApiError> {
    let response = check(Request::get("/api/v1/flow-labels").build(), None).await?;
    read_json(response).await
}

/// Applies one catalog edit; the reply is the catalog as the hub now has it.
pub async fn edit_flow_labels(edit: &LabelEdit) -> Result<FlowLabelsView, ApiError> {
    let response = check(Request::post("/api/v1/flow-labels").json(edit), None).await?;
    read_json(response).await
}

/// Switches label `label` (`pick`, `on` or `off`); the reply names the flows turned on and off
/// and the ones skipped.
pub async fn switch_flow_label(
    label: &str,
    action: LabelAction,
) -> Result<LabelSwitchResult, ApiError> {
    let url = format!("/api/v1/flow-labels/{}/switch", encode_id(label));
    let response = check(Request::post(&url).json(&LabelSwitchBody { action }), None).await?;
    read_json(response).await
}

/// Sets the labels flow `id` carries; the reply is the flow as the hub now has it.
pub async fn set_flow_labels(id: &str, labels: &[String]) -> Result<FlowView, ApiError> {
    let url = format!("/api/v1/flows/{}/labels", encode_id(id));
    let body = serde_json::json!({ "labels": labels });
    let response = check(Request::post(&url).json(&body), None).await?;
    read_json(response).await
}
