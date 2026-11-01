# Nidoo rename, phase 3 (namespace) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove every remaining `oikos` from the hub, its docs, the Android app and the live house, with a one-time migration command for the house's broker and database.

**Architecture:**
- **Hub PR (branch `refactor/nidoo-rename-3`):**
  1. A throwaway `nidoo migrate-oikos` subcommand in `crates/nidoo/src/migrate/`. It has a pure rewrite core, broker I/O, database I/O and a CLI front.
  2. The code-wide rename, with no transition code.
  3. Removal of the `OIKOS_*` and `oikos.toml` fallbacks.
  4. CI, docs and website.
- **App PR (branch `refactor/nidoo-namespace`, app repo):** the full rebrand.
- **Switch-over evening:** the user's go at each live step, then a cleanup PR that deletes the command and makes the brand check absolute.

**Tech Stack:** Rust (tokio, rumqttc 0.25, sqlx 0.8 Postgres, serde_json, base64 0.22, clap 4), TimescaleDB 2.17 / PG 17, bash, GitHub Actions; Android Kotlin/Gradle.

**Spec:** `docs/superpowers/specs/2026-11-01-nidoo-rename-3-namespace-design.md`. Read it first: the mapping table, the migration order and the switch-over checklist are authoritative.

## Global Constraints

- **The mapping:**
  - `oikos/` → `nidoo/`, `oikos_` → `nidoo_`, `oikos` → `nidoo`, `Oikos` → `Nidoo`, `OIKOS` → `NIDOO`.
  - `oikos-push` legacy: removed.
  - client_id `nidoo`, `WEB_INSTALL_ID` `nidoo-web`, MCP `nidoo://snapshot` and server name `nidoo`.
  - Database name, user and bundled Mosquitto user: `nidoo`. Browser keys `nidoo.*`, with no fallback. CSS `--nidoo-*`.
- No transition code anywhere: no reading both prefixes, no id aliases, no browser-key fallback. The **only** code that may contain `oikos` is `crates/nidoo/src/migrate/**` and its test file `crates/nidoo/tests/migrate.rs`. Both are deleted in Task 12.
- `docs/superpowers/**` is never edited, except to add this plan.
- No personal names, rooms, hostnames or IPs in tracked files. Use `<person>` and `phone` in fixtures. Never write the English word for a bird's home.
- Commit identity: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`. End each message with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Stage explicit paths. Before pushing, check `git log origin/main..HEAD`.
- Long builds: run with `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0` per invocation. Run `df -h ~` first, and check `du -sh ~/dev-scratch/cargo-target`: `cargo clean` if it is over 5 GB.
- The full check, used by every hub task's last step: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo check -p nidoo-core -p nidoo-api -p nidoo-web --target wasm32-unknown-unknown`.
  - Database tests need `TEST_DATABASE_URL`. Locally they skip without it, and CI sets it.
- Every live action (broker, database, house server, Firebase, GitHub rename, `~/.claude.json`) needs the user's explicit go at that step.

## Review Focus

- **A retained payload that is not UTF-8, or is empty:** the plan must leave it untouched unless its bytes contain `oikos`. If they do, the plan refuses it (reported) rather than corrupting it. Task 1's `binary_payload_untouched` and `binary_payload_with_oikos_refused`.
- **A JSON string that contains `oikos` mid-word or in user text** (`"Oikos alarm"`, `"smoke_test_oikos"`): it is rewritten case by case, and the dry run lists it. Task 1's `rewrites_case_variants_everywhere`.
- **The migration run twice, or after a crash between publish and clear:** the second run finds the `nidoo/` copies, re-verifies them, clears what is left of `oikos/`, and reports nothing new. Task 2's `plan_after_partial_apply_only_clears`.
- **An `oikos_x` id whose `nidoo_x` row already exists** (the new hub ran briefly before the migration): no duplicate key error and no lost rows. The `oikos_` rows are renamed except where `(entity_id, time)` collides, and a collision keeps the `nidoo_` row. Task 3's `entity_id_collision_keeps_new_row`.
- **Rows in a compressed chunk:** they are renamed too. Task 3's `renames_rows_in_compressed_chunk`.

---

## Part A: Hub PR (`refactor/nidoo-rename-3`, already holds the spec commit)

### Task 1: Pure rewrite core

**Files:**
- Create: `crates/nidoo/src/migrate/mod.rs` (module doc: throwaway, deleted after the switch-over; `pub mod rewrite; pub mod broker; pub mod db;`), `crates/nidoo/src/migrate/rewrite.rs`
- Modify: `crates/nidoo/src/lib.rs` (`pub mod migrate;`)
- Modify: `tools/ci/brand-allow.txt`: add `path:crates/nidoo/src/migrate/**` and `path:crates/nidoo/tests/migrate.rs`, with a comment saying they are removed in Task 12.

