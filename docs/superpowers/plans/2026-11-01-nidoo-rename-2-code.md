# Nidoo rename, phase 2: code and deployment names — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The crates, the binary, the Docker image and containers, and the environment variables are named `nidoo`, while every existing install keeps working.

**Architecture:** A mechanical rename of the ten crates and their Rust paths; the hub crate becomes package and binary `nidoo`. Environment variables move to `NIDOO_*` with `OIKOS_*` still read underneath (`NIDOO_*` wins, a one-time warning names the old ones). Compose gets new image and container names; data stays in the same bind mounts, so `docker compose up -d` only recreates containers. The internal namespace (MQTT topics, entity ids, database name and user, storage keys, the legacy `oikos.toml` import) is **phase 3**, outlined at the end.

**Tech Stack:** Cargo workspace (10 crates), figment (`Env::prefixed`), clap, Trunk, Docker Compose, bash (`tools/*.sh`), GitHub Actions.

**Spec:** Same decisions as phase 1 (`2026-11-01-nidoo-rename-1-brand.md`), which must be merged first.

## Global Constraints

- Crate map (directory = package name; Rust path uses `_`):
  `oikos-os` → `nidoo` · `oikos-core` → `nidoo-core` · `oikos-api` → `nidoo-api` · `oikos-web` → `nidoo-web` · `oikos-climate` → `nidoo-climate` · `oikos-push` → `nidoo-push` · `oikos-alarm` → `nidoo-alarm` · `oikos-frigate` → `nidoo-frigate` · `oikos-watchdog` → `nidoo-watchdog` · `oikos-solcast` → `nidoo-solcast`.
- Binary: `nidoo` (clap `name = "nidoo"`), installed at `/usr/local/bin/nidoo`.
- Docker: image `nidoo:local`; containers `nidoo-hub`, `nidoo-mosquitto`, `nidoo-timescaledb`; anchor `x-nidoo-build`; mount `/run/nidoo-bundled`.
- Env vars: `NIDOO_<SECTION>__<KEY>` and `NIDOO_DATA_DIR`. `OIKOS_*` stays readable until phase 3; when both are set, `NIDOO_*` wins.
- Unchanged in this phase: `oikos/…` topics, `oikos_*` entity ids, the `oikos` database name and user (`POSTGRES_USER`/`POSTGRES_DB`, CI `TEST_DATABASE_URL`), `oikos.*` web storage keys, the legacy `oikos.toml` import file name.
- `docs/superpowers/**` is never edited.
- Long builds: `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0` per invocation; check `df -h` and the shared cargo target size before the first build (clean if > 5 GB).
- No open PRs should be in flight when Task 1 merges (it touches every Rust file); check `gh pr list` first.
- Commit identity: `-c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com`.

## Review Focus

- The live hub's container env still says `OIKOS_HISTORY__URL` (or any `OIKOS_*`) after the update → it must keep working. Task 2's `oikos_env_still_read` test.
- A host with both `OIKOS_MQTT__HOST` and `NIDOO_MQTT__HOST` set to different values → `NIDOO_*` wins. Task 2's `nidoo_env_wins_over_oikos`.
- String settings set through `OIKOS_*` (passwords like `007`) read as numbers after the change → they stay raw strings under both prefixes. Task 2's `string_keys_read_under_both_prefixes`.
- `update.sh` on a host whose old `oikos-hub` container still runs → after the update exactly one hub runs, named `nidoo-hub`, data untouched. Task 3's update test case.
- A host that runs its own database container named `oikos-timescaledb` with a hub-only compose override → the update must not stop, rename or recreate it. Task 3's hub-only test case.

---

### Task 1: Rename the crates and the binary

**Files:**
- Move: `crates/oikos-*` → `crates/nidoo-*` (`git mv`), `crates/oikos-os` → `crates/nidoo`
- Modify: root `Cargo.toml` (members, `[workspace.dependencies]` paths), every crate's `Cargo.toml`, `Cargo.lock` (regenerated)
- Modify: every `use oikos_…`/`oikos_…::` path in `crates/**` and tests
- Modify: `crates/nidoo/src/cli.rs:6` (`name = "nidoo"`), `Dockerfile` (`-p nidoo`, `-p nidoo-web`, `cd crates/nidoo-web`, `target/release/nidoo`, `/usr/local/bin/nidoo`, `ENTRYPOINT`), `.github/workflows/ci.yml` (`-p nidoo-core`, `-p nidoo-api`, `-p nidoo-web`, `working-directory: crates/nidoo-web`), `tools/restart.sh` (`nidoo` binary, `pgrep -f 'nidoo run'`, `crates/nidoo-web`, log `target/nidoo.log`), `crates/nidoo-web/compress.sh`, `crates/nidoo-web/Trunk.toml` if it names the crate
- Modify: `docs/contributors/*.md`, `docs/reference/*.md`, `docs/users/*.md`, `README.md` — commands and paths (`cargo run -p nidoo`, `crates/nidoo-web`)

