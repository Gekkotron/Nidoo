# Nidoo rename, phase 3: no "oikos" left anywhere — design

Date: 2026-11-01. Status: awaiting approval.

Phase 1 (#129) renamed the brand, and phase 2 (#130) renamed the crates, binary, containers and env vars. Phase 3 renames
everything that still says `oikos`: the MQTT namespace, entity and device ids, other values on the wire, the database,
browser storage keys, the leftover `OIKOS_*` / `oikos.toml` fallbacks, the docs site, and the Android app, including
its own identity and its Firebase project.

## Why

The user chose Nidoo as the product's name (phase 1). Every leftover `oikos` is a seam: the user sees it in the broker,
the database, the MCP server name and the app, and a new user reading topics or ids in the docs sees two names for
one product. The user's ruling: **rename everything**.

## Facts that shape the design

- **One install.** The user's house is the only Nidoo hub running and the only app install. No one else upgrades from
  an Oikos version. So **no transition code**: no reading both prefixes, no id aliases, no browser-key fallback. A
  one-time migration moves the house's data, and a follow-up PR deletes it.
- **Downtime is acceptable.** The hub can be stopped for the switch-over (about 15–30 min). The app shows nothing
  between stopping the hub and installing the new app.
- **Live sweep (2026-11-01, read only):**
  - **Broker:** about 340 retained messages under `oikos/`:
    - 168 `oikos/ui/*`
    - 37 flows
    - weather and solar configs and states
    - climate zones
    - groups and `group/_migrated`
    - virtuals
    - alarm
    - `push/history`, `push/tokens/*`
    - `presence/<person>`
    - `voice/shortcut/*` (the app publishes these)
    - `device/water_heater/config` and `device/ds3_*/config`, published by hand per `docs/users/water-heater-controller.md`
  - Nothing is retained under `oikos-push/`.
  - Two retained messages outside `oikos/` carry the name:
    - `homeassistant/alarm_control_panel/alarm/house/config`, a hand copy of the hub's alarm config.
    - `homeassistant/device_tracker/oikos_<person>/config`, published by the app.
  - Payloads inside `oikos/` refer to `oikos_weather` (26 times), `oikos_solar` (10), `oikos_virtual` (8), `oikos_alarm_house` (7), `oikos_group` (3), `oikos_presence_<person>` (2) and `oikos_alarm` (1). Some flows refer to `oikos_alarm_house`.
  - **Database** (PostgreSQL 17.2, TimescaleDB 2.17.2; database `oikos`, user `oikos`, the only role, superuser, SCRAM password):
    - `oikos_*` ids in `entity_state` (76 rows, 2026-10-21 → today), `history_hourly` (22), `entity_latest` (2), `entity_availability` (1) and `availability_latest` (1). `climate_decision` has none.
    - The ids are only `oikos_alarm_house` and `oikos_presence_<person>`. Weather and solar sensors have non-`oikos` entity ids: only their *device identifiers* say `oikos_*`.
    - `entity_state` compresses after 7 days (40 of 42 chunks are compressed), and `entity_availability` and `climate_decision` after 30.
    - The `settings` table overrides the config defaults. `advanced` holds `"oikos"`, `"oikos/ui"`, `"oikos/automation"` and `"oikos/alarm"`. `climate` holds `"oikos/climate"`. `push` holds `"oikos/push"` and the current Firebase service account (`legacy_prefix` is already `""`).
  - The hub's database URL lives in `<data>/hub.json` (`database_url`). The live container may also set it through an env var in its compose override; read that at switch-over.
  - The house runs its **own** database container, named `oikos-timescaledb`, outside Nidoo's compose. The hub is updated with `tools/update.sh --hub-only`.
- **Repo sweep:** about 1,500 hits in 117 files, mostly tests, fixtures and docs. The full inventory lives in the
  session notes. The categories and their key places are listed under "What changes" below.
- **Android app** (`Gekkotron/Oikos-app`, Kotlin): the namespace part is already done, on branch
  `refactor/nidoo-namespace`, commit `263332a`, local and not pushed. The app now reads and publishes only `nidoo/…`.
  It publishes presence as `homeassistant/device_tracker/nidoo_<person>/config` with unique_id
  `nidoo_presence_<person>`. A saved profile prefix `oikos` or a push topic `oikos/…` is read as the `nidoo`
  equivalent. The old `oikos-push` cleanup is gone. The app's own identity (name, package, classes, link scheme) is
  not done yet.

## Goals

- After the switch-over, a case-insensitive search for `oikos` finds nothing in:
  - the hub repo, outside `docs/superpowers/` (history)
  - the app repo, outside its own `docs/superpowers/` history
  - the live broker's retained messages
  - the live database: names, roles, rows and settings
  - this machine's MCP configuration
- The house behaves as before the switch-over:
  - the same flows, overrides, dashboards, ambiances, groups, virtuals, climate zones, alarm state and settings
  - history for the renamed ids is continuous
  - push notifications work from the new Firebase project
- A fresh install from `main` creates only `nidoo` names (database, user, Mosquitto user, topics, ids).
- CI keeps it that way: the brand check fails on any `oikos`, in any case, outside the history folders.

## Non-goals

- No compatibility with an Oikos-named install other than the user's. Nothing reads old names after the switch-over.
- No renaming of local folders (`~/superconductor/projects/Oikos-os`, `~/Documents/SourcesPerso/Oikos`). They are outside the repos, so this is the user's call, and it is left alone.
- `docs/superpowers/**` in either repo is never edited (history).
- No Play Store publication. If it ever happens, the listing name is "Nidoo Home", because a Spanish household-budget app is already called Nidoo.

## Anti-goals (failure even if it "works")

- Losing house data: a flow, an override, a dashboard, a group definition, a climate schedule, history rows, or a setting.
- A half-renamed house, for example the hub on `nidoo/` while some retained config is still on `oikos/` and silently ignored. The migration verifies before it clears anything.
- A personal name, room name, hostname or IP address in a tracked file: the project rule. Fixtures and examples use generic names (`<person>`, `phone`).
- Writing the English word for a bird's home anywhere tracked. `tools/ci/brand-check.sh` already enforces that.
- Any live action (broker, database, house server, Firebase, GitHub repo rename) without the user's explicit go at that step.

## The mapping (decided)

| Old | New |
|---|---|
| Topic prefix `oikos/…` (every subtree: ui, automation, climate, alarm, push, group, network, virtual, solar, weather, presence, voice, device, discovery of hub entities) | `nidoo/…` |
| `oikos-push/…` legacy topics and `legacy_prefix` | removed (no setting, no subscription) |
| Entity / unique ids `oikos_*` (`oikos_alarm_house`, `oikos_presence_<person>`, …) | `nidoo_*` |
| Device identifiers `oikos_alarm`, `oikos_group`, `oikos_virtual`, `oikos_weather`, `oikos_solar`, `oikos_network_<id>` | `nidoo_alarm`, `nidoo_group`, `nidoo_virtual`, `nidoo_weather`, `nidoo_solar`, `nidoo_network_<id>` |
| MQTT `client_id` default `oikos-os` | `nidoo` |
| `WEB_INSTALL_ID` `oikos-web` | `nidoo-web` |
| MCP resource `oikos://snapshot`; MCP server name `oikos` in the Settings snippets, README, website and docs | `nidoo://snapshot`; `nidoo` |
| Database name, user, bundled Mosquitto user `oikos` (`BUNDLED_NAME`, wizard defaults, compose, CI `TEST_DATABASE_URL`, `stack-check.sh`, test helpers) | `nidoo` |
| Browser keys `oikos.theme`, `oikos.lang`, `oikos.setup.skipped`, `oikos.alerts.last_seen`, `oikos.flowList` (+ the `index.html` theme script) | `nidoo.*`, no fallback |
| Website CSS variables `--oikos-*` (26) | `--nidoo-*` |
| `OIKOS_*` env fallback, `legacy_data_dir`, the `oikos.toml` import / default path / `update.sh` move | removed |
| Comments and docs mentioning OikosPush / OikosHistory (the old Python services) | reworded or removed; `brand-allow.txt` loses them |
| App: name "Oikos", package `com.gekkotron.oikos`, `Oikos*` classes, `oikos.*` Gradle plugins, resources, `oikos://` link scheme, keystore and WorkManager names, CI artifact and release keystore names | "Nidoo", `com.gekkotron.nidoo`, `Nidoo*`, `nidoo.*`, `nidoo://`, … |
| App repo `Gekkotron/Oikos-app` | `Gekkotron/Nidoo-app` (GitHub redirects the old URL) |
| Firebase project (old id contains `oikos`, ids cannot be renamed) | a new project `nidoo`, the app registered as `com.gekkotron.nidoo` |

Ids that don't start with `oikos` keep their form: `group_*`, `virtual_*`, `network_*`, `weather_*` and Zigbee ids.

## What changes

### 1. Hub code (one PR, merged before the switch-over, deployed on the switch-over evening)

- **Defaults:** every place that holds an `oikos` default:
  - `crates/nidoo/src/config.rs` (topics, push, climate, alarm, client_id)
  - `crates/nidoo-core/src/registry/mod.rs:60-61`
  - `crates/nidoo/src/hub.rs:374` (the discovery-prefix fallback)
  - the setup wizard JSON in `crates/nidoo-web/src/setup/services/mod.rs`
  - `crates/nidoo/src/mcp/mod.rs:358`
- **Hard-coded literals:** `climate/proposals.rs:378` compares against `"oikos/push/history"`. It must use the configured push prefix, not a new literal.
- **Wire constants:** `ALARM_ENTITY_ID`, the device identifiers, `GROUPS_DEVICE`, `WEATHER_DEVICE`, `WEB_INSTALL_ID`, `SNAPSHOT_URI` and `BUNDLED_NAME`. The web's alarm settings screen also refers to the alarm id.
- **Push:** drop `legacy_prefix` and everything that serves it: `nidoo-push/src/service.rs` legacy filters and routing, `mqtt.rs` `static_filters` handling for it, tests and docs.
- **Env and config file:** remove `LEGACY_ENV_PREFIX`, `legacy_env_in_use`, `legacy_env_provider`, the `OIKOS_*` merging, `legacy_data_dir`, the warning in `main.rs`, the `oikos.toml` import path (`setup/import.rs`, `setup/bundled.rs`, `setup/boot.rs`), the `Config::load` default path, the `update.sh` move to `data/oikos.toml`, and `restart.sh`'s use of `oikos.toml` and `OIKOS_LOG`.
  - **Builder's call:** whether `Config::load` keeps a default file name at all (for example `nidoo.toml` for dev runs) or `--config` becomes the only way. The user's local dev runs use the repo-root `oikos.toml`. On switch-over day it is renamed or replaced to match whatever the builder picks.
- **Web:** storage keys, the theme script, the MCP snippets and the user-facing strings that name `oikos.toml` (weather, solar).
- **Tools and CI:** `record-fixtures.sh`, `stack-check.sh`, `ci.yml`, `update.sh` and its test (the old `oikos-*` container switch-over logic from phase 2 can go: the live hub already runs `nidoo-hub`), `.gitignore` and `.dockerignore`.
- **Tests and fixtures:** rename mechanically, including the fixture file `device_tracker__oikos_phone.json`, `legacy-oikos.toml` (deleted with the import) and `topics.txt`. Tests for removed features are deleted, not renamed.
- **Docs and site:** every `docs/` page (the heaviest are `reference/mqtt-topics.md`, `users/push-notifications.md`, `reference/configuration.md` and `users/install-on-linux.md`), the README and `website/`.
  - The "Upgrading from `oikos.toml`" section and the "Switching over from OikosPush" section are removed, and their inbound links fixed.
  - The hand-published water heater discovery example moves to `nidoo/device/…`.

### 2. One-time migration command (in the same hub PR; deleted in the cleanup PR)

A hub subcommand, for example `nidoo migrate-oikos`. The name is the builder's call, and it may contain `oikos` because it is deleted afterwards. It runs with the hub stopped, against the live broker and database, and has three modes: `--dry-run` (the default: print every change, change nothing), `--apply`, and a final `--verify`. Running it twice is safe: a second `--apply` finds nothing to do.

**Broker, in this order:**
1. Read every retained message under `oikos/#` and `oikos-push/#`, plus the two known `homeassistant/` exceptions. As a general rule, include any retained `homeassistant/#` message whose *payload* contains `oikos`.
2. For each one, compute the new topic and the new payload:
   - **Topic:** a leading `oikos/` becomes `nidoo/`, and **every topic segment** that starts with `oikos_` becomes `nidoo_` (for example `oikos/ui/oikos_alarm_house` → `nidoo/ui/nidoo_alarm_house`, `homeassistant/device_tracker/oikos_<x>/config` → `…/nidoo_<x>/config`).
   - **Payload (decided: one blanket rule):** walk the JSON, both **every string value and every object key**, and replace `oikos` → `nidoo`, `Oikos` → `Nidoo`, `OIKOS` → `NIDOO`. That covers topics, ids, identifier arrays, `via_device`, templates, availability topics, device names and user-authored text (a flow name or notification text that says "Oikos" becomes "Nidoo", which matches "rename everything"). A plain byte replace on the raw JSON is not enough, because it would also hit escaped content.
   - A non-JSON payload (plain text) gets the same three replacements.
   - `--dry-run` lists **every** changed string (topic, JSON path, old → new), so the user reviews the user-authored ones before `--apply`.
3. **Push tokens are not copied.** They belong to the old Firebase project and are useless. The new app registers a fresh token on connect. The web UI does not use Firebase: `WEB_INSTALL_ID` is only the install id the hub gives replies sent from the web, so there is nothing to register for it. Push history is copied.
4. Publish all new retained messages, then read them back and compare.
5. Only after every new message is confirmed: clear the old topics (empty retained payload).
6. `--verify` (and the end of `--apply`): no retained topic under `oikos/` or `oikos-push/`, and no retained payload anywhere containing `oikos`.

**Database, in this order:**
1. Back up first. The command checks that a dump exists or makes one, at the builder's choice. The switch-over checklist also takes a `pg_dump` to the external disk before anything runs.
2. In one transaction, run `UPDATE … SET entity_id = 'nidoo_' || substr(entity_id, 7) WHERE entity_id LIKE 'oikos\_%'` on `entity_state`, `history_hourly`, `entity_latest`, `entity_availability`, `availability_latest` and `climate_decision`, plus any table that later holds entity ids (look it up from the schema, not hard-coded).
   - Compressed chunks: TimescaleDB 2.17 allows `UPDATE` on compressed chunks. The builder confirms that on a copy and decompresses the affected chunks if it does not.
   - A target id that already exists, because the new hub ran before the migration, is merged or reported, never duplicated.
3. Rewrite `oikos` values in `settings` JSON (the topic prefixes in `advanced`, `climate` and `push`), JSON-aware. The old Firebase service account inside `push` is replaced by hand later (see the switch-over), not by the command.
4. Rename the database and the role. PostgreSQL refuses to rename the session's own user, and `oikos` is the only role, so the command does this (decided):
   - Connected as `oikos`, it creates a temporary superuser role with a random password.
   - It reconnects to the `postgres` database as that role. With no other sessions open (the hub is stopped), it runs `ALTER DATABASE oikos RENAME TO nidoo` and `ALTER ROLE oikos RENAME TO nidoo`.
   - It reconnects as `nidoo` and drops the temporary role.
   - If a run stops halfway, the next run finds the temporary role, reuses it, and finishes.
   - The current password is SCRAM, which survives a role rename (PostgreSQL only clears MD5). The command checks that `nidoo` can log in with the old password, and re-sets it if not.
5. Update the hub's `<data>/hub.json` `database_url` (user and database name). The env var in the server's compose override is updated by hand in the switch-over checklist.
   - **Builder's call:** whether this step lives in the command or in the checklist.
6. `--verify`: no database, role, row or `settings` value contains `oikos`.

**Failure behaviour:** every step reports what it did. A failure stops the command without clearing anything not yet
verified, and re-running resumes. The broker's old messages are cleared only after the new ones are confirmed. The
database steps are transactional, except the renames, which are idempotent.

### 3. Android app (`Gekkotron/Oikos-app` → `Nidoo-app`)

- Start from branch `refactor/nidoo-namespace` (commit `263332a`). Keep its namespace changes and the saved-profile migration.
- Rebrand on top of it:
  - `app_name` "Nidoo" (EN and FR), including the onboarding strings
  - `applicationId` and every Gradle `namespace` `com.gekkotron.nidoo`
  - Kotlin packages (`git mv`)
  - `Oikos*` classes → `Nidoo*`
  - `oikos.*` convention plugins and `rootProject.name`
  - drawable, colour and widget names
  - the `oikos://` link scheme → `nidoo://`
  - the keystore secrets alias, the geofence request id and the WorkManager names
  - the CI artifact and release keystore names. The keystore *file* is the user's: rename only the references, and leave the signing key itself to the user.
  - README and docs (outside `docs/superpowers/`)
- The new package is a new app on the phone. The user reinstalls, sets the hub connection up again (QR or manual) and grants permissions again. Local settings and caches are not carried over.
- `google-services.json` comes from the new Firebase project, for `com.gekkotron.nidoo`.
- The same goal holds: no `oikos` left outside the app's `docs/superpowers/`.

### 4. Firebase

The user creates a new Firebase project (`nidoo`), registers the Android app `com.gekkotron.nidoo`, downloads
`google-services.json` for the app, and generates a service-account key for the hub. The builder writes the exact
console steps into the plan. The hub's key goes where it lives today: `secrets/firebase.json` and the `push`
settings, updated through the settings API or the UI. The old project is deleted by the user once push works from the
new one. It is a live step, so it needs the user's go.

### 5. Cleanup PR (after the switch-over succeeds)

- Delete the migration command and its tests.
- `tools/ci/brand-check.sh`: fail on case-insensitive `oikos` outside `docs/superpowers/`. Drop the capitalised-only rule and the `"oikos."` wording rule, and empty or delete `brand-allow.txt`. The fixtures exemption goes too, because fixtures are renamed.
- Apply the same check to the app repo, if it has one, or add a small equivalent: the builder's call.

## Switch-over evening (checklist; the user gives the go at each live step)

1. **Before:** the hub PR and the app PR are approved and CI is green. The new app APK is built and signed. The Firebase project exists, and its key and `google-services.json` are in hand. The hub image for the new `main` is ready to deploy.
2. **Back up:**
   - a TimescaleDB dump to the external disk
   - a full dump of the broker's retained messages, made by the migration command's own `--backup <file>` (decided: a lossless JSON file of topic, base64 payload, QoS and the retain flag). The command also has `--restore <file>`, which republishes it exactly. It is deleted with the command in the cleanup PR, after the backup file is safely on the external disk.
3. **Stop everything that writes `oikos/…`:**
   - Uninstall the old app, or at least force-stop it and switch off its presence. Otherwise it keeps republishing `oikos/presence/…`, its `oikos_<person>` tracker and voice shortcuts, which would recreate old topics after the clear.
   - Stop the hub (`nidoo-hub`).
   - The live sweep found no other publisher on `oikos/`: Zigbee2MQTT, Frigate and the other bridges use their own topics, and the water heater and DS3 configs were published by hand.
   - The hub's and the app's broker login does not contain `oikos` (checked), so no broker account changes.
   - Read the house broker's config for any ACL or topic rule naming `oikos/`. If one exists, add the `nidoo/` equivalent before step 6.
4. Run the migration with `--dry-run`, and the user reviews the summary. Then run `--apply`, then `--verify`.
5. Update the server: rename the database URL env var's user and database in the compose override, if set there. Rename the house's own `oikos-timescaledb` container to `nidoo-timescaledb`. Read its definition first, since it is owned outside Nidoo's compose (a `docker rename` or its own compose file). In that definition:
   - Change `POSTGRES_USER` / `POSTGRES_DB` to `nidoo`. They only act on an empty data directory, but leaving `oikos` would break a future re-init and keeps the name around.
   - Change any healthcheck such as `pg_isready -U oikos`, which would otherwise fail after the role rename.
   - Check for any `OIKOS_*` left in the env file.
6. Deploy the new hub (`tools/update.sh --hub-only`). Set the new Firebase key. Check the log for warnings.
7. Install the new app and set it up. Uninstall the old app. Delete the hand-made `homeassistant/` alarm copy if the app no longer needs it: the app now reads `nidoo/#` hub discovery, so it should not.
8. **Check:**
   - the web UI and MCP answer
   - flows are listed and enabled as before
   - the alarm arms and disarms from the app
   - presence updates
   - a push notification arrives on the phone (through the new Firebase project) and appears in the web UI's alerts
   - only one presence tracker exists, `nidoo_<person>`: the migrated copy and the new app's publish land on the same topic and id, so there is no duplicate
   - history shows the alarm and presence continuously
   - climate zones keep their schedules
9. Rename this machine's MCP entry `oikos` → `nidoo` in `~/.claude.json` (the user's go), then restart the session.
10. On GitHub, rename `Gekkotron/Oikos-app` → `Gekkotron/Nidoo-app` (the user's go). Merge the app PR.
11. Delete the old Firebase project once push has worked for a day (the user's go).
12. Open the cleanup PR.

**Rollback:** until step 4's clearing, the old messages still exist. After it, restore from the step 2 backups:
`--restore` the retained file after clearing the `nidoo/` copies, and restore the database dump. The old hub image and the old app APK are kept until
step 12.

## Constraints

- Commit identity Gekkotron: `-c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com`.
- Branch from `origin/main`, stage explicit paths, check `origin/main..HEAD` before pushing (shared worktree sessions).
- Long builds: run with `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0` per invocation. Check `df -h` and the shared cargo target size first, and clean if it is over 5 GB.
- The hub PR touches many files: check `gh pr list` for open PRs before it merges, to avoid conflicts.
- No personal names, rooms, hostnames or IPs in tracked files. Use `<person>` and generic fixtures.

## Left to the builder

- How to split the hub PR into commits or tasks, and whether docs and site go in the same PR.
- The migration command's name, structure, output format, and whether it reuses the hub's MQTT and database code.
- Whether `Config::load` keeps a default file name.
- Whether the `hub.json` URL update is in the command or the checklist.
- How the app's mechanical rename is done (IDE refactor or a scripted `git mv` plus replace), as long as the build, the tests and the no-`oikos` check pass.
- The exact Firebase console steps in the plan.
