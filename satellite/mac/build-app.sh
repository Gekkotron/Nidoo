#!/usr/bin/env bash
# Builds "Nidoo Satellite.app" from source on this Mac, ad-hoc signed.
#
#   ./build-app.sh            → $CARGO_TARGET_DIR/Nidoo Satellite.app
#   ./build-app.sh --install  → the same, then copied to ~/Applications
set -euo pipefail

install=false
case "${1:-}" in
  --install) install=true ;;
  "") ;;
  *) echo "usage: $0 [--install]" >&2; exit 2 ;;
esac

here="$(cd "$(dirname "$0")" && pwd)"
cd "$here"
cargo build --release

target="${CARGO_TARGET_DIR:-$here/target}"
app="$target/Nidoo Satellite.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp Info.plist "$app/Contents/Info.plist"
cp "$target/release/nidoo-satellite-mac" "$app/Contents/MacOS/nidoo-satellite-mac"
codesign --force --sign - "$app"
echo "built $app"

if "$install"; then
  mkdir -p "$HOME/Applications"
  rm -rf "$HOME/Applications/Nidoo Satellite.app"
  cp -R "$app" "$HOME/Applications/"
  echo "installed $HOME/Applications/Nidoo Satellite.app"
fi
