---
title: Backup and restore
sidebar_position: 11.5
description: Back up the whole house (settings, flows, dashboards, history and Zigbee2MQTT's network data) to an FTP server or a download, run it every night from a flow, restore it on a new machine from the setup wizard, and open a backup file by hand.
---

A backup is one encrypted file that brings a house back: settings, flows, dashboards, the full history and, when you keep it on, your Zigbee network. Nidoo sends it to an FTP server (a NAS, for example) or hands it to your browser as a download. If the machine dies, you install Nidoo on a new one and restore from the setup wizard.

## What is in a backup

A backup holds:

- **Settings**: every **Settings** page, secrets included (push key, Solcast key, FTP password, backup password). The file is encrypted, so they are safe without the password.
- **House fields of `hub.json`**: location, time zone, web address, log level and the Zigbee2MQTT base topic.
- **Configuration kept on the broker**: flows, dashboards, ambiances, room and device overrides, flow labels, groups, network watches, climate zone settings, alarm settings, push tokens and push history, and the solar cache.
- **The history**: every table of the database, with all its rows.
- **Zigbee2MQTT's data**, when the **Include Zigbee2MQTT data** switch is on (it is on by default). This is the zip Zigbee2MQTT makes for its own backup: the Zigbee network key, the paired devices and the coordinator backup. See [Zigbee2MQTT data](#zigbee2mqtt-data).

A backup does not hold:

- **Runtime state** the hub works out again: the alarm's state (a backup taken while the alarm is armed never restores an armed alarm), climate decisions, weather and solar values, and the discovery entries the hub republishes itself. The [MQTT topics](../reference/mqtt-topics.md) page marks each topic **Backed up** yes or no.
- **The machine's own connections**: the broker and database addresses and passwords in `hub.json`. The new machine has its own.
- **Things that are not Nidoo's**: the Mosquitto password file, the speech models in `data/assist/` and the database's files on disk. The Zigbee2MQTT part is the zip of its data folder with the logs left out; nothing else from Zigbee2MQTT's folder is read.

Every backup is a full one. The file is named `nidoo-backup-YYYYMMDD-HHMMSS.tar.zst.age`, with the time in UTC.

## The backup password

Every backup is encrypted with a backup password. Set it in **Settings → Backup**, typed twice, at least 8 characters.

**If you lose this password, your backups cannot be opened.** There is no way to recover it. Keep it somewhere safe, outside the hub.

The hub stores the password so it can back up on its own. It is write-only, like every secret: the page shows only whether it is set. Changing it affects new backups only. Older files keep the password they were made with.

## Back up to an FTP server

In **Settings → Backup**, fill in the **FTP server** block: host, port (21 by default), user, password and folder. Use a name like `nas.local` and a folder like `/backups`. The connection is plain FTP in passive mode. The file itself is encrypted, but the login is not, so use it on your home network only.

Press **Test connection**. It tries four steps and shows which worked: **Connect to the server**, **Log in**, **Open the folder**, **Write a test file** (and delete it). Fix the first step that fails.

**Backups to keep** (under **Retention**) is how many files stay on the server. The default is 7 and the lowest is 1. After each good backup, the hub deletes the oldest files beyond that number. It only ever deletes files named like a backup.

A backup is first uploaded as `<name>.partial` and renamed only when the last byte is in. A half-sent file is never listed as a backup and never counts toward the number to keep. A leftover `.partial` file older than a day is deleted.

**Notify me when a backup fails or is incomplete** is on by default. A failed FTP backup sends a push "Backup failed" with the reason, and an FTP backup saved without its Zigbee2MQTT data sends a push "Backup incomplete" (see [Zigbee2MQTT data](#zigbee2mqtt-data)). A good backup sends nothing, and neither does a backup you cancel. This switch does not depend on the system notifications switch. See [Push notifications](push-notifications.md).

## Back up now, or download

- **Back up now** sends a backup to the FTP server. The page shows the part being written and the bytes so far, and a **Cancel** button.
- **Download a backup** makes the same file and streams it to your browser. Nothing is kept on the hub. If you close the download, the backup is cancelled.

Only one backup runs at a time. The hub keeps working meanwhile: lights, flows and the web UI do not wait for it.

A backup stops with "time limit reached" after 2 hours.

**Last backup** shows the date, destination, file, size and result of the last backup that ended, with the reason when it failed. A backup saved without its Zigbee2MQTT data shows an amber warning with the reason instead of a plain OK. It survives a restart.

The [system health](system-health.md#the-sensors) sensor `system_last_backup` ("Last backup") holds the time of the last **successful** backup, for dashboards and flows. A backup saved without its Zigbee2MQTT data is still a successful one: its file is complete and restorable.

## A nightly backup with a flow

The hub has no schedule of its own: a flow sets the time. The **Back up** card runs a backup to the FTP server and waits until it ends. This lets a flow wake a NAS, back up and shut the NAS down, so the NAS is awake only as long as needed.

1. A **Schedule** card, for example `45 22 * * *` (22:45 every day).
2. A **Command** card that wakes the NAS.
3. A card that waits until the NAS is online: a **Delay**, or a [network](network-presence.md) watch on the NAS with a **When** card.
4. A **Back up** card.
5. A **Command** card that shuts the NAS down, wired to the `ok`, the `partial` and the `failed` ports of **Back up**, so the NAS goes off whatever the result.
6. A **Notify** card on the `failed` port, to hear about a failure.

The **Back up** card has three outputs, `ok`, `partial` and `failed`. All three count as a finished step: the run goes on along the port that matches. A flow is not stopped by a failed backup.

- `ok`: the backup is complete, Zigbee2MQTT data included when it was on.
- `partial`: the backup is complete for Nidoo, but its Zigbee2MQTT data was left out (see [Zigbee2MQTT data](#zigbee2mqtt-data)). If nothing is wired to `partial`, the run goes on along `ok`, so an existing flow that wires only `ok` and `failed` keeps working and still shuts the NAS down on a partial night. The card's help reads: "Partial: saved without Zigbee2MQTT data. Left unwired, it goes on as Backed up."
- `failed`: the backup did not save.

Its **Give up after (minutes)** setting (`timeout_minutes`) is how long the card waits, from 1 to 1440 minutes (120 by default). If a backup is already running when the card starts, the card joins it and waits for it. If the limit passes first, the card takes `failed` with "time limit reached" and the backup keeps running. A flow that is disabled or restarted while it waits does not stop the backup.

You can add the card before FTP is set up. It then takes `failed` at run time, with the reason ("FTP not configured" or "no backup password set"). See [Flows](flows.md) and the [`backup.run` reference](../reference/flow-nodes.md#backuprun).

## Zigbee2MQTT data

Zigbee2MQTT keeps your Zigbee network in its data folder: the network key, the coordinator backup and the list of paired devices. Lose that folder and every Zigbee device has to be paired again. The backup keeps a copy of it, so a new Zigbee2MQTT can pick the existing network back up.

The switch is **Include Zigbee2MQTT data** in **Settings → Backup**, with the help "Keeps your Zigbee network (key, paired devices) so a new Zigbee2MQTT doesn't need re-pairing." It is on by default. With it on, each backup asks Zigbee2MQTT for its own backup over MQTT, under its base topic (`zigbee2mqtt` by default, the `[topics] z2m_base` setting), and puts the zip inside the encrypted backup file. The key is never on the FTP server in clear.

What happens at backup time:

- **Zigbee2MQTT is not on this hub at all** (the hub has never seen a Zigbee2MQTT bridge): nothing is asked and the backup is a plain **ok**.
- **Zigbee2MQTT is offline**: nothing is asked and the backup is **partial** at once, with "Zigbee2MQTT is offline".
- **Zigbee2MQTT is online**: the hub asks and waits up to 60 seconds. If it does not answer in time, or refuses, the backup is **partial** with the reason.

The reasons a backup can be **partial** with are:

| Reason | Meaning |
| --- | --- |
| Zigbee2MQTT is offline | The bridge was offline when the backup started. |
| Zigbee2MQTT didn't answer within 60 s | The request got no answer in time. |
| Zigbee2MQTT refused: `<error>` | Zigbee2MQTT answered with an error. |
| Zigbee2MQTT's backup is too big (over 24 MB) | The zip is larger than the hub takes. A typical Zigbee2MQTT data folder is far below this. |
| Zigbee2MQTT's answer could not be read | The answer was not a valid backup. |
| broker: `<error>` | The hub could not talk to the broker on the request's connection. |

The hub takes the zip on its own short-lived MQTT connection, which accepts packets up to 32 MiB. The broker must allow a message that size: Mosquitto's default `message_size_limit` does, but a broker set to a lower `message_size_limit` must be raised to at least 32 MB for Zigbee2MQTT's backup to reach the hub.

A partial backup is still saved. Nidoo's own data is complete, and only the Zigbee2MQTT part is missing. It is reported as:

- **A push**, for an FTP backup, under the **Notify me when a backup fails or is incomplete** switch: title "Backup incomplete", body "Backup saved without Zigbee2MQTT data: `<reason>`". A browser download sends no push, since you are looking at the page; its warning shows on the page when the download ends.
- **The Last backup line**, as an amber "Saved without Zigbee2MQTT data: `<reason>`".
- **The `partial` output** of the **Back up** card, as described above.

A partial night is the one the warning exists for. Once Zigbee2MQTT is back online, run **Back up now** to get a complete backup.

### Putting Zigbee2MQTT back

After a restore, the Zigbee2MQTT data is kept on the hub, not restored into Zigbee2MQTT. Nidoo does not restore Zigbee2MQTT by itself: you put the data back by hand, while Zigbee2MQTT is stopped.

1. Restore Nidoo from the backup (see [Restore on a new machine](#restore-on-a-new-machine)). The restore summary says "Includes Zigbee2MQTT data (`<size>`). You can download it from Settings → Backup once restored. To bring your Zigbee network back, put the Zigbee2MQTT data into its folder before starting it — see the backup guide."
2. Open **Settings → Backup**. The card **Zigbee2MQTT data from the restored backup** shows the size and two buttons: **Download** and **Delete**. **Download** asks for your backup password (the zip holds your Zigbee network key); type it and press **Confirm**, and the browser saves `zigbee2mqtt-backup.zip`. A wrong password shows "Wrong password" and nothing is sent.
3. Stop Zigbee2MQTT.
4. Unzip `zigbee2mqtt-backup.zip` into Zigbee2MQTT's data folder, replacing the files there. That is the folder mounted as `/app/data` in Docker, or the `data/` folder of a bare-metal install.
5. Start Zigbee2MQTT. Its devices come back without pairing again.
6. Once Zigbee2MQTT is back, press **Delete** on the card. The zip holds your Zigbee network key in clear, so do not keep it.

Without the file from **Settings → Backup** you can take it out of the backup yourself. It is the same file as in any other backup, so the steps are the ones in [Open a backup by hand](#open-a-backup-by-hand): decrypt and decompress the file, then join the pieces of `z2m/` into one zip:

```sh
age -d nidoo-backup-20261111-224512.tar.zst.age | zstd -d | tar x
cat z2m/*.zip > zigbee2mqtt.zip
```

Run these in an empty folder, since `tar x` writes into the current one. Then unzip `zigbee2mqtt.zip` into Zigbee2MQTT's data folder as in step 4. Delete the extracted files when you are done: they hold the Zigbee network key in clear.

## Restore on a new machine

Restore lives in the setup wizard only. It is for a new, empty hub. It does not roll back a hub that is running.

1. Install Nidoo and open it in the browser. On the first screen, choose **Restore from a backup**.
2. Fill in the MQTT broker and the database, as in a normal setup. The Docker install pre-fills them.
3. On **Restore a backup**, choose **From an FTP server** (host, port, user, password, folder, then **Find backups**) or **Upload a file**.
4. From FTP, the backups found are listed, newest first, with date and size. The newest is chosen. If the server cannot be reached, the page says "Can't reach the FTP server. Is it switched on?" and you can try again.
5. Enter the password the backup was made with. A wrong password is caught before anything is written ("Wrong password").
6. The page shows what the backup holds: its date, the Nidoo version, the number of automations and dashboards, the history size and, when it holds one, the Zigbee2MQTT data. Press **Restore**.
7. A progress bar follows the work. The history is the long part. When it ends, the hub restarts and opens on the usual home screen. If the backup held Zigbee2MQTT data, it is now kept for download (see [Putting Zigbee2MQTT back](#putting-zigbee2mqtt-back)).

The hub restores in one pass. Until it succeeds, the hub runs in a restricted mode: only the setup screens and the restore, with no flows and no history written. **Set up without a backup** leaves this mode and goes back to the normal setup.

Some rules to know:

- **A hub that already holds data is refused.** The restore stops if the database already has history rows or the broker already holds flows or settings. Nothing is overwritten and there is no force option. Point the wizard at an empty database and broker.
- **A new database or broker starts the check again.** If you change the database or the broker in the wizard while restoring, the "already holds data" check is done again for the new one.
- **A newer backup is refused.** A backup restores into the same or a newer Nidoo, not an older one. A file made by a newer Nidoo, or with a newer database level, is refused with a message asking you to update first. Backups made before the Zigbee2MQTT data was added still restore.
- **A bad file is refused.** A file that is not a backup, or that is damaged or cut short, is refused. The restore counts the rows of each table against the file's manifest and fails on a mismatch, so a half history is never kept.
- **The database must be reachable.** Without it the restore answers "database unreachable".
- **A failed restore can be retried.** The hub stays in restore mode. A retry clears what the first attempt wrote and starts again: from FTP it downloads again, from a file you pick it again. The "already holds data" check is made only on the first attempt, unless the database or the broker changed since (in the wizard, in `hub.json` or through a `NIDOO_*` variable): then it is made again. If you reload the page while a restore runs, it follows the restore again.
- **Your FTP settings continue.** When the restore came from FTP and the backup holds other FTP settings than the ones you typed, the restored settings use the ones you typed, since they worked. Otherwise the backup's are kept, and the nightly flow keeps working with no re-typing. The backup password is the same one.

## Troubleshooting

- **A very large history fails with "out of shared memory".** A backup locks every history chunk (`ACCESS SHARE`) for as long as it reads them. Years of history mean many chunks, which can exceed PostgreSQL's lock table: raise `max_locks_per_transaction` (for example to 256) and restart PostgreSQL.
- **"This backup's password protection is too costly for this machine to open".** The backup was made on a much faster machine, and unlocking it here would take too long. Restore it on a faster machine.
- **"Zigbee2MQTT's backup is too big (over 24 MB)" or a Zigbee2MQTT part that never arrives.** Check the broker's `message_size_limit` (it must allow 32 MB), and that the `[topics] z2m_base` setting is the base topic Zigbee2MQTT publishes under. See [Zigbee2MQTT data](#zigbee2mqtt-data).

## Open a backup by hand

On a bad day, the file opens with standard tools, with no Nidoo:

```sh
age -d nidoo-backup-20261111-224512.tar.zst.age | zstd -d | tar x
```

`age` asks for the backup password. The file is a tar of these entries:

| Entry | Holds |
| --- | --- |
| `manifest.json` | The format version (2 for a file made since Zigbee2MQTT data was added; older files are format 1 and still restore), the Nidoo version, the database migration level, the date, the host name, each table's columns and row count, and the Zigbee2MQTT zip's size when it holds one. It comes first. |
| `hub.json` | The house fields of `hub.json`. |
| `settings.json` | The stored settings, as `key → value`. Secrets are in clear here, so keep what you extract private. |
| `z2m/000001.zip`, `z2m/000002.zip`, … | Zigbee2MQTT's own backup zip, cut into pieces of at most 8 MiB. Concatenated in order they are the zip. Absent when the backup holds no Zigbee2MQTT data. The zip holds the Zigbee network key in clear. |
| `retained/000001.json`, `retained/000002.json`, … | The configuration kept on the broker, as compact JSON arrays of `{"topic", "payload_b64"}`. The payload is base64. The first entry always exists, and is `[]` when there is nothing. |
| `db/<table>/000001.copy`, `000002.copy`, … | Each table's rows as PostgreSQL `COPY` text, cut into entries of at most 8 MiB. A table with no rows has none. |
| `end.json` | The rows each table's entries hold. |

To load a table into a database by hand, concatenate its entries in order and feed them to `COPY <table> (<columns>) FROM STDIN` in `psql`, with the columns listed in `manifest.json`. Do this on a database whose tables already exist, which the setup wizard creates.
