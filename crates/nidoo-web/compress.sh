#!/bin/sh
# Trunk post_build hook: writes `.gz` (and `.br` when `brotli` is installed) next to the
# wasm, js and css of a release build, for the hub to send to browsers that accept them.
# Debug builds (`trunk serve`) are left alone to keep the dev loop fast.
set -eu
[ "${TRUNK_PROFILE:-}" = release ] || exit 0
cd "$TRUNK_STAGING_DIR"
for f in *.wasm *.js *.css; do
  [ -f "$f" ] || continue
  gzip -9 -n -k -f "$f"
  if command -v brotli >/dev/null 2>&1; then
    brotli -q 9 -k -f "$f"
  fi
done
