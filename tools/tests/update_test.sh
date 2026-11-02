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
  cat > "$T/bin/docker" <<STUB
#!/usr/bin/env bash
echo "docker \$*" >> "$LOG"
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
assert_out() { case "$OUT" in *"$1"*) ;; *) fail "output lacks: $1" ;; esac; }

HUB='docker compose up -d --build hub'
ALL='docker compose up -d --build'

test_moves_legacy_files() {
  setup
  mkdir -p "$T/repo/secrets" "$T/repo/data"
  echo '{}' > "$T/repo/data/hub.json"
  echo fb > "$T/repo/secrets/firebase.json"
  echo sc > "$T/repo/secrets/solcast.key"
  echo readme > "$T/repo/secrets/README.md"
  run_update
  assert_content "$T/repo/data/secrets/firebase.json" fb
  assert_content "$T/repo/data/secrets/solcast.key" sc
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
  echo oldkey > "$T/repo/secrets/firebase.json"
  echo newkey > "$T/repo/data/secrets/firebase.json"
  run_update
  assert_content "$T/repo/data/secrets/firebase.json" newkey
  assert_content "$T/repo/secrets/firebase.json" oldkey
  assert_out "data/secrets/firebase.json already exists; secrets/firebase.json left in place"
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
  assert_log "sudo mv -n secrets/firebase.json data/secrets/firebase.json"
  assert_content "$T/repo/data/secrets/firebase.json" fb
}

test_rejects_bad_arguments() {
  setup
  if (cd "$T/repo" && PATH="$T/bin:$PATH" bash tools/update.sh --bogus >/dev/null 2>&1); then fail "--bogus accepted"; fi
  if (cd "$T/repo" && PATH="$T/bin:$PATH" bash tools/update.sh --all --hub-only >/dev/null 2>&1); then fail "--all --hub-only accepted"; fi
  [ ! -s "$LOG" ] || fail "git or docker ran despite bad arguments"
}

for t in test_moves_legacy_files test_never_overwrites_data test_bundled_runs_everything \
  test_override_runs_everything test_flags_win test_reexecs_after_pull \
  test_uses_sudo_when_data_is_not_writable test_rejects_bad_arguments; do
  echo "==> $t"
  "$t"
done
echo "all update.sh tests passed"
