//! A fake Hugging Face server for the speech tests: model info, file trees and file bodies.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

/// The only token a gated repository accepts for its files.
pub const TOKEN: &str = "hf_test";

/// One repository the fake serves.
#[derive(Clone)]
pub struct FakeRepo {
    pub repo: String,
    pub sha: String,
    /// Gated repositories refuse file fetches without the right token (listing stays open).
    pub gated: bool,
    pub licence: Option<String>,
    pub files: Vec<(String, Vec<u8>)>,
    /// A file streamed in small chunks with a delay: (path, chunk size, delay).
    pub slow: Option<(String, usize, std::time::Duration)>,
    /// Announced `lfs.oid` values that replace the real hash: (path, oid).
    pub oids: Vec<(String, String)>,
}

impl FakeRepo {
    pub fn new(repo: &str, sha: &str, files: Vec<(&str, &[u8])>) -> FakeRepo {
        FakeRepo {
            repo: repo.into(),
            sha: sha.into(),
            gated: false,
            licence: None,
            slow: None,
            oids: vec![],
            files: files
                .into_iter()
                .map(|(p, b)| (p.to_string(), b.to_vec()))
                .collect(),
        }
    }
}

/// One request the fake saw.
#[derive(Debug, Clone)]
pub struct Logged {
    pub path: String,
    pub authorization: Option<String>,
}

/// Every request the fake saw, in order.
#[derive(Clone, Default)]
pub struct FakeHfLog(Arc<Mutex<Vec<Logged>>>);

impl FakeHfLog {
    pub fn requests(&self) -> Vec<Logged> {
        self.0.lock().unwrap().clone()
    }
}

#[derive(Clone)]
struct Fake {
    repos: Arc<Vec<FakeRepo>>,
    log: FakeHfLog,
}

/// Serves `repos` on `127.0.0.1:0`; returns the base URL and the request log.
pub async fn serve(repos: Vec<FakeRepo>) -> (String, FakeHfLog) {
    let log = FakeHfLog::default();
    let state = Fake {
        repos: Arc::new(repos),
        log: log.clone(),
    };
    let app = Router::new().fallback(handle).with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (base, log)
}

fn find<'a>(repos: &'a [FakeRepo], path: &'a str) -> Option<(&'a FakeRepo, &'a str)> {
    let mut it = path.splitn(3, '/');
    let (owner, name, rest) = (it.next()?, it.next()?, it.next().unwrap_or(""));
    let repo = repos.iter().find(|r| r.repo == format!("{owner}/{name}"))?;
    Some((repo, rest))
}

async fn handle(State(st): State<Fake>, uri: Uri, headers: HeaderMap) -> Response {
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    st.log.0.lock().unwrap().push(Logged {
        path: uri.to_string(),
        authorization: authorization.clone(),
    });
    let path = uri.path().trim_start_matches('/');
    if let Some(rest) = path.strip_prefix("api/models/") {
        let Some((repo, tail)) = find(&st.repos, rest) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let known = |rev: &str| rev == repo.sha || rev == "main";
        if tail.is_empty() || tail.strip_prefix("revision/").is_some_and(known) {
            let mut body = json!({ "sha": repo.sha, "gated": if repo.gated { json!("auto") } else { json!(false) } });
            if let Some(l) = &repo.licence {
                body["cardData"] = json!({ "license": l });
            }
            return axum::Json(body).into_response();
        }
        if tail.strip_prefix("tree/").is_some_and(known) {
            let entries: Vec<_> = repo
                .files
                .iter()
                .map(|(p, b)| {
                    let oid = repo
                        .oids
                        .iter()
                        .find(|(n, _)| n == p)
                        .map_or_else(|| hex(&Sha256::digest(b)), |(_, o)| o.clone());
                    json!({
                        "type": "file", "path": p, "size": b.len(),
                        "lfs": { "oid": oid, "size": b.len() },
                    })
                })
                .collect();
            return axum::Json(entries).into_response();
        }
        return StatusCode::NOT_FOUND.into_response();
    }
    // `{owner}/{repo}/resolve/{rev}/{path}`
    let Some((repo, tail)) = find(&st.repos, path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(file) = tail
        .strip_prefix("resolve/")
        .and_then(|t| t.split_once('/'))
        .map(|(_, f)| f)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if repo.gated && authorization.as_deref() != Some(&format!("Bearer {TOKEN}")) {
        return StatusCode::FORBIDDEN.into_response();
    }
    match repo.files.iter().find(|(p, _)| p == file) {
        Some((p, bytes)) => match &repo.slow {
            Some((slow, chunk, delay)) if slow == p => {
                let parts: Vec<Vec<u8>> = bytes.chunks(*chunk).map(<[u8]>::to_vec).collect();
                let delay = *delay;
                let stream =
                    futures_util::stream::unfold(parts.into_iter(), move |mut it| async move {
                        let part = it.next()?;
                        tokio::time::sleep(delay).await;
                        Some((Ok::<_, std::convert::Infallible>(part), it))
                    });
                let mut resp = axum::body::Body::from_stream(stream).into_response();
                resp.headers_mut()
                    .insert(header::CONTENT_LENGTH, bytes.len().into());
                resp
            }
            _ => bytes.clone().into_response(),
        },
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
