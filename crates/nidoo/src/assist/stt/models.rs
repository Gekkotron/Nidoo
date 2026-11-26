//! The recognition models, fetched from the pinned sherpa-onnx release and checked byte for
//! byte: the hub refuses any file whose size or SHA-256 differs from the table below.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use nidoo_api::{DownloadView, JobState, STT_MODEL_ID};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

use super::SttError;
use crate::assist::tts::download::{CHUNK_TIMEOUT, DISK_MARGIN_BYTES, free_bytes};

/// The archive being fetched.
const PART_FILE: &str = ".download.part";
/// Where files are checked before they move into place.
const STAGING_DIR: &str = ".staging";

/// One file the hub keeps, with its exact size and SHA-256.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedFile {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

/// One download: a `.tar.bz2` whose `top/<name>` members are kept (into the model folder),
/// or, with no `top`, one plain file (the VAD).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pinned {
    pub url: String,
    pub top: Option<String>,
    pub files: Vec<PinnedFile>,
}

/// Parakeet TDT 0.6B v3 (int8) and Silero VAD, as the sherpa-onnx `asr-models` release
/// serves them.
pub fn pinned_models() -> Vec<Pinned> {
    const RELEASE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models";
    let file = |name: &str, size: u64, sha256: &str| PinnedFile {
        name: name.to_owned(),
        size,
        sha256: sha256.to_owned(),
    };
    vec![
        Pinned {
            url: format!("{RELEASE}/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2"),
            top: Some("sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8".to_owned()),
            files: vec![
                file(
                    "encoder.int8.onnx",
                    652_184_281,
                    "acfc2b4456377e15d04f0243af540b7fe7c992f8d898d751cf134c3a55fd2247",
                ),
                file(
                    "decoder.int8.onnx",
                    11_845_275,
                    "179e50c43d1a9de79c8a24149a2f9bac6eb5981823f2a2ed88d655b24248db4e",
                ),
                file(
                    "joiner.int8.onnx",
                    6_355_277,
                    "3164c13fc2821009440d20fcb5fdc78bff28b4db2f8d0f0b329101719c0948b3",
                ),
                file(
                    "tokens.txt",
                    93_939,
                    "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
                ),
            ],
        },
        Pinned {
            url: format!("{RELEASE}/silero_vad.onnx"),
            top: None,
            files: vec![file(
                "silero_vad.onnx",
                643_854,
                "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6",
            )],
        },
    ]
}

struct Job {
    view: DownloadView,
    cancel: watch::Sender<bool>,
}

type Shared = Arc<Mutex<Option<Job>>>;

fn update(job: &Shared, f: impl FnOnce(&mut DownloadView)) {
    if let Some(j) = job.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        f(&mut j.view);
    }
}

/// The one model download speech input runs at a time.
pub struct SttModels {
    job: Shared,
    free: fn(&Path) -> Option<u64>,
}

impl Default for SttModels {
    fn default() -> Self {
        Self::with_free(free_bytes)
    }
}

impl SttModels {
    /// With another free-space probe (tests).
    pub fn with_free(free: fn(&Path) -> Option<u64>) -> SttModels {
        SttModels {
            job: Shared::default(),
            free,
        }
    }