**Interfaces:**
- Produces: packages and paths per the crate map; binary `nidoo`. Tasks 2 and 3 use `crates/nidoo/src/config.rs`, `crates/nidoo/src/cli.rs`, `/usr/local/bin/nidoo`.

- [ ] **Step 1: Move and rewrite**

`git mv` each directory; then rewrite `oikos-<crate>` → `nidoo-<crate>` and `oikos_<crate>` → `nidoo_<crate>` for the ten names only (an exact word list, not a blanket `oikos` replace: `oikos/`, `oikos_alarm_house`, `oikos.theme` must survive). The hub crate's Rust path `oikos_os` → `nidoo`.

- [ ] **Step 2: Prove nothing old is left**

Run: `git grep -n -E 'oikos[-_](os|core|api|web|climate|push|alarm|frigate|watchdog|solcast)\b' -- . ':!docs/superpowers'`
Expected: no output.

- [ ] **Step 3: Build, lint, test**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo check -p nidoo-core -p nidoo-api -p nidoo-web --target wasm32-unknown-unknown && cargo test --workspace`
Expected: all pass, same test count as `main` (note it before starting).

- [ ] **Step 4: Web bundle and browser tests**

Run: `cd crates/nidoo-web && trunk build --release && wasm-pack test --headless --chrome`
Expected: build succeeds; tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A crates Cargo.toml Cargo.lock Dockerfile .github tools docs/contributors docs/reference docs/users README.md
git commit -m "refactor: rename the crates and the binary to nidoo"
```

### Task 2: `NIDOO_*` env vars, `OIKOS_*` still read

**Files:**
- Modify: `crates/nidoo/src/config.rs:844-866` (env layers), `crates/nidoo/src/cli.rs` (data dir env)
- Modify: `Dockerfile` (`ENV NIDOO_HTTP__BIND`, `ENV NIDOO_DATA_DIR`), `compose.yml` comments, `docs/reference/configuration.md` (env table + one line: "`OIKOS_*` names still work; `NIDOO_*` wins when both are set.")
- Test: `crates/nidoo/src/config.rs` (new `mod env_prefix_tests`), `crates/nidoo/src/cli.rs` tests

**Interfaces:**
- Produces: `Config::env_provider() -> Env` returns the `NIDOO_` provider; `Config::legacy_env_provider() -> Env` the `OIKOS_` one; `Config::load` merges legacy then new. `pub const ENV_PREFIX: &str = "NIDOO_"`, `pub const LEGACY_ENV_PREFIX: &str = "OIKOS_"`. Every other caller of `env_provider()` (the hub.json boot path in `setup/boot.rs`) merges both the same way.

