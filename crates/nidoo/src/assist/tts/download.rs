//! The background job that fetches one speech model from Hugging Face.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use nidoo_api::{DownloadRequest, DownloadView, JobState, ModelSource};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

use super::TtsError;
use super::hf::{HfClient, RepoFile};
use super::source::{HfSource, parse_source};
use super::store::{
    MAX_SMALL_FILE_BYTES, MAX_WEIGHTS_BYTES, Manifest, ManifestFile, ModelStore, SMALL_FILES,
    WEIGHT_FILES, quant_of,
};

/// Free space kept beyond the download itself, in bytes (256 MiB).
pub const DISK_MARGIN_BYTES: u64 = 256 * 1024 * 1024;

/// How long to wait for the next piece of a file before giving up.
pub const CHUNK_TIMEOUT: Duration = Duration::from_secs(60);

/// Free bytes on the disk that holds `path`: the disk whose mount point is the
/// longest prefix of the canonical path. `None` when it cannot be told.
pub fn free_bytes(path: &Path) -> Option<u64> {
    let path = path.canonicalize().ok()?;
    let disks = sysinfo::Disks::new_with_refreshed_list().ok()?;
    disks
        .iter()
        .filter(|d| path.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
}

struct Job {
    view: DownloadView,
    cancel: watch::Sender<bool>,
}

type Shared = Arc<Mutex<Option<Job>>>;

/// The one model download the hub runs at a time.
pub struct Downloads {
    job: Shared,
    free: fn(&Path) -> Option<u64>,
    chunk_timeout: Duration,
}

impl Default for Downloads {
    fn default() -> Self {
        Downloads {
            job: Shared::default(),
            free: free_bytes,
            chunk_timeout: CHUNK_TIMEOUT,
        }
    }
}

fn update(job: &Shared, f: impl FnOnce(&mut DownloadView)) {
    if let Some(j) = job.lock().unwrap().as_mut() {
        f(&mut j.view);
    }
}

fn valid_part(s: &str) -> bool {
    !s.is_empty()
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

impl Downloads {
    /// For tests: a different free-space probe and chunk timeout.
    pub fn with_limits(free: fn(&Path) -> Option<u64>, chunk_timeout: Duration) -> Downloads {
        Downloads {
            job: Shared::default(),
            free,
            chunk_timeout,
        }
    }

    /// Starts the download; `done` runs with the manifest once the model is installed.
    pub fn start(
        &self,
        store: Arc<ModelStore>,
        client: HfClient,
        req: DownloadRequest,
        done: impl FnOnce(Manifest) + Send + 'static,
    ) -> Result<DownloadView, TtsError> {
        let quant = quant_of(&req.weights)
            .filter(|_| WEIGHT_FILES.contains(&req.weights.as_str()))
            .ok_or_else(|| TtsError::Invalid(format!("unknown weights file: {}", req.weights)))?;
        let src = parse_source(&req.repo)?;
        if src.rev.is_some() || !valid_part(&req.revision) {
            return Err(TtsError::Invalid("bad repository or revision".into()));
        }
        let id = format!("{}--{quant}", src.repo.replace('/', "--")).to_lowercase();
        let mut guard = self.job.lock().unwrap();
        if guard
            .as_ref()
            .is_some_and(|j| j.view.state == JobState::Running)
        {
            return Err(TtsError::Conflict("a download is already running".into()));
        }
        if store.get(&id).is_some() {
            return Err(TtsError::Conflict(format!(
                "model {id} is already installed"
            )));
        }
        let view = DownloadView {
            model: id.clone(),
            state: JobState::Running,
            done: 0,
            total: 0,
            file: String::new(),
            error: None,
        };
        let (cancel, cancelled) = watch::channel(false);
        *guard = Some(Job {
            view: view.clone(),
            cancel,
        });
        drop(guard);
        let job = self.job.clone();
        let (free, chunk_timeout) = (self.free, self.chunk_timeout);
        tokio::spawn(async move {
            let ctx = Ctx {
                job: job.clone(),
                store,
                client,
                free,
                chunk_timeout,
            };
            let mut cancelled = cancelled;
            let outcome = tokio::select! {
                r = ctx.run(id, src, req) => Some(r),
                _ = cancelled.wait_for(|c| *c) => None,
            };
            match outcome {
                Some(Ok(manifest)) => {
                    done(manifest);
                    update(&job, |v| v.state = JobState::Done);
                }
                Some(Err(e)) => update(&job, |v| {
                    v.state = JobState::Failed;
                    v.error = Some(e.to_string());
                }),
                None => update(&job, |v| v.state = JobState::Cancelled),
            }
        });
        Ok(view)
    }

    /// The running job, or the last finished one.
    pub fn current(&self) -> Option<DownloadView> {
        self.job.lock().unwrap().as_ref().map(|j| j.view.clone())
    }

    /// Stops the running job; `NotFound` when none runs.
    pub fn cancel(&self) -> Result<(), TtsError> {
        match self.job.lock().unwrap().as_ref() {
            Some(j) if j.view.state == JobState::Running => {
                let _ = j.cancel.send(true);
                Ok(())
            }
            _ => Err(TtsError::NotFound("no download is running".into())),
        }
    }
}

struct Ctx {
    job: Shared,
    store: Arc<ModelStore>,
    client: HfClient,
    free: fn(&Path) -> Option<u64>,
    chunk_timeout: Duration,
}

impl Ctx {
    async fn run(
        &self,
        id: String,
        src: HfSource,
        req: DownloadRequest,
    ) -> Result<Manifest, TtsError> {
        let listed = self.client.files(&src.repo, &req.revision).await?;
        let wanted: Vec<RepoFile> = listed
            .into_iter()
            .filter(|f| f.path == req.weights || SMALL_FILES.contains(&f.path.as_str()))
            .collect();
        if !wanted.iter().any(|f| f.path == req.weights) {
            return Err(TtsError::NotFound(format!(
                "{} in {}",
                req.weights, src.repo
            )));
        }
        for f in &wanted {
            let cap = if f.path == req.weights {
                MAX_WEIGHTS_BYTES
            } else {
                MAX_SMALL_FILE_BYTES
            };
            if f.size > cap {
                return Err(TtsError::TooLarge(format!(
                    "{} is {} bytes",
                    f.path, f.size
                )));
            }
        }
        let total: u64 = wanted.iter().map(|f| f.size).sum();
        update(&self.job, |v| v.total = total);
        let need = total + DISK_MARGIN_BYTES;
        let probe = self.store.dir("x");
        match (self.free)(probe.parent().unwrap_or(&probe)) {
            Some(free) if free < need => return Err(TtsError::NoSpace { need, free }),
            Some(_) => {}
            None => tracing::warn!("cannot tell the free disk space; skipping the check"),
        }
        let staging = self.store.staging()?;
        let mut files = Vec::new();
        for f in &wanted {
            update(&self.job, |v| v.file = f.path.clone());
            files.push(
                self.fetch(&src.repo, &req.revision, f, &staging.dir)
                    .await?,
            );
        }
        // Best effort: the licence comes from the repository metadata.
        let licence = self
            .client
            .inspect(&HfSource {
                repo: src.repo.clone(),
                rev: Some(req.revision.clone()),
            })
            .await
            .ok()
            .and_then(|l| l.licence);
        let manifest = Manifest {
            id,
            repo: Some(src.repo),
            revision: Some(req.revision),
            weights: req.weights,
            files,
            licence,
            languages: Vec::new(),
            source: ModelSource::Download,
            installed_at: chrono::Utc::now().timestamp(),
        };
        staging.finish(&self.store, manifest)
    }

    async fn fetch(
        &self,
        repo: &str,
        rev: &str,
        f: &RepoFile,
        dir: &Path,
    ) -> Result<ManifestFile, TtsError> {
        let io = |e: std::io::Error| TtsError::Io(format!("write {}: {e}", f.path));
        let resp = self.client.get(repo, rev, &f.path).await?;
        let mut stream = resp.bytes_stream();
        let mut out = tokio::fs::File::create(dir.join(&f.path))
            .await
            .map_err(io)?;
        let mut hash = Sha256::new();
        let mut got = 0u64;
        loop {
            let next = tokio::time::timeout(self.chunk_timeout, stream.next())
                .await
                .map_err(|_| TtsError::Upstream("Hugging Face stopped sending".into()))?;
            let Some(chunk) = next else { break };
            let chunk = chunk
                .map_err(|_| TtsError::Upstream("the connection to Hugging Face broke".into()))?;
            got += chunk.len() as u64;
            if got > f.size {
                return Err(TtsError::Invalid(format!(
                    "{} is bigger than its listed size",
                    f.path
                )));
            }
            hash.update(&chunk);
            out.write_all(&chunk).await.map_err(io)?;
            update(&self.job, |v| v.done += chunk.len() as u64);
        }
        out.flush().await.map_err(io)?;
        if got != f.size {
            return Err(TtsError::Invalid(format!(
                "{} is {got} bytes, expected {}",
                f.path, f.size
            )));
        }
        let sha: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if let Some(want) = &f.sha256
            && !want.eq_ignore_ascii_case(&sha)
        {
            return Err(TtsError::Invalid(format!(
                "{} does not match its checksum",
                f.path
            )));
        }
        Ok(ManifestFile {
            name: f.path.clone(),
            size: got,
            sha256: Some(sha),
        })
    }
}
