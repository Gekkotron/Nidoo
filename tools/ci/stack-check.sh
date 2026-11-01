#!/usr/bin/env bash
# Proves the bundled stack with the real images: compose.yml parses, the hub
# does not depend on the bundled services, and Mosquitto and TimescaleDB
# start on the credentials init-bundled wrote and enforce them.
# Run from the repo root on a Linux host with Docker, jq and cargo.
set -euo pipefail

cd "$(dirname "$0")/../.."

# Checked before the trap exists, so cleanup never deletes a file of the user's.
if [ -e compose.override.yml ]; then
  echo "compose.override.yml already exists; move it away before running this check" >&2
  exit 1
fi

# Also before the trap: cleanup runs `docker compose down -v`, which would tear
# down a developer's own stack, and the check would write into its ./data.
if [ -e data ]; then
  echo "./data already exists; this check writes credentials there; move it away first" >&2
  exit 1
fi
if [ -n "$(docker compose ps -q)" ]; then
  echo "a compose stack is already running here; stop it before running this check" >&2
  exit 1
fi

cleanup() {
  status=$?
  if [ "$status" -ne 0 ]; then
    docker compose logs mosquitto timescaledb || true
  fi
  docker compose down -v || true
  rm -f compose.override.yml
  exit "$status"
}
trap cleanup EXIT

echo "== compose file"
docker compose config -q
config=$(docker compose config --format json)
echo "$config" | jq -e '.services.hub.depends_on == null' >/dev/null
echo "$config" | jq -e '.services | has("init") and has("mosquitto") and has("timescaledb")' >/dev/null

echo "== init-bundled"
cargo run -q -p nidoo -- --data ./data init-bundled

# The migration guide's override shape: a second port for Postgres.
cat > compose.override.yml <<'YAML'
services:
  timescaledb:
    command: postgres -c port=5433
YAML

# --no-deps: init already ran natively; starting it would build the hub image.
docker compose up -d --no-deps mosquitto timescaledb

mqtt_password=$(jq -er .mqtt_password data/bundled.json)
db_password=$(jq -er .db_password data/bundled.json)

echo "== timescaledb"
ready=0
for _ in $(seq 1 60); do
  if docker exec oikos-timescaledb pg_isready -h localhost -p 5433 >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 1
done
[ "$ready" -eq 1 ] || { echo "timescaledb not ready after 60 s" >&2; exit 1; }

# The image trusts loopback connections, so only a non-loopback address
# really tests the password (it goes through the scram-sha-256 line).
host_ip=$(hostname -I | awk '{print $1}')
[ -n "$host_ip" ] || { echo "no non-loopback address found" >&2; exit 1; }

extension=$(docker exec oikos-timescaledb psql \
  "postgresql://oikos:${db_password}@${host_ip}:5433/oikos" \
  -tAc "select extname from pg_extension where extname='timescaledb'")
[ "$extension" = "timescaledb" ] || { echo "timescaledb extension missing: '$extension'" >&2; exit 1; }
if docker exec oikos-timescaledb psql \
  "postgresql://oikos:wrong@${host_ip}:5433/oikos" -tAc "select 1" >/dev/null 2>&1; then
  echo "timescaledb accepted a wrong password" >&2
  exit 1
fi

# An override host has no password file (init skipped), yet the wrapper must
# still start on the already-initialised database: the password set the first
# time keeps working.
echo "== timescaledb without a password file"
docker compose stop timescaledb
mv data/bundled/db-password data/bundled/db-password.away
docker compose up -d --no-deps timescaledb
ready=0
for _ in $(seq 1 60); do
  if docker exec oikos-timescaledb pg_isready -h localhost -p 5433 >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 1
done
[ "$ready" -eq 1 ] || { echo "timescaledb not ready without a password file after 60 s" >&2; exit 1; }
extension=$(docker exec oikos-timescaledb psql \
  "postgresql://oikos:${db_password}@${host_ip}:5433/oikos" \
  -tAc "select extname from pg_extension where extname='timescaledb'")
[ "$extension" = "timescaledb" ] || { echo "right password refused without a password file: '$extension'" >&2; exit 1; }

echo "== mosquitto"
published=0
for _ in $(seq 1 20); do
  if docker exec oikos-mosquitto mosquitto_pub -h localhost \
    -u oikos -P "$mqtt_password" -t oikos/ci -m ok >/dev/null 2>&1; then
    published=1
    break
  fi
  sleep 1
done
[ "$published" -eq 1 ] || { echo "mosquitto refused the right password for 20 s" >&2; exit 1; }
if docker exec oikos-mosquitto mosquitto_pub -h localhost \
  -u oikos -P wrong -t oikos/ci -m ok >/dev/null 2>&1; then
  echo "mosquitto accepted a wrong password" >&2
  exit 1
fi

echo "stack ok"
