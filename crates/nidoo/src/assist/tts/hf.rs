//! A small Hugging Face client: repository metadata, file listing and file fetches.

use nidoo_api::{RepoListing, WeightFile};
use reqwest::StatusCode;
use serde::Deserialize;

use super::TtsError;
use super::source::HfSource;
use super::store::{SMALL_FILES, WEIGHT_FILES};

/// One file of a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoFile {
    /// The path inside the repository.
    pub path: String,
    /// The size in bytes.
    pub size: u64,
    /// The SHA-256 Hugging Face records for large files (`lfs.oid`).
    pub sha256: Option<String>,
}

#[derive(Deserialize)]
struct ModelInfo {
    sha: String,
    #[serde(default)]
    gated: serde_json::Value,
    #[serde(rename = "cardData", default)]
    card_data: Option<CardData>,
}

#[derive(Deserialize)]
struct CardData {
    license: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct TreeEntry {
    #[serde(rename = "type")]
    kind: String,
    path: String,
    #[serde(default)]
    size: u64,
    lfs: Option<Lfs>,
}

#[derive(Deserialize)]
struct Lfs {
    oid: String,
    size: u64,
}

/// Talks to one Hugging Face base address, with an optional token.
#[derive(Clone)]
pub struct HfClient {
    base: String,
    token: Option<String>,
    http: reqwest::Client,
}

impl HfClient {
    /// A client for `base` (no trailing slash needed); the token goes only in the header.
    pub fn new(base: &str, token: Option<String>) -> HfClient {
        HfClient {
            base: base.trim_end_matches('/').to_string(),
            token: token.filter(|t| !t.is_empty()),
            http: reqwest::Client::new(),
        }
    }

    async fn send(&self, url: String) -> Result<reqwest::Response, TtsError> {
        let mut req = self.http.get(url);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }
        // The transport error text can carry the URL, so only its kind is kept.
        let resp = req
            .send()
            .await
            .map_err(|_| TtsError::Upstream("could not reach Hugging Face".into()))?;
        match resp.status() {
            s if s.is_success() => Ok(resp),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(TtsError::Denied),
            StatusCode::NOT_FOUND => Err(TtsError::NotFound("repository or file".into())),
            s => Err(TtsError::Upstream(format!("status {}", s.as_u16()))),
        }
    }

    /// Reads a repository and says which Phonon weights it offers.
    pub async fn inspect(&self, src: &HfSource) -> Result<RepoListing, TtsError> {
        let repo = &src.repo;
        let info: ModelInfo = self
            .send(match &src.rev {
                Some(rev) => format!("{}/api/models/{repo}/revision/{rev}", self.base),
                None => format!("{}/api/models/{repo}", self.base),
            })
            .await?
            .json()
            .await
            .map_err(|_| TtsError::Upstream("unreadable repository answer".into()))?;
        // Always the commit the metadata described, never a moving branch name.
        let revision = info.sha;
        let files = self.files(repo, &revision).await?;
        let size_of = |name: &str| files.iter().find(|f| f.path == name).map(|f| f.size);
        let weights: Vec<WeightFile> = WEIGHT_FILES
            .iter()
            .filter_map(|n| {
                size_of(n).map(|size| WeightFile {
                    name: (*n).to_string(),
                    size,
                })
            })
            .collect();
        if size_of("config.json").is_none()
            || size_of("tokenizer.json").is_none()
            || weights.is_empty()
        {
            return Err(TtsError::Invalid("not a Phonon model".into()));
        }
        let small_bytes = SMALL_FILES.iter().filter_map(|n| size_of(n)).sum();
        let licence = info
            .card_data
            .and_then(|c| c.license)
            .and_then(|l| l.as_str().map(str::to_string));
        Ok(RepoListing {
            repo: repo.clone(),
            revision,
            gated: match info.gated {
                serde_json::Value::Bool(b) => b,
                serde_json::Value::String(_) => true,
                _ => false,
            },
            licence,
            weights,
            small_bytes,
        })
    }

    /// Lists every file of a repository at a revision.
    pub async fn files(&self, repo: &str, rev: &str) -> Result<Vec<RepoFile>, TtsError> {
        let entries: Vec<TreeEntry> = self
            .send(format!(
                "{}/api/models/{repo}/tree/{rev}?recursive=true",
                self.base
            ))
            .await?
            .json()
            .await
            .map_err(|_| TtsError::Upstream("unreadable file list".into()))?;
        Ok(entries
            .into_iter()
            .filter(|e| e.kind == "file")
            .map(|e| RepoFile {
                size: e.lfs.as_ref().map_or(e.size, |l| l.size),
                sha256: e.lfs.map(|l| l.oid),
                path: e.path,
            })
            .collect())
    }

    /// Starts fetching one file; the caller streams the body.
    pub async fn get(
        &self,
        repo: &str,
        rev: &str,
        path: &str,
    ) -> Result<reqwest::Response, TtsError> {
        self.send(format!("{}/{repo}/resolve/{rev}/{path}", self.base))
            .await
    }
}
