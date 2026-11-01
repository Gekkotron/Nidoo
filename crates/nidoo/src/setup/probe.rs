//! Connection tests for the broker and the database. Nothing here returns or logs a password.

use std::time::Duration;

use nidoo_api::TestResult;
use rumqttc::{AsyncClient, ConnectReturnCode, ConnectionError, Event, MqttOptions, Packet};
use serde_json::json;
use sqlx::Row;

use crate::config::MqttConfig;
use crate::history::db::Db;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

fn result(ok: bool, message: &str) -> TestResult {
    TestResult {
        ok,
        message: message.to_owned(),
        detail: None,
    }
}

fn refusal(code: ConnectReturnCode) -> TestResult {
    match code {
        ConnectReturnCode::BadUserNamePassword | ConnectReturnCode::NotAuthorized => {
            result(false, "bad credentials")
        }
        _ => result(false, "refused"),
    }
}

/// Connects with a `<client_id>-probe` client id and waits for the broker's answer.
pub async fn mqtt(cfg: &MqttConfig) -> TestResult {
    let mut options = MqttOptions::new(
        format!("{}-probe", cfg.client_id),
        cfg.host.clone(),
        cfg.port,
    );
    if let Some(user) = &cfg.user {
        options.set_credentials(user, cfg.password.as_deref().unwrap_or(""));
    }
    options.set_clean_session(true);
    let (client, mut eventloop) = AsyncClient::new(options, 4);
    let outcome = tokio::time::timeout(PROBE_TIMEOUT, async {
        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Packet::ConnAck(ack))) => {
                    return if ack.code == ConnectReturnCode::Success {
                        result(true, "connected")
                    } else {
                        refusal(ack.code)
                    };
                }
                Ok(_) => {}
                Err(ConnectionError::ConnectionRefused(code)) => return refusal(code),
                Err(_) => return result(false, "refused"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| result(false, "timed out"));
    let _ = client.try_disconnect();
    outcome
}

fn db_failure(error: &sqlx::Error) -> TestResult {
    match error {
        sqlx::Error::PoolTimedOut => result(false, "timed out"),
        sqlx::Error::Database(e) if matches!(e.code().as_deref(), Some("28P01" | "28000")) => {
            result(false, "bad credentials")
        }
        sqlx::Error::Database(e) => result(false, e.message()),
        _ => result(false, "refused"),
    }
}

/// Connects, checks TimescaleDB is there, runs the migrations and reports how far back the
/// history goes (`detail.history_since`, RFC 3339 or null). `url` may hold a password: nothing
/// from it is echoed.
pub async fn database(url: &str) -> TestResult {
    let Ok(db) = Db::connect_lazy(url) else {
        return result(false, "the address is not valid");
    };
    let outcome = check_database(&db).await;
    db.close().await;
    outcome
}

async fn check_database(db: &Db) -> TestResult {
    let ext = sqlx::query("SELECT 1 FROM pg_available_extensions WHERE name = 'timescaledb'")
        .fetch_optional(db.pool())
        .await;
    match ext {
        Err(e) => return db_failure(&e),
        Ok(None) => return result(false, "TimescaleDB extension is not available"),
        Ok(Some(_)) => {}
    }
    if let Err(e) = db.migrate().await {
        return result(false, &format!("migration failed: {e}"));
    }
    let since = sqlx::query("SELECT MIN(time) FROM entity_state")
        .fetch_one(db.pool())
        .await
        .and_then(|row| row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(0));
    match since {
        Ok(t) => TestResult {
            ok: true,
            message: "connected".into(),
            detail: Some(json!({ "history_since": t.map(|t| t.to_rfc3339()) })),
        },
        Err(e) => db_failure(&e),
    }
}

/// One sites request to Solcast with `key`. `detail.sites` lists `{id, name}` per rooftop site.
/// Nothing from the request (the key, the URL) is echoed.
pub async fn solcast(base: &str, key: &str) -> TestResult {
    let Ok(client) = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("nidoo/", env!("CARGO_PKG_VERSION")))
        .build()
    else {
        return result(false, "refused");
    };
    let url = format!("{}/rooftop_sites?format=json", base.trim_end_matches('/'));
    let response = match client.get(url).bearer_auth(key.trim()).send().await {
        Ok(r) => r,
        Err(e) if e.is_timeout() => return result(false, "timed out"),
        Err(_) => return result(false, "refused"),
    };
    match response.status().as_u16() {
        401 | 403 => return result(false, "bad credentials"),
        429 => return result(false, "daily limit reached"),
        s if !(200..300).contains(&s) => return result(false, "refused"),
        _ => {}
    }
    let Ok(body) = response.json::<serde_json::Value>().await else {
        return result(false, "unexpected answer");
    };
    let sites: Vec<serde_json::Value> = body
        .get("sites")
        .and_then(serde_json::Value::as_array)
        .map(|sites| {
            sites
                .iter()
                .map(|s| {
                    json!({
                        "id": s.get("resource_id").and_then(serde_json::Value::as_str),
                        "name": s.get("name").and_then(serde_json::Value::as_str),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    TestResult {
        ok: true,
        message: "connected".into(),
        detail: Some(json!({ "sites": sites })),
    }
}

/// Logs in to Frigate (when `cfg` has a login) and reads `/api/config` within five seconds.
/// `detail.cameras` lists the enabled camera ids. Error text from the request is never echoed.
pub async fn frigate(cfg: &crate::config::FrigateConfig) -> TestResult {
    let mut cfg = cfg.clone();
    cfg.connect_timeout_ms = 5000;
    let Some(proxy) = crate::media::MediaProxy::new(cfg) else {
        return result(false, "the address is not valid");
    };
    match tokio::time::timeout(PROBE_TIMEOUT, proxy.camera_ids()).await {
        Err(_) => result(false, "timed out"),
        Ok(Ok(cameras)) => TestResult {
            ok: true,
            message: "connected".into(),
            detail: Some(json!({ "cameras": cameras })),
        },
        Ok(Err(crate::media::MediaError::Forbidden)) => result(false, "refused"),
        Ok(Err(crate::media::MediaError::Unavailable(m))) => {
            if m.contains("http 401") || m.contains("http 403") {
                result(false, "bad credentials")
            } else if m.contains("timed out") {
                result(false, "timed out")
            } else {
                result(false, "refused")
            }
        }
    }
}
