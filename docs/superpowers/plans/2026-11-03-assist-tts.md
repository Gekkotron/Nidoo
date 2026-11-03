# Assist TTS (Phonon in the hub) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The hub speaks French with a locally installed Phonon model, managed from an Assist settings screen, and serves the speech as a streamed WAV address any client can fetch.

**Architecture:** A new hub module `crates/nidoo/src/assist/tts/` owns everything: Hugging Face address parsing and repo listing, a model store on disk under `data/assist/tts/`, one background download job, upload sessions, the voice catalog, an engine thread that owns the one loaded model behind a `Speaker` trait (Phonon through the `ptts` crate behind cargo feature `tts`; a tone generator in tests), and in-memory speech clips streamed as WAV. A `Tts` facade is handed to the `Hub` like the history recorder (`set_tts`), and `crates/nidoo/src/http/tts.rs` exposes it under `/api/v1/assist/…`. Shared request/response types live in `nidoo-api`. The web gets `/assistant/settings`, reached by ⚙ in the Assistant page header.

**Tech Stack:** Rust 2024 workspace, axum 0.8 + tokio (hub), reqwest 0.13 with `stream`, sha2, sysinfo (free disk), `ptts` 0.6.0 (Phonon), Leptos wasm web (`nidoo-web`), gloo-net.

**Spec:** `docs/superpowers/specs/2026-11-03-assist-tts-design.md`

## Global Constraints

