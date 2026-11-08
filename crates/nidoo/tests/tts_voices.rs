//! The voice catalogue against a fake Hugging Face server.

use crate::common;

use common::fake_hf::{FakeRepo, serve};
use nidoo::assist::tts::TtsError;
use nidoo::assist::tts::hf::HfClient;
use nidoo::assist::tts::voices::{
    CatalogVoice, VOICE_CATALOG_REPO, VOICE_CATALOG_REV, VoiceCatalog,
};

const JSONL: &str = concat!(
    "{\"filename\":\"voices/AAA.safetensors\",\"name\":\"Noémie\",\"voice_id\":\"AAA\",\"language\":\"fr\",\"locale\":\"fr-FR\",\"gender\":\"f\",\"age\":\"adult\",\"description\":\"calm\"}\n",
    "not json at all\n",
    "{\"filename\":\"voices/BBB.safetensors\",\"name\":\"Bob\",\"voice_id\":\"BBB\",\"language\":\"en\"}\n",
);

fn catalog_repo() -> FakeRepo {
    FakeRepo::new(
        VOICE_CATALOG_REPO,
        VOICE_CATALOG_REV,
        vec![
            ("voices.jsonl", JSONL.as_bytes()),
            ("voices/AAA.safetensors", b"voicebytes"),
        ],
    )
}

fn voice(filename: &str) -> CatalogVoice {
    CatalogVoice {
        name: "X".into(),
        filename: filename.into(),
        voice_id: None,
        language: None,
        gender: None,
        age: None,
        description: None,
    }
}

#[tokio::test]
async fn catalog_is_fetched_once_then_cached() {
    let (base, log) = serve(vec![catalog_repo()]).await;
    let dir = tempfile::tempdir().unwrap();
    let cat = VoiceCatalog::new(dir.path().to_path_buf());
    let client = HfClient::new(&base, None);
    let (first, offline) = cat.list(&client).await;
    assert!(!offline);
    assert_eq!(first.len(), 2, "the bad line is skipped");
    assert_eq!(first[0].name, "Noémie");
    let (second, _) = cat.list(&client).await;
    assert_eq!(second.len(), 2);
    assert_eq!(log.requests().len(), 1);
    assert!(dir.path().join("voices.jsonl").is_file());
}

#[tokio::test]
async fn catalog_fetch_never_sends_the_token() {
    let (base, log) = serve(vec![catalog_repo()]).await;
    let dir = tempfile::tempdir().unwrap();
    let cat = VoiceCatalog::new(dir.path().to_path_buf());
    let client = HfClient::new(&base, Some("hf_secret".into()));
    cat.list(&client).await;
    let v = cat.list(&client).await.0.into_iter().next().unwrap();
    cat.fetch(&client, &v).await.unwrap();
    let reqs = log.requests();
    assert!(reqs.len() >= 2);
    assert!(reqs.iter().all(|r| r.authorization.is_none()));
}

#[tokio::test]
async fn offline_without_cache_is_empty_and_flagged() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let dir = tempfile::tempdir().unwrap();
    let cat = VoiceCatalog::new(dir.path().to_path_buf());
    let (list, offline) = cat.list(&HfClient::new(&base, None)).await;
    assert!(list.is_empty());
    assert!(offline);
}

#[tokio::test]
async fn fetch_refuses_paths_outside_voices() {
    let (base, log) = serve(vec![catalog_repo()]).await;
    let dir = tempfile::tempdir().unwrap();
    let cat = VoiceCatalog::new(dir.path().to_path_buf());
    let client = HfClient::new(&base, None);
    for bad in [
        "../token",
        "voices/../token",
        "voices/a/b.safetensors",
        "voices/a.bin",
        "model.safetensors",
        "voices/.safetensors",
        "/voices/a.safetensors",
    ] {
        let err = cat.fetch(&client, &voice(bad)).await.unwrap_err();
        assert!(matches!(err, TtsError::Invalid(_)), "{bad}");
    }
    assert!(log.requests().is_empty());
}

#[tokio::test]
async fn fetch_reuses_a_voice_on_disk() {
    let (base, log) = serve(vec![catalog_repo()]).await;
    let dir = tempfile::tempdir().unwrap();
    let cat = VoiceCatalog::new(dir.path().to_path_buf());
    let client = HfClient::new(&base, None);
    let v = voice("voices/AAA.safetensors");
    let path = cat.fetch(&client, &v).await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"voicebytes");
    assert!(!dir.path().join("AAA.safetensors.part").exists());
    let again = cat.fetch(&client, &v).await.unwrap();
    assert_eq!(again, path);
    assert_eq!(log.requests().len(), 1);
}

#[tokio::test]
async fn fetch_refuses_an_oversized_voice() {
    let big = vec![0u8; 2 * 1024 * 1024 + 1];
    let repo = FakeRepo::new(
        VOICE_CATALOG_REPO,
        VOICE_CATALOG_REV,
        vec![("voices/BIG.safetensors", &big)],
    );
    let (base, _) = serve(vec![repo]).await;
    let dir = tempfile::tempdir().unwrap();
    let cat = VoiceCatalog::new(dir.path().to_path_buf());
    let err = cat
        .fetch(
            &HfClient::new(&base, None),
            &voice("voices/BIG.safetensors"),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, TtsError::TooLarge(_)));
    assert!(!dir.path().join("BIG.safetensors").exists());
    assert!(!dir.path().join("BIG.safetensors.part").exists());
}
