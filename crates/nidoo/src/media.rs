//! Frigate media behind the hub: alert snapshots and clips live on Frigate's login port, which
//! the browser cannot reach without the session cookie. The hub rewrites those URLs to
//! `/api/v1/media?url=…` and fetches them itself with the `frigate_token` cookie. With
//! `[frigate] live` it also serves live camera frames, from a separate, narrower set of paths.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use futures_util::TryStreamExt;
use reqwest::header::{
    ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, COOKIE, RANGE, SET_COOKIE,
};
use reqwest::redirect::Policy;
use tokio::sync::Mutex;
use url::Url;

use crate::config::FrigateConfig;

/// The path the web fetches media through; the original URL goes in the `url` query key.
pub const MEDIA_PATH: &str = "/api/v1/media";
const COOKIE_NAME: &str = "frigate_token";
/// The only Frigate files the hub fetches: an event's snapshot, clip and thumbnail.
const EVENT_FILES: [&str; 3] = ["snapshot.jpg", "clip.mp4", "thumbnail.jpg"];
/// A login that takes longer than this has failed.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(10);
/// The camera list (`/api/config`) and a `latest.jpg` frame must arrive within this. The MJPEG
/// stream has no total timeout.
const SHORT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the camera list is reused before `/api/config` is read again.
const CAMERAS_TTL: Duration = Duration::from_secs(60);
/// An unknown camera id re-reads the list at most this often.
const CAMERAS_MISS_REFRESH: Duration = Duration::from_secs(10);
/// After a failed read of the list, Frigate is not asked again for this long: the requests in
/// between answer at once instead of each waiting out a connect timeout.
const CAMERAS_RETRY: Duration = Duration::from_secs(10);
/// The MJPEG stream's frame rate.
const STREAM_FPS: u32 = 5;

/// A live camera file the hub may fetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraFile {
    /// `/api/{id}/latest.jpg`, one frame.
    Latest,
    /// `/api/{id}`, the MJPEG stream.
    Stream,
}

/// Whether `id` looks like a Frigate camera id: 1 to 64 of `[A-Za-z0-9_-]`. Anything else
/// (a slash, a dot, a space) never reaches Frigate.
pub fn is_camera_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// `scheme://host[:port]` of an absolute http(s) URL, host lowercased, default port dropped.
pub fn origin_of(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return None;
    }
    Some(parsed.origin().ascii_serialization())
}

/// The hub path that serves `url`, when `url` is on `media_origin`; `None` leaves it alone.
pub fn proxied(url: &str, media_origin: &str) -> Option<String> {
    if origin_of(url)? != media_origin {
        return None;
    }
    let query: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("url", url)
        .finish();
    Some(format!("{MEDIA_PATH}?{query}"))
}

/// Whether `url` names an event's media (`/api/events/<id>/{snapshot.jpg,clip.mp4,thumbnail.jpg}`,
/// query allowed). The hub never fetches anything else from Frigate: its login would otherwise
/// open Frigate's whole API (live frames, recordings, the raw config) to the LAN.
pub fn is_event_media(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    let Some(segments) = parsed.path_segments() else {
        return false;
    };
    let segments: Vec<&str> = segments.collect();
    matches!(
        segments.as_slice(),
        ["api", "events", id, file] if !id.is_empty() && EVENT_FILES.contains(file)
    )
}

/// Why a media fetch did not reach Frigate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MediaError {
    /// The URL is not on the configured Frigate origin (or no Frigate is configured).
    #[error("media URL is not on the Frigate origin")]
    Forbidden,
    #[error("frigate: {0}")]
    Unavailable(String),
}

/// Frigate's reply, as far as the browser needs it.
pub struct Upstream {
    pub status: u16,
    pub content_type: Option<String>,
    pub content_length: Option<String>,
    pub content_range: Option<String>,
    pub accept_ranges: Option<String>,
    pub body: Body,
}

/// Fetches Frigate media with the session cookie, logging in on demand.
pub struct MediaProxy {
    client: reqwest::Client,
    base: String,
    origin: String,
    login: Option<(String, String)>,
    token: Mutex<Option<String>>,
    /// The last failure warned about, so a Frigate that stays down is logged once.
    last_warning: std::sync::Mutex<Option<String>>,
    live: bool,
    /// The camera list. Held across a read, so concurrent callers share one.
    cameras: Mutex<CameraCache>,
}