- `ptts` pinned exactly: `ptts = { version = "=0.6.0", default-features = false, features = ["hf"], optional = true }`; **never** the `hub` feature (it reads `HF_TOKEN` and downloads by itself).
- Cargo feature `tts` on the `nidoo` crate, **on by default**; without it the hub builds and answers "this hub was built without speech".
- `ptts` is a dependency of `crates/nidoo` only. `nidoo-api` and `nidoo-web` must keep building for `wasm32-unknown-unknown`.
- Never commit, bundle or embed model weights or voice files; tests use a fake Hugging Face server and a fake synthesiser. CI never downloads a model.
- The token is sent only to the configured Hugging Face base, only in `Authorization: Bearer …`, never in a URL, a log line, an error, or a response body (reads give `{ "set": bool }`).
- Only `huggingface.co` sources are accepted (the test base URL is injected through `TtsConfig::hf_base`, never parsed from user input).
- Speech is French for every request (`Lang::Fr`).
- Audio out: WAV, **24 kHz** (the model's `sample_rate()`), **16-bit, mono**, streamed as it is generated.
- Speech failure never stops the hub or text Assist: `Tts::start` is infallible; errors become `EngineState::Error`.
- Generic: no house, room or person name in code, tests or docs. The docs' prose must pass `tools/ci/brand-check.sh` (no Google smart-home brand word).
- CI must pass: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo check -p nidoo-api -p nidoo-web --target wasm32-unknown-unknown`, `cargo test --workspace`, web `bare_text` guard.
- Build/test with `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`; check `df -h ~` first and run per-crate tests when under 5 GB free.
- Commits as Gekkotron: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`, message ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Stage explicit paths only.

### Pinned values (used by several tasks)

| Name | Value | Where |
|---|---|---|
| `HF_BASE` | `"https://huggingface.co"` | `tts/mod.rs` |
| `SMALL_FILES` | `config.json`, `generation_config.json`, `processor_config.json`, `tokenizer.json`, `tokenizer.5.0.0.json`, `tokenizer.model`, `tokenizer_config.json`, `chat_template.jinja`, `default-voice.safetensors` | `tts/store.rs` |
| `WEIGHT_FILES` | `model.q8.gguf`, `model.q4k.gguf`, `model.safetensors` (minus any the Task 1 spike shows `ptts` cannot load on CPU) | `tts/store.rs` |
| `REQUIRED_FILES` | `config.json`, `tokenizer.json` (+ exactly one weight file) | `tts/store.rs` |
| `MAX_WEIGHTS_BYTES` | 200 MiB | `tts/store.rs` |
| `MAX_SMALL_FILE_BYTES` | 8 MiB | `tts/store.rs` |
| `DISK_MARGIN_BYTES` | 256 MiB | `tts/download.rs` |
| `QUEUE_LEN` | 4 | `tts/engine.rs` |
| `MAX_SPEECH_CHARS` | 500 | `tts/speech.rs` |
| `CLIP_TTL_S` | 60 | `tts/speech.rs` |
| `VOICE_CATALOG_REPO` / `VOICE_CATALOG_REV` | `"gradium/phonon-voices"` / `"da6c9ec0d47be8df5c87c730a3aa97187f3229c3"` (the commit `ptts` 0.6.0 pins) | `tts/voices.rs` |
| `MAX_VOICE_BYTES` | 2 MiB | `tts/voices.rs` |
| `DEFAULT_VOICE` | `"default"` | `nidoo-api` |
| Poll interval (web) | 1000 ms while a download runs | web screen |
| Test sentence (web) | `Bonjour, je suis l'assistant de la maison.` | web i18n |

### On-disk layout (root = `<data_dir>/assist/tts`)

```
models/<model-id>/manifest.json + the model's files
models/.staging-<random>/          (download or upload in progress; deleted on boot)
voices/voices.jsonl                (catalog list, cached)
voices/<voice_id>.safetensors      (catalog voices fetched on demand)
state.json                         { "active": Option<String>, "voice": String }
token                              (file mode 0600 on unix)
```

`<model-id>`: downloaded `"{owner}--{repo}--{q8|q4k|f32}"` (lowercase, `/` → `--`); uploaded `"upload--{slug of config.json model_id, else 'model'}--{quant}"`, suffixed `-2`, `-3`… when taken. Quant from the weight file name: `model.q8.gguf` → `q8`, `model.q4k.gguf` → `q4k`, `model.safetensors` → `f32`.

## Review Focus

1. **A model folder that disappears or breaks under a running hub** (removed while active, weights file truncated by a full disk before boot). Expected: status `Error` with a readable message or `NoModel`, text Assist unaffected, the list still renders. Test in Task 4 (boot) and Task 8 (remove active).
2. **The browser fetching the speech URL twice** (an `<audio>` element often re-requests, sometimes with `Range`). Expected: both requests get the whole WAV from the start while the clip lives (60 s); `Range` is ignored with a `200`. Test in Task 9.
3. **Pasted URLs in the shapes people actually copy**: `https://huggingface.co/gradium/phonon-40m-fr/tree/main`, `…/blob/main/model.q8.gguf`, `…/resolve/main/config.json`, trailing slash, `?` query, uppercase host. Expected: all resolve to the repo; `http://evil.example/gradium/x` and `hf.co.evil.example` refused. Test in Task 3.
4. **A second click on Download, or activating a model while it is still loading**. Expected: `409` with a reason for the second download; an activation while loading replaces the pending load (last one wins) and the status ends on the last chosen model. Test in Task 5 and Task 6.
5. **Token mistakes**: a token with whitespace pasted around it, and a token that is valid but has not accepted the gated terms. Expected: whitespace trimmed before storing; the `403` becomes the fixed "accept the terms" message, and the staging folder is gone. Test in Task 4 (trim) and Task 5 (403).

---

### Task 1: The `ptts` dependency, the feature, and the speed spike

**Files:**
- Modify: `crates/nidoo/Cargo.toml` (dependency + `[features]`)
- Create: `crates/nidoo/tests/tts_real.rs` (one `#[ignore]` test)
- Create: `docs/superpowers/plans/2026-11-03-assist-tts-spike.md` (numbers)

**Interfaces:**
- Produces: feature `tts` (default); the facts Task 6 needs: the import path of the trait that carries `say_with` / `stream_with` / `voices` / `languages` / `add_voice_file`, how `xn` chooses its CPU thread count, and whether `model.q4k.gguf` loads on CPU.

- [ ] **Step 1: Add the dependency and feature**

```toml
[features]
default = ["tts"]
tts = ["dep:ptts"]

[dependencies]
ptts = { version = "=0.6.0", default-features = false, features = ["hf"], optional = true }
```

- [ ] **Step 2: Build with and without the feature**

Run: `CARGO_PROFILE_DEV_DEBUG=0 cargo build -p nidoo && CARGO_PROFILE_DEV_DEBUG=0 cargo build -p nidoo --no-default-features`
Expected: both succeed. Record the extra clean-build time and the release binary size difference (`cargo build --release -p nidoo` with and without) in the spike notes.

- [ ] **Step 3: Write `tts_real.rs`**

One test, `#[ignore]`, `#[cfg(feature = "tts")]`: reads the model folder from `NIDOO_TTS_MODEL_DIR` (skips with a message when unset), loads it with `Checkpoint::resolve(dir, ResolveOptions { quant: Quant::Q80, weights: Some("model.q8.gguf") })` → `builder(Lang::Fr).device(DeviceKind::Cpu).build()` → `register_voices`, then for each of `"Salon allumé (3 lumières)."` and `"Il fait 17,5 °C dans le salon."` prints load time, time to the first `stream_with` chunk, total synthesis time, audio duration (samples / `sample_rate()`), and real-time factor. Asserts only that audio is non-empty.

- [ ] **Step 4: Run it on this Mac with the public English model**

Download `gradium/phonon-31m-en` (public, Apache-2.0) into the scratchpad with `curl` (files from the Facts list + `model.q8.gguf`), then:
Run: `NIDOO_TTS_MODEL_DIR=<folder> cargo test --release -p nidoo --test tts_real -- --ignored --nocapture`
Expected: numbers printed. Also try `model.q4k.gguf` (`Quant::Q4k`); if it fails to load, drop it from `WEIGHT_FILES`.

- [ ] **Step 5: Ask the user for the mini PC numbers**

The hub host is reachable only by the user. Ask them to run, on the mini PC, the release `ptts` command (GitHub release archive for x86_64 Linux, or `docker run ghcr.io/gradium-ai/ptts-openai-server:0.6.0` if simpler) with `HF_TOKEN` set: `time ptts --model hf://gradium/phonon-40m-fr --quant q8 --lang fr "Il fait 17,5 °C dans le salon." -o /tmp/t.wav`, run twice (second run = warm cache). Record their output.

- [ ] **Step 6: Gate**

Write the spike notes file: Mac numbers, mini PC numbers, trait path, thread-count finding, q4k result, build cost. **Stop and report to the user** if the mini PC misses the spec's target (first audio under 1 s, synthesis faster than real time) — do not start Task 2 without their answer.

- [ ] **Step 7: Commit**

```bash
git add crates/nidoo/Cargo.toml Cargo.lock crates/nidoo/tests/tts_real.rs docs/superpowers/plans/2026-11-03-assist-tts-spike.md
git commit -m "build(assist): Phonon (ptts 0.6.0) behind the tts feature, with the speed spike"
```

---

### Task 2: Shared types in `nidoo-api`

**Files:**
- Create: `crates/nidoo-api/src/tts.rs`
- Modify: `crates/nidoo-api/src/lib.rs` (`mod tts; pub use tts::*;`)

**Interfaces:**
- Produces (all `#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]`, `#[serde(rename_all = "snake_case")]` on enums):
  - `pub const DEFAULT_VOICE: &str = "default";`
  - `EngineState` — `#[serde(tag = "state")]`: `NoModel`, `Loading { model: String }`, `Ready { model: String }`, `Error { model: Option<String>, message: String }`
  - `TtsStatus { engine: EngineState, active: Option<String>, voice: String, token_set: bool, download: Option<DownloadView>, speech_built: bool }`
  - `ModelSource { Download, Upload }`
  - `ModelView { id: String, repo: Option<String>, revision: Option<String>, weights: String, size: u64, licence: Option<String>, languages: Vec<String>, french: bool, source: ModelSource, installed_at: i64, active: bool }`
  - `InspectRequest { url: String }`
  - `WeightFile { name: String, size: u64 }`
  - `RepoListing { repo: String, revision: String, gated: bool, licence: Option<String>, weights: Vec<WeightFile>, small_bytes: u64 }`
  - `DownloadRequest { repo: String, revision: String, weights: String }`
  - `JobState { Running, Done, Failed, Cancelled }`
  - `DownloadView { model: String, state: JobState, done: u64, total: u64, file: String, error: Option<String> }`
  - `ActiveRequest { model: Option<String> }`
  - `VoiceSource { Model, Catalog }`
  - `VoiceView { name: String, language: Option<String>, gender: Option<String>, age: Option<String>, description: Option<String>, source: VoiceSource, on_disk: bool }`
  - `VoicesView { voices: Vec<VoiceView>, chosen: String, fell_back: bool, catalog_offline: bool }`
  - `VoiceRequest { voice: String }`
  - `TokenInput { #[serde(default)] token: Secret }` (`Deserialize` only, like `PushInput`)
  - `UploadSession { id: String }`
  - `SpeechRequest { text: String, #[serde(default)] voice: Option<String> }`
  - `SpeechTicket { id: String, url: String }`

- [ ] **Step 1: Write the failing tests** (in `tts.rs`)

```rust
#[test]
fn engine_state_is_tagged() {
    let v = serde_json::to_value(EngineState::Ready { model: "m".into() }).unwrap();
    assert_eq!(v, serde_json::json!({"state": "ready", "model": "m"}));
    let v = serde_json::to_value(EngineState::NoModel).unwrap();
    assert_eq!(v, serde_json::json!({"state": "no_model"}));
}

#[test]
fn token_input_follows_secret_rules() {
    let keep: TokenInput = serde_json::from_str("{}").unwrap();
    assert_eq!(keep.token, Secret::Keep);
    let clear: TokenInput = serde_json::from_str(r#"{"token": null}"#).unwrap();
    assert_eq!(clear.token, Secret::Clear);
}

#[test]
fn speech_request_voice_is_optional() {
    let r: SpeechRequest = serde_json::from_str(r#"{"text": "Bonjour"}"#).unwrap();
    assert_eq!(r.voice, None);
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p nidoo-api tts::`
Expected: FAIL (module missing).

- [ ] **Step 3: Write the types** listed under Interfaces, each field with a one-line doc comment.

- [ ] **Step 4: Run the tests and the wasm check**

Run: `cargo test -p nidoo-api tts:: && cargo check -p nidoo-api --target wasm32-unknown-unknown`
Expected: PASS, check clean.

- [ ] **Step 5: Commit**

```bash
git add crates/nidoo-api/src/tts.rs crates/nidoo-api/src/lib.rs
git commit -m "feat(assist): speech types shared by the hub and the web"
```

---

### Task 3: Hugging Face addresses and repo listing

**Files:**
- Create: `crates/nidoo/src/assist/tts/mod.rs` (module skeleton, `TtsError`, `HF_BASE`)
- Create: `crates/nidoo/src/assist/tts/source.rs`
- Create: `crates/nidoo/src/assist/tts/hf.rs`
- Modify: `crates/nidoo/src/assist/mod.rs` (`pub mod tts;`)
- Test: unit tests in `source.rs`; `crates/nidoo/tests/tts_hf.rs` with a fake HF server (axum on `127.0.0.1:0`)
- Create: `crates/nidoo/tests/common/fake_hf.rs` (shared by Tasks 3, 5, 7, 10)

**Interfaces:**
- Produces:
  - `pub enum TtsError { NotFound(String), Invalid(String), Conflict(String), Busy, Upstream(String), Denied, NoSpace { need: u64, free: u64 }, TooLarge(String), Io(String) }` with `thiserror` messages; `Denied` reads exactly `"Hugging Face refused the download: accept the model's terms on its page, or check the token"`; `Busy` reads `"speech is busy, try again in a moment"`.
  - `pub struct HfSource { pub repo: String, pub rev: Option<String> }`
  - `pub fn parse_source(input: &str) -> Result<HfSource, TtsError>` (`Invalid` on refusal)
  - `pub struct HfClient` with `pub fn new(base: &str, token: Option<String>) -> HfClient`, `pub async fn inspect(&self, src: &HfSource) -> Result<RepoListing, TtsError>`, `pub async fn files(&self, repo: &str, rev: &str) -> Result<Vec<RepoFile>, TtsError>`, `pub async fn get(&self, repo: &str, rev: &str, path: &str) -> Result<reqwest::Response, TtsError>`
  - `pub struct RepoFile { pub path: String, pub size: u64, pub sha256: Option<String> }` (`sha256` from `lfs.oid`)
  - test helper `fake_hf::serve(repos: Vec<FakeRepo>) -> (String /*base*/, FakeHfLog)`; `FakeRepo { repo, sha, gated, licence, files: Vec<(path, bytes)> }`; `FakeHfLog` records each request's path and `Authorization` header; gated repos answer `403` to file fetches without the right token `"hf_test"`.

- [ ] **Step 1: Write the failing parse tests**

```rust
#[test]
fn accepts_the_shapes_people_paste() {
    for s in [
        "https://huggingface.co/gradium/phonon-40m-fr",
        "https://huggingface.co/gradium/phonon-40m-fr/",
        "https://HuggingFace.co/gradium/phonon-40m-fr/tree/main",
        "https://huggingface.co/gradium/phonon-40m-fr/blob/main/model.q8.gguf",
        "https://huggingface.co/gradium/phonon-40m-fr/resolve/main/config.json?download=true",
        "hf://gradium/phonon-40m-fr",
        "gradium/phonon-40m-fr",
        "  gradium/phonon-40m-fr  ",
    ] {
        assert_eq!(parse_source(s).unwrap().repo, "gradium/phonon-40m-fr", "{s}");
    }
}

#[test]
fn keeps_a_pinned_revision() {
    assert_eq!(parse_source("hf://gradium/x@abc123").unwrap().rev.as_deref(), Some("abc123"));
    assert_eq!(parse_source("https://huggingface.co/gradium/x/tree/abc123").unwrap().rev.as_deref(), Some("abc123"));
}

#[test]
fn refuses_other_hosts_and_shapes() {
    for s in [
        "http://evil.example/gradium/x",
        "https://huggingface.co.evil.example/gradium/x",
        "https://evil.example/huggingface.co/gradium/x",
        "https://huggingface.co/datasets/gradium/x",
        "gradium",
        "gradium/x/y",
        "../etc/passwd",
        "",
    ] {
        assert!(parse_source(s).is_err(), "{s}");
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p nidoo --lib assist::tts::source`
Expected: FAIL.

- [ ] **Step 3: Implement `parse_source`**

Use `url::Url` for `http(s)` inputs; host compared case-insensitively to exactly `huggingface.co`; owner and repo each match `[A-Za-z0-9][A-Za-z0-9._-]*` (no `..`); a `tree|blob|resolve/<rev>` segment gives `rev`; `datasets/` and `spaces/` refused.

- [ ] **Step 4: Write the failing inspect tests** (`tests/tts_hf.rs`, against `fake_hf`)

- `inspect_lists_runnable_weights_and_pins_the_commit`: repo with the three weight files and small files → `RepoListing.revision == sha`, `weights` names in the `WEIGHT_FILES` order with their sizes, `gated` and `licence` copied.
- `inspect_without_config_is_not_a_phonon_model`: no `config.json` → `TtsError::Invalid` whose message contains `"not a Phonon model"`.
- `inspect_of_a_missing_repo_is_not_found`: `404` → `TtsError::NotFound`.
- `inspect_sends_the_token_only_in_the_header`: with token `"hf_test"`, every logged request has `Authorization: Bearer hf_test` and no path contains `hf_test`.

- [ ] **Step 5: Implement `HfClient`**

`GET {base}/api/models/{repo}` (→ `sha`, `gated` (`false` or a string), `cardData.license`), then `GET {base}/api/models/{repo}/tree/{rev}?recursive=true`. File fetch: `GET {base}/{repo}/resolve/{rev}/{path}`. Map `401/403` → `Denied`, `404` → `NotFound`, other non-2xx and transport errors → `Upstream` with the status only (never the body or headers).

- [ ] **Step 6: Run the tests**

Run: `cargo test -p nidoo --lib assist::tts::source && cargo test -p nidoo --test tts_hf`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/nidoo/src/assist/mod.rs crates/nidoo/src/assist/tts/mod.rs crates/nidoo/src/assist/tts/source.rs crates/nidoo/src/assist/tts/hf.rs crates/nidoo/tests/tts_hf.rs crates/nidoo/tests/common/fake_hf.rs crates/nidoo/tests/common/mod.rs
git commit -m "feat(assist): read Hugging Face addresses and list a Phonon repo"
```

---

### Task 4: The model store on disk, state and token

**Files:**
- Create: `crates/nidoo/src/assist/tts/store.rs`
- Test: unit tests in `store.rs` (with `tempfile`)

**Interfaces:**
- Consumes: `ModelView`, `ModelSource` (Task 2), `TtsError` (Task 3).
- Produces:
  - constants `SMALL_FILES`, `WEIGHT_FILES`, `REQUIRED_FILES`, `MAX_WEIGHTS_BYTES`, `MAX_SMALL_FILE_BYTES` (table above)
  - `pub fn quant_of(weights: &str) -> Option<&'static str>` (`"q8" | "q4k" | "f32"`)
  - `pub struct Manifest { id, repo: Option<String>, revision: Option<String>, weights: String, files: Vec<ManifestFile>, licence: Option<String>, languages: Vec<String>, source: ModelSource, installed_at: i64 }`, `ManifestFile { name: String, size: u64, sha256: Option<String> }`, `impl Manifest { pub fn view(&self, active: bool) -> ModelView }` (`size` = sum of files, `french` = languages contains `"fr"`)
  - `pub struct ModelStore`; `pub fn open(root: &Path) -> ModelStore` (creates dirs, deletes `.staging-*` and folders without a valid `manifest.json`; logs, never fails); `pub fn list(&self) -> Vec<Manifest>` (sorted by id); `pub fn get(&self, id: &str) -> Option<Manifest>`; `pub fn dir(&self, id: &str) -> PathBuf`; `pub fn remove(&self, id: &str) -> Result<(), TtsError>`; `pub fn staging(&self) -> Result<Staging, TtsError>`; `pub fn free_id(&self, base: &str) -> String`
  - `pub struct Staging { pub dir: PathBuf }`; `pub fn finish(self, store: &ModelStore, manifest: Manifest) -> Result<Manifest, TtsError>` (checks every manifest file exists with its size, required files present, exactly one weight file, reads `languages` from `config.json`, writes `manifest.json`, renames into `models/<id>`); `Drop` deletes the folder unless finished
  - `pub struct TtsState { pub active: Option<String>, pub voice: String }` (serde, default voice `DEFAULT_VOICE`); `pub fn read_state(&self) -> TtsState`; `pub fn write_state(&self, s: &TtsState) -> Result<(), TtsError>` (write temp + rename)
  - `pub fn token(&self) -> Option<String>`; `pub fn set_token(&self, input: Secret) -> Result<(), TtsError>` (trims; empty after trim = clear; file mode `0o600` on unix)

- [ ] **Step 1: Write the failing tests**

- `boot_deletes_staging_and_folders_without_a_manifest`: create `models/.staging-x/a`, `models/broken/model.q8.gguf` and a valid model → after `open`, only the valid one is listed and the other two folders are gone.
- `finish_refuses_a_folder_missing_a_required_file`: staging with only `model.q8.gguf` → `Invalid`, staging folder deleted.
- `finish_refuses_two_weight_files`: → `Invalid`.
- `finish_refuses_a_size_mismatch`: manifest says 10 bytes, file has 9 → `Invalid`.
- `finish_reads_languages_from_config`: `config.json` `{"languages":["fr"]}` → `view().french == true`.
- `dropped_staging_cleans_up`: `staging()` then drop → folder gone.
- `remove_deletes_the_folder` and `remove_unknown_is_not_found`.
- `free_id_suffixes_taken_ids`: `"a"` taken → `"a-2"`.
- `state_round_trips_and_defaults`: missing file → `{ active: None, voice: "default" }`.
- `token_is_trimmed_private_and_clearable`: `Set("  hf_x \n")` → `token() == Some("hf_x")`; on unix mode `0o600`; `Set("   ")` and `Clear` → `None`; `Keep` keeps.

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p nidoo --lib assist::tts::store`
Expected: FAIL.

- [ ] **Step 3: Implement the store** as specified under Interfaces.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p nidoo --lib assist::tts::store`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/nidoo/src/assist/tts/store.rs crates/nidoo/src/assist/tts/mod.rs
git commit -m "feat(assist): keep speech models, the chosen one and the token on disk"
```

---

### Task 5: The download job

**Files:**
- Create: `crates/nidoo/src/assist/tts/download.rs`
- Test: `crates/nidoo/tests/tts_download.rs` (fake HF server, temp store)

**Interfaces:**
- Consumes: `HfClient`, `RepoFile` (Task 3); `ModelStore`, `Staging`, `Manifest`, constants (Task 4); `DownloadRequest`, `DownloadView`, `JobState` (Task 2).
- Produces:
  - `pub struct Downloads` (`Default`); `pub fn start(&self, store: Arc<ModelStore>, client: HfClient, req: DownloadRequest, done: impl FnOnce(Manifest) + Send + 'static) -> Result<DownloadView, TtsError>`; `pub fn current(&self) -> Option<DownloadView>`; `pub fn cancel(&self) -> Result<(), TtsError>` (`NotFound` when nothing runs)
  - `pub fn free_bytes(path: &Path) -> Option<u64>` (sysinfo: the disk whose mount point is the longest prefix of `path`)

- [ ] **Step 1: Write the failing tests**

- `downloads_small_files_and_the_chosen_weights_only`: request `model.q8.gguf` → the installed folder has the small files present in the repo plus `model.q8.gguf`, not `model.q4k.gguf` / `model.safetensors`; final view `state == Done`, `done == total`; `done` callback received the manifest with `revision` = the request's.
- `progress_is_visible_while_running`: fake server streams the weight file slowly (chunks with a delay); `current()` shows `Running` with `0 < done < total` and `file == "model.q8.gguf"` at some poll.
- `second_start_while_running_is_a_conflict`: → `TtsError::Conflict`.
- `cancel_stops_and_cleans_up`: cancel mid-file → state `Cancelled`, no `.staging-*` left, nothing listed.
- `sha_mismatch_fails_and_cleans_up`: fake tree announces a wrong `lfs.oid` → `Failed`, error mentions the file name, staging gone.
- `forbidden_becomes_the_terms_message`: gated repo, token `"wrong"` → `Failed` with `error == TtsError::Denied.to_string()`, staging gone.
- `unknown_weights_name_is_invalid`: `weights: "evil.bin"` → `Invalid` before any request.
- `not_enough_disk_is_refused`: inject `free_bytes` through a `Downloads::with_free_bytes(fn(&Path) -> Option<u64>)` constructor used only by tests → `NoSpace { need, free }` before any file request.

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p nidoo --test tts_download`
Expected: FAIL.

- [ ] **Step 3: Implement `Downloads`**

One `tokio::spawn`ed job; shared `Mutex<Option<Job>>` with progress counters and an `Arc<AtomicBool>` cancel flag checked between chunks. Files list = `client.files(repo, rev)` ∩ (`SMALL_FILES` ∪ {chosen weights}); need = their total + `DISK_MARGIN_BYTES`. Each file streamed with `bytes_stream()` into the staging folder while updating a `Sha256` and the counter; size and (when known) sha256 checked at the end of each file; any error ends the job `Failed` with that error's message (the `Staging` drop cleans). On success `Staging::finish` then `done(manifest)`. A finished job stays readable through `current()` until the next `start`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p nidoo --test tts_download`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/nidoo/src/assist/tts/download.rs crates/nidoo/src/assist/tts/mod.rs crates/nidoo/tests/tts_download.rs crates/nidoo/tests/common/fake_hf.rs
git commit -m "feat(assist): download a speech model in the background with progress"
```

---

### Task 6: The engine thread and the Phonon adapter

**Files:**
- Create: `crates/nidoo/src/assist/tts/engine.rs`
- Create: `crates/nidoo/src/assist/tts/phonon.rs` (`#[cfg(feature = "tts")]`)
- Create: `crates/nidoo/src/assist/tts/tone.rs` (fake synthesiser, `pub` so integration tests can use it)
- Test: unit tests in `engine.rs`

**Interfaces:**
- Consumes: `EngineState` (Task 2); spike facts (Task 1).
- Produces:
  - `pub trait Speaker { fn sample_rate(&self) -> u32; fn languages(&self) -> Vec<String>; fn voices(&self) -> Vec<String>; fn add_voice(&mut self, name: &str, path: &Path) -> Result<(), String>; fn speak(&mut self, text: &str, voice: Option<&str>, chunk: &mut dyn FnMut(&[f32]) -> bool) -> Result<(), String>; }` (`chunk` returning `false` stops early)
  - `pub trait SpeakerLoader: Send + Sync { fn load(&self, dir: &Path, weights: &str) -> Result<Box<dyn Speaker>, String>; }` — called on the engine thread, so a `Speaker` need not be `Send`
  - `pub struct ToneLoader` (each word → 0.1 s of a 440 Hz tone at 24 kHz in 2 chunks; voices `["default"]`; languages `["fr"]`; a folder containing a file named `fail` → `Err("broken model")`); `pub struct NoSpeechLoader` (always `Err("this hub was built without speech")`); `#[cfg(feature = "tts")] pub struct PhononLoader`
  - `pub struct Engine`; `pub fn start(loader: Arc<dyn SpeakerLoader>) -> Engine` (spawns one `std::thread` named `"tts"`); `pub fn state(&self) -> EngineState`; `pub fn load(&self, model: &str, dir: PathBuf, weights: String)`; `pub fn unload(&self)`; `pub fn add_voice(&self, name: String, path: PathBuf)`; `pub fn languages(&self) -> Vec<String>`; `pub fn voices(&self) -> Vec<String>`; `pub fn sample_rate(&self) -> Option<u32>`; `pub fn speak(&self, text: String, voice: Option<String>, sink: Box<dyn FnMut(Option<Result<Vec<f32>, String>>) -> bool + Send>) -> Result<(), TtsError>` (`Busy` when `QUEUE_LEN` requests already wait; `Conflict` when not `Ready`; the sink gets `Some(Ok(chunk))`… then `None` at the end or `Some(Err(_))`)

- [ ] **Step 1: Write the failing tests** (with `ToneLoader`, polling `state()` with a 2 s deadline)

- `loads_then_ready`: `load("m", dir, "model.q8.gguf")` → `Loading { model: "m" }` then `Ready { model: "m" }`; `sample_rate() == Some(24_000)`.
- `a_broken_model_is_an_error_state`: dir with `fail` → `Error { model: Some("m"), message: "broken model" }`.
- `last_load_wins`: `load("a")`, `load("b")` back to back → ends `Ready { model: "b" }`.
- `unload_returns_to_no_model`.
- `speak_streams_chunks_then_ends`: "un deux" → 2 words × 2 chunks then `None`; total samples = 2 × 2 400.
- `speak_before_ready_is_a_conflict`.
- `a_full_queue_is_busy`: a loader whose `speak` blocks on a barrier; 1 running + `QUEUE_LEN` waiting accepted, the next → `TtsError::Busy`.
- `a_sink_returning_false_stops_the_speech` (the `chunk` closure sees `false` and the loader stops).

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p nidoo --lib assist::tts::engine`
Expected: FAIL.

- [ ] **Step 3: Implement `Engine`**

A `std::sync::mpsc` channel of control messages to the thread; the shared state (`EngineState`, voices, languages, sample rate) in a `Mutex` read by the getters; an `AtomicUsize` of waiting speak requests for `Busy`. A `load` drains any newer `load` already queued before loading (last wins). Speak requests run one at a time on the thread.

- [ ] **Step 4: Implement `PhononLoader`**

`Checkpoint::resolve(dir, ResolveOptions { quant, weights: Some(weights) })` with `quant` from `quant_of` (`q8` → `Quant::Q80`, `q4k` → `Quant::Q4k`, `f32` → `Quant::F32`), `builder(Lang::Fr).device(DeviceKind::Cpu)`, the thread setting the spike found (half the logical cores, at least 1), `build()`, `register_voices`. `speak` uses `stream_with(text, &SpeechOptions::default().voice(v))` when a voice is given, else `stream(text)`; errors become `String`s.

- [ ] **Step 5: Run the tests, with and without the feature**

Run: `cargo test -p nidoo --lib assist::tts::engine && cargo build -p nidoo --no-default-features`
Expected: PASS, build clean.

- [ ] **Step 6: Commit**

```bash
git add crates/nidoo/src/assist/tts/engine.rs crates/nidoo/src/assist/tts/phonon.rs crates/nidoo/src/assist/tts/tone.rs crates/nidoo/src/assist/tts/mod.rs
git commit -m "feat(assist): a speech engine thread that loads one model at a time"
```

---

### Task 7: Voices from the model and the catalog

**Files:**
- Create: `crates/nidoo/src/assist/tts/voices.rs`
- Test: `crates/nidoo/tests/tts_voices.rs` (fake HF server serving `gradium/phonon-voices` at `VOICE_CATALOG_REV`)

**Interfaces:**
- Consumes: `HfClient` (Task 3); `VoiceView`, `VoicesView`, `VoiceSource`, `DEFAULT_VOICE` (Task 2).
- Produces:
  - `pub struct CatalogVoice { pub name: String, pub filename: String, pub voice_id: Option<String>, pub language: Option<String>, pub gender: Option<String>, pub age: Option<String>, pub description: Option<String> }` (serde, unknown fields ignored)
  - `pub struct VoiceCatalog`; `pub fn new(dir: PathBuf) -> VoiceCatalog`; `pub async fn list(&self, client: &HfClient) -> (Vec<CatalogVoice>, bool /*offline*/)` (cached `voices.jsonl` is used when present; fetched once otherwise, without token; offline = fetch failed and nothing cached → empty list); `pub async fn fetch(&self, client: &HfClient, voice: &CatalogVoice) -> Result<PathBuf, TtsError>` (refuses a `filename` that is not `voices/<name>.safetensors` with no `..`; ≤ `MAX_VOICE_BYTES`; reuses the file on disk)
  - `pub fn view(model_voices: &[String], languages: &[String], catalog: &[CatalogVoice], on_disk: impl Fn(&CatalogVoice) -> bool, chosen: &str, offline: bool) -> VoicesView` — model voices first (source `Model`, `on_disk: true`), then catalog voices whose `language` is in `languages`, sorted by name; `fell_back` = `chosen` is in neither list, in which case `chosen` becomes `DEFAULT_VOICE`

- [ ] **Step 1: Write the failing tests**

- unit `view_filters_by_the_model_languages`: catalog fr/en/de, languages `["fr"]` → `default` + the fr voices only.
- unit `view_falls_back_to_default`: chosen `"Ghost"` → `chosen == "default"`, `fell_back`.
- `catalog_is_fetched_once_then_cached`: two `list` calls → one request logged; the file is on disk.
- `catalog_fetch_never_sends_the_token`: client built with a token → the logged request has no `Authorization`.
- `offline_without_cache_is_empty_and_flagged`: fake server down → `(vec![], true)`.
- `fetch_refuses_paths_outside_voices`: `filename: "../token"` → `Invalid`.
- `fetch_reuses_a_voice_on_disk`: second fetch → no request.

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p nidoo --test tts_voices && cargo test -p nidoo --lib assist::tts::voices`
Expected: FAIL.

- [ ] **Step 3: Implement** `voices.rs`; `HfClient` gains `pub fn without_token(&self) -> HfClient` if needed.

- [ ] **Step 4: Run the tests**

Run: same as Step 2. Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/nidoo/src/assist/tts/voices.rs crates/nidoo/src/assist/tts/hf.rs crates/nidoo/src/assist/tts/mod.rs crates/nidoo/tests/tts_voices.rs crates/nidoo/tests/common/fake_hf.rs
git commit -m "feat(assist): the model's voice plus catalog voices in its languages"
```

---

### Task 8: The `Tts` facade, hub wiring, management routes and uploads

**Files:**
- Modify: `crates/nidoo/src/assist/tts/mod.rs` (`Tts`, `TtsConfig`)
- Create: `crates/nidoo/src/assist/tts/upload.rs`
- Modify: `crates/nidoo/src/hub.rs` (field `tts: OnceLock<Arc<Tts>>`, `set_tts`, `tts`)
- Modify: `crates/nidoo/src/main.rs` (Run mode: `hub.set_tts(Tts::start(…))` right after `Hub::new`)
- Create: `crates/nidoo/src/http/tts.rs`; Modify: `crates/nidoo/src/http/mod.rs` (`mod tts;`), `crates/nidoo/src/http/api.rs` (`.merge(super::tts::routes())` in `routes()`; `impl From<TtsError> for HttpError`)
- Test: `crates/nidoo/tests/tts_http.rs`

**Interfaces:**
- Consumes: everything from Tasks 2–7.
- Produces:
  - `pub struct TtsConfig { pub root: PathBuf, pub hf_base: String, pub loader: Arc<dyn SpeakerLoader> }`; `impl TtsConfig { pub fn for_data_dir(data_dir: &Path) -> TtsConfig }` (root `data_dir/assist/tts`, `HF_BASE`, `PhononLoader` with the feature else `NoSpeechLoader`)
  - `pub struct Tts` with: `pub fn start(cfg: TtsConfig) -> Arc<Tts>` (opens the store, reads state, loads the active model if it exists else clears `active`, applies the chosen catalog voice once ready); `pub fn status(&self) -> TtsStatus`; `pub fn models(&self) -> Vec<ModelView>`; `pub fn remove(&self, id: &str) -> Result<(), TtsError>` (active → unload + `active: None`); `pub fn set_active(&self, model: Option<String>) -> Result<TtsStatus, TtsError>`; `pub async fn inspect(&self, url: &str) -> Result<RepoListing, TtsError>`; `pub fn start_download(&self, req: DownloadRequest) -> Result<DownloadView, TtsError>`; `pub fn download(&self) -> Option<DownloadView>`; `pub fn cancel_download(&self) -> Result<(), TtsError>`; `pub fn upload_open(&self) -> Result<UploadSession, TtsError>`; `pub async fn upload_file(&self, id: &str, name: &str, body: axum::body::Body) -> Result<(), TtsError>`; `pub fn upload_finish(&self, id: &str) -> Result<ModelView, TtsError>`; `pub fn upload_abort(&self, id: &str) -> Result<(), TtsError>`; `pub async fn voices(&self) -> VoicesView`; `pub async fn set_voice(&self, voice: &str) -> Result<VoicesView, TtsError>`; `pub fn set_token(&self, input: Secret) -> Result<(), TtsError>`
  - `Hub::set_tts(&self, tts: Arc<Tts>)`, `Hub::tts(&self) -> Option<&Arc<Tts>>` (routes answer `409 "speech is not set up on this hub"` when `None`)
  - Routes (all under `/api/v1`): `GET /assist/tts/status`; `GET /assist/tts/models`; `DELETE /assist/tts/models/{id}`; `PUT /assist/tts/active` (`ActiveRequest`); `POST /assist/tts/inspect` (`InspectRequest`); `POST /assist/tts/downloads` (`DownloadRequest` → `202`); `GET /assist/tts/downloads/current` (`404` when none); `DELETE /assist/tts/downloads/current`; `POST /assist/tts/uploads` (→ `UploadSession`); `PUT /assist/tts/uploads/{id}/files/{name}` (raw body, `DefaultBodyLimit::disable()` on this route only); `POST /assist/tts/uploads/{id}/finish` (→ `ModelView`); `DELETE /assist/tts/uploads/{id}`; `GET /assist/tts/voices`; `PUT /assist/tts/voice` (`VoiceRequest`); `PUT /assist/tts/token` (`TokenInput` → `204`)
  - `TtsError` → status: `NotFound` 404, `Invalid` 422, `Conflict` 409, `Busy` 503, `Upstream` 502, `Denied` 502, `NoSpace` 507, `TooLarge` 413, `Io` 500

- [ ] **Step 1: Write the failing HTTP tests** (router built with `http::router`, a hub with `set_tts(Tts::start(TtsConfig { root: tempdir, hf_base: fake, loader: Arc::new(ToneLoader) }))`)

- `status_with_nothing_installed`: `engine.state == "no_model"`, `token_set == false`, `voice == "default"`.
- `download_then_activate_then_ready`: inspect → download q8 → poll `downloads/current` until `done` (5 s deadline) → `models` lists it with `french: true` → `PUT active` → status reaches `ready`.
- `state_survives_a_restart`: after the above, a new `Tts::start` on the same root → `ready` with the same model and voice.
- `remove_active_unloads`: `DELETE models/{id}` → status `no_model`, `active: null`, models empty.
- `activate_unknown_is_404`.
- `token_is_write_only`: `PUT token {"token":"hf_test"}` → `204`; `GET status` → `token_set: true`; no response body in the whole test contains `hf_test`.
- `upload_round_trip`: open → PUT each fake model file → finish → model with `source: "upload"` listed.
- `upload_refuses_unknown_names_and_oversize`: name `evil.sh` → 422; a weight body over `MAX_WEIGHTS_BYTES` (stream of zeros, counted, never buffered) → 413 and the session's folder is gone.
- `upload_finish_without_config_is_422`.
- `other_routes_keep_the_default_body_limit`: a 3 MB JSON to `POST /assist/tts/inspect` → 413.
- `hub_without_tts_answers_409`: a hub with no `set_tts` → `GET /assist/tts/status` 409.
- `boot_with_a_broken_active_model_still_serves`: state names a model whose folder has `fail` → status `error`, and `POST /api/v1/assist {text:"allume le salon", simulate:true}` still answers `200`.

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p nidoo --test tts_http`
Expected: FAIL.

- [ ] **Step 3: Implement `upload.rs`**

Sessions in memory (`id` = 128-bit random hex from `getrandom`), each owning a `Staging`. `upload_file`: name must be in `SMALL_FILES` ∪ `WEIGHT_FILES`; a second weight file → `Invalid`; stream the body to disk counting bytes, past the cap (`MAX_WEIGHTS_BYTES` / `MAX_SMALL_FILE_BYTES`) → delete the session and `TooLarge`. `upload_finish`: build the `Manifest` (no repo, revision or sha256; `source: Upload`; id from `free_id(upload--<slug>--<quant>)`) and `Staging::finish`. The upload is not test-loaded: the engine loads it only when it is activated, and a model that fails to load then shows `Error`.

- [ ] **Step 4: Implement `Tts`, the hub field, `main.rs` wiring, the routes and the error mapping**

`main.rs`: `hub.set_tts(Tts::start(TtsConfig::for_data_dir(&data_dir)));` right after `Hub::new`. The download's `done` callback is a no-op beyond the store (activation stays the user's choice). `set_voice`: a catalog voice is fetched, then `engine.add_voice`; state saved.

- [ ] **Step 5: Run the tests and clippy**

Run: `cargo test -p nidoo --test tts_http && cargo clippy -p nidoo --all-targets -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/nidoo/src/assist/tts/mod.rs crates/nidoo/src/assist/tts/upload.rs crates/nidoo/src/hub.rs crates/nidoo/src/main.rs crates/nidoo/src/http/tts.rs crates/nidoo/src/http/mod.rs crates/nidoo/src/http/api.rs crates/nidoo/tests/tts_http.rs
git commit -m "feat(assist): manage speech models, voices and the token over HTTP"
```

---

### Task 9: The speech address (streamed WAV)

**Files:**
- Create: `crates/nidoo/src/assist/tts/speech.rs`
- Modify: `crates/nidoo/src/assist/tts/mod.rs` (`Tts::speak`, `Tts::clip`), `crates/nidoo/src/http/tts.rs`
- Test: unit tests in `speech.rs`; `crates/nidoo/tests/tts_speech.rs`

**Interfaces:**
- Consumes: `Engine::speak` (Task 6); `Tts` (Task 8); `SpeechRequest`, `SpeechTicket` (Task 2).
- Produces:
  - `pub fn wav_header(sample_rate: u32, data_len: Option<u32>) -> [u8; 44]` (PCM, 1 channel, 16 bits; `None` → RIFF size and data size `0xFFFF_FFFF`)
  - `pub fn to_pcm16(samples: &[f32], out: &mut Vec<u8>)` (clamp to [-1, 1], ×32767, little-endian)
  - `pub struct Clip` (bytes so far, `done`, `error`, a `tokio::sync::Notify`, `created`); `pub struct Clips` with `pub fn create(&self) -> (String, Arc<Clip>)` (purges clips older than `CLIP_TTL_S`) and `pub fn get(&self, id: &str) -> Option<Arc<Clip>>`; `impl Clip { pub fn body(self: Arc<Self>, sample_rate: u32) -> axum::body::Body }` (header with exact sizes when already done, else unknown; then every byte from the start, following new chunks until done; an engine error mid-way ends the body)
  - `Tts::speak(&self, req: SpeechRequest) -> Result<SpeechTicket, TtsError>` (trimmed text empty → `Invalid`; > `MAX_SPEECH_CHARS` chars → `Invalid`; engine not ready → `Conflict` with the state; voice = request's, else the chosen one; synthesis starts now, so the clip fills even before the GET)
  - Routes: `POST /assist/speech` (→ `201` + `SpeechTicket`, `url = "/api/v1/assist/speech/{id}.wav"`); `GET /assist/speech/{file}` (`file` must end in `.wav`; unknown or expired → 404; headers `Content-Type: audio/wav`, `Cache-Control: no-store`; any `Range` ignored → `200`)

- [ ] **Step 1: Write the failing tests**

- unit `header_with_unknown_length`: bytes 4..8 and 40..44 are `FF FF FF FF`; bytes 24..28 = 24 000 LE; 34..36 = 16.
- unit `header_with_known_length`: data 4 800 → RIFF size 4 836, data size 4 800.
- unit `pcm16_clamps`: `[2.0, -2.0, 0.0]` → `FF 7F 01 80 00 00`.
- `speech_streams_a_wav`: ready tone model → `POST {text:"un deux"}` → `201`; `GET url` → `200`, `audio/wav`, body = 44 + 2 × 2 400 × 2 bytes.
- `a_second_fetch_gets_the_whole_clip`: two GETs (the second with `Range: bytes=100-`) → both `200` with identical full bodies; the second's header has exact sizes.
- `expired_or_unknown_is_404`: with `tokio::time::pause` + advance past 60 s and another `POST` (purge) → old URL 404; `GET /assist/speech/nope.wav` → 404.
- `speech_without_a_ready_model_is_409`, `empty_text_is_422`, `text_over_500_chars_is_422`.
- `queue_full_is_503` (blocking loader from Task 6).
- `the_text_is_not_in_the_url`: the returned `url` does not contain the text.

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p nidoo --lib assist::tts::speech && cargo test -p nidoo --test tts_speech`
Expected: FAIL.

- [ ] **Step 3: Implement** `speech.rs`, `Tts::speak`, `Tts::clip` and the two routes.

- [ ] **Step 4: Run the tests**

Run: same as Step 2. Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/nidoo/src/assist/tts/speech.rs crates/nidoo/src/assist/tts/mod.rs crates/nidoo/src/http/tts.rs crates/nidoo/tests/tts_speech.rs
git commit -m "feat(assist): speak text as a streamed WAV address"
```

---

### Task 10: The Assist settings screen

**Files:**
- Create: `crates/nidoo-web/src/api/tts.rs`; Modify: `crates/nidoo-web/src/api/mod.rs`
- Create: `crates/nidoo-web/src/i18n/assist_settings.rs`; Modify: `crates/nidoo-web/src/i18n/mod.rs`
- Create: `crates/nidoo-web/src/screens/assist_settings.rs`; Modify: `crates/nidoo-web/src/screens/mod.rs`
- Modify: `crates/nidoo-web/src/app.rs` (route `/assistant/settings`), `crates/nidoo-web/src/screens/assistant.rs` (⚙ link in `assist__head`), `crates/nidoo-web/styles/assist.css`
- Test: unit tests in `assist_settings.rs`; a `wasm_bindgen_test` render test next to the existing web ones

**Interfaces:**
- Consumes: the routes and types of Tasks 2, 8, 9.
- Produces:
  - `api::tts`: `get_status() -> Result<TtsStatus, ApiError>`, `get_models() -> Result<Vec<ModelView>, ApiError>`, `remove_model(id)`, `set_active(Option<&str>) -> Result<TtsStatus, ApiError>`, `inspect(url) -> Result<RepoListing, ApiError>`, `start_download(&DownloadRequest) -> Result<DownloadView, ApiError>`, `get_download() -> Result<Option<DownloadView>, ApiError>` (404 → `None`), `cancel_download()`, `upload_open() -> Result<UploadSession, ApiError>`, `upload_file(id, name, &web_sys::File)`, `upload_finish(id) -> Result<ModelView, ApiError>`, `upload_abort(id)`, `get_voices() -> Result<VoicesView, ApiError>`, `set_voice(&str) -> Result<VoicesView, ApiError>`, `set_token(Option<&str>)` (sends `{"token": "<value>"}` or `{"token": null}` built with `serde_json::json!`), `speak(&SpeechRequest) -> Result<SpeechTicket, ApiError>` — all through `super::core::{check, encode_id, read_json}` like `api/assist.rs`
  - `AssistSettingsText` enum + `assist_settings_text(lang, key) -> &'static str` + `use_assist_settings_text()`, following `i18n/assistant.rs`
  - pure helpers in `assist_settings.rs`: `pub fn size_label(bytes: u64) -> String` (`"74,6 Mo"` FR / `"74.6 MB"` EN with `Lang`), `pub fn progress_pct(done: u64, total: u64) -> u8` (0 when total is 0, capped at 100), `pub fn preselect(weights: &[WeightFile]) -> Option<String>` (`model.q8.gguf`, else `model.q4k.gguf`, else the first)
  - `#[component] pub fn AssistSettingsScreen() -> impl IntoView`

- [ ] **Step 1: Write the failing unit tests**

```rust
#[test]
fn preselects_q8_then_q4k_then_first() {
    let w = |n: &str| WeightFile { name: n.into(), size: 1 };
    assert_eq!(preselect(&[w("model.safetensors"), w("model.q8.gguf")]).as_deref(), Some("model.q8.gguf"));
    assert_eq!(preselect(&[w("model.safetensors"), w("model.q4k.gguf")]).as_deref(), Some("model.q4k.gguf"));
    assert_eq!(preselect(&[w("model.safetensors")]).as_deref(), Some("model.safetensors"));
    assert_eq!(preselect(&[]), None);
}

#[test]
fn progress_never_divides_by_zero_or_overflows() {
    assert_eq!(progress_pct(5, 0), 0);
    assert_eq!(progress_pct(50, 200), 25);
    assert_eq!(progress_pct(300, 200), 100);
}

#[test]
fn sizes_read_in_each_language() {
    assert_eq!(size_label(74_553_568, Lang::Fr), "74,6 Mo");
    assert_eq!(size_label(74_553_568, Lang::En), "74.6 MB");
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p nidoo-web assist_settings`
Expected: FAIL.

- [ ] **Step 3: Implement the API client, i18n and helpers**

- [ ] **Step 4: Implement the screen** — sections in the spec's order (§8): Speech status · Test · Voice · Installed models · Add a model (+ Upload from this computer) · Hugging Face token. Behaviour that is decided here:
  - Polls `get_status` every 1000 ms only while `status.download` is `Running` or the engine is `Loading`; stops on leaving the page.
  - Test: `speak` → set the `<audio>` `src` to the ticket URL → `play()`; time to first sound = click → the element's `playing` event, shown in ms; an error shows the hub's message.
  - Remove uses `components::confirm::ConfirmBar` like other deletions.
  - Upload: `<input type="file" multiple webkitdirectory>`; files whose names are not in the allowed lists are skipped client-side (the hub refuses them anyway); one `upload_file` per file, sequential, then `upload_finish`; any failure → `upload_abort`.
  - Token: a password input, Save / Clear, shows "set" / "not set" from `token_set`; the field is emptied after Save.
  - A model row with `french: false` shows "does not speak French" next to it, and the Speech section repeats it when that model is active.
  - After **List files**, a `gated` repo with `token_set == false` shows "this model needs a Hugging Face token" above Download (Download stays enabled; the hub's `Denied` message covers a wrong token).
  - `VoicesView.fell_back` shows "the chosen voice is not available with this model; Default is used"; `catalog_offline` shows "catalog voices unavailable offline".
  - The licence line: the 40m models are gated and CC BY-NC (non-commercial) and must be accepted on their Hugging Face page first, with a link to the repo when one is shown.
  - ⚙ in `assist__head` is an `<a href="/assistant/settings">` with an i18n `title`; the settings screen has a back link to `/assistant`.

- [ ] **Step 5: Add a wasm render test** that mounts `AssistSettingsScreen` with no hub (requests fail) and checks the six section headings render, following the existing `wasm_bindgen_test` setup in `crates/nidoo-web/tests/`.

- [ ] **Step 6: Run the web checks**

Run: `cargo test -p nidoo-web && cargo check -p nidoo-web --target wasm32-unknown-unknown && cargo test -p nidoo-web --test bare_text`
Expected: PASS (no new entry in `bare_text_allow.txt`).

- [ ] **Step 7: Look at it**

Run the hub locally against a temp data dir with a public model uploaded (the `phonon-31m-en` folder from Task 1) and use the `run` skill to open `/assistant/settings`: upload, activate, play the test sentence, switch voice, remove. Fix what looks wrong.

- [ ] **Step 8: Commit**

```bash
git add crates/nidoo-web/src/api/tts.rs crates/nidoo-web/src/api/mod.rs crates/nidoo-web/src/i18n/assist_settings.rs crates/nidoo-web/src/i18n/mod.rs crates/nidoo-web/src/screens/assist_settings.rs crates/nidoo-web/src/screens/mod.rs crates/nidoo-web/src/app.rs crates/nidoo-web/src/screens/assistant.rs crates/nidoo-web/styles/assist.css crates/nidoo-web/tests/
git commit -m "feat(assist): settings screen to install models, pick a voice and test speech"
```

---

### Task 11: Docs, TODOS and the full check

**Files:**
- Modify: `docs/users/assistant.md` (section "Voice"), `docs/reference/http-api.md` (the routes of Tasks 8–9), `docs/contributors/architecture.md` (the `assist::tts` module and the `tts` feature), `TODOS.md` (tick "Speech out" with the spec path; keep the satellite items)

- [ ] **Step 1: Write the docs** — user doc covers: getting a token, accepting the gated terms, the licence (non-commercial), adding by URL, upload offline, Active / Remove, voices (Default + catalog, downloaded on first choice), Test, and that Assist does not speak its replies yet. Generic examples only.

- [ ] **Step 2: Run the whole CI set**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo check -p nidoo-core -p nidoo-api -p nidoo-web --target wasm32-unknown-unknown && cargo test --workspace && tools/ci/brand-check.sh`
Expected: all pass.

- [ ] **Step 3: Commit**

```bash
git add docs/users/assistant.md docs/reference/http-api.md docs/contributors/architecture.md TODOS.md
git commit -m "docs(assist): speech models, voices and the speech address"
```

- [ ] **Step 4: Push and open the PR against `main`** (`git push -u origin feat/assist-tts`; if the push gets a 403, use the Gekkotron credential helper from memory). PR body: what it does, the spike numbers, that the hub image must be rebuilt, and that the user must accept the model's terms and paste a token.
