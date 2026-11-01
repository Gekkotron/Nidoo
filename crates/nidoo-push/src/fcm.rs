//! FCM HTTP v1 client with service-account OAuth, ported from OikosPush `fcm_sender.py`.
//!
//! Secrets (private key, access token, full registration tokens) never reach logs or `Debug`.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::time::{Duration, Instant};

use futures_util::future::{BoxFuture, join_all};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use url::Url;

use crate::normalize::Priority;

const SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";

#[derive(Debug, thiserror::Error)]
pub enum FcmError {
    #[error("cannot read service account {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid service account {path}: {source}")]
    Parse {
        path: String,
        source: serde_json::Error,
    },
}

/// The fields of a Google service-account key file that FCM needs.
#[derive(Clone, Deserialize)]
pub struct ServiceAccount {
    pub project_id: String,
    pub client_email: String,
    pub private_key: String,
    pub token_uri: String,
}

impl fmt::Debug for ServiceAccount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceAccount")
            .field("project_id", &self.project_id)
            .field("client_email", &self.client_email)
            .field("private_key", &"<hidden>")
            .field("token_uri", &self.token_uri)
            .finish()
    }
}

impl ServiceAccount {
    /// Parses the key JSON itself (not a path).
    pub fn from_json(raw: &str) -> Result<ServiceAccount, serde_json::Error> {
        serde_json::from_str(raw)
    }

