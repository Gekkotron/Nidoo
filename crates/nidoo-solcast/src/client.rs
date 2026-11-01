//! A small Solcast HTTP client: one call fetches a rooftop site's forecast.

use std::fmt;
use std::time::Duration;

use crate::period::{ParseError, Period, parse_forecasts};

/// Where the Solcast API lives.
pub const BASE_URL: &str = "https://api.solcast.com.au";

/// A Solcast API key. It never prints: `Debug` hides it and there is no `Display`.
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    /// Wraps a key, trimming the whitespace a key file usually ends with.
    pub fn new(key: String) -> ApiKey {
        ApiKey(key.trim().to_owned())
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(***)")
    }
}

/// Why a Solcast call failed. No variant carries the API key.
#[derive(Debug, thiserror::Error)]
pub enum SolcastError {
    /// HTTP 401 or 403.
    #[error("Solcast rejected the API key")]
    Unauthorized,
    /// HTTP 429.
    #[error("Solcast daily request limit reached")]
    QuotaExceeded,
    /// Any other non-success status.
    #[error("Solcast answered HTTP {0}")]
    Status(u16),
    /// The request failed or the body could not be read.
    #[error("network error: {0}")]
    Network(String),
    /// The body is not a forecast.
    #[error(transparent)]
    Parse(#[from] ParseError),
    /// The HTTP client could not be built.
    #[error("cannot build the HTTP client: {0}")]
    Build(String),
}

/// A Solcast client: 5 s to connect, 20 s in all.
pub struct Client {
    http: reqwest::Client,
    base: String,
    key: ApiKey,
}

impl Client {
    /// Builds a client for `base` (usually [`BASE_URL`]) authenticating with `key`.
    pub fn new(base: impl Into<String>, key: ApiKey) -> Result<Client, SolcastError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("nidoo/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| SolcastError::Build(e.without_url().to_string()))?;
        Ok(Client {
            http,
            base: base.into().trim_end_matches('/').to_owned(),
            key,
        })
    }

    /// Fetches the next 192 hours of forecast for one rooftop site.
    pub async fn site_forecast(&self, resource_id: &str) -> Result<Vec<Period>, SolcastError> {
        let url = format!(
            "{}/rooftop_sites/{}/forecasts?format=json&hours=192",
            self.base, resource_id
        );
        let response = self
            .http
            .get(url)
            .bearer_auth(&self.key.0)
            .send()
            .await
            .map_err(network)?;
        let status = response.status();
        match status.as_u16() {
            401 | 403 => return Err(SolcastError::Unauthorized),
            429 => return Err(SolcastError::QuotaExceeded),
            _ if !status.is_success() => return Err(SolcastError::Status(status.as_u16())),
            _ => {}
        }
        let body = response.text().await.map_err(network)?;
        Ok(parse_forecasts(&body)?)
    }
}

fn network(e: reqwest::Error) -> SolcastError {
    SolcastError::Network(e.without_url().to_string())
}
