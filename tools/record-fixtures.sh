#!/usr/bin/env bash
# Record retained broker messages as test fixtures for nidoo-core.
# Env: MQTT_HOST (required), MQTT_USER, MQTT_PASS (optional), MQTT_PORT (default 1883).
# Never reads credentials from a file; never writes them anywhere.
set -euo pipefail

: "${MQTT_HOST:?set MQTT_HOST}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/crates/nidoo-core/tests/fixtures"
RAW="$(mktemp)"
trap 'rm -f "$RAW"' EXIT

args=(-h "$MQTT_HOST" -p "${MQTT_PORT:-1883}" -W 6 -F '%r %t %p')
[ -n "${MQTT_USER:-}" ] && args+=(-u "$MQTT_USER")
[ -n "${MQTT_PASS:-}" ] && args+=(-P "$MQTT_PASS")
for t in 'homeassistant/#' 'oikos/#' 'zigbee2mqtt/bridge/info' \
         'zigbee2mqtt/bridge/devices' 'zigbee2mqtt/bridge/state' 'zigbee2mqtt/#'; do
  args+=(-t "$t")
done

# -W exits non-zero on timeout; that is the expected end of the window.
rc=0
mosquitto_sub "${args[@]}" >"$RAW" || rc=$?
if [ "$rc" -ne 0 ] && [ "$rc" -ne 27 ]; then
  echo "mosquitto_sub failed (exit $rc); fixtures left untouched" >&2
  exit 1
fi
if ! grep -q '^1 ' "$RAW"; then
  echo "no retained messages received; fixtures left untouched" >&2
  exit 1
fi

mkdir -p "$OUT"
rm -rf "$OUT/discovery" "$OUT/states" "$OUT/ui"
mkdir -p "$OUT/discovery" "$OUT/states" "$OUT/ui"

python3 -I - "$RAW" "$OUT" <<'PY'
import json, os, re, sys
raw, out = sys.argv[1], sys.argv[2]
msgs = {}
cur = None
for line in open(raw, encoding="utf-8", errors="replace").read().split("\n"):
    m = re.match(r"^([01]) (\S+) ?(.*)$", line)
    if m:
        cur = None
        # Only retained messages count; keep the first one per topic.
        if m.group(1) == "1" and m.group(2) not in msgs:
            cur = m.group(2)
            msgs[cur] = ["1", m.group(3)]
    elif cur is not None:
        msgs[cur][1] += "\n" + line
SECRET_KEYS = {"password", "network_key", "pan_id", "ext_pan_id", "extended_pan_id"}
def scrub(o, parent=None):
    # Blank secrets anywhere in Z2M's config; leave schema descriptors (dict values) alone.
    if isinstance(o, dict):
        for k, v in o.items():
            if isinstance(v, (dict, list)) and not (k in SECRET_KEYS and isinstance(v, list)):
                scrub(v, k)
            elif k in SECRET_KEYS or (k == "user" and parent == "mqtt"):
                o[k] = "***"
    elif isinstance(o, list):
        for v in o:
            scrub(v, parent)
    return o
def write(rel, text):
    p = os.path.join(out, rel)
    with open(p, "w", encoding="utf-8") as f:
        f.write(text.rstrip("\n") + "\n")
rec = []
for topic in sorted(msgs):
    retain, payload = msgs[topic]
    if retain != "1" or not payload.strip():
        continue
    if topic == "zigbee2mqtt/bridge/info":
        write("bridge_info.json", json.dumps(scrub(json.loads(payload)), indent=2, ensure_ascii=False))
    elif topic == "zigbee2mqtt/bridge/devices":
        write("bridge_devices.json", payload)
    elif topic == "zigbee2mqtt/bridge/state":
        write("bridge_state.json", payload)
    elif topic.startswith("homeassistant/") and topic.endswith("/config"):
        name = topic[len("homeassistant/"):-len("/config")].replace("/", "__")
        write("discovery/%s.json" % name, payload)
    elif topic == "oikos/ui/_areas":
        write("ui/_areas.json", payload)
    elif topic.startswith("oikos/ui/"):
        write("ui/%s.json" % topic[len("oikos/ui/"):].replace("/", "__"), payload)
    elif topic.startswith("zigbee2mqtt/") and not topic.startswith("zigbee2mqtt/bridge/") \
            and not re.search(r"/(set|get|availability)$", topic):
        write("states/%s.json" % topic[len("zigbee2mqtt/"):].replace("/", "__"), payload)
    else:
        continue
    rec.append("%s %s" % (retain, topic))
write("topics.txt", "\n".join(rec))
print("recorded", len(rec), "topics")
PY
# shellcheck disable=SC2012 # plain file names, count only
for d in discovery states ui; do echo "$d: $(ls "$OUT/$d" | wc -l | tr -d ' ') files"; done
