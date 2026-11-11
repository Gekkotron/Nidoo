# Backup and restore — design

Date: 2026-11-11 · Status: draft, awaiting approval

## Why

Nidoo has no backup today. If the hub's disk dies, everything is gone: every flow, dashboard,
ambiance, label, setting, and years of history. The only advice is a hand-run `pg_dump` in
`docs/users/history.md`, which also misses everything stored as MQTT retained documents (flows,
dashboards, labels…).

The user's own case: the hub runs on a mini PC; a Raspberry Pi NAS on the LAN accepts plain FTP
uploads into a backup folder. If the mini PC dies, the user buys another one, installs Nidoo
fresh, and wants to be back where they were — flows **and** history — from the setup wizard,
without remembering any command.

Nidoo is used by others too, so the feature is generic: FTP first, plus a browser download for
people with no NAS, and room for more destinations later.

## Goals

- One **backup file** format holding everything needed to bring a house back: settings,
  retained configuration documents, and the full history.
- The backup is **always encrypted** with a backup password the user sets.
- Two destinations in this version:
  1. **FTP** server (host, port, login, password, remote folder), with retention.
  2. **Browser download** of the same file from Settings.
- A **flow action** "Back up" that runs a backup to FTP and *waits until it ends*, so a nightly
  flow can wake a NAS → back up → shut the NAS down, with the NAS awake exactly as long as needed.
- A **Back up now** button, the **last backup** status (when, size, OK / failed + reason), and a
  **push notification** when a backup fails.
- **Restore from the setup wizard** on a fresh install: from FTP (list the backups found, newest
  first) or from an uploaded file; asks for the backup password; brings back settings, flows and
  history; ends on the usual home screen.

## Non-goals (this version)

- Other destinations: a folder on the hub host, SFTP, SMB, S3-compatible storage. The design must
  make each one "a new way to send/list/fetch a file", not a new format.
- FTPS / TLS on FTP. Plain FTP only (the file itself is encrypted). Can come with SFTP later.
- Restoring into a running, already-configured hub from Settings (rollback). Restore exists only in
  the setup wizard.
- Partial restore (only flows, only one dashboard…).
- Incremental / differential backups. Every backup is a full one.
- A schedule built into the backup feature. Timing is done with flows (cron trigger + the action).
- Waking a sleeping NAS from the wizard. If the FTP server is unreachable, the wizard says so.
- Backing up things that are not Nidoo's: Zigbee2MQTT's own data, the Mosquitto password file,
  speech models under `data/assist/` (large, re-downloadable), TimescaleDB's on-disk files.

## Anti-goals (would be a failure even if it "works")

- A backup that **looks** fine but cannot be restored (half-uploaded file listed as a backup,
  missing retained documents, wrong password accepted silently, history silently truncated).
- A restore that **overwrites existing data** without the user asking for it.
- Any secret readable in the backup file without the password, or returned to the browser by the
  API (the backup password and FTP password are write-only, like every other Nidoo secret).
- A backup that stalls the hub: lights, flows and the UI keep working while it runs.
- Filling the hub's disk with a temporary copy of the whole backup.
- A file only Nidoo can open. On a very bad day the user must be able to open it with standard
  tools (see *File format*).

## Behaviour details (decided)

### What goes in a backup

