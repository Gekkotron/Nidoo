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
                    None => {
                        b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                            .to_vec()
                    }
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
