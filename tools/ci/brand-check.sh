#!/usr/bin/env bash
# Fails when the old product name "Oikos" shows up in a tracked file. Code
# identifiers (oikos-core, oikos/ topics, OIKOS_* env vars) are lowercase or
# upper case and stay out of this check; docs/superpowers/ is history.
# tools/ci/brand-allow.txt lists the git pathspecs and fixed strings that may
# still carry the name, one per line ('#' starts a comment).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

# git grep exits 1 when nothing matches; anything above (a bad pathspec, say)
# must fail the check instead of passing with nothing scanned.
scan() {
  local rc=0
  git grep "$@" || rc=$?
  [ "$rc" -le 1 ] || { echo "brand-check: git grep failed ($rc)" >&2; return "$rc"; }
}

allow=tools/ci/brand-allow.txt
pathspecs=(. ':!docs/superpowers' ":!$allow" ':!tools/ci/brand-check.sh')
strings=()
while IFS= read -r line; do
  line="${line%%#*}"
  line="${line%"${line##*[![:space:]]}"}"
  [ -n "$line" ] || continue
  case "$line" in
    path:*) pathspecs+=(":!${line#path:}") ;;
    *) strings+=("$line") ;;
  esac
done < "$allow"

hits=$(scan -n 'Oikos' -- "${pathspecs[@]}")
for s in ${strings[@]+"${strings[@]}"}; do
  hits=$(printf '%s\n' "$hits" | grep -vF -- "$s" || true)
done
hits=$(printf '%s\n' "$hits" | sed '/^$/d')

status=0
if [ -n "$hits" ]; then
  echo "The old name \"Oikos\" is still here (rename it to Nidoo, or allow it in $allow):"
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
# Wording the rename gets wrong: the web UI's lowercase wordmark, the docs
# address /Nidoo/ (GitHub Pages paths follow the repo name's case) and the article.
wording=$(scan -n -E -e '"oikos\."' -e 'github\.io/nidoo' -e '(^|[^[:alnum:]])[Aa]n Nidoo([^[:alnum:]]|$)' -- \
  . ':!docs/superpowers' ':!tools/ci/brand-check.sh')
if [ -n "$wording" ]; then
  echo "Brand wording to fix: wordmark \"nidoo.\", docs address /Nidoo/, \"a Nidoo\":"
  printf '%s\n' "$wording"
  status=1
fi
exit "$status"