- [ ] **Step 1: Write the failing tests** (figment `Jail`, the file's existing style)

```rust
#[test] fn oikos_env_still_read()            // OIKOS_MQTT__PORT=1884 → cfg.mqtt.port == 1884
#[test] fn nidoo_env_wins_over_oikos()       // OIKOS_MQTT__HOST=a, NIDOO_MQTT__HOST=b → "b"
#[test] fn string_keys_read_under_both_prefixes() // OIKOS_MQTT__PASSWORD=007 → "007"; NIDOO_MQTT__PASSWORD=008 → "008"
#[test] fn data_dir_falls_back_to_oikos_env() // cli.rs: only OIKOS_DATA_DIR=/data → data_dir == "/data"; NIDOO_DATA_DIR=/n beside it → "/n"
```

- [ ] **Step 2: Run them (fail)**

Run: `cargo test -p nidoo env_prefix_tests data_dir_falls_back`
Expected: the `NIDOO_*` and fallback cases fail.

- [ ] **Step 3: Implement**

`load` and the boot path: `.merge(legacy_env_provider()).merge(env_provider())`; the string-key loop reads `OIKOS_<KEY>` then `NIDOO_<KEY>` (later merge wins). clap: `env = "NIDOO_DATA_DIR"`; after parsing, when the value is the default and `NIDOO_DATA_DIR` is unset, use `OIKOS_DATA_DIR` if set. At startup log once at `warn`: `OIKOS_* environment variables are deprecated; rename them to NIDOO_*` when any `OIKOS_` var is present.

- [ ] **Step 4: Run the tests (pass) and the whole crate**

Run: `cargo test -p nidoo`
Expected: PASS, including the existing `OIKOS_DATA_DIR` test at `cli.rs:142` (kept as the legacy case).

- [ ] **Step 5: Commit**

```bash
git add crates/nidoo Dockerfile compose.yml docs/reference/configuration.md
git commit -m "feat(config): NIDOO_* env vars, OIKOS_* still read underneath"
```

### Task 3: Docker names and the update switchover

**Files:**
- Modify: `compose.yml` (anchor, `image: nidoo:local`, `container_name` ×3, `/run/nidoo-bundled` mount + entrypoint path, comments, `--config` stays `/checkout/oikos.toml`)
- Modify: `crates/nidoo/src/cli.rs:207-221` only if it names a container (the `/checkout/oikos.toml` path stays)
- Modify: `tools/update.sh`, `tools/ci/stack-check.sh` (`docker exec nidoo-timescaledb`, `nidoo-mosquitto`; DB URL user/db stay `oikos`), `tools/tests/update_test.sh`
- Modify: `docs/users/install-on-linux.md`, `docs/users/getting-started.md` (container names, `docker logs nidoo-hub`)

**Interfaces:**
- Consumes: Task 1's binary path, Task 2's env names.

- [ ] **Step 1: Write the failing update tests** (in `update_test.sh`'s style, docker stubbed)

- `update_recreates_renamed_containers`: stubbed `docker ps` lists `oikos-hub`; after `update.sh` the log shows `docker rm -f oikos-hub` before `docker compose up -d`.
- `update_hub_only_leaves_own_db_alone`: with a `compose.override.yml` that runs the hub only and a stubbed `docker ps` listing `oikos-timescaledb`, the log never names `oikos-timescaledb` in a `stop`/`rm`.

- [ ] **Step 2: Run them (fail)**

Run: `tools/tests/update_test.sh`
Expected: the two new cases fail.

- [ ] **Step 3: Implement**

`update.sh`: before `docker compose up -d`, for each of `oikos-hub oikos-mosquitto oikos-timescaledb` that exists **and** carries this project's `com.docker.compose.project` label, `docker rm -f` it (compose labels prove it is ours; a hand-made container is left alone). Then the existing up. Data lives in `./data`, untouched.

- [ ] **Step 4: Run the shell checks and the stack check**

Run: `tools/tests/update_test.sh && shellcheck tools/*.sh tools/ci/*.sh tools/tests/*.sh && tools/ci/stack-check.sh`
Expected: all pass; `docker ps` afterwards lists `nidoo-hub`, `nidoo-mosquitto`, `nidoo-timescaledb` and no `oikos-*`.

- [ ] **Step 5: Commit**

```bash
git add compose.yml crates/nidoo/src/cli.rs tools docs/users
git commit -m "feat(deploy): nidoo image and container names, update.sh switchover"
```

### Task 4: Release

- [ ] **Step 1:** `tools/ci/brand-check.sh` still passes; PR CI green.
- [ ] **Step 2 (user's go):** before updating the live hub, read its `compose.override.yml` and env file on the server: note every `OIKOS_*` (they keep working) and confirm the house's own database container is not one compose owns.
- [ ] **Step 3 (user's go):** update the live hub; `docker ps` shows `nidoo-hub` running, the web UI and MCP answer, history and flows intact, one `deprecated OIKOS_*` warning in the log.
- [ ] **Step 4:** Rename `OIKOS_*` to `NIDOO_*` in the server's env file when convenient; the warning disappears.

---

## Phase 3 (later, own design first): the internal namespace

Not planned here: it moves stored data and changes the Android app's contract, so it starts with a brainstorm and a spec. Scope to cover:

- **MQTT topics** `oikos/…` → `nidoo/…` (automation, ui, group, climate, alarm, push, network, virtual, ambiances, discovery for hub entities, presence): read both prefixes during a transition, republish every retained message under `nidoo/`, clear the `oikos/` copies, then drop the old prefix. Defaults live in `config.rs:86-89` and `registry/mod.rs:60-61`.
- **Entity ids** `oikos_*` (`oikos_alarm_house`, `oikos_weather`, `oikos_solar`, `oikos_group…`, `oikos_virtual…`, `oikos_presence…`): flows, dashboards, overrides and history rows refer to them; rename in one migration (history `UPDATE … SET entity_id`) or keep an alias map.
- **Database** name and user `oikos` → `nidoo` (`ALTER DATABASE … RENAME`, `ALTER ROLE … RENAME`, password re-set because a role rename clears an MD5 password), compose, CI `TEST_DATABASE_URL`, `stack-check.sh`.
- **Web storage keys** `oikos.theme`, `oikos.lang`, `oikos.setup.skipped`, `oikos.alerts.last_seen` (+ the theme script in `index.html`): read the old key once, write the new one.
- **`OIKOS_*` env vars and the legacy `oikos.toml` import:** removed.
- **Android app:** topics and ids it reads, released together with the hub.
- Remove `OikosPush` from `brand-allow.txt` when its last mention goes.
