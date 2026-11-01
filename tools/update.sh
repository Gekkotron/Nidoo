#!/usr/bin/env bash
# Upgrade a Docker install: pull the latest code, move a legacy ./oikos.toml
# and ./secrets/ into ./data/, then rebuild and restart.
#
# Usage: tools/update.sh [--hub-only|--all]
#   --hub-only  rebuild and restart only the hub (you run your own broker and database)
#   --all       rebuild and restart the whole compose stack (hub, Mosquitto, TimescaleDB)
#
# Without a flag the script picks: the whole stack when data/bundled.json or
# compose.override.yml exists, the hub only when data/oikos.toml or
# data/hub.json exists (own broker and database), the whole stack otherwise.
#
# Runs from the repo root whatever the current directory, so compose.yml and
# ./data are the ones next to this checkout.
set -euo pipefail

# Everything lives in main so bash has read the whole script before
# `git pull` rewrites this file; the pulled version then runs the rest.
usage_error() {
  echo "$1" >&2
  echo "usage: tools/update.sh [--hub-only|--all]" >&2
  exit 2
}

main() {
  ROOT="$(cd "$(dirname "$0")/.." && pwd)"
  cd "$ROOT"

  # Check the arguments before touching anything. --pulled is internal: the
  # re-exec after `git pull` sets it, so the new version skips the pull.
  mode=""
  pulled=0
  for arg in "$@"; do
    case "$arg" in
      --hub-only|--all)
        new=hub
        [ "$arg" = --all ] && new=all
        if [ -n "$mode" ] && [ "$mode" != "$new" ]; then
          usage_error "--hub-only and --all are mutually exclusive"
        fi
        mode="$new"
        ;;
      --pulled) pulled=1 ;;
      *) usage_error "unknown option: $arg" ;;
    esac
  done

  if [ "$pulled" = 0 ]; then
    echo "==> git pull --rebase"
    git pull --rebase
    exec "$ROOT/tools/update.sh" --pulled "$@"
  fi

  migrate_legacy_files
  warn_old_key_paths

  reason=""
  if [ -n "$mode" ]; then
    reason="requested on the command line"
  elif [ -e data/bundled.json ] || [ -e compose.override.yml ]; then
    mode=all
    reason="bundled stack or compose.override.yml found"
  elif [ -e data/oikos.toml ] || [ -e data/hub.json ]; then
    mode=hub
    reason="own broker and database"
  else
    mode=all
    reason="fresh install"
  fi

  if [ "$mode" = hub ]; then
    echo "==> $reason: rebuilding the hub only"
    services=(hub)
  else
    echo "==> $reason: rebuilding the whole stack"
    services=()
  fi
  switch_renamed_containers ${services[@]+"${services[@]}"}
  echo "==> docker compose up -d --build${services[*]+ ${services[*]}}"
  docker compose up -d --build ${services[@]+"${services[@]}"}

  echo "==> docker compose logs --tail 20 hub"
  docker compose logs --tail 20 hub
}

# The containers were named oikos-* before the Nidoo rename. Compose finds them
# by its own labels (`docker compose ps` lists this project's containers only,
# for the services about to start), so a host's hand-made container that
# happens to carry an old name is never touched, and neither is one that
# compose.override.yml still names. The images are built first, so the old
# containers stop only for the switch itself; the stop is graceful, so the
# database flushes. Data stays where it is mounted.
switch_renamed_containers() {
  local old=() name config
  config="$(docker compose config 2>/dev/null || true)"
  while IFS= read -r name; do
    case "$name" in
      oikos-hub|oikos-mosquitto|oikos-timescaledb)
        grep -Eq "container_name: \"?$name\"?\$" <<<"$config" || old+=("$name")
        ;;
    esac
  done < <(docker compose ps -a --format '{{.Name}}' "$@")
  [ "${#old[@]}" -gt 0 ] || return 0
  echo "==> renamed containers: building first, then replacing ${old[*]}"
  docker compose build "$@"
  for name in "${old[@]}"; do
    echo "==> docker stop -t 120 $name && docker rm $name"
    docker stop -t 120 "$name"
    docker rm "$name"
  done
}

# Runs a command through sudo when data/ is not writable by this user.
SUDO=""
priv() {
  if [ -n "$SUDO" ]; then
    sudo "$@"
  else
    "$@"
  fi
}

# Old installs kept ./oikos.toml and ./secrets/*; the hub now reads ./data/.
# Never overwrite anything already in ./data/.
migrate_legacy_files() {
  # The old compose bind-mounted ./data, so Docker created it as root.
  if [ -d data ] && [ ! -w data ]; then
    echo "data/ is owned by root; using sudo to move files into it"
    SUDO=sudo
  fi

  if [ -f oikos.toml ]; then
    if [ -e data/oikos.toml ]; then
      echo "data/oikos.toml already exists; ./oikos.toml left in place"
    else
      priv mkdir -p data
      priv mv -n oikos.toml data/oikos.toml
      echo "moved oikos.toml -> data/oikos.toml"
    fi
  fi

  if [ -d secrets ]; then
    for f in secrets/*; do
      [ -f "$f" ] || continue
      name="$(basename "$f")"
      [ "$name" = README.md ] && continue
      if [ -e "data/secrets/$name" ]; then
        echo "data/secrets/$name already exists; $f left in place"
      else
        priv mkdir -p -m 700 data/secrets
        priv mv -n "$f" "data/secrets/$name"
        echo "moved $f -> data/secrets/$name"
      fi
    done
  fi
}

# Once data/hub.json exists the TOML is never read again, so the warning
# would only confuse.
warn_old_key_paths() {
  if [ ! -f data/hub.json ] && [ -f data/oikos.toml ] && grep -q '/etc/oikos/secrets' data/oikos.toml; then
    echo "WARNING: data/oikos.toml names key files under /etc/oikos/secrets; change them to /data/secrets/... or delete those lines (the defaults find data/secrets/)" >&2
  fi
}

main "$@"
