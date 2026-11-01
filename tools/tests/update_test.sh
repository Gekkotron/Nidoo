#!/usr/bin/env bash
# Tests for tools/update.sh. Each test builds a throwaway repo layout, puts
# stub `git` and `docker` first on PATH (they only log their arguments) and
# asserts on the files and the log afterwards.
#
# Usage: tools/tests/update_test.sh
set -euo pipefail

SRC="$(cd "$(dirname "$0")/../.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

T=""
OUT=""
LOG=""

fail() { echo "FAIL: $*" >&2; [ -n "$OUT" ] && echo "--- output ---" >&2 && echo "$OUT" >&2; [ -n "$LOG" ] && [ -f "$LOG" ] && echo "--- log ---" >&2 && cat "$LOG" >&2; exit 1; }

# Fresh layout in $T with the stubs; sets T, LOG.
setup() {
  T="$(mktemp -d "$WORK/case.XXXXXX")"
  LOG="$T/calls.log"
  OUT=""
  mkdir -p "$T/repo/tools" "$T/bin"
  cp "$SRC/tools/update.sh" "$T/repo/tools/update.sh"
  : > "$LOG"
  cat > "$T/bin/git" <<STUB
#!/usr/bin/env bash
echo "git \$*" >> "$LOG"
if [ "\${1:-}" = pull ] && [ -n "\${STUB_GIT_NEW_SCRIPT:-}" ]; then
  printf '%s\n' '#!/usr/bin/env bash' 'echo "NEW VERSION \$*"' > "$T/repo/tools/update.sh"
fi
STUB
  # `docker compose ps` answers with STUB_COMPOSE_PS (this project's containers, as compose's
  # labels find them); `docker ps` with STUB_DOCKER_PS (every container on the host).
  cat > "$T/bin/docker" <<STUB
#!/usr/bin/env bash
echo "docker \$*" >> "$LOG"
case "\${1:-} \${2:-}" in
  "compose ps") for n in \${STUB_COMPOSE_PS:-}; do echo "\$n"; done ;;
  "compose config")
    if [ "\${3:-}" = --services ]; then
      for n in \${STUB_COMPOSE_SERVICES:-}; do echo "\$n"; done
    else
      printf '%b' "\${STUB_COMPOSE_CONFIG:-}"
    fi
    ;;
  "ps "*) for n in \${STUB_DOCKER_PS:-}; do echo "\$n"; done ;;
esac
STUB
  chmod +x "$T/bin/git" "$T/bin/docker"
}

run_update() {
  OUT="$(cd "$T/repo" && PATH="$T/bin:$PATH" bash tools/update.sh "$@" 2>&1)" || fail "update.sh exited non-zero"
}

assert_log() { grep -Fxq -- "$1" "$LOG" || fail "log lacks line: $1"; }
assert_no_log() { ! grep -Fxq -- "$1" "$LOG" || fail "log has unexpected line: $1"; }
assert_file() { [ -f "$1" ] || fail "missing file: $1"; }
assert_content() { [ "$(cat "$1")" = "$2" ] || fail "$1 is '$(cat "$1")', expected '$2'"; }
assert_no_out() { case "$OUT" in *"$1"*) fail "output has unexpected: $1" ;; esac; }
assert_out() { case "$OUT" in *"$1"*) ;; *) fail "output lacks: $1" ;; esac; }

HUB='docker compose up -d --build hub'
ALL='docker compose up -d --build'

# Line number of the first log line equal to $1 (0 when absent).
log_line() { grep -Fxn -- "$1" "$LOG" | head -1 | cut -d: -f1 || true; }
assert_before() {
  local a b
  a="$(log_line "$1")"
  b="$(log_line "$2")"
  [ -n "$a" ] && [ -n "$b" ] && [ "$a" -lt "$b" ] || fail "expected '$1' before '$2'"
}

test_moves_legacy_files() {
  setup
  mkdir -p "$T/repo/secrets"
  echo 'bind = "0.0.0.0:8081"' > "$T/repo/oikos.toml"
  echo fb > "$T/repo/secrets/firebase.json"
  echo sc > "$T/repo/secrets/solcast.key"
  echo readme > "$T/repo/secrets/README.md"
  run_update
  assert_file "$T/repo/data/oikos.toml"
  assert_content "$T/repo/data/secrets/firebase.json" fb
  assert_content "$T/repo/data/secrets/solcast.key" sc
  [ ! -e "$T/repo/oikos.toml" ] || fail "oikos.toml not moved"
  [ ! -e "$T/repo/secrets/firebase.json" ] || fail "firebase.json not moved"
  assert_file "$T/repo/secrets/README.md"
  [ ! -e "$T/repo/data/secrets/README.md" ] || fail "README.md was moved"
  assert_out "moved "
  assert_log "$HUB"
  assert_no_log "$ALL"
  assert_log "docker compose logs --tail 20 hub"
}