/// The last camera list Frigate gave, and when the last read failed.
#[derive(Default)]
struct CameraCache {
    ids: Vec<String>,
    read_at: Option<Instant>,
    failed_at: Option<Instant>,
}

impl MediaProxy {
    /// `None` without a `url`. The login is used only when both `user` and `password` are set.
    pub fn new(cfg: FrigateConfig) -> Option<Arc<MediaProxy>> {
        let raw = cfg.url.as_deref()?.trim().trim_end_matches('/').to_owned();
        let origin = origin_of(&raw)?;
        let login = match (cfg.user, cfg.password) {
            (Some(user), Some(password)) if !user.is_empty() && !password.is_empty() => {
                Some((user, password))
            }
            _ => None,
        };
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(cfg.connect_timeout_ms))
            .redirect(Policy::none())
            .build()
            .ok()?;
        Some(Arc::new(MediaProxy {
            client,
            base: raw,
            origin,
            login,
            token: Mutex::new(None),
            last_warning: std::sync::Mutex::new(None),
            live: cfg.live,
            cameras: Mutex::new(CameraCache::default()),
        }))
    }

    /// `[frigate] live`: whether live camera frames may be served.
    pub fn live(&self) -> bool {
        self.live
    }

    /// `scheme://host[:port]` the proxy serves.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Fetches `url` (which must be on the origin) with the cookie when a login is configured,
    /// passing `range` through. A 401 triggers one new login and one retry; a second 401 is
    /// returned as is.
    pub async fn fetch(&self, url: &str, range: Option<&str>) -> Result<Upstream, MediaError> {
        if origin_of(url).as_deref() != Some(self.origin.as_str()) || !is_event_media(url) {
            return Err(MediaError::Forbidden);
        }
        self.get(url, range, None).await.map(upstream)
    }

    /// The enabled camera ids from Frigate's `/api/config` (only the keys of `cameras` are
    /// kept), read again once they are older than [`CAMERAS_TTL`]. When a read fails the last
    /// list stays in use, and Frigate is left alone for [`CAMERAS_RETRY`].
    pub async fn camera_ids(&self) -> Result<Vec<String>, MediaError> {
        let mut cache = self.cameras.lock().await;
        self.cached_ids(&mut cache, CAMERAS_TTL).await
    }

    /// Whether `id` is one of Frigate's enabled cameras. A miss reads the list again when it
    /// is older than [`CAMERAS_MISS_REFRESH`], so a camera just added works straight away.
    /// Within [`CAMERAS_RETRY`] of a failure (a list read or a frame) it fails at once.
    pub async fn has_camera(&self, id: &str) -> Result<bool, MediaError> {
        let mut cache = self.cameras.lock().await;
        if cache
            .failed_at
            .is_some_and(|at| at.elapsed() < CAMERAS_RETRY)
        {
            return Err(MediaError::Unavailable("Frigate failed lately".into()));
        }
        if self
            .cached_ids(&mut cache, CAMERAS_TTL)
            .await?
            .iter()
            .any(|known| known == id)
        {
            return Ok(true);
        }
        let ids = self.cached_ids(&mut cache, CAMERAS_MISS_REFRESH).await?;
        Ok(ids.iter().any(|known| known == id))
    }

    /// The list when it is younger than `max_age`, else a fresh read; see [`Self::camera_ids`].
    async fn cached_ids(
        &self,
        cache: &mut CameraCache,
        max_age: Duration,
    ) -> Result<Vec<String>, MediaError> {
        if cache.read_at.is_some_and(|at| at.elapsed() < max_age) {
            return Ok(cache.ids.clone());
        }
        let failed_lately = cache
            .failed_at
            .is_some_and(|at| at.elapsed() < CAMERAS_RETRY);
        let result = if failed_lately {
            Err(MediaError::Unavailable(
                "camera list: Frigate failed lately".into(),
            ))
        } else {
            self.read_camera_ids().await
        };
        match result {
            Ok(ids) => {
                cache.ids = ids.clone();
                cache.read_at = Some(Instant::now());
                cache.failed_at = None;
                Ok(ids)
            }
            Err(err) => {
                if !failed_lately {
                    cache.failed_at = Some(Instant::now());
                }
                match cache.read_at {
                    Some(_) => Ok(cache.ids.clone()),
                    None => Err(err),
                }
            }
        }
    }

    /// Fetches a live frame (`latest.jpg`) or the MJPEG stream of camera `id` at `height`
    /// pixels. Both height spellings are sent because Frigate versions differ; Frigate ignores
    /// the one it does not know. `Forbidden` for an id that fails [`is_camera_id`].
    pub async fn fetch_camera(
        &self,
        id: &str,
        file: CameraFile,
        height: u32,
    ) -> Result<Upstream, MediaError> {
        if !is_camera_id(id) {
            return Err(MediaError::Forbidden);
        }
        let base = &self.base;
        let (url, timeout) = match file {
            CameraFile::Latest => (
                format!("{base}/api/{id}/latest.jpg?h={height}&height={height}"),
                Some(SHORT_TIMEOUT),
            ),
            CameraFile::Stream => (
                format!("{base}/api/{id}?h={height}&height={height}&fps={STREAM_FPS}"),
                None,
            ),
        };
        match self.get(&url, None, timeout).await {
            Ok(response) => Ok(upstream(response)),
            Err(err) => {
                // Frigate is likely down: spare it (and the browser's connections) for a while.
                self.cameras.lock().await.failed_at = Some(Instant::now());
                Err(err)
            }
        }
    }

    async fn read_camera_ids(&self) -> Result<Vec<String>, MediaError> {
        let url = format!("{}/api/config", self.base);
        let response = self.get(&url, None, Some(SHORT_TIMEOUT)).await?;
        let status = response.status();
        if !status.is_success() {
            let message = format!("camera list http {}", status.as_u16());
            self.warn_once(message.clone());
            return Err(MediaError::Unavailable(message));
        }
        let config: serde_json::Value = response.json().await.map_err(|err| {
            let message = format!("camera list: {err}");
            self.warn_once(message.clone());
            MediaError::Unavailable(message)
        })?;
        Ok(config
            .get("cameras")
            .and_then(serde_json::Value::as_object)
            .map(|cameras| {
                cameras
                    .iter()
                    .filter(|(_, camera)| camera.get("enabled") != Some(&false.into()))
                    .map(|(id, _)| id.clone())
                    .collect()
            })
            .unwrap_or_default())
    }

    /// GETs `url` with the cookie when a login is configured, passing `range` through and
    /// bounding the whole exchange by `timeout` when set. A 401 triggers one new login and one
    /// retry; a second 401 is returned as is.
    async fn get(
        &self,
        url: &str,
        range: Option<&str>,
        timeout: Option<Duration>,
    ) -> Result<reqwest::Response, MediaError> {
        let mut retried = false;
        loop {
            let token = match &self.login {
                Some(_) => Some(self.token().await?),
                None => None,
            };
            let mut request = self.client.get(url);
            if let Some(token) = &token {
                request = request.header(COOKIE, format!("{COOKIE_NAME}={token}"));
            }
            if let Some(range) = range {
                request = request.header(RANGE, range);
            }
            if let Some(timeout) = timeout {
                request = request.timeout(timeout);
            }
            let response = request.send().await.map_err(|err| {
                self.warn_once(format!("media fetch failed: {err}"));
                MediaError::Unavailable(err.to_string())
            })?;
            if response.status() == reqwest::StatusCode::UNAUTHORIZED && token.is_some() && !retried
            {
                retried = true;
                *self.token.lock().await = None;
                continue;
            }
            return Ok(response);
        }
    }

    /// The cached token, or a fresh one from `POST /api/login`. Logins are serialised so
    /// concurrent fetches share one.
    async fn token(&self) -> Result<String, MediaError> {
        let mut slot = self.token.lock().await;
        if let Some(token) = slot.as_ref() {
            return Ok(token.clone());
        }
        let (user, password) = self.login.as_ref().ok_or(MediaError::Forbidden)?;
        let token = self.login(user, password).await.map_err(|err| {
            self.warn_once(format!("Frigate {err}"));
            err
        })?;
        *slot = Some(token.clone());
        Ok(token)
    }

    async fn login(&self, user: &str, password: &str) -> Result<String, MediaError> {
        let response = self
            .client
            .post(format!("{}/api/login", self.base))
            .timeout(LOGIN_TIMEOUT)
            .json(&serde_json::json!({ "user": user, "password": password }))
            .send()
            .await
            .map_err(|err| MediaError::Unavailable(format!("login: {err}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(MediaError::Unavailable(format!(
                "login http {}",
                status.as_u16()
            )));
        }
        response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find_map(token_in)
            .ok_or_else(|| MediaError::Unavailable("no token in Frigate's login answer".into()))
    }

    /// Warns about `message` unless it is the same as the last warning.
    fn warn_once(&self, message: String) {
        let mut last = self
            .last_warning
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if last.as_deref() != Some(message.as_str()) {
            tracing::warn!(frigate = %self.origin, "{message}");
            *last = Some(message);
        }
    }
}

