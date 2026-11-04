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
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// Serves `files` by path, as many requests as come; counts the requests.
    async fn serve(files: Vec<(&'static str, Vec<u8>)>) -> (String, Arc<AtomicUsize>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                counter.fetch_add(1, Ordering::SeqCst);
                let files = files.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).to_string();
                    let path = head.split_whitespace().nth(1).unwrap_or("").to_owned();
                    let reply = match files.iter().find(|(p, _)| *p == path) {
                        Some((_, body)) => {
                            let mut r = format!(
                                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                                body.len()
                            )
                            .into_bytes();
                            r.extend_from_slice(body);
                            r
                        }
                        None => b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_vec(),
                    };
                    let _ = sock.write_all(&reply).await;
                });
            }
        });
        (base, hits)
    }

    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// A `.tar.bz2` holding `top/<name>` for each member, plus an extra file.
    fn archive(top: &str, members: &[(&str, &[u8])]) -> Vec<u8> {
        let enc = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
        let mut tar = tar::Builder::new(enc);
        let mut add = |path: String, body: &[u8]| {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, path, body).unwrap();
        };
        for (name, body) in members {
            add(format!("{top}/{name}"), body);
        }
        add(format!("{top}/test_wavs/en.wav"), b"not kept");
        tar.into_inner().unwrap().finish().unwrap()
    }

    fn file(name: &str, body: &[u8]) -> PinnedFile {
        PinnedFile {
            name: name.into(),
            size: body.len() as u64,
            sha256: sha(body),
        }
    }

    fn pinned(base: &str) -> Vec<Pinned> {
        vec![
            Pinned {
                url: format!("{base}/model.tar.bz2"),
                top: Some("model-v1".into()),
                files: vec![
                    file("encoder.int8.onnx", b"enc"),
                    file("tokens.txt", b"tok"),
                ],
            },
            Pinned {
                url: format!("{base}/silero_vad.onnx"),
                top: None,
                files: vec![file("silero_vad.onnx", b"vad")],
            },
        ]
    }

    fn plenty(_: &Path) -> Option<u64> {
        Some(u64::MAX)
    }

    fn tiny(_: &Path) -> Option<u64> {
        Some(10)
    }

    async fn wait_done(models: &SttModels) -> DownloadView {
        for _ in 0..200 {
            if let Some(v) = models.current()
                && v.state != JobState::Running
            {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the download never ended")
    }

    fn layout(dir: &Path) -> (PathBuf, PathBuf) {
        (dir.join("model"), dir.join("silero_vad.onnx"))
    }

    #[tokio::test]
    async fn download_keeps_only_listed_members() {
        let body = archive(
            "model-v1",
            &[("encoder.int8.onnx", b"enc"), ("tokens.txt", b"tok")],
        );
        let (base, _) = serve(vec![
            ("/model.tar.bz2", body),
            ("/silero_vad.onnx", b"vad".to_vec()),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let models = SttModels::with_free(plenty);
        let done = Arc::new(AtomicUsize::new(0));
        let flag = done.clone();
        let (model_dir, vad) = layout(dir.path());
        models
            .start(
                dir.path(),
                model_dir.clone(),
                vad.clone(),
                pinned(&base),
                move || {
                    flag.fetch_add(1, Ordering::SeqCst);
                },
            )
            .unwrap();
        let view = wait_done(&models).await;
        assert_eq!(view.state, JobState::Done, "{view:?}");
        assert_eq!(
            std::fs::read(model_dir.join("encoder.int8.onnx")).unwrap(),
            b"enc"
        );
        assert_eq!(std::fs::read(model_dir.join("tokens.txt")).unwrap(), b"tok");
        assert!(!model_dir.join("test_wavs").exists());
        assert_eq!(std::fs::read(&vad).unwrap(), b"vad");
        assert_eq!(done.load(Ordering::SeqCst), 1);
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(left.len(), 2, "no part file or staging left: {left:?}");
    }

    #[tokio::test]
    async fn wrong_hash_fails_and_leaves_nothing() {
        let body = archive(
            "model-v1",
            &[("encoder.int8.onnx", b"ENC"), ("tokens.txt", b"tok")],
        );
        let (base, _) = serve(vec![
            ("/model.tar.bz2", body),
            ("/silero_vad.onnx", b"vad".to_vec()),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let models = SttModels::with_free(plenty);
        let (model_dir, vad) = layout(dir.path());
        models
            .start(dir.path(), model_dir.clone(), vad, pinned(&base), || {})
            .unwrap();
        let view = wait_done(&models).await;
        assert_eq!(view.state, JobState::Failed);
        assert!(view.error.unwrap().contains("encoder.int8.onnx"));
        assert!(!model_dir.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn not_enough_space_is_refused_before_any_byte() {
        let (base, hits) = serve(vec![]).await;
        let dir = tempfile::tempdir().unwrap();
        let models = SttModels::with_free(tiny);
        let (model_dir, vad) = layout(dir.path());
        models
            .start(dir.path(), model_dir, vad, pinned(&base), || {})
            .unwrap();
        let view = wait_done(&models).await;
        assert_eq!(view.state, JobState::Failed);
        assert!(view.error.unwrap().contains("not enough disk space"));
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cancel_removes_the_part_file() {
        // A server that sends the headers and a few bytes, then nothing.
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await;
            let _ = sock
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 1000000\r\n\r\nBZh9")
                .await;
            tokio::time::sleep(Duration::from_secs(30)).await;
        });
        let dir = tempfile::tempdir().unwrap();
        let models = SttModels::with_free(plenty);
        let (model_dir, vad) = layout(dir.path());
        models
            .start(dir.path(), model_dir, vad, pinned(&base), || {})
            .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            matches!(
                models.start(
                    dir.path(),
                    dir.path().join("m"),
                    dir.path().join("v"),
                    Vec::new(),
                    || {}
                ),
                Err(SttError::Conflict(_))
            ),
            "second download while running"
        );
        models.cancel().unwrap();
        let view = wait_done(&models).await;
        assert_eq!(view.state, JobState::Cancelled);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        assert!(matches!(models.cancel(), Err(SttError::NotFound(_))));
    }

    #[test]
    fn the_pinned_models_are_the_spec_ones() {
        let all = pinned_models();
        assert_eq!(all.len(), 2);
        assert_eq!(
            all[0].url,
            "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2"
        );
        let names: Vec<&str> = all[0].files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "encoder.int8.onnx",
                "decoder.int8.onnx",
                "joiner.int8.onnx",
                "tokens.txt"
            ]
        );
        assert_eq!(
            all[1].files[0].sha256,
            "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6"
        );
    }
}