test_never_overwrites_data() {
  setup
  mkdir -p "$T/repo/secrets" "$T/repo/data/secrets"
  echo old > "$T/repo/oikos.toml"
  echo new > "$T/repo/data/oikos.toml"
  echo oldkey > "$T/repo/secrets/firebase.json"
  echo newkey > "$T/repo/data/secrets/firebase.json"
  run_update
  assert_content "$T/repo/data/oikos.toml" new
  assert_content "$T/repo/data/secrets/firebase.json" newkey
  assert_content "$T/repo/oikos.toml" old
  assert_content "$T/repo/secrets/firebase.json" oldkey
  assert_out "data/oikos.toml already exists; ./oikos.toml left in place"
}

test_bundled_runs_everything() {
  setup
  mkdir -p "$T/repo/data"
  echo '{}' > "$T/repo/data/bundled.json"
  run_update
  assert_log "$ALL"
  assert_no_log "$HUB"
}

test_override_runs_everything() {
  setup
  mkdir -p "$T/repo/data"
  echo '{}' > "$T/repo/data/hub.json"
  echo 'services: {}' > "$T/repo/compose.override.yml"
  run_update
  assert_log "$ALL"
  assert_no_log "$HUB"
}

test_flags_win() {
  setup
  mkdir -p "$T/repo/data"
  echo '{}' > "$T/repo/data/hub.json"
  run_update --all
  assert_log "$ALL"
  assert_no_log "$HUB"
  setup
  mkdir -p "$T/repo/data"
  echo '{}' > "$T/repo/data/bundled.json"
  run_update --hub-only
  assert_log "$HUB"
  assert_no_log "$ALL"
}

test_warns_about_old_key_paths() {
  setup
  echo 'key = "/etc/oikos/secrets/firebase.json"' > "$T/repo/oikos.toml"
  run_update
  assert_out "/etc/oikos/secrets"
}

test_no_old_key_warning_once_hub_json_exists() {
  setup
  mkdir -p "$T/repo/data"
  echo '{}' > "$T/repo/data/hub.json"
  echo 'key = "/etc/oikos/secrets/firebase.json"' > "$T/repo/data/oikos.toml"
  run_update
  assert_no_out "/etc/oikos/secrets"
}

test_reexecs_after_pull() {
  setup
  STUB_GIT_NEW_SCRIPT=1 run_update
  assert_out "NEW VERSION"
  [ "$(grep -c '^git pull --rebase$' "$LOG")" = 1 ] || fail "pull should run exactly once"
  assert_no_log "$ALL"
}

test_uses_sudo_when_data_is_not_writable() {
  if [ "$(id -u)" = 0 ]; then echo "    skipped: running as root"; return; fi
  setup
  mkdir -p "$T/repo/secrets" "$T/repo/data"
  echo 'bind = "0.0.0.0:8081"' > "$T/repo/oikos.toml"
  echo fb > "$T/repo/secrets/firebase.json"
  cat > "$T/bin/sudo" <<STUB
#!/usr/bin/env bash
echo "sudo \$*" >> "$LOG"
chmod u+w "$T/repo/data"
"\$@"
STUB
  chmod +x "$T/bin/sudo"
  chmod a-w "$T/repo/data"
  run_update || { chmod u+w "$T/repo/data"; exit 1; }
  chmod -R u+w "$T/repo/data"
  assert_out "data/ is owned by root; using sudo to move files into it"
  assert_log "sudo mv -n oikos.toml data/oikos.toml"
  assert_log "sudo mv -n secrets/firebase.json data/secrets/firebase.json"
  assert_file "$T/repo/data/oikos.toml"
  assert_content "$T/repo/data/secrets/firebase.json" fb
}

test_update_recreates_renamed_containers() {
  setup
  mkdir -p "$T/repo/data"
  echo '{}' > "$T/repo/data/bundled.json"
  STUB_COMPOSE_PS="oikos-hub oikos-mosquitto oikos-timescaledb nidoo-hub" run_update
  assert_log "docker compose ps -a --format {{.Name}}"
  # Built first, so the old containers stop only for the switch itself.
  assert_before "docker compose build" "docker stop -t 120 oikos-hub"
  for name in oikos-hub oikos-mosquitto oikos-timescaledb; do
    # A graceful stop first: the database gets time to flush, not a SIGKILL.
    assert_before "docker stop -t 120 $name" "docker rm $name"
    assert_before "docker rm $name" "$ALL"
  done
  ! grep -q 'nidoo-hub' <(grep -E '^docker (stop|rm)' "$LOG") || fail "touched nidoo-hub"
  assert_out "renamed containers"
}

test_update_hub_only_removes_only_the_old_hub() {
  setup
  mkdir -p "$T/repo/data"
  echo '{}' > "$T/repo/data/hub.json"
  STUB_COMPOSE_PS="oikos-hub" run_update
  assert_log "docker compose ps -a --format {{.Name}} hub"
  assert_before "docker compose build hub" "docker stop -t 120 oikos-hub"
  assert_before "docker rm oikos-hub" "$HUB"
}