| Part | Source | Notes |
|---|---|---|
| Hub file | `data/hub.json` | Only the house-level fields: `location`, `timezone`, `http`, `log`, `z2m_base`. **Not** `mqtt` / `database_url` / `step` / `finished`: those belong to the new machine. |
| Settings sections | `settings` table (`Sections`, all `SECTION_KEYS` + the new `backup` section) | Secrets included (push key, Solcast key, FTP password, backup password…), protected by the encryption. |
| Retained documents | Nidoo-owned retained MQTT topics that hold **definitions and user data**: flows, dashboards, ambiances, room/area/type/home overrides, flow labels, groups, network watches, climate zone *configuration*, alarm *settings*, push tokens and push history, solar cache | Captured as topic → payload. **Excluded: runtime state** the hub recomputes or that would be wrong on a new machine: alarm `state`/`panel` (a backup taken while armed must not restore an armed alarm), climate `proposal`/`decision`/`override`/`eco`, virtual/group/network *states*, weather/solar/system values, every discovery config the hub republishes itself. *(Design call.)* The list comes from one place in code, and `mqtt-topics.md` gets a "Backed up" column; a test checks every retained Nidoo topic is explicitly marked yes or no, so a future topic cannot be forgotten silently. |
| History | Every table of the hub's database, including hypertables (`entity_state`, `entity_availability`, `climate_decision`) and plain tables | Row-level export, see *Database export*. |
| Manifest | generated | Format version, Nidoo version, migration level, creation time, hub name/host, row counts and byte sizes per part. |

### File format

- An **age**-encrypted file (passphrase / scrypt recipient) of a **zstd**-compressed **tar**
  stream. File name: `nidoo-backup-YYYYMMDD-HHMMSS.tar.zst.age` (UTC).
  On a bad day: `age -d f.tar.zst.age | zstd -d | tar x` opens it. Mention this in the user docs.
