//! The model download job against a fake Hugging Face server.

use crate::common;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::fake_hf::{FakeHfLog, FakeRepo, serve};
use nidoo::assist::tts::TtsError;
use nidoo::assist::tts::download::Downloads;
use nidoo::assist::tts::hf::HfClient;
use nidoo::assist::tts::store::{Manifest, ModelStore};
use nidoo_api::{DownloadRequest, DownloadView, JobState};

const REPO: &str = "Gradium/Phonon";

fn repo() -> FakeRepo {
    FakeRepo::new(
        REPO,
        "c0ffee",
        vec![
            ("model.q8.gguf", b"0123456789abcdef"),
            ("model.q4k.gguf", b"q4k"),
            ("model.safetensors", b"safe"),
            ("config.json", b"{\"languages\":[\"fr\"]}"),
            ("tokenizer.json", b"{}"),
            ("README.md", b"ignored"),
        ],
    )
}

fn req(weights: &str) -> DownloadRequest {
    DownloadRequest {
        repo: REPO.into(),
        revision: "c0ffee".into(),
        weights: weights.into(),
    }
}

fn plenty(_: &Path) -> Option<u64> {
    Some(u64::MAX / 2)
}

fn none_free(_: &Path) -> Option<u64> {
    Some(0)
}

struct Rig {
    _dir: tempfile::TempDir,
    store: Arc<ModelStore>,
    base: String,
    log: FakeHfLog,
}

async fn rig(repo: FakeRepo) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(ModelStore::open(dir.path()));
    let (base, log) = serve(vec![repo]).await;
    Rig {
        _dir: dir,
        store,
        base,
        log,
    }
}

fn dl() -> Downloads {
    Downloads::with_limits(plenty, Duration::from_secs(5))
}

async fn finished(d: &Downloads) -> DownloadView {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let v = d.current().unwrap();
        if v.state != JobState::Running {
            return v;
        }
        assert!(Instant::now() < end, "job never ended");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn staging_left(r: &Rig) -> bool {
    std::fs::read_dir(r._dir.path().join("models"))
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with(".staging-"))
}

fn file_requests(r: &Rig) -> usize {
    r.log
        .requests()
        .iter()
        .filter(|l| l.path.contains("/resolve/"))
        .count()
}

#[tokio::test]
async fn downloads_small_files_and_the_chosen_weights_only() {
    let r = rig(repo()).await;
    let d = dl();
    let got: Arc<Mutex<Option<Manifest>>> = Arc::default();
    let sink = got.clone();
    let v = d
        .start(
            r.store.clone(),
            HfClient::new(&r.base, None),
            req("model.q8.gguf"),
            move |m| *sink.lock().unwrap() = Some(m),
        )
        .unwrap();
    assert_eq!(v.model, "gradium--phonon--q8");
    let end = finished(&d).await;
    assert_eq!(end.state, JobState::Done, "{:?}", end.error);
    assert_eq!(end.done, end.total);
    assert!(end.total > 0);
    let dir = r.store.dir("gradium--phonon--q8");
    for n in [
        "model.q8.gguf",
        "config.json",
        "tokenizer.json",
        "manifest.json",
    ] {
        assert!(dir.join(n).is_file(), "{n}");
    }
    for n in ["model.q4k.gguf", "model.safetensors", "README.md"] {
        assert!(!dir.join(n).exists(), "{n}");
    }
    let m = got.lock().unwrap().clone().unwrap();
    assert_eq!(m.revision.as_deref(), Some("c0ffee"));
    assert_eq!(m.languages, vec!["fr"]);
    assert_eq!(r.store.list().len(), 1);
}