    pub fn load(path: &Path) -> Result<ServiceAccount, FcmError> {
        let shown = path.display().to_string();
        let raw = std::fs::read_to_string(path).map_err(|source| FcmError::Read {
            path: shown.clone(),
            source,
        })?;
        serde_json::from_str(&raw).map_err(|source| FcmError::Parse {
            path: shown,
            source,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Sent,
    /// The registration token is dead and should be dropped.
    Invalid,
    /// The message itself is rejected (too big, malformed); retrying will not help.
    Message(String),
    /// Worth retrying later.
    Transient(String),
}

pub fn classify(status: u16, body: &str) -> Outcome {
    if (200..300).contains(&status) {
        return Outcome::Sent;
    }
    let json: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let error = &json["error"];
    let details = error["details"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let codes: Vec<&str> = details
        .iter()
        .filter_map(|d| d["errorCode"].as_str())
        .collect();
    if codes.contains(&"UNREGISTERED") {
        return Outcome::Invalid;
    }
    if error["status"].as_str() == Some("INVALID_ARGUMENT") || codes.contains(&"INVALID_ARGUMENT") {
        let message = error["message"].as_str().unwrap_or_default();
        return if message.to_lowercase().contains("registration token") {
            Outcome::Invalid
        } else {
            Outcome::Message(message.to_string())
        };
    }
    Outcome::Transient(format!("FCM answered HTTP {status}"))
}

pub trait FcmTransport: Send + Sync {
    fn send<'a>(
        &'a self,
        token: &'a str,
        data: &'a BTreeMap<String, String>,
        priority: Priority,
        ttl: u32,
    ) -> BoxFuture<'a, Outcome>;
}

#[derive(Serialize)]
struct Claims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    iat: u64,
    exp: u64,
}

struct CachedToken {
    value: String,
    valid_until: Instant,
}

pub struct HttpFcm {
    account: ServiceAccount,
    base: Url,
    http: reqwest::Client,
    token: Mutex<Option<CachedToken>>,
}

impl HttpFcm {
    pub fn new(account: ServiceAccount, fcm_base: Url) -> HttpFcm {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client builds");
        HttpFcm {
            account,
            base: fcm_base,
            http,
            token: Mutex::new(None),
        }
    }

    async fn access_token(&self) -> Result<String, String> {
        let mut guard = self.token.lock().await;
        if let Some(t) = guard.as_ref()
            && Instant::now() < t.valid_until
        {
            return Ok(t.value.clone());
        }
        let (value, expires_in) = self.fetch_token().await?;
        let valid_until = Instant::now() + Duration::from_secs(expires_in.saturating_sub(60));
        *guard = Some(CachedToken {
            value: value.clone(),
            valid_until,
        });
        Ok(value)
    }

    async fn fetch_token(&self) -> Result<(String, u64), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs();
        let claims = Claims {
            iss: &self.account.client_email,
            scope: SCOPE,
            aud: &self.account.token_uri,
            iat: now,
            exp: now + 3600,
        };
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(self.account.private_key.as_bytes())
            .map_err(|e| format!("service account key unusable: {e}"))?;
        let jwt = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
            &claims,
            &key,
        )
        .map_err(|e| format!("cannot sign OAuth assertion: {e}"))?;
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", GRANT_TYPE)
            .append_pair("assertion", &jwt)
            .finish();
        let resp = self
            .http
            .post(&self.account.token_uri)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form)
            .send()
            .await
            .map_err(|e| format!("OAuth request failed: {}", e.without_url()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("OAuth endpoint answered HTTP {status}"));
        }
        #[derive(Deserialize)]
        struct TokenReply {
            access_token: String,
            expires_in: u64,
        }
        let reply: TokenReply = resp
            .json()
            .await
            .map_err(|e| format!("OAuth reply unreadable: {}", e.without_url()))?;
        Ok((reply.access_token, reply.expires_in))
    }

    async fn drop_token(&self) {
        *self.token.lock().await = None;
    }

    async fn send_one(
        &self,
        token: &str,
        data: &BTreeMap<String, String>,
        priority: Priority,
        ttl: u32,
    ) -> Outcome {
        let access = match self.access_token().await {
            Ok(a) => a,
            Err(e) => return Outcome::Transient(e),
        };
        let url = match self.base.join(&format!(
            "v1/projects/{}/messages:send",
            self.account.project_id
        )) {
            Ok(u) => u,
            Err(e) => return Outcome::Transient(format!("bad FCM url: {e}")),
        };
        let body = serde_json::json!({
            "message": {
                "token": token,
                "data": data,
                "android": {
                    "priority": match priority {
                        Priority::High => "high",
                        Priority::Normal => "normal",
                    },
                    "ttl": format!("{ttl}s"),
                },
            }
        });
        let resp = match self
            .http
            .post(url)
            .bearer_auth(access)
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return Outcome::Transient(format!("FCM request failed: {}", e.without_url()));
            }
        };
        let status = resp.status().as_u16();
        if status == 401 {
            self.drop_token().await;
            return Outcome::Transient("FCM rejected the access token (HTTP 401)".into());
        }
        let text = resp.text().await.unwrap_or_default();
        let outcome = classify(status, &text);
        if outcome != Outcome::Sent {
            let shown: String = token.chars().take(8).collect();
            tracing::debug!(token = %format!("{shown}…"), status, ?outcome, "FCM send not delivered");
        }
        outcome
    }
}

impl FcmTransport for HttpFcm {
    fn send<'a>(
        &'a self,
        token: &'a str,
        data: &'a BTreeMap<String, String>,
        priority: Priority,
        ttl: u32,
    ) -> BoxFuture<'a, Outcome> {
        Box::pin(self.send_one(token, data, priority, ttl))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SendResult {
    pub sent: usize,
    pub invalid: Vec<String>,
    pub transient: Vec<String>,
    pub message_errors: usize,
    pub last_message_error: String,
}

/// Sends one data message to every token concurrently and sorts the outcomes.
pub async fn send_data(
    t: &dyn FcmTransport,
    data: &BTreeMap<String, String>,
    tokens: &[String],
    priority: Priority,
    ttl: u32,
) -> SendResult {
    let outcomes = join_all(tokens.iter().map(|tok| t.send(tok, data, priority, ttl))).await;
    let mut r = SendResult::default();
    for (tok, outcome) in tokens.iter().zip(outcomes) {
        match outcome {
            Outcome::Sent => r.sent += 1,
            Outcome::Invalid => r.invalid.push(tok.clone()),
            Outcome::Message(m) => {
                r.message_errors += 1;
                r.last_message_error = m;
            }
            Outcome::Transient(_) => r.transient.push(tok.clone()),
        }
    }
    r
}
