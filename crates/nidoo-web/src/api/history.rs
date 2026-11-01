//! Typed wrappers over `/api/v1/history/*`. Every failure comes back as a [`HistoryApiError`]
//! whose variant pins the hub's status for the UI to render.

use super::encode_id;
use crate::i18n::current_lang;
use crate::i18n::errors::{WebError, web_error_text};
use gloo_net::http::Request;
use nidoo_api::{
    EnergyKind, GridDay, HistoryDaily, HistoryRange, HistorySeries, HistoryToday, LastChange,
};

/// Why a history call failed: variant per status class the hub returns.
#[derive(Debug, Clone, thiserror::Error)]
pub enum HistoryApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("not recorded")]
    NotRecorded,
    #[error("history unavailable")]
    Unavailable,
    #[error("no grid meter")]
    NoGridMeter,
    #[error("{0}")]
    Other(String),
}

/// Serde wire name of a range (`1h`, `6h`, `24h`, `7d`, `30d`, `1y`).
fn range_wire(range: HistoryRange) -> &'static str {
    match range {
        HistoryRange::Hour => "1h",
        HistoryRange::SixHours => "6h",
        HistoryRange::Day => "24h",
        HistoryRange::Week => "7d",
        HistoryRange::Month => "30d",
        HistoryRange::Year => "1y",
    }
}

/// Serde wire name of an energy kind (`power` or `counter`).
fn kind_wire(kind: EnergyKind) -> &'static str {
    match kind {
        EnergyKind::Power => "power",
        EnergyKind::Counter => "counter",
    }
}

pub(crate) fn series_url(entity: &str, range: HistoryRange) -> String {
    format!(
        "/api/v1/history/series?entity={}&range={}",
        encode_id(entity),
        range_wire(range)
    )
}

pub(crate) fn daily_url(entity: &str, days: u16, kind: EnergyKind) -> String {
    format!(
        "/api/v1/history/daily?entity={}&days={}&kind={}",
        encode_id(entity),
        days,
        kind_wire(kind)
    )
}

pub(crate) fn today_url(entity: &str, kind: EnergyKind) -> String {
    format!(
        "/api/v1/history/today?entity={}&kind={}",
        encode_id(entity),
        kind_wire(kind)
    )
}

pub(crate) fn grid_day_url(date: Option<&str>) -> String {
    match date {
        Some(d) => format!("/api/v1/history/grid_day?date={}", encode_id(d)),
        None => "/api/v1/history/grid_day".to_string(),
    }
}

pub(crate) fn last_change_url(ids: &[&str]) -> String {
    let mut out = String::from("/api/v1/history/last_change");
    for (i, id) in ids.iter().enumerate() {
        out.push(if i == 0 { '?' } else { '&' });
        out.push_str("entity=");
        out.push_str(&encode_id(id));
    }
    out
}

pub(crate) fn chunk_ids(ids: &[String], max: usize) -> Vec<Vec<&str>> {
    ids.chunks(max)
        .map(|c| c.iter().map(String::as_str).collect())
        .collect()
}

pub(crate) fn classify_status(status: u16, body: &str) -> HistoryApiError {
    match status {
        400 => {
            HistoryApiError::BadRequest(error_message(body).unwrap_or_else(|| "bad_request".into()))
        }
        404 if error_message(body).as_deref() == Some("no_grid_meter") => {
            HistoryApiError::NoGridMeter
        }
        404 => HistoryApiError::NotRecorded,
        503 => HistoryApiError::Unavailable,
        _ => HistoryApiError::Other(
            error_message(body)
                .unwrap_or_else(|| web_error_text(current_lang(), &WebError::Http(status))),
        ),
    }
}

/// Pull the hub's own `error` field out of a JSON error body if present.
fn error_message(body: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct E {
        error: String,
    }
    serde_json::from_str::<E>(body).ok().map(|e| e.error)
}

async fn fetch<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, HistoryApiError> {
    let response = Request::get(url).send().await.map_err(|e| {
        HistoryApiError::Other(web_error_text(
            current_lang(),
            &WebError::Network(e.to_string()),
        ))
    })?;
    let status = response.status();
    if status >= 400 {
        let body = response.text().await.unwrap_or_default();
        return Err(classify_status(status, &body));
    }
    response.json::<T>().await.map_err(|e| {
        HistoryApiError::Other(web_error_text(
            current_lang(),
            &WebError::Decode(e.to_string()),
        ))
    })
}

pub async fn fetch_series(
    entity: &str,
    range: HistoryRange,
) -> Result<HistorySeries, HistoryApiError> {
    fetch(&series_url(entity, range)).await
}