- Inside the tar: `manifest.json`, `hub.json`, `settings.json`, `retained.json` (or one file per
  topic, builder's call), `db/<table>.copy` per table, `db/schema.json` (table → column list).
  `manifest.json` comes first so a restore can read it before downloading the rest.
- The whole pipeline **streams**: DB export → tar → zstd → age → FTP upload (or HTTP response),
  with bounded memory and no full-size temp file.

### Database export

- No `pg_dump` (not in the runtime image, and a TimescaleDB logical dump ties the restore to the
  extension version). Instead, inside one `REPEATABLE READ` read-only transaction for a consistent
  snapshot, `COPY (SELECT <columns> FROM <table>) TO STDOUT` for each table (sqlx `copy_out_raw`).
  Reading a compressed hypertable through SELECT works and decompresses on the fly.
- Restore: on a database where migrations have already run (the wizard's Database step does it),
  `COPY <table> (<columns>) FROM STDIN` per table, then refresh anything derived (e.g. rollups) if
  needed. TimescaleDB's compression policy recompresses old chunks later on its own.
- Compatibility rule: a backup restores into **the same or a newer** Nidoo version. A backup with a
  newer format version or a newer migration level than the running hub is refused with a clear
  message. Columns added by newer migrations take their defaults; a column in the backup that no
  longer exists is an error the builder resolves per case (mapping in code) — never silently dropped.

### FTP destination

- Settings: host, port (default 21), username, password (write-only secret), remote folder, number
  of backups to keep (default **7**, minimum 1). Passive mode.
- Upload writes `<name>.partial`, then renames to `<name>` only after the last byte is accepted.
  Listings only show complete `nidoo-backup-*.tar.zst.age` files.
- After a successful upload, delete the oldest complete backups beyond the keep count. Never delete
  anything that does not match the backup file name pattern. Leftover `.partial` files older than a
  day are deleted too.
- **Test connection** button: connects, logs in, checks the folder exists and is writable (write and
  delete a small probe file), and reports which of those steps failed.

### Backup password

- Required to turn backups on (FTP or download). Set in the Backup settings page, typed twice.
  Stored as a write-only secret in the new `backup` settings section (the hub needs it to run
  unattended). Never returned by the API; `SecretView {set}` like other secrets.
- The page warns: "If you lose this password, your backups cannot be opened."
- Changing it affects new backups only; old ones keep their old password.
- Minimum length 8 (builder may add a strength hint, not a blocker).

### Running a backup

- One backup at a time, run as a **hub-level job** (the TTS download job is the pattern: single
  slot, conflict if busy, spawned task, cancel channel). A flow run being aborted (restart mode,
  flow disabled/deleted) does **not** cancel the backup.
- Triggered by: the flow action, **Back up now** (FTP), or **Download a backup** (browser).
- The browser download streams the file as the HTTP response; it uses the same job slot.
- Default time limit **2 hours** per backup (FTP), then it fails with "time limit reached".
- While it runs, the Backup page shows progress (current part, bytes so far). After it ends, the
  **last backup** record keeps: start, end, destination, file name, size, result, error detail.
  The record survives restarts (stored in the database or a small file under `data/`, builder's call).
- **Failure → push notification** "Backup failed: <reason>". A success sends nothing. It has its
  **own switch** in the Backup page, "Notify me when a backup fails", **on by default**, and does
  not follow the general system-notifications switch (`system.notify`, off by default), otherwise a
  default install would never hear about failing backups. *(Design call.)*
- Also exposed as a system sensor (e.g. `system_last_backup` timestamp + a problem binary sensor),
  so dashboards and flows can use it. Builder may keep this to one entity if two is awkward.

### Flow action `backup.run`

- New node kind, title "Back up" (FR "Sauvegarder"). Params: `timeout_minutes` (default 120).
  Destination is FTP (the only unattended one in this version); keep a `destination` param with a
  single value `ftp` so future destinations don't change the schema.
- Starts a backup job, or **joins the one already running**, and waits for it to end.
- It has **two output ports, `ok` and `failed`**, like the Ask node. *(Design call:* flows stop at
  the first failed step and have no failure path, so without ports a failed backup would leave the
  NAS awake.)* The step itself is recorded as `Ok` in both cases, so the run continues on the
  matching port:
  - `ok`, detail "Backed up <size> to <host> as <file>".
  - `failed`, detail with the reason (FTP unreachable, login refused, no backup password set, FTP
    not configured, time limit, …).
  The user wires both ports to "shut down the NAS" (plus a notification on `failed` if wanted).
  This touches the `nidoo-core` node model, validation and the editor card beyond the `flow.label`
  template; follow how Ask declares its ports.
- Save check: refuse saving a flow with this action when backups are not configured? **No** —
  allow it, fail at run time with a clear detail (the user may configure FTP afterwards).

### Restore in the setup wizard

- The Welcome step gets a second path: **Restore from a backup**. The normal path is unchanged.
- The restore path goes Welcome → MQTT → Database (unchanged, pre-filled from the bundled stack) →
  **Restore** → Done. It skips Devices and Services: the backup brings those.
- **Restore-pending mode** *(design call)*. Today, completing the Database step makes `hub.json`
  complete and restarts the hub into full Run mode, which immediately publishes its own retained
  topics and writes history rows, so "the target already holds data" would always be true. When
  the user picks the restore path, `hub.json` records `restore: pending`. A hub whose file is
  complete but has `restore: pending` boots in a restricted mode: HTTP (setup screens and restore
  routes) and a database pool only. No flow engine, history writer, system task, discovery, or
  stores publishing on MQTT. The restore job opens its own plain MQTT connection to publish the
  retained documents. The flag is cleared when the restore succeeds, then the hub restarts into
  normal Run mode. The user can abandon the restore path, which clears the flag and returns to the
  normal setup.
- The Restore step:
  1. Choose **From an FTP server** (host, port, login, password, folder → **Find backups**) or
     **Upload a file**.
  2. FTP: lists backups found, newest first, with date and size; newest pre-selected.
     Unreachable server → "Can't reach the FTP server. Is it switched on?" with a retry.
  3. Asks the backup password. A wrong password is detected from the manifest, before anything is
     written, and says "Wrong password".
  4. Shows what the backup holds (date, Nidoo version, flows/dashboards count, history size) and a
     **Restore** button.
  5. Refuses if the target database already has history rows or the broker already holds Nidoo
     retained documents, and explains why (anti-goal: no silent overwrite). No "force" option in
     this version.
  6. Restores in **one pass**, in file order: settings + hub file fields → retained documents →
     history, then clears `restore: pending` and **restarts the hub** once. *(Design call:* an
     uploaded file is a single stream and cannot continue across a restart, and a temp copy on disk
     is an anti-goal. So FTP and upload follow the same single pass, and the house comes back when
     the history is loaded too.)* A progress bar shows the current part and bytes; the history is
     the long part.
  7. When the FTP settings in the backup match the source used for the restore, they're kept, so
     nightly backups continue without re-typing. The backup password stays the same.
- If the restore fails partway, the hub stays in restore-pending mode. **Retry** truncates what was
  written (settings rows, history tables, and clears the retained documents it published) and runs
  the whole pass again: from FTP it downloads again; from an upload the user picks the file again.
  "The target already holds data" (step 5) is checked only at the start of the first attempt.

## HTTP API (hub)

All under `/api/v1/`, same no-auth LAN model as the rest. Shapes left to the builder, following the
`nidoo-api` wire-type pattern.

- `GET/PUT /settings/backup` — the new section (FTP fields, keep count, password as `Secret`).
- `POST /backup/test` — FTP test connection.
- `POST /backup/runs` — start a backup to FTP; `409` if one is running.
- `GET /backup/current`, `DELETE /backup/current` — progress, cancel.
- `GET /backup/last` — last backup record.
- `GET /backup/download` — runs a backup and streams the file.
- Setup routes for the wizard: list FTP backups, read a manifest (with password), start a restore
  from FTP or from a streamed upload (raw streamed PUT like the TTS upload, no multipart), restore
  progress.

MCP: `run_backup` and `get_last_backup` tools are welcome but optional in this version.

## Web UI

- **Settings → Backup** (new `SettingsPage`, between System and Database, icon of the builder's
  choice): backup password, FTP block with Test connection, keep count, **Back up now**,
  **Download a backup**, progress while running, last backup record.
- **Flow editor**: the "Back up" action card in the library, with its timeout field.
- **Setup wizard**: the second path on Welcome and the Restore step described above.
- All text through FR/EN i18n (the `bare_text` guard).

## Docs

- `docs/users/backup.md` (new): what's in a backup, setting up FTP, the backup password warning,
  the recommended nightly NAS flow (wake → wait for online → Back up → shut down), restoring on a
  new machine, opening a backup by hand with `age`/`zstd`/`tar`.
- Replace the `pg_dump` section of `docs/users/history.md` with a link to it; update the "Back up
  `./data/`" line in `install-on-linux.md`.
- Reference: `http-api.md`, `flow-nodes.md`, `configuration.md` (new section), `mcp-tools.md` if
  the MCP tools are added.

## Constraints

- Rust workspace conventions: `cargo test --workspace`, clippy `-D warnings`, fmt; `nidoo-core`,
  `nidoo-api` and `nidoo-web` keep building for `wasm32` (no FTP/crypto crates in those).
- New dependencies expected in the hub crate only: an async FTP client (e.g. `suppaftp` with tokio),
  `age`, `zstd` (or `async-compression`). Check licences and that they build in the Docker image.
- Retained documents stay backward compatible; new node kind follows the `flow.label` template.
- Public repo: generic names only, nothing about the author's own NAS or network.
- The hub keeps serving while a backup runs (DB pool has 4 connections: the export uses one).

## Left to the builder

- Exact crate choices, tar entry layout, COPY text vs binary format, chunk sizes.
- Where the last-backup record lives; exact system sensor ids.
- How retained documents are read at backup time (from the hub's in-memory stores vs a fresh
  subscription to the broker) — whichever guarantees completeness.
- Progress granularity and the cancel UX.
- Icon, exact wording (keep the warning about the password).

## Follow-up for the author's install (not code)

Once shipped, change the "NAS : sauvegarde nocturne" flow from two fixed crons (wake 22:45,
shut down 23:15) to: cron 22:45 → wake NAS → wait until the NAS is online → **Back up** → both
`ok` and `failed` → shut down the NAS.
