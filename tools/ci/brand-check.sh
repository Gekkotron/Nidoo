#!/usr/bin/env bash
# Fails when the old name "oikos", in any case, shows up in a tracked file.
# docs/superpowers/ is history and keeps it.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

# git grep exits 1 when nothing matches; anything above (a bad pathspec, say)
# must fail the check instead of passing with nothing scanned.
scan() {
  local rc=0
  git grep "$@" || rc=$?
  [ "$rc" -le 1 ] || { echo "brand-check: git grep failed ($rc)" >&2; return "$rc"; }
}

status=0
hits=$(scan -n -i oikos -- . ':!docs/superpowers' ':!tools/ci/brand-check.sh')
if [ -n "$hits" ]; then
  echo "The old name \"oikos\" is still here; rename it to nidoo:"
  printf '%s\n' "$hits"
  status=1
fi

# The English word for a bird's home is a Google smart-home brand: keep it out
# of the prose and the logo files (code such as axum's .nest() is not scanned).
word=$(printf '%s' 'bmVzdA==' | base64 -d)
brand=$(scan -n -i -w "$word" -- README.md docs website 'crates/*/assets' ':!docs/superpowers')
if [ -n "$brand" ]; then
  echo "A Google brand name is in the docs or the logo files; describe it another way:"
  printf '%s\n' "$brand"
  status=1
fi
# Nidoo stays generic: the music server it was first tried with is never named
# (the word is encoded so this script does not name it either).
player=$(printf '%s' 'cGluc29u' | base64 -d)
# Everywhere, docs/superpowers included: its specs and plans are tracked text too.
named=$(scan -n -i -w "$player" -- .)
if [ -n "$named" ]; then
  echo "A music server is named; Nidoo is generic, say \"a media player\":"
  printf '%s\n' "$named"
  status=1
fi
# Wording the rename gets wrong: the docs address /Nidoo/ (GitHub Pages paths
# follow the repo name's case) and the article.
wording=$(scan -n -E -e 'github\.io/nidoo' -e '(^|[^[:alnum:]])[Aa]n Nidoo([^[:alnum:]]|$)' -- \
  . ':!docs/superpowers' ':!tools/ci/brand-check.sh')
if [ -n "$wording" ]; then
  echo "Brand wording to fix: docs address /Nidoo/, \"a Nidoo\":"
  printf '%s\n' "$wording"
  status=1
fi
exit "$status"