test_update_keeps_old_names_the_override_still_sets() {
  setup
  mkdir -p "$T/repo/data"
  echo '{}' > "$T/repo/data/hub.json"
  echo 'services: {}' > "$T/repo/compose.override.yml"
  # A host that followed the move guide before the rename pins the old names itself.
  STUB_COMPOSE_CONFIG='    container_name: oikos-mosquitto\n    container_name: oikos-timescaledb\n    container_name: nidoo-hub\n' \
    STUB_COMPOSE_PS="oikos-hub oikos-mosquitto oikos-timescaledb" run_update
  assert_log "docker rm oikos-hub"
  ! grep -Eq '^docker (stop|rm).*oikos-(mosquitto|timescaledb)' "$LOG" || fail "removed a container the override still names"
  assert_log "$ALL"
}

# The house runs its own database container under the old name, in another compose project.
# Before the rename the bundled one clashed on that name and never started; now --all would
# start nidoo-timescaledb beside it, so the update refuses and points at --hub-only.
test_all_refuses_beside_own_old_named_db() {
  setup
  mkdir -p "$T/repo/data"
  echo '{}' > "$T/repo/data/hub.json"
  echo 'services: {}' > "$T/repo/compose.override.yml"
  OUT="$(cd "$T/repo" && STUB_DOCKER_PS="oikos-timescaledb" STUB_COMPOSE_SERVICES="init mosquitto timescaledb hub" \
    PATH="$T/bin:$PATH" bash tools/update.sh 2>&1)" && fail "update.sh went ahead beside the host's own oikos-timescaledb"
  assert_out "oikos-timescaledb"
  assert_out "--hub-only"
  ! grep -Eq '^docker (stop|rm|rename|compose (up|build))' "$LOG" || fail "changed something before refusing"
}

test_hub_only_beside_own_old_named_db() {
  setup
  mkdir -p "$T/repo/data"
  echo '{}' > "$T/repo/data/hub.json"
  echo 'services: {}' > "$T/repo/compose.override.yml"
  STUB_DOCKER_PS="oikos-timescaledb oikos-mosquitto" STUB_COMPOSE_SERVICES="init mosquitto timescaledb hub" \
    run_update --hub-only
  ! grep -Eq '^docker (stop|rm|rename).*oikos-(timescaledb|mosquitto)' "$LOG" || fail "touched the host's own containers"
  assert_log "$HUB"
}

test_all_goes_ahead_when_the_override_drops_the_bundled_db() {
  setup
  mkdir -p "$T/repo/data"
  echo '{}' > "$T/repo/data/hub.json"
  echo 'services: {}' > "$T/repo/compose.override.yml"
  STUB_DOCKER_PS="oikos-timescaledb oikos-mosquitto" STUB_COMPOSE_SERVICES="init hub" run_update
  ! grep -Eq '^docker (stop|rm|rename).*oikos-(timescaledb|mosquitto)' "$LOG" || fail "touched the host's own containers"
  assert_log "$ALL"
}

test_no_switchover_without_old_containers() {
  setup
  mkdir -p "$T/repo/data"
  echo '{}' > "$T/repo/data/bundled.json"
  STUB_COMPOSE_PS="nidoo-hub nidoo-mosquitto nidoo-timescaledb" run_update
  assert_no_log "docker compose build"
  ! grep -q '^docker rm' "$LOG" || fail "removed a container without an old name"
  assert_log "$ALL"
}

test_rejects_bad_arguments() {
  setup
  if (cd "$T/repo" && PATH="$T/bin:$PATH" bash tools/update.sh --bogus >/dev/null 2>&1); then fail "--bogus accepted"; fi
  if (cd "$T/repo" && PATH="$T/bin:$PATH" bash tools/update.sh --all --hub-only >/dev/null 2>&1); then fail "--all --hub-only accepted"; fi
  [ ! -s "$LOG" ] || fail "git or docker ran despite bad arguments"
}

for t in test_moves_legacy_files test_never_overwrites_data test_bundled_runs_everything \
  test_override_runs_everything test_flags_win test_warns_about_old_key_paths test_no_old_key_warning_once_hub_json_exists test_reexecs_after_pull \
  test_uses_sudo_when_data_is_not_writable test_update_recreates_renamed_containers \
  test_update_hub_only_removes_only_the_old_hub test_all_refuses_beside_own_old_named_db \
  test_hub_only_beside_own_old_named_db test_all_goes_ahead_when_the_override_drops_the_bundled_db \
  test_update_keeps_old_names_the_override_still_sets \
  test_no_switchover_without_old_containers test_rejects_bad_arguments; do
  echo "==> $t"
  "$t"
done
echo "all update.sh tests passed"
