---
title: Install on Linux (Docker)
sidebar_position: 2
description: Install Nidoo on a Linux host from a git clone with one Docker Compose command, then set it up in the browser, upgrade and run it.
---

Nidoo is published only as source. On the Linux box that will run it, `git clone` the repo and let Docker Compose build the hub and start a broker and a database next to it. You finish the setup in the browser.

## Prerequisites on the host

- Docker Engine (24+) with the Compose plugin (2.24+).
- `git`.
- Zigbee2MQTT (or any other producer publishing Home Assistant MQTT discovery), on this host or on another machine that can reach it.
- Optional: Frigate for camera alerts.

You do not need to install a broker or a database: the stack carries both. If you already run your own, see [Bring your own broker and database](#bring-your-own-broker-and-database).

## Install

```bash
git clone https://github.com/Gekkotron/nidoo.git
cd nidoo
docker compose up -d --build
docker compose logs hub
```

The log ends with one line per network address of the host:

```text
Open http://192.168.1.20:8081 to set up your home
```

Open that address from any device on the LAN. The [setup wizard](getting-started.md) walks you through the rest. Once setup is done the same line reads `… to open your home`.

## What runs

`docker compose up` starts four services, all on the host network:

| Service | What it is |
| --- | --- |
| `init` | A one-shot job. The first time, it generates a password for the broker and one for the database, then exits. |
| `mosquitto` | The MQTT broker (container `oikos-mosquitto`), on port 1883. |
| `timescaledb` | The history and settings database (container `oikos-timescaledb`), on port 5432. |
| `hub` | Nidoo itself (container `oikos-hub`), on port 8081. |

Everything the stack writes lives in `./data/` next to `compose.yml`:

| Path | Holds |
| --- | --- |
| `data/hub.json` | The hub's own settings: broker, database, address, language, location. |
| `data/bundled.json` | The generated broker and database passwords, readable only by its owner (mode 0600). The wizard pre-fills them from here. |
| `data/mosquitto/` | The broker's config, retained messages and log. |
| `data/timescaledb/` | The database files. |
| `data/secrets/` | Only on upgraded installs: the old key files moved from `secrets/` (see [Upgrading from `oikos.toml`](#upgrading-from-oikostoml)). Keys you upload in the wizard are stored in the database, not here. |

Back up `./data/`, and keep `data/bundled.json`: without it `tools/update.sh` rebuilds only the hub, so you would have to use `tools/update.sh --all`. If it is lost, the database password is still in `data/bundled/db-password` and in the database URL of `data/hub.json`, but `init` refuses to write a new password for a database that already exists.

## Point Zigbee2MQTT at the bundled broker

The bundled broker listens on port 1883 on every interface and does not accept anonymous clients. In Zigbee2MQTT's `configuration.yaml`:

```yaml
mqtt:
  server: mqtt://192.168.1.20:1883
  user: oikos
  password: <mqtt_password from data/bundled.json>   # sudo jq -r .mqtt_password data/bundled.json
```

Then set the discovery prefix as described in [Zigbee2MQTT](zigbee2mqtt.md). If you prefer a separate login for Zigbee2MQTT, add a second user inside the broker container and restart it:

```bash
docker exec -it oikos-mosquitto mosquitto_passwd /mosquitto/config/passwd zigbee2mqtt
docker compose restart mosquitto
```

## Bring your own broker and database

If this host already runs a Mosquitto on port 1883 or a Postgres on port 5432, a plain `docker compose up` fails for the bundled services: with host networking they cannot bind the same port, and a database with no generated password keeps restarting. The hub does not depend on them, so start it alone:

```bash
docker compose up -d --build hub
```

The wizard asks for your broker and database addresses instead of pre-filling the bundled ones. The first start of such a host is always `docker compose up -d --build hub`, never the Install command above. If a plain `docker compose up` already ran, clear the failed services (the restart-looping `oikos-mosquitto` and `oikos-timescaledb` containers):

```bash
docker compose rm -sf mosquitto timescaledb init
```

Then, unless Settings → Broker and Database point at the bundled services, remove what `init` generated, or the next `tools/update.sh` would tie this host to the full stack:

```bash
sudo rm -rf data/bundled.json data/bundled data/mosquitto data/timescaledb
```

### If Docker says a container name is already in use

If your own database or broker container is already named `oikos-timescaledb` or `oikos-mosquitto`, a plain `docker compose up` stops with:

```text
Error response from daemon: Conflict. The container name "/oikos-timescaledb" is already in use by container "<id>". You have to remove (or rename) that container to be able to reuse that name.
```

That container is your own database (or broker), not a leftover from the stack. **Do not remove or rename it** as Docker suggests. The bundled service would then start on an empty `data/timescaledb/`, and your history would be left in a volume nothing uses any more, where a `docker volume prune` deletes it for good. Start the hub alone instead:

```bash
docker compose rm -sf mosquitto timescaledb init   # clears only this stack's containers; yours belong to another project and are untouched
docker compose up -d --build hub
docker compose ps                                  # oikos-hub is up
docker ps --filter name=oikos                      # your own oikos-timescaledb / oikos-mosquitto are still up
```

Then check for `data/bundled.json` and remove what `init` generated, as above. From then on upgrade with `tools/update.sh`, which picks the hub only. Letting the stack take over that container's name is the job of the move guide below: it backs up the data and renames the old container first.

Want one stack in the end? [Move an existing broker and database into the stack](move-into-the-stack.md).

## Upgrade

```bash
cd nidoo   # or the folder you cloned it into
tools/update.sh
```

The first time you upgrade past the change that introduced this stack, run `git pull --rebase` yourself, then `tools/update.sh`: the copy of the script on your host predates it. The old script would otherwise start the new `compose.yml` without moving your files. If you already ran the old script, see [Recovering from the old update script](#recovering-from-the-old-update-script).

The script runs `git pull --rebase`, then re-runs itself from the pulled version, then rebuilds. It decides what to rebuild:

- the whole stack (`docker compose up -d --build`) when `data/bundled.json` or a `compose.override.yml` exists, or on a fresh install;
- the hub only (`docker compose up -d --build hub`) when you have `data/oikos.toml` or `data/hub.json` but no bundled stack, which means you run your own broker and database.

Force one with `tools/update.sh --all` or `tools/update.sh --hub-only`. If you run your own broker and database but have not finished the first start yet, there is no `hub.json` or `oikos.toml` to go by, so the script would pick the full stack: use `tools/update.sh --hub-only`. It ends by printing the last lines of the hub's log.

## Upgrading from `oikos.toml`

Older installs were configured by `oikos.toml` and kept keys in `secrets/`. The hub now reads everything from `./data/`, and `compose.yml` no longer mounts the old locations, except that it mounts the checkout read-only at `/checkout` so a `./oikos.toml` that has not moved yet is still imported. `tools/update.sh` moves them for you, never overwriting anything already in `./data/`:

- `./oikos.toml` becomes `data/oikos.toml`;
- every key in `./secrets/` (not its `README.md`) goes to `data/secrets/`.

It uses `sudo` for these moves when `data/` is owned by root, which Docker did on older installs. To do it by hand:

```bash
sudo mkdir -p -m 700 data/secrets
sudo mv -n oikos.toml data/oikos.toml
sudo mv -n secrets/firebase.json data/secrets/    # one line per key file you actually have
sudo mv -n secrets/solcast.key data/secrets/
```

`sudo` is needed when `data/` is owned by root; drop it otherwise. Skip the lines for files you do not have.

On its first start the hub imports `data/oikos.toml` into its database without asking anything, and logs:

```text
oikos.toml imported into the database; it is no longer read and can be deleted
```

After that line you can delete the file. The import reads the push and Solcast keys from `data/secrets/` once, by default, and stores them in the database. If the old file has no `[history] url`, the hub imports only MQTT and the location, then opens the wizard at the Database step so you can finish there. If your TOML names key files under `/etc/oikos/secrets`, change those paths to `/data/secrets/...` or delete the lines; `update.sh` warns when it sees them.

If you run your own broker and database, keep using `docker compose up -d --build hub` (or `tools/update.sh`, which picks it). A plain `docker compose up` would start a second broker and database next to yours.

## Recovering from the old update script

If you ran the old `tools/update.sh` on a host still configured by `./oikos.toml`, `init` may have generated bundled credentials and `docker compose up` started `oikos-mosquitto` and `oikos-timescaledb`, which restart in a loop when your own broker or database holds the ports. Clean up:

1. Remove the failed services: `docker compose rm -sf init mosquitto timescaledb`. If `docker compose up` stopped on "container name already in use", that container is your own: leave it alone (see [If Docker says a container name is already in use](#if-docker-says-a-container-name-is-already-in-use)).
2. If you never used the bundled services, remove their files: `sudo rm -rf data/bundled.json data/bundled data/mosquitto data/timescaledb`.
3. Upgrade properly: `git pull --rebase && tools/update.sh --hub-only`.

## Logs and status

```bash
docker compose logs -f hub          # follow the hub's logs
docker compose ps                   # container status
docker compose restart hub          # restart in place
docker compose down                 # stop and remove the containers
```

`docker compose down` keeps `./data/`, so your settings and history survive.

## Networking

The compose file uses `network_mode: host`. The hub reaches the broker, the database and Frigate on the host via `localhost`, and the host's firewall decides who can reach the ports. Allow `8081` inbound on your LAN interface before pointing the Android app at the hub, and `1883` if Zigbee2MQTT runs on another machine.

## Push notifications (optional)

The hub sends Firebase Cloud Messaging alerts with a service-account key (see [Push notifications](push-notifications.md)). Upload the key in the wizard's Services step, or later under Settings. It is stored in the hub's database, so there is nothing to copy onto the host.

## Overrides without editing tracked files

Any setting can also come from an `OIKOS_*` environment variable (`__` between levels, for example `OIKOS_MQTT__PASSWORD`). Put them in a `.env` file next to `compose.yml`: compose hands it to the hub when it exists, and git ignores it.
