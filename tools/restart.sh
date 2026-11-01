#!/usr/bin/env bash
# Build the web UI and the hub, stop the running hub, start the new one.
#
# Usage: tools/restart.sh [--no-web] [--debug] [--fast] [--sweep]
#   --no-web  skip `trunk build` (reuse crates/nidoo-web/dist)
#   --debug   build and run the debug profile instead of release
#   --fast    debug profile + trunk dev build (no wasm-opt); fastest dev loop
#   --sweep   first delete build artifacts untouched for 14 days (needs
#             `cargo install cargo-sweep`); the current builds are kept
#
# A warning is printed when target/ is over TARGET_WARN_GB (default 10).
#
# The hub runs detached from this shell, from the repo root (so it reads
# ./oikos.toml), and logs to target/nidoo.log (OIKOS_LOG overrides).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE=release
BUILD_WEB=1
SWEEP=0
WEB_RELEASE=1
for arg in "$@"; do
  case "$arg" in
    --no-web) BUILD_WEB=0 ;;
    --debug) PROFILE=debug ;;
    --fast) PROFILE=debug; WEB_RELEASE=0 ;;
    --sweep) SWEEP=1 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

# Ask cargo where it writes artifacts: honours CARGO_TARGET_DIR, config.toml, and CLI flags.
TARGET_DIR="$(cd "$ROOT" && cargo metadata --no-deps --format-version=1 2>/dev/null \
  | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p' | head -1)"
TARGET_DIR="${TARGET_DIR:-$ROOT/target}"
BIN="$TARGET_DIR/$PROFILE/nidoo"
LOG="${OIKOS_LOG:-$TARGET_DIR/nidoo.log}"
BIND="$(sed -n 's/.*"bind": *"\([^"]*\)".*/\1/p' "$ROOT/data/hub.json" 2>/dev/null | head -1)"
if [ -z "$BIND" ]; then
  BIND="$(sed -n 's/^bind *= *"\(.*\)"/\1/p' "$ROOT/oikos.toml" 2>/dev/null | head -1)"
fi
HEALTH="http://${BIND:-127.0.0.1:8081}/health"

# Inline branches below instead of arrays: macOS's system bash 3.2 treats an
# empty array under `set -u` as unbound and aborts on `"${arr[@]}"`.

# 0. Keep target/ in check: stale artifacts pile up because cargo never deletes old ones.
if [ "$SWEEP" = 1 ]; then
  if cargo sweep --version >/dev/null 2>&1; then
    echo "==> cargo sweep --time 14"
    (cd "$ROOT" && cargo sweep --time 14 | tail -n 1)
  else
    echo "    --sweep ignored: cargo-sweep is not installed (cargo install cargo-sweep)" >&2
  fi
fi
if [ -d "$TARGET_DIR" ]; then
  target_gb="$(du -sg "$TARGET_DIR" 2>/dev/null | cut -f1)"
  if [ "${target_gb:-0}" -gt "${TARGET_WARN_GB:-10}" ]; then
    echo "WARNING: $TARGET_DIR is ${target_gb} GB; run tools/restart.sh --sweep (or cargo clean) before the disk fills" >&2
  fi
fi

# 1. Build the web UI first: the hub embeds dist/ at compile time.
if [ "$BUILD_WEB" = 1 ]; then
  if [ "$WEB_RELEASE" = 1 ] && [ "$PROFILE" = release ]; then
    echo "==> trunk build (release)"
    (cd "$ROOT/crates/nidoo-web" && trunk build --release)
  else
    echo "==> trunk build (dev)"
    (cd "$ROOT/crates/nidoo-web" && trunk build)
  fi
fi

# 2. Build the hub.
echo "==> cargo build -p nidoo ($PROFILE)"
if [ "$PROFILE" = release ]; then
  (cd "$ROOT" && cargo build --release -p nidoo)
else
  (cd "$ROOT" && cargo build -p nidoo)
fi

# 3. Stop the previous hub: SIGTERM, wait up to 10 s, then SIGKILL.
pids="$(pgrep -f 'nidoo run' || true)"
if [ -n "$pids" ]; then
  echo "==> stopping previous hub (pid $pids)"
  # shellcheck disable=SC2086 # several pids, split on purpose
  kill $pids 2>/dev/null || true
  for _ in $(seq 1 20); do
    pgrep -f 'nidoo run' >/dev/null || break
    sleep 0.5
  done
  if pgrep -f 'nidoo run' >/dev/null; then
    echo "    still alive, sending SIGKILL"
    pkill -9 -f 'nidoo run' || true
    sleep 0.5
  fi
fi

# 4. Launch the new one, detached, logging to $LOG.
echo "==> starting $BIN run (log: $LOG)"
mkdir -p "$(dirname "$LOG")"
cd "$ROOT"
nohup "$BIN" run >>"$LOG" 2>&1 < /dev/null &
NEW_PID=$!
disown "$NEW_PID" 2>/dev/null || true

# 5. Wait for /health.
for _ in $(seq 1 40); do
  if ! kill -0 "$NEW_PID" 2>/dev/null; then
    echo "hub exited early; last log lines:" >&2
    tail -n 20 "$LOG" >&2
    exit 1
  fi
  if curl -fsS --max-time 1 "$HEALTH" >/dev/null 2>&1; then
    echo "==> hub up (pid $NEW_PID) at $HEALTH"
    exit 0
  fi
  sleep 0.5
done
echo "hub (pid $NEW_PID) did not answer $HEALTH within 20 s; last log lines:" >&2
tail -n 20 "$LOG" >&2
exit 1