pub async fn fetch_daily(
    entity: &str,
    days: u16,
    kind: EnergyKind,
) -> Result<HistoryDaily, HistoryApiError> {
    fetch(&daily_url(entity, days, kind)).await
}

pub async fn fetch_today(entity: &str, kind: EnergyKind) -> Result<HistoryToday, HistoryApiError> {
    fetch(&today_url(entity, kind)).await
}

/// `/grid_day` for `date` (`YYYY-MM-DD`), today when `None`.
pub async fn fetch_grid_day(date: Option<&str>) -> Result<GridDay, HistoryApiError> {
    fetch(&grid_day_url(date)).await
}

/// `/last_change` for `ids`; chunks into 50-id requests and concatenates the `items` in input
/// order. Returns an empty `LastChange { items: [] }` for an empty input (no HTTP call).
pub async fn fetch_last_change(ids: &[String]) -> Result<LastChange, HistoryApiError> {
    if ids.is_empty() {
        return Ok(LastChange { items: Vec::new() });
    }
    let chunks = chunk_ids(ids, 50);
    let mut items = Vec::with_capacity(ids.len());
    for chunk in chunks {
        let url = last_change_url(&chunk);
        let batch: LastChange = fetch(&url).await?;
        items.extend(batch.items);
    }
    Ok(LastChange { items })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_day_url_with_and_without_date() {
        assert_eq!(grid_day_url(None), "/api/v1/history/grid_day");
        assert_eq!(
            grid_day_url(Some("2026-10-23")),
            "/api/v1/history/grid_day?date=2026-10-23"
        );
    }

    #[test]
    fn no_grid_meter_is_its_own_error() {
        assert!(matches!(
            classify_status(404, r#"{"error":"no_grid_meter"}"#),
            HistoryApiError::NoGridMeter
        ));
        assert!(matches!(
            classify_status(404, r#"{"error":"not_recorded"}"#),
            HistoryApiError::NotRecorded
        ));
    }

    #[test]
    fn series_url_uses_wire_range_names_and_encodes_id() {
        assert_eq!(
            series_url("sensor.living/room temp", HistoryRange::Day),
            "/api/v1/history/series?entity=sensor.living%2Froom%20temp&range=24h"
        );
        assert!(series_url("x", HistoryRange::Hour).ends_with("&range=1h"));
        assert!(series_url("x", HistoryRange::SixHours).ends_with("&range=6h"));
        assert!(series_url("x", HistoryRange::Year).ends_with("&range=1y"));
        assert!(series_url("x", HistoryRange::Week).ends_with("&range=7d"));
        assert!(series_url("x", HistoryRange::Month).ends_with("&range=30d"));
    }

    #[test]
    fn daily_url_uses_days_and_kind() {
        assert_eq!(
            daily_url("counter.a", 30, EnergyKind::Counter),
            "/api/v1/history/daily?entity=counter.a&days=30&kind=counter"
        );
        assert!(daily_url("x", 7, EnergyKind::Power).ends_with("&days=7&kind=power"));
    }

    #[test]
    fn today_url_uses_kind() {
        assert_eq!(
            today_url("p", EnergyKind::Power),
            "/api/v1/history/today?entity=p&kind=power"
        );
    }

    #[test]
    fn last_change_url_repeats_entity_param_in_order() {
        let ids = ["a", "b/c", "d"];
        assert_eq!(
            last_change_url(&ids),
            "/api/v1/history/last_change?entity=a&entity=b%2Fc&entity=d"
        );
    }

    #[test]
    fn classify_maps_statuses() {
        assert!(matches!(
            classify_status(400, r#"{"error":"not a 24h"}"#),
            HistoryApiError::BadRequest(ref s) if s == "not a 24h"
        ));
        assert!(matches!(
            classify_status(400, ""),
            HistoryApiError::BadRequest(_)
        ));
        assert!(matches!(
            classify_status(404, ""),
            HistoryApiError::NotRecorded
        ));
        assert!(matches!(
            classify_status(503, ""),
            HistoryApiError::Unavailable
        ));
        assert!(matches!(
            classify_status(500, "oops"),
            HistoryApiError::Other(_)
        ));
    }

    #[test]
    fn chunk_ids_batches_at_50() {
        let ids: Vec<String> = (0..103).map(|i| format!("e{i}")).collect();
        let chunks = chunk_ids(&ids, 50);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 50);
        assert_eq!(chunks[1].len(), 50);
        assert_eq!(chunks[2].len(), 3);
        assert_eq!(chunks[2][0], "e100");
    }

    #[test]
    fn chunk_ids_empty_input() {
        let ids: Vec<String> = Vec::new();
        assert!(chunk_ids(&ids, 50).is_empty());
    }
}
