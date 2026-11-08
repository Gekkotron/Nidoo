#!/usr/bin/env bash
# Runs one shard of the web UI browser tests, so CI can spread them over parallel jobs.
# Usage: tools/ci/web-tests.sh INDEX TOTAL
# The shard holds every file of crates/nidoo-web/tests whose alphabetical position modulo TOTAL
# is INDEX. Each file stays its own binary with a page of its own: many tests leave their app
# mounted, so files cannot share a page.
set -euo pipefail

index=${1:?usage: web-tests.sh INDEX TOTAL}
total=${2:?usage: web-tests.sh INDEX TOTAL}

cd "$(dirname "$0")/../../crates/nidoo-web"

args=()
i=0
for file in tests/*.rs; do
  name=$(basename "$file" .rs)
  # A native test; cargo test runs it.
  [ "$name" = bare_text ] && continue
  if [ $((i % total)) -eq "$index" ]; then
    args+=(--test "$name")
  fi
  i=$((i + 1))
done

echo "shard $index of $total: $((${#args[@]} / 2)) of $i test files"
wasm-pack test --headless --chrome --chromedriver "${CHROMEWEBDRIVER:?}/chromedriver" -- --no-fail-fast "${args[@]}"
