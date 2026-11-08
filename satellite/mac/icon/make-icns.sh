#!/usr/bin/env bash
# Renders AppIcon.svg to AppIcon.icns (the app's Finder and Dock icon), with
# Google Chrome headless for the SVG and the Mac's own sips and iconutil.
# Run it after editing the SVG; build-app.sh only copies the .icns.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
chrome="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Drawn at 2x and scaled down: smoother edges than a 1x render.
# Its own profile, so an open Chrome window does not take the job over.
"$chrome" --headless --hide-scrollbars --force-device-scale-factor=2 \
  --user-data-dir="$work/profile" --no-first-run \
  --default-background-color=00000000 --window-size=1024,1024 \
  --screenshot="$work/2048.png" "file://$here/AppIcon.svg" >"$work/chrome.log" 2>&1 || true
if [ ! -s "$work/2048.png" ]; then
  echo "Chrome did not render AppIcon.svg:" >&2
  tail -5 "$work/chrome.log" >&2
  exit 1
fi

set_dir="$work/AppIcon.iconset"
mkdir "$set_dir"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$work/2048.png" --out "$set_dir/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$work/2048.png" --out "$set_dir/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil --convert icns "$set_dir" --output "$here/AppIcon.icns"
echo "wrote $here/AppIcon.icns"