**Interfaces:**
- Produces:
  - `pub fn rename_text(s: &str) -> String`: replaces `oikos`→`nidoo`, `Oikos`→`Nidoo` and `OIKOS`→`NIDOO`. It is case-sensitive per variant, and other casings such as `oIKOS` are left as they are, then reported by verify.
  - `pub fn rename_topic(topic: &str) -> String`: a leading `oikos/` becomes `nidoo/`, and every `/`-separated segment that starts with `oikos_` becomes `nidoo_…`. A leading `oikos-push/` is not renamed: those topics are dropped (see `plan`).
  - `pub fn rename_json(v: serde_json::Value) -> (serde_json::Value, Vec<Change>)`: walks recursively and applies `rename_text` to every string value and every object key. Each change is recorded.
  - `pub struct Change { pub path: String, pub old: String, pub new: String }`, where `path` is a JSON pointer such as `/nodes/3/name`.
  - `pub struct Retained { pub topic: String, pub payload: Vec<u8>, pub qos: u8 }`
  - `pub enum Action { Move { from: String, to: Retained, changes: Vec<Change> }, Drop { topic: String, reason: &'static str }, Refuse { topic: String, reason: String } }`
  - `pub fn plan(retained: &[Retained]) -> Vec<Action>`. It selects every message whose topic starts with `oikos/` or `oikos-push/`, or contains a segment starting with `oikos_`, or whose payload bytes contain `oikos` in any of the three casings.
    - `oikos/push/tokens/*` and `oikos-push/*` become `Drop` (reason `"old Firebase token"` / `"legacy push topic"`).
    - A UTF-8 JSON payload goes through `rename_json`. Any other UTF-8 text goes through `rename_text`.
    - A non-UTF-8 payload that contains `oikos` is a `Refuse`. Without `oikos` it is a `Move` with the payload unchanged.
    - A message whose new topic equals its old topic is still a `Move` when the payload changed: the `homeassistant/…` alarm copy is one example. Publishing over the same topic is then the whole operation, and there is nothing to clear.

- [ ] **Step 1: Write the failing tests** (in `rewrite.rs` `#[cfg(test)]`):
  - `rename_topic_prefix_and_segments`: `oikos/ui/oikos_alarm_house` → `nidoo/ui/nidoo_alarm_house`; `homeassistant/device_tracker/oikos_phone/config` → `homeassistant/device_tracker/nidoo_phone/config`; `zigbee2mqtt/lamp` unchanged.
  - `rewrites_case_variants_everywhere`: `{"name":"Oikos alarm","unique_id":"oikos_alarm_house","device":{"identifiers":["oikos_alarm"]},"stat_t":"oikos/alarm/panel","OIKOS_KEY":1}` → every string and key renamed. 5 `Change`s, with paths `/name`, `/unique_id`, `/device/identifiers/0`, `/stat_t`, `/OIKOS_KEY`.
  - `plan_drops_tokens_and_legacy`: `oikos/push/tokens/abc` → `Drop`; `oikos-push/history` → `Drop`.
  - `plan_moves_flow_with_id_refs`: a flow payload that references `oikos_alarm_house` → `Move` to `nidoo/automation/<id>`, with the reference renamed.
  - `plan_rewrites_in_place_outside_namespace`: `homeassistant/alarm_control_panel/alarm/house/config` with `oikos/…` topics inside → `Move` with the same topic and a renamed payload.
  - `plan_ignores_clean_messages`: `zigbee2mqtt/lamp` with `{"state":"ON"}` → no action.
  - `binary_payload_untouched`: `oikos/x` with payload `[0xff, 0x00]` → `Move` to `nidoo/x`, payload identical.
  - `binary_payload_with_oikos_refused`: payload `b"\xffoikos"` → `Refuse`.
- [ ] **Step 2: Run them and see them fail.** Run: `CARGO_PROFILE_TEST_DEBUG=0 cargo test -p nidoo --lib migrate::rewrite`. Expected: compile errors or FAIL.
- [ ] **Step 3: Implement the interfaces above** in `rewrite.rs`.
- [ ] **Step 4: Run them and see them pass.** Same command. Expected: 8 passed.
- [ ] **Step 5: Commit:** `feat(migrate): pure rewrite rules for the oikos→nidoo switch-over`.

### Task 2: Broker side (backup, restore, apply, verify)

**Files:**
- Create: `crates/nidoo/src/migrate/broker.rs`