    /// Starts fetching `pinned` into `model_dir` (archives) and `vad` (a plain file), with
    /// `root` for the part file and the staging folder; `done` runs once all is in place.
    pub fn start(
        &self,
        root: &Path,
        model_dir: PathBuf,
        vad: PathBuf,
        pinned: Vec<Pinned>,
        done: impl FnOnce() + Send + 'static,
    ) -> Result<DownloadView, SttError> {
        let mut guard = self.job.lock().unwrap_or_else(|e| e.into_inner());
        if guard
            .as_ref()
            .is_some_and(|j| j.view.state == JobState::Running)
        {
            return Err(SttError::Conflict("a download is already running".into()));
        }
        let view = DownloadView {
            model: STT_MODEL_ID.to_owned(),
            state: JobState::Running,
            done: 0,
            total: 0,
            file: String::new(),
            error: None,
        };
        let (cancel, mut cancelled) = watch::channel(false);
        *guard = Some(Job {
            view: view.clone(),
            cancel,
        });
        drop(guard);
        let ctx = Ctx {
            job: self.job.clone(),
            root: root.to_owned(),
            model_dir,
            vad,
            free: self.free,
        };
        tokio::spawn(async move {
            let outcome = tokio::select! {
                r = ctx.run(&pinned) => Some(r),
                _ = cancelled.wait_for(|c| *c) => None,
            };
            ctx.clean();
            match outcome {
                Some(Ok(())) => {
                    done();
                    update(&ctx.job, |v| v.state = JobState::Done);
                }
                Some(Err(e)) => update(&ctx.job, |v| {
                    v.state = JobState::Failed;
                    v.error = Some(e.to_string());
                }),
                None => update(&ctx.job, |v| v.state = JobState::Cancelled),
            }
        });
        Ok(view)
    }

    /// The running job, or the last finished one.
    pub fn current(&self) -> Option<DownloadView> {
        self.job
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|j| j.view.clone())
    }

    /// Stops the running job; `NotFound` when none runs.
    pub fn cancel(&self) -> Result<(), SttError> {
        match self.job.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            Some(j) if j.view.state == JobState::Running => {
                let _ = j.cancel.send(true);
                Ok(())
            }
            _ => Err(SttError::NotFound("no download is running".into())),
        }
    }
}

struct Ctx {
    job: Shared,
    root: PathBuf,
    model_dir: PathBuf,
    vad: PathBuf,
    free: fn(&Path) -> Option<u64>,
}

impl Ctx {
    fn part(&self) -> PathBuf {
        self.root.join(PART_FILE)
    }

    fn staging(&self) -> PathBuf {
        self.root.join(STAGING_DIR)
    }

    /// Removes the part file and the staging folder, whatever happened.
    fn clean(&self) {
        let _ = std::fs::remove_file(self.part());
        let _ = std::fs::remove_dir_all(self.staging());
    }

    async fn run(&self, pinned: &[Pinned]) -> Result<(), SttError> {
        // An archive's members are no bigger than the archive: room for both, plus a margin.
        let need = pinned
            .iter()
            .map(|p| {
                let files: u64 = p.files.iter().map(|f| f.size).sum();
                if p.top.is_some() { 2 * files } else { files }
            })
            .sum::<u64>()
            + DISK_MARGIN_BYTES;
        std::fs::create_dir_all(&self.root).map_err(|e| SttError::Io(e.to_string()))?;
        match (self.free)(&self.root) {
            Some(free) if free < need => return Err(SttError::NoSpace { need, free }),
            Some(_) => {}
            None => tracing::warn!("cannot tell the free disk space; skipping the check"),
        }
        for p in pinned {
            self.fetch(p).await?;
        }
        Ok(())
    }

