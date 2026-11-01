//! One forecast period and the parser for Solcast's `forecasts` response.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// One forecast period: mean power in kW over the `minutes` ending at `end`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Period {
    /// End of the period (UTC).
    pub end: DateTime<Utc>,
    /// Length of the period in minutes.
    pub minutes: u32,
    /// Central estimate, mean kW.
    pub estimate: f64,
    /// Pessimistic (10th percentile) estimate, mean kW.
    pub low: f64,
    /// Optimistic (90th percentile) estimate, mean kW.
    pub high: f64,
}

impl Period {
    /// Start of the period, `end` minus its length.
    pub fn start(&self) -> DateTime<Utc> {
        self.end - Duration::minutes(i64::from(self.minutes))
    }

    /// Energy of the central estimate over the period, in kWh.
    pub fn kwh(&self) -> f64 {
        self.estimate * f64::from(self.minutes) / 60.0
    }
}

/// Why a Solcast response could not be parsed.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// The body is not the expected JSON.
    #[error("invalid Solcast JSON: {0}")]
    Json(String),
    /// A `period_end` is not an RFC 3339 timestamp.
    #[error("invalid period_end `{0}`")]
    Time(String),
}

#[derive(Deserialize)]
struct Response {
    forecasts: Vec<Raw>,
}

#[derive(Deserialize)]
struct Raw {
    pv_estimate: f64,
    pv_estimate10: f64,
    pv_estimate90: f64,
    period_end: String,
    #[serde(default)]
    period: Option<String>,
}

/// Minutes in an ISO-8601 `PT<n>M` / `PT<n>H` duration; anything else is 30.
fn period_minutes(period: Option<&str>) -> u32 {
    let parse = |s: &str| -> Option<u32> {
        let rest = s.strip_prefix("PT")?;
        if let Some(n) = rest.strip_suffix('M') {
            n.parse().ok()
        } else if let Some(n) = rest.strip_suffix('H') {
            n.parse::<u32>().ok()?.checked_mul(60)
        } else {
            None
        }
    };
    period.and_then(parse).filter(|m| *m > 0).unwrap_or(30)
}

/// Parse a Solcast `forecasts` response body into periods, in the order given.
pub fn parse_forecasts(body: &str) -> Result<Vec<Period>, ParseError> {
    let response: Response =
        serde_json::from_str(body).map_err(|e| ParseError::Json(e.to_string()))?;
    response
        .forecasts
        .into_iter()
        .map(|raw| {
            let end = raw
                .period_end
                .parse::<DateTime<Utc>>()
                .map_err(|_| ParseError::Time(raw.period_end.clone()))?;
            Ok(Period {
                end,
                minutes: period_minutes(raw.period.as_deref()),
                estimate: raw.pv_estimate,
                low: raw.pv_estimate10,
                high: raw.pv_estimate90,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(period: &str) -> String {
        format!(
            r#"{{"forecasts":[{{"pv_estimate":1.0,"pv_estimate10":0.5,"pv_estimate90":1.5,"period_end":"2026-10-23T12:00:00.0000000Z"{period}}}]}}"#
        )
    }

    #[test]
    fn parses_documented_shape() {
        let p = parse_forecasts(include_str!("../tests/fixtures/forecasts.json")).unwrap();
        assert_eq!(p.len(), 48);
        assert_eq!(
            p[0].end,
            "2026-10-22T22:30:00Z".parse::<DateTime<Utc>>().unwrap()
        );
        assert_eq!(p[0].minutes, 30);
        let total: f64 = p.iter().map(Period::kwh).sum();
        assert!((total - 12.4).abs() < 1e-6);
    }

    #[test]
    fn period_duration_variants() {
        for (field, want) in [
            (r#","period":"PT30M""#, 30),
            (r#","period":"PT15M""#, 15),
            (r#","period":"PT1H""#, 60),
            ("", 30),
            (r#","period":"garbage""#, 30),
        ] {
            let p = parse_forecasts(&one(field)).unwrap();
            assert_eq!(p[0].minutes, want, "{field}");
        }
    }

    #[test]
    fn start_is_end_minus_duration() {
        let p = parse_forecasts(&one(r#","period":"PT30M""#)).unwrap();
        assert_eq!(
            p[0].start(),
            "2026-10-23T11:30:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }

    #[test]
    fn bad_json_and_bad_time_are_errors() {
        assert!(matches!(parse_forecasts("{"), Err(ParseError::Json(_))));
        let bad = one("").replace("2026-10-23T12:00:00.0000000Z", "yesterday");
        assert!(matches!(parse_forecasts(&bad), Err(ParseError::Time(t)) if t == "yesterday"));
    }

    #[test]
    fn empty_forecasts_is_ok_and_empty() {
        assert!(parse_forecasts(r#"{"forecasts":[]}"#).unwrap().is_empty());
    }
}
