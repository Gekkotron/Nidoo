# Zigbee2MQTT data in the backup — design

Date: 2026-11-12 · Status: draft, awaiting approval · Extends
[`2026-11-11-backup-design.md`](2026-11-11-backup-design.md)

## Why

The backup design left Zigbee2MQTT's own data out (non-goal). But if the machine running Z2M dies,
losing that data means re-pairing every Zigbee device: the network key, the coordinator backup and
the device database are what let a new Z2M pick the existing network back up. The user wants that
data on the FTP server every night, next to the Nidoo backup.

It must stay **generic**: any Nidoo user, whatever way Z2M is installed (Docker, bare metal, another
machine), with no file access from the hub to Z2M's folder.

## Goals

- An optional switch in Settings → Backup, **"Include Zigbee2MQTT data"**, that adds Z2M's own
  backup to every Nidoo backup (FTP, Back up now, browser download, flow action).
- The hub gets the data **over MQTT** from Z2M's built-in backup request
  (`<z2m_base>/bridge/request/backup` → `<z2m_base>/bridge/response/backup`, whose `data.zip` is
  the base64 zip of Z2M's data folder, logs excluded). `z2m_base` is the one in `hub.json`.
- The zip goes **inside the encrypted Nidoo backup file**: the Zigbee network key it holds is never
  on the FTP server in clear.
- When Z2M does not answer, the backup **is still saved** with Nidoo's data, and ends as a new
  third outcome, **partial**: warning push, warning on the Backup page, and a `partial` port on
  the flow card.
- The setup wizard tells when a backup holds Z2M data and offers it as a **download**, and the docs
  explain how to put it back.

## Non-goals

- Nidoo restoring Z2M by itself. Z2M has no restore request over MQTT; putting the zip back is a
  manual step (stop Z2M, unzip into its data folder, start Z2M).
- Reading Z2M's data folder from disk, or backing it up on its own schedule / to its own file.
- A tool usable without Nidoo (a script on the Z2M host). Discussed and dropped: it would leave the
  network key unencrypted on plain FTP and give no failure notice.
- Several Z2M instances. Only the one at `z2m_base`.

## Anti-goals

- A backup reported **ok** that silently lacks the Z2M data while the switch is on.
- A Z2M problem making the **whole backup fail**: Nidoo's own data must still be saved.
- An existing nightly flow (only `ok` and `failed` wired) that **stops at the backup card** on a
  partial night and leaves the NAS awake.
- The network key readable anywhere without the backup password (FTP, logs, API, push text).

## Behaviour (decided)

### Setting

- New field in the `backup` settings section: `include_z2m` (bool). `BackupView` / `BackupInput`
  gain it. **Default on** for a section that has never stored it (serde default `true`, like
  `notify_failure`). *(Design call: the user did not object; anyone who uses Nidoo uses Z2M.)*
- Shown in Settings → Backup as a switch "Include Zigbee2MQTT data" with one line of help: "Keeps
  your Zigbee network (key, paired devices) so a new Zigbee2MQTT doesn't need re-pairing."

### Getting the data

- At backup time, with the switch on, the hub publishes on `<z2m_base>/bridge/request/backup` with
  a fresh `transaction` id and waits for the matching `<z2m_base>/bridge/response/backup`.
- **Wait limit: 60 s** *(design call)*. No answer, `status: "error"`, a missing/undecodable
  `data.zip` → the Z2M part is skipped with a reason (e.g. "Zigbee2MQTT didn't answer within 60 s",
  "Zigbee2MQTT refused: <error>"). The reason text never includes the payload.
- **Fetched before the archive starts** (the manifest is the first entry and must say whether the
  zip is there). A Z2M failure never aborts the archive. The zip is held in memory.
- **Packet size** *(design call)*. The hub's MQTT clients accept at most 4 MiB per packet
  (`MAX_INCOMING_PACKET` in `mqtt.rs`, `MAX_PACKET` in `backup/retained.rs`), and Z2M sends the
  zip base64-encoded in one publish, so a zip over ~3 MB would break the connection and look like
  "no answer". The request therefore uses **its own short-lived MQTT connection** with an incoming
  limit of **32 MiB**, which caps the zip at about **24 MB** — far above a typical Z2M data folder.
  The main connection's limit is unchanged. A response over the limit is a skip with the reason
  "Zigbee2MQTT's backup is too big (over 24 MB)", not a generic timeout. Mosquitto's default
  `message_size_limit` allows this; the docs mention it for brokers that set a lower one.
- **When Z2M is not there** *(design call)*:
  - The topic base always has a value (`topics.z2m_base`, default `zigbee2mqtt`), so "no Z2M" is
    read from the registry's bridge availability (`bridge/state`, retained by Z2M):
  - **Unknown** (this hub has never seen a Z2M bridge) → no request, not included, plain **ok**.
  - **Offline** → skip at once, no 60 s wait, outcome **partial** with "Zigbee2MQTT is offline".
    This is exactly the bad night the warning exists for.
  - **Online** → send the request and wait as above.
- With the switch off, no request is sent and the outcome is plain ok.

### In the file

- New tar entries `z2m/000001.zip`, `z2m/000002.zip`… (the zip as Z2M sent it, decoded from
  base64, cut at `MAX_ENTRY` = 8 MiB; concatenated they are the zip), placed **after `settings.json` and before `retained/`**
  so the wizard reaches it early when scanning a file. Exact layout is the builder's call.
- The `Manifest` gains `z2m: Option<Z2mManifest>` (`bytes`). `None` = not included (switch off, or skipped). Old backups without the field read as
  `None`. **`FORMAT` goes to 2** *(design call)* for every new file: an older hub refuses it with
  its existing "newer Nidoo" message instead of misreading it; a new hub still reads format 1.
- The restore reader (`restore_into` in `archive.rs`) rejects unknown entries between
  `settings.json` and `retained/` today; it must accept `z2m/…` there (and skip it for the
  restore itself, see *Restore wizard*).
- The archive header comment in `archive.rs` and the "opening a backup by hand" docs list the new
  entry.

### The partial outcome

The backup ended with the Nidoo data uploaded/streamed completely, but the switch was on and the
Z2M part was skipped.

- **Last backup record** (`LastBackup`): add a `warning: Option<String>` holding the skip reason
  (`ok` stays `true`, so `last_ok_at` still moves). *(Design call: a partial backup is a usable
  Nidoo backup; `ok` keeps meaning "the file is complete and restorable".)* Old records without
  the field read as no warning.
- **Push**: "Backup saved without Zigbee2MQTT data: <reason>", sent under the same switch as
  failures, renamed in the UI to **"Notify me when a backup fails or is incomplete"** (the field
  stays `notify_failure`). Applies to FTP backups, like the failure push. A browser download does
  not push (the user is looking at the page).
- **Backup page**: the last-backup line shows an amber warning with the reason instead of green.
  During a browser download the warning shows on the page when the job ends.
- **System sensor**: unchanged. There is only `system_last_backup` (last *successful* backup),
  which moves for a partial backup since its file is complete.
- **Flow action `backup.run`**: a third port **`partial`** (`BACKUP_PARTIAL_PORT`), detail "Backed
  up <size> to <host> as <file>, without Zigbee2MQTT data: <reason>". Step outcome `Ok`, like the
  other two.
  - **Fallback**: when the flow has **no edge on `partial`**, the run continues on **`ok`**. Old
    flows keep working unchanged; wiring `partial` is opt-in. The engine checks
    `flow.successors(node, Some("partial"))` is empty and returns `ok` instead.
  - The editor card shows the third port (label "Partial" / "Partielle", placed between `ok` and
    `failed`), and its help says it falls back to Ok when left unwired.
- **MCP** `get_last_backup`, if it exists, carries the warning.

### Restore wizard

- The summary step (`BackupSummary`) gains `z2m_bytes: Option<u64>`; when present the summary shows
  "Includes Zigbee2MQTT data (<size>). You can download it from Settings → Backup once restored."
- The restore pass keeps the zip: it writes `data/zigbee2mqtt-restored.zip` (mode 0600). Same for
  FTP and uploaded files, since an upload is a single stream that can't be read twice.
- After the restart, **Settings → Backup** shows a "Zigbee2MQTT data from the restored backup"
  card while that file exists: size, **Download** (`zigbee2mqtt-backup.zip`) and **Delete**
  (it holds the Zigbee network key in clear on the hub's disk, next to `hub.json`).
- The restore itself does **not** touch Z2M. The Done screen reminds: "To bring your Zigbee network
  back, put the Zigbee2MQTT data into its folder before starting it — see the backup guide."

## Docs

- `docs/users/backup.md`: what's in a backup now includes Z2M data (when the switch is on), remove
  it from "Things that are not Nidoo's", the partial outcome and its push, the `partial` port and
  its fallback, and a **"Putting Zigbee2MQTT back"** section: stop Z2M, unzip into its data folder
  (Docker bind mount or install dir), start it; by hand: `age -d … | zstd -d | tar x` then concatenate
  `z2m/*.zip` (`cat z2m/*.zip > zigbee2mqtt.zip`).
- `docs/users/zigbee2mqtt.md`: one line pointing to it.
- Reference: `flow-nodes.md` (third port), `http-api.md` (new fields), `configuration.md`
  (`include_z2m`), `mqtt-topics.md` (the hub now publishes `bridge/request/backup`).

## Constraints

- Same as the parent design: workspace tests, clippy `-D warnings`, fmt; `nidoo-core`, `nidoo-api`,
  `nidoo-web` stay `wasm32`-clean.
- Wire changes are additive and backward compatible (new optional fields, new port).
- The hub has no Z2M request/response helper today; this adds the first one. Keep it small and
  reusable (topic, transaction id, timeout).
- Public repo: generic wording, nothing about the author's own NAS.

## Left to the builder

- Where the request/response helper lives and how it subscribes (on its own connection, see
  *Packet size*).
- Icons, exact wording beyond the strings above (FR/EN via i18n).

## Follow-up for the author's install (not code)

Nothing to change in the nightly NAS flow: `partial` falls back to `ok`, which already shuts the
NAS down. Optionally wire `partial` to a notification, though the push already covers it.
