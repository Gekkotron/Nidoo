//! The Hugging Face client against a fake server.

mod common;

use common::fake_hf::{FakeRepo, TOKEN, serve};
use nidoo::assist::tts::TtsError;
use nidoo::assist::tts::hf::HfClient;
use nidoo::assist::tts::source::parse_source;

fn phonon(extra: Vec<(&'static str, &'static [u8])>) -> FakeRepo {
    let mut files: Vec<(&str, &[u8])> = vec![
        ("model.safetensors", b"0123456789"),
        ("model.q8.gguf", b"012345"),
        ("config.json", b"{}"),
        ("tokenizer.json", b"{}{}"),
        ("README.md", b"ignored"),
    ];
    files.extend(extra);
    let mut repo = FakeRepo::new("gradium/phonon", "c0ffee", files);
    repo.gated = true;
    repo.licence = Some("cc-by-4.0".into());
    repo
}

#[tokio::test]
async fn inspect_lists_runnable_weights_and_pins_the_commit() {
    let (base, _log) = serve(vec![phonon(vec![])]).await;
    let l = HfClient::new(&base, None)
        .inspect(&parse_source("gradium/phonon").unwrap())
        .await
        .unwrap();
    assert_eq!(l.revision, "c0ffee");
    let names: Vec<_> = l
        .weights
        .iter()
        .map(|w| (w.name.as_str(), w.size))
        .collect();
    assert_eq!(names, [("model.q8.gguf", 6), ("model.safetensors", 10)]);
    assert!(l.gated);
    assert_eq!(l.licence.as_deref(), Some("cc-by-4.0"));
    assert_eq!(l.small_bytes, 6);
}

#[tokio::test]
async fn inspect_without_config_is_not_a_phonon_model() {
    let repo = FakeRepo::new(
        "a/b",
        "s",
        vec![("model.q8.gguf", b"1"), ("tokenizer.json", b"{}")],
    );
    let (base, _log) = serve(vec![repo]).await;
    let err = HfClient::new(&base, None)
        .inspect(&parse_source("a/b").unwrap())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, TtsError::Invalid(m) if m.contains("not a Phonon model")),
        "{err}"
    );
}

#[tokio::test]
async fn inspect_of_a_missing_repo_is_not_found() {
    let (base, _log) = serve(vec![]).await;
    let err = HfClient::new(&base, None)
        .inspect(&parse_source("a/b").unwrap())
        .await
        .unwrap_err();
    assert!(matches!(err, TtsError::NotFound(_)), "{err}");
}

#[tokio::test]
async fn inspect_sends_the_token_only_in_the_header() {
    let (base, log) = serve(vec![phonon(vec![])]).await;
    let client = HfClient::new(&base, Some(TOKEN.into()));
    client
        .inspect(&parse_source("gradium/phonon").unwrap())
        .await
        .unwrap();
    client
        .get("gradium/phonon", "c0ffee", "config.json")
        .await
        .unwrap();
    let seen = log.requests();
    assert!(seen.len() >= 3);
    for r in &seen {
        assert_eq!(
            r.authorization.as_deref(),
            Some("Bearer hf_test"),
            "{}",
            r.path
        );
        assert!(!r.path.contains(TOKEN), "{}", r.path);
    }
}

#[tokio::test]
async fn a_gated_file_without_the_token_is_denied() {
    let (base, _log) = serve(vec![phonon(vec![])]).await;
    let err = HfClient::new(&base, None)
        .get("gradium/phonon", "c0ffee", "config.json")
        .await
        .unwrap_err();
    assert!(matches!(err, TtsError::Denied), "{err}");
}

#[tokio::test]
async fn a_branch_address_is_pinned_to_its_commit() {
    let (base, _log) = serve(vec![phonon(vec![])]).await;
    let src = parse_source("https://huggingface.co/gradium/phonon/tree/main").unwrap();
    let l = HfClient::new(&base, None).inspect(&src).await.unwrap();
    assert_eq!(l.revision, "c0ffee");
    let src = parse_source("hf://gradium/phonon@nope").unwrap();
    let err = HfClient::new(&base, None).inspect(&src).await.unwrap_err();
    assert!(matches!(err, TtsError::NotFound(_)), "{err}");
}
