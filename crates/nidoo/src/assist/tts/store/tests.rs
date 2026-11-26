use super::*;
use nidoo_api::{ModelSource, Secret};
use std::fs;

fn manifest(id: &str, weights: &str, size: u64) -> Manifest {
    Manifest {
        id: id.to_owned(),
        repo: None,
        revision: None,
        weights: weights.to_owned(),
        files: vec![ManifestFile {
            name: weights.to_owned(),
            size,
            sha256: None,
        }],
        licence: None,
        languages: vec![],
        source: ModelSource::Upload,
        installed_at: 1,
    }
}

fn write_valid_model(root: &Path, id: &str) {
    let dir = root.join("models").join(id);
    fs::create_dir_all(&dir).unwrap();
    let m = manifest(id, "model.q8.gguf", 3);
    fs::write(dir.join("manifest.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    fs::write(dir.join("model.q8.gguf"), b"abc").unwrap();
}

fn fill(dir: &Path, files: &[(&str, &[u8])]) {
    for (n, b) in files {
        fs::write(dir.join(n), b).unwrap();
    }
}

#[test]
fn boot_deletes_staging_and_folders_without_a_manifest() {
    let t = tempfile::tempdir().unwrap();
    let models = t.path().join("models");
    fs::create_dir_all(models.join(".staging-x")).unwrap();
    fs::write(models.join(".staging-x/a"), b"x").unwrap();
    fs::create_dir_all(models.join("broken")).unwrap();
    fs::write(models.join("broken/model.q8.gguf"), b"x").unwrap();
    write_valid_model(t.path(), "good");
    let store = ModelStore::open(t.path());
    let ids: Vec<_> = store.list().into_iter().map(|m| m.id).collect();
    assert_eq!(ids, vec!["good"]);
    assert!(!models.join(".staging-x").exists());
    assert!(!models.join("broken").exists());
}

#[test]
fn finish_refuses_a_folder_missing_a_required_file() {
    let t = tempfile::tempdir().unwrap();
    let store = ModelStore::open(t.path());
    let st = store.staging().unwrap();
    let dir = st.dir.clone();
    fill(&dir, &[("model.q8.gguf", b"abc")]);
    let r = st.finish(&store, manifest("m", "model.q8.gguf", 3));
    assert!(matches!(r, Err(TtsError::Invalid(_))));
    assert!(!dir.exists());
    assert!(store.get("m").is_none());
}

#[test]
fn finish_refuses_two_weight_files() {
    let t = tempfile::tempdir().unwrap();
    let store = ModelStore::open(t.path());
    let st = store.staging().unwrap();
    fill(
        &st.dir,
        &[
            ("model.q8.gguf", b"abc"),
            ("model.safetensors", b"abc"),
            ("config.json", b"{}"),
            ("tokenizer.json", b"{}"),
        ],
    );
    let mut m = manifest("m", "model.q8.gguf", 3);
    m.files.push(ManifestFile {
        name: "model.safetensors".into(),
        size: 3,
        sha256: None,
    });
    assert!(matches!(st.finish(&store, m), Err(TtsError::Invalid(_))));
}

#[test]
fn finish_refuses_a_size_mismatch() {
    let t = tempfile::tempdir().unwrap();
    let store = ModelStore::open(t.path());
    let st = store.staging().unwrap();
    fill(
        &st.dir,
        &[
            ("model.q8.gguf", b"123456789"),
            ("config.json", b"{}"),
            ("tokenizer.json", b"{}"),
        ],
    );
    let mut m = manifest("m", "model.q8.gguf", 10);
    for n in ["config.json", "tokenizer.json"] {
        m.files.push(ManifestFile {
            name: n.into(),
            size: 2,
            sha256: None,
        });
    }
    assert!(matches!(st.finish(&store, m), Err(TtsError::Invalid(_))));
}

#[test]
fn finish_reads_languages_from_config() {
    let t = tempfile::tempdir().unwrap();
    let store = ModelStore::open(t.path());
    let st = store.staging().unwrap();
    let cfg = br#"{"languages":["fr"]}"#;
    fill(
        &st.dir,
        &[
            ("model.q8.gguf", b"abc"),
            ("config.json", cfg),
            ("tokenizer.json", b"{}"),
        ],
    );
    let mut m = manifest("m", "model.q8.gguf", 3);
    m.files.push(ManifestFile {
        name: "config.json".into(),
        size: cfg.len() as u64,
        sha256: None,
    });
    m.files.push(ManifestFile {
        name: "tokenizer.json".into(),
        size: 2,
        sha256: None,
    });
    let done = st.finish(&store, m).unwrap();
    assert_eq!(done.languages, vec!["fr"]);
    assert!(done.view(vec![]).used_by.is_empty());
    assert_eq!(store.get("m").unwrap().languages, vec!["fr"]);
    assert!(store.dir("m").join("manifest.json").exists());
}

#[test]
fn dropped_staging_cleans_up() {
    let t = tempfile::tempdir().unwrap();
    let store = ModelStore::open(t.path());
    let st = store.staging().unwrap();
    let dir = st.dir.clone();
    assert!(dir.exists());
    drop(st);
    assert!(!dir.exists());
}

#[test]
fn remove_deletes_the_folder() {
    let t = tempfile::tempdir().unwrap();
    write_valid_model(t.path(), "good");
    let store = ModelStore::open(t.path());
    store.remove("good").unwrap();
    assert!(!store.dir("good").exists());
    assert!(store.get("good").is_none());
}

#[test]
fn remove_unknown_is_not_found() {
    let t = tempfile::tempdir().unwrap();
    let store = ModelStore::open(t.path());
    assert!(matches!(store.remove("nope"), Err(TtsError::NotFound(_))));
    assert!(matches!(store.remove("../x"), Err(TtsError::NotFound(_))));
}

#[test]
fn free_id_suffixes_taken_ids() {
    let t = tempfile::tempdir().unwrap();
    write_valid_model(t.path(), "a");
    let store = ModelStore::open(t.path());
    assert_eq!(store.free_id("b"), "b");
    assert_eq!(store.free_id("a"), "a-2");
    write_valid_model(t.path(), "a-2");
    assert_eq!(store.free_id("a"), "a-3");
}

#[test]
fn state_round_trips_and_defaults() {
    let t = tempfile::tempdir().unwrap();
    let store = ModelStore::open(t.path());
    let d = store.read_state();
    assert_eq!(d.active, None);
    assert_eq!(d.voice, "default");
    let s = TtsState {
        languages: vec![super::super::languages::LanguageEntry {
            code: "fr".into(),
            model: Some("m".into()),
            voice: "v".into(),
        }],
        ..TtsState::default()
    };
    store.write_state(&s).unwrap();
    assert_eq!(store.read_state(), s);
    // The single-model fields of older hubs are read, never written back.
    std::fs::write(t.path().join("state.json"), r#"{"active":"m","voice":"v"}"#).unwrap();
    let old = store.read_state();
    assert_eq!(old.active.as_deref(), Some("m"));
    store.write_state(&old).unwrap();
    let saved = std::fs::read_to_string(t.path().join("state.json")).unwrap();
    assert!(!saved.contains("active"), "{saved}");
}

#[test]
fn token_is_trimmed_private_and_clearable() {
    let t = tempfile::tempdir().unwrap();
    let store = ModelStore::open(t.path());
    assert_eq!(store.token(), None);
    store.set_token(Secret::Set("  hf_x \n".into())).unwrap();
    assert_eq!(store.token().as_deref(), Some("hf_x"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(t.path().join("token"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    store.set_token(Secret::Keep).unwrap();
    assert_eq!(store.token().as_deref(), Some("hf_x"));
    store.set_token(Secret::Set("   ".into())).unwrap();
    assert_eq!(store.token(), None);
    store.set_token(Secret::Set("a".into())).unwrap();
    store.set_token(Secret::Clear).unwrap();
    assert_eq!(store.token(), None);
}
