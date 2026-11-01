---
title: Development
sidebar_position: 1
description: Build, test and run the hub and the web UI from a checkout, with the dev loop, the release build order and the helper scripts.
---

This page covers working on the code. For running the hub as a user, see [Getting started](../users/getting-started.md). For how the crates fit together, see [Architecture](architecture.md).

## Checks

Run these before opening a pull request:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo check -p oikos-core --target wasm32-unknown-unknown
cargo check -p oikos-api --target wasm32-unknown-unknown
cargo check -p oikos-web --target wasm32-unknown-unknown
```

The `wasm32-unknown-unknown` target comes with [`rust-toolchain.toml`](https://github.com/Gekkotron/Nidoo/blob/main/rust-toolchain.toml) (stable channel). `oikos-core` and `oikos-api` must keep building for it, because the web UI shares them.

The tests run against recorded broker messages under `crates/oikos-core/tests/fixtures` and never need a broker.

### Recording fixtures

Refresh the recording from a live house with:

```sh
MQTT_HOST=<host> MQTT_USER=<user> MQTT_PASS=<password> tools/record-fixtures.sh
```

The script needs `mosquitto_sub` and `python3`. It listens for six seconds to `homeassistant/#`, `oikos/#` and `zigbee2mqtt/…`, then rewrites the `discovery`, `states` and `ui` folders under `crates/oikos-core/tests/fixtures`. It leaves the fixtures untouched when no retained message arrives. `MQTT_PORT` (default 1883) is optional. Credentials are read from the environment only and never written anywhere.

## Run the hub

```sh
cargo run -p oikos-os
```

The hub keeps its data in `./data` (`--data <dir>` picks another folder) and serves the setup wizard at `http://localhost:8081/setup` on a first start. For a local broker and database, start the bundled ones: `cargo run -p oikos-os -- init-bundled && docker compose up -d --no-deps mosquitto timescaledb` (host networking needs Linux, or Docker Desktop's host networking enabled on macOS), then use the credentials in `data/bundled.json` in the wizard. See [Configuration](../reference/configuration.md) for every key.

## Web UI

The web UI is a Leptos app that runs in the browser as wasm, built with Trunk. It lives in `crates/oikos-web`.

### Install the tools

```sh
cargo install trunk --locked
cargo install wasm-pack --locked
```

`wasm-pack` fetches a matching chromedriver on first use; Google Chrome must be installed for the smoke test.

### Dev loop

Development runs in two terminals:

```sh
cargo run -p oikos-os -- run       # the hub on 8081
cd crates/oikos-web && trunk serve # the UI on 127.0.0.1:3000, rebuilt on save
```

Open `http://127.0.0.1:3000`. [`Trunk.toml`](https://github.com/Gekkotron/Nidoo/blob/main/crates/oikos-web/Trunk.toml) proxies `/api/v1/`, `/ws` and `/mcp/` to the hub on `127.0.0.1:8081`, so the UI talks to a real hub while Trunk rebuilds it on save. If your hub binds elsewhere, change the three `backend` entries.

### Release build order

A release build embeds the UI, so build the UI before the hub:

```sh
cd crates/oikos-web && trunk build --release
cd ../.. && cargo build --release -p oikos-os
```

The hub embeds `crates/oikos-web/dist/` with `rust-embed` when it is compiled. Rebuild the hub after every `trunk build`. A binary built before `trunk build` has no UI to serve: every path that is not `/api`, `/ws` or `/mcp` answers a `503` page that says the UI has not been built. `index.html` is never cached; hashed assets are cached for a year. A release `trunk build` also writes `.gz` copies of the wasm, js and css (and `.br` copies when `brotli` is installed), through the `post_build` hook in `crates/oikos-web/compress.sh`; the hub sends them to browsers that accept them. Browsers offer brotli only over HTTPS, so a plain-HTTP hub sends gzip, which takes the wasm from about 11 MB to about 3.4 MB.

### Smoke test

The smoke test mounts the app in headless Chrome and checks the shell:

```sh
cd crates/oikos-web && wasm-pack test --headless --chrome
```

The other tests under `crates/oikos-web/tests` (cards, flow canvas, group wizard and so on) run the same way.

## `tools/restart.sh`

[`tools/restart.sh`](https://github.com/Gekkotron/Nidoo/blob/main/tools/restart.sh) builds the web UI and the hub, stops the running hub and starts the new one, detached, from the repo root so it reads `./data`. It waits up to 20 seconds for `/health`, and logs to `<cargo target dir>/oikos-os.log` (set `OIKOS_LOG` to log elsewhere).

| Flag | Effect |
| --- | --- |
| `--no-web` | Skip `trunk build` and reuse `crates/oikos-web/dist`. |
| `--debug` | Build and run the debug profile instead of release. |
| `--fast` | Debug profile and a Trunk dev build (no `wasm-opt`): the fastest dev loop. |
| `--sweep` | First delete build artifacts untouched for 14 days (needs `cargo install cargo-sweep`); the current builds are kept. |

It warns when the target directory passes 10 GB (`TARGET_WARN_GB` changes the limit). It stops previous hubs with `SIGTERM`, waits up to 10 seconds, then sends `SIGKILL`.