**Interfaces:**
- Consumes: Task 1's `Retained`, `Action`, `plan`, `rename_text`.
- Produces:
  - `pub struct BrokerOpts { pub host: String, pub port: u16, pub user: Option<String>, pub password: Option<String> }`, built from `HubFile.mqtt` by Task 4. The client id is `nidoo-migrate-<pid>`.
  - `pub async fn read_retained(o: &BrokerOpts, quiet: Duration) -> anyhow::Result<Vec<Retained>>`: subscribes to `#` and collects messages with the retain flag. It stops after `quiet` (default 3 s) with no new retained message.
  - `pub fn write_backup(path: &Path, msgs: &[Retained]) -> std::io::Result<()>` and `pub fn read_backup(path: &Path) -> std::io::Result<Vec<Retained>>`: JSON Lines, one `{"topic":…, "payload_b64":…, "qos":…}` per line. Retain is implied.
  - `pub async fn restore(o: &BrokerOpts, msgs: &[Retained]) -> anyhow::Result<usize>`: republishes each message retained.
  - `pub async fn apply(o: &BrokerOpts, actions: &[Action]) -> anyhow::Result<ApplyReport>`, in four phases:
    1. Publish every `Move.to`, retained, at its QoS.
    2. Re-read the retained messages and check that every `to` topic holds exactly the new payload. Any mismatch is an error, and nothing gets cleared.
    3. Clear (an empty retained message) every `Move.from` that differs from `to.topic`, and every `Drop.topic`.
    4. Re-read and check that the cleared topics are gone.
  - `pub fn verify_retained(msgs: &[Retained]) -> Vec<String>`: one line per topic that is still under `oikos/` or `oikos-push/`, or whose topic or payload contains `oikos` in any casing (case-insensitive).
  - Refusal rule: when any `Action::Refuse` exists, `apply` returns an error before publishing anything.

- [ ] **Step 1: Write the failing tests** (unit, no broker):
  - `backup_round_trips_binary_and_empty`: messages with payloads `b""`, `[0xff]` and UTF-8 JSON round-trip byte for byte.
  - `plan_after_partial_apply_only_clears`: feed `plan` a set where `oikos/ui/a` and its migrated `nidoo/ui/a` both exist with matching content. `plan` yields a `Move` whose `to` equals the existing message, so `apply` republishes the same bytes, which is harmless, and then clears. Assert the plan has 1 `Move` and no `Refuse`.
  - `verify_flags_any_casing`: `nidoo/x` with payload `"OiKoS"` → 1 line.
- [ ] **Step 2: Run them and see them fail.** Run: `CARGO_PROFILE_TEST_DEBUG=0 cargo test -p nidoo --lib migrate::broker`. Expected: FAIL.
- [ ] **Step 3: Implement the interfaces above** with rumqttc's `AsyncClient` and eventloop, the way `crates/nidoo/src/mqtt.rs` builds its options. Broker I/O gets its end-to-end proof in Task 4's stack-check.
- [ ] **Step 4: Run them and see them pass.** Expected: 3 passed.
- [ ] **Step 5: Commit:** `feat(migrate): broker backup, restore and verified move`.

### Task 3: Database side

