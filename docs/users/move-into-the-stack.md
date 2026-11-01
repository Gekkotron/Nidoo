---
title: Move an existing broker and database into the stack
sidebar_position: 2.5
description: Replace a hand-run Mosquitto and TimescaleDB with the ones in Nidoo's compose stack, keeping their data, with a backup and a rollback.
---

If you already run Mosquitto and TimescaleDB in Docker for the hub, you can let the compose stack own them without losing a retained message or a history row. The stack's services read their data from folders you choose, so the move is mostly: stop the old containers, point the stack at the same folders, start it.

You do not have to do this. A hub with its own broker and database keeps working with `docker compose up -d --build hub`. See [Bring your own broker and database](install-on-linux.md#bring-your-own-broker-and-database).

## Step 0: upgrade first

Run `git pull --rebase && tools/update.sh`, then check that `data/hub.json` exists. The hub must already be configured, or the stack's `init` would generate credentials for services you are about to replace. See [Upgrade](install-on-linux.md#upgrade).

## What to note first

Write these five facts down for each of the two old containers (`docker inspect <name>` shows them):

1. **Image tag.** The stack pins `eclipse-mosquitto:2.0.22` and `timescale/timescaledb:2.30.2-pg17`. Reuse the tags you run today in the override below. Changing the TimescaleDB tag upgrades the extension in the database, and that cannot be undone.
2. **Folders.** The host folders mounted on `/mosquitto/config`, `/mosquitto/data`, `/mosquitto/log` and `/var/lib/postgresql/data`. A named volume works too.
3. **Ports.** 1883 for the broker, and the database port (5432 unless you changed it).
4. **How they start.** A compose project, a `docker run` or a systemd unit. You will stop them, and later disable that autostart.
5. **Their names**, in case they are the ones the new containers take: `nidoo-mosquitto` and `nidoo-timescaledb`.

## Check that the broker persists

The hub keeps its flows, alarm and climate settings as retained messages on the broker. Open the broker's `mosquitto.conf` and look for `persistence true`. Without it, stopping the broker drops every retained message. If it is missing, add it and restart the broker first, then confirm that `mosquitto.db` appears in the data folder.

## Back up

```bash
# The database, in a restorable archive
docker exec <old-timescaledb> pg_dump -U oikos -Fc oikos > oikos-db.dump

# The broker's config and data
tar czf mosquitto-backup.tgz <config-folder> <data-folder>
```

Use your own container names and folders. During the switch you also make cold copies, below.

## Write the override

Create `compose.override.yml` next to `compose.yml`. Compose merges it on top: the service names, container names and mount targets are the ones the stack uses, so only the images, folders and port change.

```yaml
services:
  mosquitto:
    image: eclipse-mosquitto:<your tag>
    container_name: nidoo-mosquitto
    restart: unless-stopped
    network_mode: host
    volumes:
      - <mosquitto config folder>:/mosquitto/config
      - <mosquitto data folder>:/mosquitto/data
      - <mosquitto log folder>:/mosquitto/log

  timescaledb:
    image: timescale/timescaledb:<your tag>
    container_name: nidoo-timescaledb
    restart: unless-stopped
    network_mode: host
    command: postgres -c port=<port>
    volumes:
      - <database folder>:/var/lib/postgresql/data
```

Notes:

- An override written before the rename names the containers `oikos-mosquitto` and `oikos-timescaledb`. That still works: the names it sets win, and `tools/update.sh` leaves those containers alone.
- Leave out `command` when the database port is 5432.
- If the database lives in a named volume, mount it as `<name>:/var/lib/postgresql/data` and declare it at the bottom of the file with `volumes: {<name>: {external: true}}`.
- You do not set any password. An initialised data folder keeps its users, and the stack only applies the generated password when it creates a database from scratch.
- You do not need to change folder ownership. The images are the same ones you ran, so the same users own the files.

Because `data/hub.json` already exists on a configured hub, `init` writes no bundled credentials and leaves your broker config alone. Running `docker compose config` shows the merged result: check the folders and tags before going on.

## Switch

1. Stop the hub: `docker compose stop hub`.
2. Stop the old containers without letting them restart, and give each time to flush:

   ```bash
   docker update --restart=no <old-mosquitto> <old-timescaledb>
   docker stop -t 60 <old-mosquitto>
   docker stop -t 120 <old-timescaledb>
   ```

3. Make cold copies of both data folders now that nothing writes to them, for example `cp -a`. They are your safety net for the rollback.
4. If an old container is already named `nidoo-mosquitto` or `nidoo-timescaledb`, rename it: the new ones take those names, and `docker compose up` fails with "container name already in use" otherwise (skip a container whose name is different):

   ```bash
   docker rename <old-mosquitto> <old-mosquitto>-retired
   docker rename <old-timescaledb> <old-timescaledb>-retired
   ```

5. Remove the old autostart (compose project, systemd unit) so the old containers do not return after a reboot.
6. Start the stack: `docker compose up -d`.

## Check

- The hub log shows no reconnect loop, and Settings shows the broker Online.
- Flows, alarm and climate settings are all there: the retained messages came across.
- The alerts history is intact, and the history charts show the days from before the move.
- Zigbee2MQTT is connected and devices still answer.
- The phone app connects.
- Reboot the host once, and check that everything comes back by itself.

## Roll back

If anything looks wrong:

1. `docker compose down`
2. Rename the override: `mv compose.override.yml compose.override.yml.off`.
3. Rename the old containers back (`docker rename <old-mosquitto>-retired <old-mosquitto>`, same for the database), or start them under the retired names. Re-enable them with `docker update --restart=unless-stopped <name> <name>`, then start the database and the broker.
4. If the new stack changed the data folders in a way you do not trust, put the cold copies back before starting.

Once the checks pass, keep the backups for a few days, then delete the old containers.