    async fn fetch(&self, p: &Pinned) -> Result<(), SttError> {
        let name = p.url.rsplit('/').next().unwrap_or(&p.url).to_owned();
        update(&self.job, |v| v.file = name.clone());
        let resp = reqwest::get(&p.url)
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| SttError::Upstream(format!("{name}: {e}")))?;
        let expected: u64 = p.files.iter().map(|f| f.size).sum();
        let length = resp.content_length().unwrap_or(expected);
        update(&self.job, |v| v.total += length);
        let staging = self.staging();
        std::fs::create_dir_all(&staging).map_err(|e| SttError::Io(e.to_string()))?;
        let target = match &p.top {
            Some(_) => self.part(),
            None => staging.join(&p.files[0].name),
        };
        let mut out = tokio::fs::File::create(&target)
            .await
            .map_err(|e| SttError::Io(format!("write {name}: {e}")))?;
        let mut stream = resp.bytes_stream();
        let mut got = 0u64;
        loop {
            let next = tokio::time::timeout(CHUNK_TIMEOUT, stream.next())
                .await
                .map_err(|_| SttError::Upstream(format!("{name}: the server stopped sending")))?;
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(|e| SttError::Upstream(format!("{name}: {e}")))?;
            got += chunk.len() as u64;
            if p.top.is_none() && got > expected {
                return Err(SttError::Invalid(format!("{name} is bigger than expected")));
            }
            out.write_all(&chunk)
                .await
                .map_err(|e| SttError::Io(format!("write {name}: {e}")))?;
            update(&self.job, |v| v.done += chunk.len() as u64);
        }
        out.flush().await.map_err(|e| SttError::Io(e.to_string()))?;
        drop(out);
        match &p.top {
            Some(top) => {
                let (part, top, files) = (self.part(), top.clone(), p.files.clone());
                let dir = staging.clone();
                update(&self.job, |v| v.file = format!("{name} (unpacking)"));
                tokio::task::spawn_blocking(move || extract(&part, &top, &files, &dir))
                    .await
                    .map_err(|e| SttError::Io(e.to_string()))??;
                let _ = std::fs::remove_dir_all(&self.model_dir);
                std::fs::rename(&staging, &self.model_dir)
                    .map_err(|e| SttError::Io(format!("install the model: {e}")))?;
                let _ = std::fs::remove_file(self.part());
            }
            None => {
                let file = &p.files[0];
                check(&target, file)?;
                std::fs::rename(&target, &self.vad)
                    .map_err(|e| SttError::Io(format!("install {}: {e}", file.name)))?;
                let _ = std::fs::remove_dir_all(&staging);
            }
        }
        Ok(())
    }
}

/// Unpacks the `top/<name>` members of `files` from the `.tar.bz2` at `archive` into `dir`,
/// checking each one; anything else in the archive is skipped.
fn extract(archive: &Path, top: &str, files: &[PinnedFile], dir: &Path) -> Result<(), SttError> {
    let io = |e: std::io::Error| SttError::Io(format!("unpack the model: {e}"));
    let reader = std::fs::File::open(archive).map_err(io)?;
    let mut tar = tar::Archive::new(bzip2::read::BzDecoder::new(reader));
    let mut found = Vec::new();
    for entry in tar.entries().map_err(io)? {
        let mut entry = entry.map_err(io)?;
        let path = entry.path().map_err(io)?.to_string_lossy().into_owned();
        let Some(file) = files
            .iter()
            .find(|f| path.trim_start_matches("./") == format!("{top}/{}", f.name))
        else {
            continue;
        };
        let mut out = std::fs::File::create(dir.join(&file.name)).map_err(io)?;
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = entry.read(&mut buf).map_err(io)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(io)?;
        }
        out.sync_all().map_err(io)?;
        check(&dir.join(&file.name), file)?;
        found.push(file.name.clone());
    }
    match files.iter().find(|f| !found.contains(&f.name)) {
        Some(missing) => Err(SttError::Invalid(format!(
            "{} is missing from the archive",
            missing.name
        ))),
        None => Ok(()),
    }
}

/// Refuses a file whose size or SHA-256 is not the pinned one.
fn check(path: &Path, file: &PinnedFile) -> Result<(), SttError> {
    let io = |e: std::io::Error| SttError::Io(format!("read {}: {e}", file.name));
    let mut reader = std::fs::File::open(path).map_err(io)?;
    let mut hash = Sha256::new();
    let mut size = 0u64;
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = reader.read(&mut buf).map_err(io)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        hash.update(&buf[..n]);
    }
    let sha: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if size != file.size || sha != file.sha256 {
        return Err(SttError::Invalid(format!(
            "{} does not match its pinned size and SHA-256",
            file.name
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