**Files:**
- Create: `crates/nidoo/src/migrate/db.rs`, `crates/nidoo/tests/migrate.rs` (uses `tests/common/db.rs`'s `fresh_db()` and skips without `TEST_DATABASE_URL`, like the other database tests)

**Interfaces:**
- Produces:
  - `pub async fn entity_id_tables(pool: &PgPool) -> sqlx::Result<Vec<String>>`: every `public` table that has an `entity_id text` column, read from `information_schema.columns`, never hard-coded. Views are excluded.
  - `pub async fn rename_entity_ids(pool: &PgPool, dry_run: bool) -> sqlx::Result<Vec<(String, u64)>>`: in one transaction, for each table, it does the following and returns (table, rows renamed):
    - First delete the `oikos_` rows whose renamed key already exists. The key is `(entity_id, time)` for the hypertables and `entity_id` for `*_latest`. For `history_hourly` it is `(entity_id, bucket)`: read each table's primary key or unique index from `pg_index` to find the key columns.
    - Then `UPDATE … SET entity_id = 'nidoo_' || substr(entity_id, 7) WHERE entity_id LIKE 'oikos\_%'`.
    - On a dry run it counts and rolls back.
  - `pub async fn rename_settings(pool: &PgPool, dry_run: bool) -> sqlx::Result<Vec<(String, Vec<Change>)>>`: for each `settings` row whose `value::text` contains `oikos` in any of the three casings, it applies `rewrite::rename_json` and writes the result back. It returns (key, changes).
  - `pub async fn rename_db_and_role(admin_url: &str, from: &str, to: &str) -> anyhow::Result<()>`, run with the hub stopped:
    1. Connected as `from`, create the role `nidoo_migrate_tmp` (`SUPERUSER LOGIN`, random 32-char password). If it already exists, reset its password.
    2. Connect as the temp role to database `postgres`. Rename the database `from`→`to` if it still exists, and the role `from`→`to` if it still exists. The role rename keeps the SCRAM password.
    3. Connect as `to` with the original password to database `to`. If that fails with an auth error, run `ALTER ROLE <to> PASSWORD '<original>'` as the temp role and retry.
    4. Drop the temp role.

    `admin_url` is the hub's database URL. The function derives the other URLs from it by swapping user, database and password.
  - `pub fn renamed_url(url: &str, from: &str, to: &str) -> String`: swaps the user and the database path segment when they equal `from`.
  - `pub async fn verify_db(pool: &PgPool) -> sqlx::Result<Vec<String>>`: reports every database name, role name, `entity_id` value and `settings` value that contains `oikos` in any casing.

- [ ] **Step 1: Write the failing tests** in `tests/migrate.rs`:
  - `renames_entity_ids_in_every_table`: insert `oikos_alarm_house` into `entity_state`, `entity_latest`, `history_hourly`, `entity_availability`, `availability_latest` and `climate_decision`. After `rename_entity_ids(false)`, there are 0 rows `LIKE 'oikos%'` and the same total row count.
  - `renames_rows_in_compressed_chunk`: insert into `entity_state` 10 days ago, run `SELECT compress_chunk(c) FROM show_chunks('entity_state') c`, rename, and the row reads back as `nidoo_alarm_house`.
  - `entity_id_collision_keeps_new_row`: both `oikos_x` and `nidoo_x` at the same `time` in `entity_state` → afterwards 1 row `nidoo_x` with the `nidoo_x` value.
  - `dry_run_changes_nothing`: the counts are returned and the rows are unchanged.
  - `renames_settings_prefixes`: an `advanced` value `{"topics":{"ui_prefix":"oikos/ui","discovery_prefixes":["oikos"]}}` → `nidoo/ui`, `["nidoo"]`.
  - `renames_database_and_role`: create a throwaway role `oikos_t<rand>` (SCRAM password) and its database. Run `rename_db_and_role(url_of_throwaway, "oikos_t<rand>", "nidoo_t<rand>")`. Afterwards a connection as `nidoo_t<rand>` to database `nidoo_t<rand>` with the same password works, no `nidoo_migrate_tmp` role is left, and a second call is a no-op. Clean up afterwards.
  - `renamed_url_swaps_user_and_db` (unit, in `db.rs`): `postgres://oikos:pw@h:5433/oikos` → `postgres://nidoo:pw@h:5433/nidoo`. A user that isn't `from` is unchanged.
- [ ] **Step 2: Run them and see them fail.** Run: `CARGO_PROFILE_TEST_DEBUG=0 TEST_DATABASE_URL=<local test db> cargo test -p nidoo --test migrate` (start a local `timescale/timescaledb:latest-pg17` container if none is running, with the same env as `ci.yml`). Expected: FAIL.
- [ ] **Step 3: Implement the interfaces above.** If `UPDATE` on a compressed chunk errors on 2.17, decompress the chunks that hold `oikos_` rows first (`decompress_chunk`) and let the compression policy recompress them.
- [ ] **Step 4: Run them and see them pass.** Expected: 7 passed.
- [ ] **Step 5: Commit:** `feat(migrate): rename entity ids, settings, database and role`.

### Task 4: `nidoo migrate-oikos` command and stack proof

**Files:**
- Modify: `crates/nidoo/src/cli.rs`: add the `RawCommand::MigrateOikos` and `Cli::MigrateOikos` variants.
- Modify: `crates/nidoo/src/main.rs`: dispatch to `migrate::run`.
- Modify: `crates/nidoo/src/migrate/mod.rs`: add `pub async fn run(args: MigrateArgs, data_dir: &Path) -> anyhow::Result<()>`.
- Modify: `tools/ci/stack-check.sh`: add a migration smoke test.

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces:
  - The CLI: `nidoo migrate-oikos [--apply | --verify] [--backup <file>] [--restore <file>] [--data DIR]`. With no flag it is a dry run.
    - `--backup <file>` writes the retained backup, then continues with the chosen mode. `--apply` refuses to run without `--backup`.
    - `--restore <file>` only restores, then exits.
  - `pub struct MigrateArgs { pub mode: Mode, pub backup: Option<PathBuf>, pub restore: Option<PathBuf> }` and `pub enum Mode { DryRun, Apply, Verify }`.
  - Behaviour:
    - It reads `<data>/hub.json` for `mqtt` and `database_url`, and errors clearly if either is missing.
    - **Dry run:** prints every broker `Action` (topic old→new and each `Change`), then the database counts per table and the settings changes. Nothing is written.
    - **Apply, in this order:** broker apply, `rename_entity_ids`, `rename_settings`, `rename_db_and_role`, then rewrite `hub.json`'s `database_url` with `renamed_url` and save it. The rewrite is decided here: it is in the command.
    - **Verify:** runs `verify_retained` and `verify_db`, and exits non-zero if anything is reported.
    - Before the apply, the command refuses to run if the hub answers on its HTTP port (`HubFile.http`), so the hub must be stopped.

- [ ] **Step 1: Write the failing CLI tests** in `cli.rs`:
  - `migrate_defaults_to_dry_run`
  - `migrate_apply_requires_backup`, which is a clap error
  - `migrate_restore_takes_file`
- [ ] **Step 2: Run them and see them fail.** Run: `cargo test -p nidoo --lib cli`. Expected: FAIL.
- [ ] **Step 3: Implement the CLI and `run`.**
- [ ] **Step 4: Add the smoke test to `stack-check.sh`.** After the stack is up:
  1. Publish retained `oikos/ui/oikos_t` with `{"name":"oikos t"}`. Use lowercase only: the current brand check rejects a capitalised `Oikos` in `tools/`.
  2. Stop the hub container.
  3. Run `nidoo migrate-oikos --data … --backup /tmp/b.jsonl --apply` inside the image.
  4. Assert that `mosquitto_sub -t 'nidoo/ui/nidoo_t' -C 1 -W 3` prints `{"name":"nidoo t"}` and that `oikos/ui/oikos_t` is gone.
  5. Run `--verify` and expect exit 0.
  6. Restart the hub.
- [ ] **Step 5: Run.** `cargo test -p nidoo --lib cli` passes. `tools/ci/stack-check.sh` passes locally (it needs Docker).
- [ ] **Step 6: Commit:** `feat(migrate): migrate-oikos command with dry run, backup and verify`.

### Task 5: Rename the namespace in the code

**Files:** every non-migrate hit from the repo sweep, in categories 1–3, 5 and 7 of the spec's "What changes §1", chiefly:
- `crates/nidoo/src/config.rs`: defaults `:51`, `:86-89`, `:171-172` (delete `legacy_prefix`), `:684` and `:718`.
- `crates/nidoo-core/src/registry/mod.rs:60-61`, `crates/nidoo/src/hub.rs:143` (`WEB_INSTALL_ID`) and `:374` (fallback), `crates/nidoo/src/home.rs:259`.
- `crates/nidoo/src/climate/proposals.rs:378`: compare with `format!("{}/history", cfg.push.prefix)` (or the push crate's topic helper), never a literal.
- The ids: `alarm/mod.rs:17`, `alarm/entity.rs:27`, `groups/discovery.rs:246`, `nidoo-core/src/registry/folding.rs:9,23`, `virtual_entities.rs:74`, `weather/entities.rs:41`, `solar/entities.rs:27`, `network/discovery.rs:26`, `nidoo-web/src/home_header/summary.rs:12` and `nidoo-web/src/screens/alarm_settings.rs:1345`.
- MCP: `mcp/mod.rs:35` (`nidoo://snapshot`) plus the help text at `:214,230,241,358`; `nidoo-web/src/i18n/settings.rs:333-337` (server name `nidoo` in all three snippets).
- Push legacy removal: `nidoo-push/src/service.rs:139-150` and its routing, `crates/nidoo/src/mqtt.rs:290-295`, `nidoo-push/src/tokens.rs:181-182`.
- Bundled names: `setup/bundled.rs:46` (`BUNDLED_NAME = "nidoo"`), `setup/routes.rs:518-519`, `nidoo-web/src/setup/connection.rs:104-105`, `nidoo-web/src/setup/services/mod.rs:621,624`, `compose.yml:57-58`.
- Web keys: `theme.rs:14`, `index.html:12`, `i18n/mod.rs:59`, `alerts.rs:18`, `setup/services/mod.rs:66`, `screens/flows/mod.rs:25`.
- Doc comments that cite topics or the old Python services (`OikosPush` / `OikosHistory`) in `nidoo-push`, `nidoo-watchdog`, `nidoo-frigate`, `nidoo-api` and `nidoo-core/src/model/*`: reword to describe the behaviour without the old name.
- Tests and fixtures: `crates/*/tests/**`, the inline `#[cfg(test)]` modules, and `crates/nidoo-core/tests/fixtures/topics.txt`. `git mv` `device_tracker__oikos_phone.json` to `device_tracker__nidoo_phone.json` and rename its content (`Nidoo Phone`, `nidoo/presence/phone/…`). Delete the tests that only exercised the push legacy prefix.

**Interfaces:**
- Produces: the defaults and constants in the spec's mapping table. Tasks 6–8 rely on `BUNDLED_NAME = "nidoo"` and on the absence of `legacy_prefix` in `PushConfig`.

- [ ] **Step 1: Rename the defaults and constants, remove the push legacy, and fix the proposals literal.**
- [ ] **Step 2: Rename tests and fixtures mechanically.** For JSON fixtures, a scripted replace of the three casings is fine: they are generic recorded payloads.
- [ ] **Step 3: Add a test that pins the proposals fix:** `proposal_reply_follows_push_prefix`. With `push.prefix = "custom/push"`, a reply on `custom/push/history` is recognised. Put it in the existing proposals tests.
- [ ] **Step 4: Check that nothing is left in the touched areas.** Run: `git grep -n -i oikos -- crates ':!crates/nidoo/src/migrate' ':!crates/nidoo/tests/migrate.rs' | grep -v -E 'OIKOS_|oikos\.toml|legacy_env|legacy_data_dir'`. Expected: no output. The remaining env and toml hits belong to Task 6.
- [ ] **Step 5: Run the full check** (Global Constraints). Expected: all green.
- [ ] **Step 6: Commit:** `refactor: nidoo MQTT namespace, ids and wire values`.

### Task 6: Remove the `OIKOS_*` and `oikos.toml` fallbacks

**Files:**
- `crates/nidoo/src/config.rs`: remove `LEGACY_ENV_PREFIX`, `legacy_env_in_use`, `legacy_env_provider` and the `OIKOS_*` merge in `merge_env`. The `Config::load` default path becomes `nidoo.toml`. **Decided:** a dev run still finds a repo-root file by default.
- `crates/nidoo/src/main.rs:47,85-87`, `crates/nidoo/src/cli.rs:15-17,94-99` (delete `legacy_data_dir` and fix the `--data` doc comment), `setup/boot.rs:29,65,74-75,319`, `setup/hub_file.rs:117`, `flows/hub.rs:34`.
- Remove the `oikos.toml` import entirely: `setup/import.rs`, the legacy-toml detection in `setup/bundled.rs:92-100,230`, the import call and log in `setup/boot.rs`, and the `settings_imported` handling only where it serves the import. Keep the field if `hub.json` files already carry it: serde `default` ignores it either way.
- The user-facing strings that name `oikos.toml`: `nidoo-web/src/i18n/weather.rs:132-133`, `i18n/solar.rs:62-63`. Point them at the settings screen instead.
- `compose.yml:27-29,81,85-88`, `Dockerfile:5,89`, `.gitignore:2` and `.dockerignore:12` (`nidoo.toml`).
- `tools/update.sh`: remove the `oikos.toml` move and the `/etc/oikos/secrets` warning. Also remove `switch_renamed_containers`: the live hub already runs `nidoo-hub`.
- `tools/restart.sh`: `NIDOO_LOG`, and read `bind =` from `$ROOT/nidoo.toml`.
- Tests: delete `oikos_env_still_read`, `nidoo_env_wins_over_oikos`, `data_dir_falls_back_to_oikos_env`, the `OIKOS_*` cases in `hub_file.rs` and `boot.rs`, `tests/fixtures/legacy-oikos.toml` and its users in `tests/config.rs` and `tests/init_bundled.rs`. Update `tools/tests/update_test.sh` for the removed functions.

- [ ] **Step 1: Add a test that pins the removal:** `oikos_env_is_ignored` in `config.rs`. With `OIKOS_MQTT__HOST=x` set and no `NIDOO_*`, the loaded host is the default. Run it and see it fail (the fallback still reads it).
- [ ] **Step 2: Remove the code, tests and script parts listed above.** The new test passes.
- [ ] **Step 3: Check.** Run: `git grep -n -i oikos -- crates compose.yml Dockerfile .gitignore .dockerignore tools ':!crates/nidoo/src/migrate' ':!crates/nidoo/tests/migrate.rs' ':!tools/ci'`. Expected: no output.
- [ ] **Step 4: Run the full check, plus** `tools/tests/update_test.sh && shellcheck tools/*.sh tools/tests/*.sh`. Expected: all green.
- [ ] **Step 5: Commit:** `refactor: drop the OIKOS_* env and oikos.toml fallbacks`.

### Task 7: CI and tools

**Files:**
- `.github/workflows/ci.yml:51-62`: `POSTGRES_USER`, `POSTGRES_PASSWORD` and `POSTGRES_DB` become `nidoo`, the healthcheck becomes `pg_isready -U nidoo`, and `TEST_DATABASE_URL` becomes `postgres://nidoo:nidoo@localhost:5432/nidoo`.
- `tools/ci/stack-check.sh:77,81,103,111,119`: database URLs, `-u nidoo`, and the topic `nidoo/ci`. Keep Task 4's smoke step, which publishes `oikos/…` on purpose.
- `tools/record-fixtures.sh:16,83,85,86`: `nidoo/#` and `nidoo/ui/`.
- `crates/nidoo/tests/common/db.rs`: any `oikos` default.

- [ ] **Step 1: Make the edits.**
- [ ] **Step 2: Check.** Run: `git grep -n -i oikos -- .github tools ':!tools/ci/brand-check.sh' ':!tools/ci/brand-allow.txt' | grep -v 'stack-check.sh.*migrate'`. Expected: only the Task 4 smoke-test lines.
- [ ] **Step 3: Run** `shellcheck tools/*.sh tools/ci/*.sh tools/tests/*.sh && tools/ci/stack-check.sh`. Expected: pass. Push the branch and confirm CI is green, since the CI database rename can only be proven there.
- [ ] **Step 4: Commit:** `ci: nidoo database name and user`.

### Task 8: Docs, README and website

**Files:**
- `docs/**` (every page outside `docs/superpowers/`). The heaviest are `reference/mqtt-topics.md`, `users/push-notifications.md`, `reference/configuration.md`, `users/install-on-linux.md` and `users/assistants-mcp.md`.
  - Delete the "Switching over from OikosPush" section (`push-notifications.md:47-61`) and the "Upgrading from `oikos.toml`" section in `install-on-linux.md`. Fix their inbound links: `getting-started.md:35` and `configuration.md`.
  - `docs/users/water-heater-controller.md`: the discovery example uses `nidoo/device/water_heater/config`.
- `README.md:47` (the MCP server name).
- `website/`: the 26 `--oikos-*` CSS variables become `--nidoo-*` (`custom.css`, `index.module.css`, `Landing.module.css`, `Features.tsx`), and `TalkToIt.tsx:31,36` uses the server name `nidoo`.

- [ ] **Step 1: Make the edits.**
- [ ] **Step 2: Check.** Run: `git grep -n -i oikos -- . ':!docs/superpowers' ':!crates/nidoo/src/migrate' ':!crates/nidoo/tests/migrate.rs' ':!tools/ci/brand-allow.txt' ':!tools/ci/brand-check.sh' | grep -v 'stack-check.sh'`. Expected: no output.
- [ ] **Step 3: Build the site** (the website's own build command from its `package.json`; the docs site deploys on merge). Expected: it builds, and the link checker, if the build has one, passes.
- [ ] **Step 4: Run** `tools/ci/brand-check.sh`. Expected: pass.
- [ ] **Step 5: Commit:** `docs: nidoo topics, ids, database and MCP name everywhere`.
- [ ] **Step 6: Open the hub PR** against `main` (`git push -u origin refactor/nidoo-rename-3`). The body links the spec and this plan and says "merge only on switch-over evening". End it with the Claude Code attribution line. **Do not merge.**

## Part B: Firebase and the app

### Task 9: New Firebase project (user, in the console)

This task is the user's. The executor gives these steps and waits.

1. https://console.firebase.google.com → **Add project** → name `nidoo` → Analytics off.
2. **Add app → Android**, package `com.gekkotron.nidoo`, nickname `Nidoo`. Download `google-services.json`.
3. **Project settings → Service accounts → Generate new private key.** This is the hub's FCM key (JSON).
4. **App Distribution → Get started**, and create the tester group `Testers` with your account.
5. App repo GitHub secrets (Settings → Secrets → Actions):
   - `GOOGLE_SERVICES_JSON` = `base64 -i google-services.json`
   - `FIREBASE_APP_ID` = the new app id (Project settings → Your apps)
   - `CREDENTIAL_FILE_CONTENT` = a service-account key with App Distribution rights. The key from step 3 works.
6. Keep the hub key file off the repo. On switch-over evening it goes into `secrets/firebase.json` and the push settings (Task 11).

- [ ] Done when the user confirms all five steps.

### Task 10: App rebrand (app repo, worktree `~/Documents/SourcesPerso/Oikos-nidoo-namespace`, branch `refactor/nidoo-namespace`)

**Files:** whole repo, except `docs/superpowers/**` and the build output.

**Interfaces:**
- Consumes: the namespace commit `263332a`, already on the branch.
- Produces: `applicationId` `com.gekkotron.nidoo`, the link scheme `nidoo://`, and the display name "Nidoo".

- [ ] **Step 1: Packages.** `git mv` every `…/com/gekkotron/oikos/…` source directory to `…/com/gekkotron/nidoo/…`. Rewrite `package` and `import` lines, every Gradle `namespace`, and `applicationId` (`app/build.gradle.kts:45`). The debug suffix `.debug` stays.
- [ ] **Step 2: Names.**
  - Code names: `Oikos*` classes and identifiers → `Nidoo*` (`OikosTheme`, `OikosApplication`, `OikosWidget`, …).
  - Build: the `oikos.*` convention plugins in `build-logic` (ids and the files that apply them), and `rootProject.name = "Nidoo"`.
  - Resources: `ic_stat_oikos`, the `oikos_widget_*` and `oikos_window` colours, `oikos_widget_info`. `app_name` = "Nidoo" in `values` and `values-fr`, and the onboarding strings (`code de configuration Nidoo`, key `scan_error_not_nidoo`).
  - Internal keys: the `oikos://` scheme (`AndroidManifest.xml:37,43,44`, `DeepLinks.kt`, `AlertNotifier.kt`, the widget), the keystore alias `nidoo.secrets.v1`, the geofence id `nidoo.presence.home` and the WorkManager names `nidoo-widget-*`. All of these are safe to rename because it is a new install.
  - Delivery: the CI artifact `nidoo-debug-apk`, `distribute.yml`'s `nidoo-release.jks` temp file name, `.gitignore` entries and `tools/setup-qr.py`.
  - Docs: README and docs outside `docs/superpowers/`, and the codecov paths. The repo slug becomes `Gekkotron/Nidoo-app` (README badges and links).
  - Hub-crate names cited in comments (`oikos-web` → `nidoo-web`, and so on).
- [ ] **Step 3: Check.** Run: `git grep -n -i oikos -- . ':!docs/superpowers'`. Expected: no output. The saved-profile migration from `263332a` (`ProfileRepository.renamedNamespace`) is the one allowed exception: it reads the literal `oikos` to migrate an old saved value. That is pointless now, because the new package starts with no saved profile, so **delete it and its test** too.
- [ ] **Step 4: Verify.** Run: `./gradlew testDebugUnitTest jvmTest assembleDebug --continue`. Expected: all pass (the known flaky `OnboardingViewModelTest` may need a re-run). Install `app-debug.apk` on an emulator, or on the phone with the user's go, and check that it launches as "Nidoo".
- [ ] **Step 5: Commit:** `refactor: rebrand the app to Nidoo (com.gekkotron.nidoo)`. Push the branch, open a PR against `master` saying "merge on switch-over evening", and end the body with the attribution line. Do not merge.

## Part C: Release

### Task 11: Switch-over evening (each live step needs the user's go)

Follow the spec's checklist, steps 1–12. The commands the executor runs:

- [ ] **1. Preconditions:**
  - Both PRs are green.
  - Task 9 is done.
  - Read the server's `compose.override.yml`, its env file, and the house's own database container definition. Note every `oikos` in them.
  - Read the house broker config for ACL entries that name `oikos/`.
- [ ] **2. Back up:**
  - A database dump to the external disk, in the same format as the 10-24 backup.
  - The broker backup comes from the command itself in step 4.
- [ ] **3. Stop the writers:** force-stop or uninstall the old app (the user does this), then stop `nidoo-hub`.
- [ ] **4. Migrate.** Merge the hub PR and build the new image (`tools/update.sh --hub-only` builds without starting it, or use `docker compose build`, the builder's choice). Run it as a one-off container with the hub's data mount:
  1. `nidoo migrate-oikos --data /data`. The user reviews the dry run, especially the user-text changes.
  2. `nidoo migrate-oikos --data /data --backup /data/oikos-retained-2026-11-XX.jsonl --apply`. Copy the backup to the external disk.
  3. `nidoo migrate-oikos --data /data --verify` → exit 0.
- [ ] **5. Update the server:**
  - Rename the database URL env var's user and database in the override, if it is set there.
  - Rename the database container to `nidoo-timescaledb`, and update `POSTGRES_USER`, `POSTGRES_DB` and the healthcheck in its definition.
  - Add any `nidoo/` ACL entries.
- [ ] **6. Deploy:** `tools/update.sh --hub-only`. Set the new FCM key: `secrets/firebase.json`, plus the push settings through `PUT /api/v1/settings/push` or the UI. Check the log for warnings and errors.
- [ ] **7. App:** merge the app PR, which triggers `distribute.yml`. Install "Nidoo" from App Distribution and set it up (QR). Uninstall the old app if it is still there. Once the alarm shows in the new app from `nidoo/#` discovery, clear the hand-made copy with an empty retained message on `homeassistant/alarm_control_panel/alarm/house/config` (the user's go). The migration renamed that copy in place, but the app no longer needs it.
- [ ] **8. Check:** every point in the spec's step 8. Then re-run `nidoo migrate-oikos --verify` after the new app has connected: still exit 0, so nothing republished `oikos`.
- [ ] **9. MCP:** rename the `oikos` entry to `nidoo` in `~/.claude.json` (the user's go), then the user restarts the session.
- [ ] **10. GitHub:** rename `Gekkotron/Oikos-app` → `Gekkotron/Nidoo-app` (the user's go: `gh repo rename Nidoo-app -R Gekkotron/Oikos-app`). Update the local remotes.
- [ ] **11. Firebase:** after a day of working push, the user deletes the old Firebase project.
- [ ] **12. Local:** rename the user's repo-root `oikos.toml` to `nidoo.toml` (dev runs), with the user's go, since it is the user's file.

### Task 12: Cleanup PR (after Task 11 is done)

**Files:**
- Delete `crates/nidoo/src/migrate/`, `crates/nidoo/tests/migrate.rs`, the `migrate-oikos` CLI variants and tests, and the `stack-check.sh` smoke step.
- `tools/ci/brand-check.sh`: one rule, `git grep -n -i oikos -- . ':!docs/superpowers' ':!tools/ci/brand-check.sh'` must be empty. Drop the capitalised-only rule, the `"oikos\."` wording rule and the allow-list loop. Delete `tools/ci/brand-allow.txt`. Keep the rule for the bird's-home word.
- App repo: add an equivalent one-line check to its `ci.yml`. **Decided:** a `git grep -i oikos` step that excludes `docs/superpowers`.

- [ ] **Step 1: Delete and edit as listed.**
- [ ] **Step 2: Check.** Run: `git grep -n -i oikos -- . ':!docs/superpowers'`. Expected: no output except inside `brand-check.sh` itself, which is excluded by the check.
- [ ] **Step 3: Run the full check plus** `tools/ci/brand-check.sh`. Expected: green.
- [ ] **Step 4: Commit** (`chore: remove the one-time migration, absolute brand check`), push a branch from `origin/main`, and open the PR.