/// The `frigate_token` value of one `Set-Cookie` header.
fn token_in(set_cookie: &str) -> Option<String> {
    set_cookie
        .split(';')
        .map(str::trim)
        .find_map(|part| {
            part.strip_prefix(COOKIE_NAME)
                .and_then(|rest| rest.strip_prefix('='))
        })
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
}

fn upstream(response: reqwest::Response) -> Upstream {
    let header = |name| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    Upstream {
        status: response.status().as_u16(),
        content_type: header(CONTENT_TYPE),
        content_length: header(CONTENT_LENGTH),
        content_range: header(CONTENT_RANGE),
        accept_ranges: header(ACCEPT_RANGES),
        body: Body::from_stream(response.bytes_stream().map_err(std::io::Error::other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FrigateConfig;

    #[test]
    fn token_in_finds_the_cookie() {
        assert_eq!(
            token_in("frigate_token=jwt; Path=/; HttpOnly").as_deref(),
            Some("jwt")
        );
        assert_eq!(token_in("other=1; Path=/"), None);
        assert_eq!(token_in("frigate_token=; Path=/"), None);
    }

    #[test]
    fn origin_of_normalises() {
        assert_eq!(
            origin_of("http://H:8971/api/events/1/snapshot.jpg").as_deref(),
            Some("http://h:8971")
        );
        assert_eq!(origin_of("http://h:80/").as_deref(), Some("http://h"));
        assert_eq!(origin_of("https://h/").as_deref(), Some("https://h"));
        assert_eq!(origin_of("nope"), None);
        assert_eq!(origin_of("/api/events/1/snapshot.jpg"), None);
    }

    #[test]
    fn proxied_only_same_origin() {
        assert_eq!(
            proxied("http://h:8971/api/events/1/snapshot.jpg", "http://h:8971").as_deref(),
            Some("/api/v1/media?url=http%3A%2F%2Fh%3A8971%2Fapi%2Fevents%2F1%2Fsnapshot.jpg")
        );
        assert_eq!(
            proxied("http://h:5000/api/events/1/snapshot.jpg", "http://h:8971"),
            None
        );
        assert_eq!(proxied("nope", "http://h:8971"), None);
    }

    #[test]
    fn only_event_media_is_fetched() {
        for ok in [
            "http://h:8971/api/events/1792401849.2-wg0y/snapshot.jpg",
            "http://h:8971/api/events/1/clip.mp4",
            "http://h:8971/api/events/1/thumbnail.jpg?h=112",
        ] {
            assert!(is_event_media(ok), "{ok}");
        }
        for bad in [
            "http://h:8971/api/config/raw",
            "http://h:8971/api/front/latest.jpg",
            "http://h:8971/api/events/1/snapshot.jpg/../../config/raw",
            "http://h:8971/api/events//snapshot.jpg",
            "http://h:8971/api/events/1/other.bin",
            "http://h:8971/api/events/1",
            "nope",
        ] {
            assert!(!is_event_media(bad), "{bad}");
        }
    }

    #[test]
    fn proxy_needs_a_url() {
        assert!(MediaProxy::new(FrigateConfig::default()).is_none());
        let proxy = MediaProxy::new(FrigateConfig {
            url: Some("http://H:8971/".into()),
            ..FrigateConfig::default()
        })
        .expect("a proxy");
        assert_eq!(proxy.origin(), "http://h:8971");
    }
}