#[tokio::test]
async fn progress_is_visible_while_running() {
    let mut fake = repo();
    fake.slow = Some(("model.q8.gguf".into(), 4, Duration::from_millis(40)));
    let r = rig(fake).await;
    let d = dl();
    d.start(
        r.store.clone(),
        HfClient::new(&r.base, None),
        req("model.q8.gguf"),
        |_| {},
    )
    .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let v = d.current().unwrap();
        if v.state == JobState::Running
            && v.file == "model.q8.gguf"
            && v.done > 0
            && v.done < v.total
        {
            break;
        }
        assert!(Instant::now() < end, "no progress seen: {v:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(finished(&d).await.state, JobState::Done);
}

#[tokio::test]
async fn second_start_while_running_is_a_conflict() {
    let mut fake = repo();
    fake.slow = Some(("model.q8.gguf".into(), 4, Duration::from_millis(60)));
    let r = rig(fake).await;
    let d = dl();
    let client = HfClient::new(&r.base, None);
    d.start(
        r.store.clone(),
        client.clone(),
        req("model.q8.gguf"),
        |_| {},
    )
    .unwrap();
    let again = d.start(r.store.clone(), client, req("model.q4k.gguf"), |_| {});
    assert!(matches!(again, Err(TtsError::Conflict(_))));
    d.cancel().unwrap();
    assert_eq!(finished(&d).await.state, JobState::Cancelled);
}

#[tokio::test]
async fn cancel_stops_and_cleans_up() {
    let mut fake = repo();
    fake.slow = Some(("model.q8.gguf".into(), 2, Duration::from_millis(200)));
    let r = rig(fake).await;
    let d = dl();
    assert!(matches!(d.cancel(), Err(TtsError::NotFound(_))));
    d.start(
        r.store.clone(),
        HfClient::new(&r.base, None),
        req("model.q8.gguf"),
        |_| {},
    )
    .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while d.current().unwrap().file != "model.q8.gguf" {
        assert!(Instant::now() < end);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    d.cancel().unwrap();
    assert_eq!(finished(&d).await.state, JobState::Cancelled);
    assert!(!staging_left(&r));
    assert!(r.store.list().is_empty());
}

#[tokio::test]
async fn a_stalled_server_fails_the_job() {
    let mut fake = repo();
    fake.slow = Some(("model.q8.gguf".into(), 2, Duration::from_millis(500)));
    let r = rig(fake).await;
    let d = Downloads::with_limits(plenty, Duration::from_millis(60));
    d.start(
        r.store.clone(),
        HfClient::new(&r.base, None),
        req("model.q8.gguf"),
        |_| {},
    )
    .unwrap();
    let end = finished(&d).await;
    assert_eq!(end.state, JobState::Failed);
    assert_eq!(
        end.error.as_deref(),
        Some("Hugging Face failed: Hugging Face stopped sending")
    );
    assert!(!staging_left(&r));
}

#[tokio::test]
async fn cancel_works_while_waiting_for_a_chunk() {
    let mut fake = repo();
    fake.slow = Some(("model.q8.gguf".into(), 2, Duration::from_secs(30)));
    let r = rig(fake).await;
    let d = dl();
    d.start(
        r.store.clone(),
        HfClient::new(&r.base, None),
        req("model.q8.gguf"),
        |_| {},
    )
    .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while d.current().unwrap().file != "model.q8.gguf" {
        assert!(Instant::now() < end);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    d.cancel().unwrap();
    assert_eq!(finished(&d).await.state, JobState::Cancelled);
    assert!(!staging_left(&r));
}

#[tokio::test]
async fn sha_mismatch_fails_and_cleans_up() {
    let mut fake = repo();
    fake.oids = vec![("model.q8.gguf".into(), "00".repeat(32))];
    let r = rig(fake).await;
    let d = dl();
    d.start(
        r.store.clone(),
        HfClient::new(&r.base, None),
        req("model.q8.gguf"),
        |_| {},
    )
    .unwrap();
    let end = finished(&d).await;
    assert_eq!(end.state, JobState::Failed);
    assert!(end.error.unwrap().contains("model.q8.gguf"));
    assert!(!staging_left(&r));
    assert!(r.store.list().is_empty());
}

#[tokio::test]
async fn forbidden_becomes_the_terms_message() {
    let mut fake = repo();
    fake.gated = true;
    let r = rig(fake).await;
    let d = dl();
    d.start(
        r.store.clone(),
        HfClient::new(&r.base, Some("wrong".into())),
        req("model.q8.gguf"),
        |_| {},
    )
    .unwrap();
    let end = finished(&d).await;
    assert_eq!(end.state, JobState::Failed);
    assert_eq!(end.error, Some(TtsError::Denied.to_string()));
    assert!(!staging_left(&r));
}

#[tokio::test]
async fn unknown_weights_name_is_invalid() {
    let r = rig(repo()).await;
    let res = dl().start(
        r.store.clone(),
        HfClient::new(&r.base, None),
        req("evil.bin"),
        |_| {},
    );
    assert!(matches!(res, Err(TtsError::Invalid(_))));
    assert!(r.log.requests().is_empty());
}

#[tokio::test]
async fn an_installed_model_is_a_conflict_before_any_request() {
    let r = rig(repo()).await;
    let d = dl();
    d.start(
        r.store.clone(),
        HfClient::new(&r.base, None),
        req("model.q8.gguf"),
        |_| {},
    )
    .unwrap();
    assert_eq!(finished(&d).await.state, JobState::Done);
    let before = r.log.requests().len();
    let again = d.start(
        r.store.clone(),
        HfClient::new(&r.base, None),
        req("model.q8.gguf"),
        |_| {},
    );
    assert!(matches!(again, Err(TtsError::Conflict(_))));
    assert_eq!(r.log.requests().len(), before);
}

#[tokio::test]
async fn not_enough_disk_is_refused() {
    let r = rig(repo()).await;
    let d = Downloads::with_limits(none_free, Duration::from_secs(5));
    d.start(
        r.store.clone(),
        HfClient::new(&r.base, None),
        req("model.q8.gguf"),
        |_| {},
    )
    .unwrap();
    let end = finished(&d).await;
    assert_eq!(end.state, JobState::Failed);
    assert!(end.error.unwrap().starts_with("not enough disk space"));
    assert_eq!(file_requests(&r), 0);
    assert!(!staging_left(&r));
}

#[test]
fn free_bytes_reads_the_disk_of_a_real_path() {
    let t = tempfile::tempdir().unwrap();
    assert!(nidoo::assist::tts::download::free_bytes(t.path()).is_some());
}
