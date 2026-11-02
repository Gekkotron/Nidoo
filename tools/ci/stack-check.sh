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
    docker logs nidoo-oikos-seed 2>&1 | tail -30 || true
  fi
  docker compose down -v || true
  docker rm -fv nidoo-oikos-seed >/dev/null 2>&1 || true
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
  if docker exec nidoo-timescaledb pg_isready -h localhost -p 5433 >/dev/null 2>&1; then
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

extension=$(docker exec nidoo-timescaledb psql \
  "postgresql://nidoo:${db_password}@${host_ip}:5433/nidoo" \
  -tAc "select extname from pg_extension where extname='timescaledb'")
[ "$extension" = "timescaledb" ] || { echo "timescaledb extension missing: '$extension'" >&2; exit 1; }
if docker exec nidoo-timescaledb psql \
  "postgresql://nidoo:wrong@${host_ip}:5433/nidoo" -tAc "select 1" >/dev/null 2>&1; then
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
  if docker exec nidoo-timescaledb pg_isready -h localhost -p 5433 >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 1
done
[ "$ready" -eq 1 ] || { echo "timescaledb not ready without a password file after 60 s" >&2; exit 1; }
extension=$(docker exec nidoo-timescaledb psql \
  "postgresql://nidoo:${db_password}@${host_ip}:5433/nidoo" \
  -tAc "select extname from pg_extension where extname='timescaledb'")
[ "$extension" = "timescaledb" ] || { echo "right password refused without a password file: '$extension'" >&2; exit 1; }

echo "== mosquitto"
published=0
for _ in $(seq 1 20); do
  if docker exec nidoo-mosquitto mosquitto_pub -h localhost \
    -u nidoo -P "$mqtt_password" -t nidoo/ci -m ok >/dev/null 2>&1; then
    published=1
    break
  fi
  sleep 1
done
[ "$published" -eq 1 ] || { echo "mosquitto refused the right password for 20 s" >&2; exit 1; }
if docker exec nidoo-mosquitto mosquitto_pub -h localhost \
  -u nidoo -P wrong -t nidoo/ci -m ok >/dev/null 2>&1; then
  echo "mosquitto accepted a wrong password" >&2
  exit 1
fi

# The switch-over, last. The bundled stack now starts as nidoo, so the old
# oikos install is seeded here: a second throwaway TimescaleDB whose bootstrap
# superuser is "postgres", holding a superuser role and a database both named
# oikos (a bootstrap role cannot be renamed, and the command refuses when the
# target names already exist, so the bundled nidoo cluster cannot play the
# old install). Mosquitto keeps the nidoo login; only the topics are old.
echo "== migrate-oikos"
seed=nidoo-oikos-seed
seed_port=5434
# A leftover from a killed local run would break the docker run below.
docker rm -fv "$seed" >/dev/null 2>&1 || true
docker run -d --name "$seed" --network host \
  -e POSTGRES_PASSWORD="$db_password" \
  timescale/timescaledb:2.30.2-pg17 postgres -c "port=${seed_port}" >/dev/null
seed_psql() {
  docker exec "$seed" psql -h localhost -p "$seed_port" -U postgres "$@"
}
ready=0
for _ in $(seq 1 60); do
  if seed_psql -d postgres -tAc "select 1" >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 1
done
[ "$ready" -eq 1 ] || { echo "seed database not ready after 60 s" >&2; exit 1; }
seed_psql -d postgres -v ON_ERROR_STOP=1 -c "create role oikos superuser login password '${db_password}'"
seed_psql -d postgres -v ON_ERROR_STOP=1 -c "create database oikos owner oikos"
# The throwaway one-off container reaches the host-network services directly.
jq -n --arg pw "$mqtt_password" --arg dbpw "$db_password" --arg port "$seed_port" '{
  mqtt: {host: "localhost", port: 1883, user: "nidoo", password: $pw},
  database_url: "postgresql://oikos:\($dbpw)@localhost:\($port)/oikos"
}' > data/hub.json
wait_hub() {
  for _ in $(seq 1 60); do
    if curl -fsS http://localhost:8081/health >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  docker compose logs hub
  return 1
}
# The hub creates its tables on boot, which the migration expects to find.
docker compose up -d --no-deps hub
wait_hub || { echo "hub not healthy on the old database" >&2; exit 1; }
tables=0
for _ in $(seq 1 60); do
  if [ "$(seed_psql -d oikos -tAc "select to_regclass('settings') is not null")" = "t" ]; then
    tables=1
    break
  fi
  sleep 1
done
[ "$tables" -eq 1 ] || { echo "hub created no tables after 60 s" >&2; exit 1; }
docker exec nidoo-mosquitto mosquitto_pub -h localhost -u nidoo -P "$mqtt_password" \
  -r -t oikos/ui/oikos_t -m '{"name":"oikos t"}'
migrate() {
  docker compose run --rm --no-deps -T hub --data /data migrate-oikos "$@"
}
# While the hub answers, the command refuses (dry run and apply), for that
# reason and no other, writing nothing.
for args in "" "--backup /data/refused.jsonl --apply"; do
  # shellcheck disable=SC2086
  if out=$(migrate $args 2>&1); then
    echo "migrate-oikos ${args:-(dry run)} ran while the hub was up" >&2
    exit 1
  fi
  grep -q "the hub answers" <<<"$out" \
    || { echo "migrate-oikos ${args:-(dry run)} refused for the wrong reason: $out" >&2; exit 1; }
done
backup=$(docker compose run --rm --no-deps -T --entrypoint sh hub -c \
  'if [ -e /data/refused.jsonl ]; then echo present; else echo absent; fi')
[ "$backup" = "absent" ] || { echo "a refused apply left a backup behind: '$backup'" >&2; exit 1; }
docker compose stop hub
migrate
migrate --backup /data/backup.jsonl --apply
got=$(docker exec nidoo-mosquitto mosquitto_sub -h localhost -u nidoo -P "$mqtt_password" \
  -t nidoo/ui/nidoo_t -C 1 -W 3)
[ "$got" = '{"name":"nidoo t"}' ] || { echo "migrated payload wrong: '$got'" >&2; exit 1; }
old=$(docker exec nidoo-mosquitto mosquitto_sub -h localhost -u nidoo -P "$mqtt_password" \
  -t oikos/ui/oikos_t -C 1 -W 3 || true)
[ -z "$old" ] || { echo "old retained topic still there: '$old'" >&2; exit 1; }
# The database and the role were renamed.
[ "$(seed_psql -d postgres -tAc "select count(*) from pg_database where datname='nidoo'")" = "1" ] \
  || { echo "database nidoo missing after the switch-over" >&2; exit 1; }
[ "$(seed_psql -d postgres -tAc "select count(*) from pg_database where datname='oikos'")" = "0" ] \
  || { echo "database oikos still there after the switch-over" >&2; exit 1; }
[ "$(seed_psql -d postgres -tAc "select count(*) from pg_roles where rolname='nidoo'")" = "1" ] \
  || { echo "role nidoo missing after the switch-over" >&2; exit 1; }
[ "$(seed_psql -d postgres -tAc "select count(*) from pg_roles where rolname='oikos'")" = "0" ] \
  || { echo "role oikos still there after the switch-over" >&2; exit 1; }
migrate --verify
# The one-off container wrote hub.json as its own user (mode 0600), so read it there.
migrate_cat() {
  docker compose run --rm --no-deps -T --entrypoint cat hub /data/hub.json
}
migrate_cat | jq -e --arg url "postgresql://nidoo:${db_password}@localhost:${seed_port}/nidoo" \
  '.database_url == $url' >/dev/null \
  || { echo "hub.json database_url is not the renamed one" >&2; exit 1; }
docker compose up -d --no-deps hub
wait_hub || { echo "hub not healthy on the renamed database" >&2; exit 1; }
# /health says ok whatever the database does, so ask Postgres itself: the hub
# must hold a session as role nidoo on database nidoo. Asked as the seed's
# superuser, so this check's own session is not the one counted; polled
# because the pool connects lazily.
session=0
for _ in $(seq 1 60); do
  if [ "$(seed_psql -d postgres -tAc \
    "select count(*) from pg_stat_activity where datname='nidoo' and usename='nidoo' and application_name <> 'psql'")" -ge 1 ]; then
    session=1
    break
  fi
  sleep 1
done
[ "$session" -eq 1 ] || { echo "the hub holds no session as nidoo on the renamed database" >&2; exit 1; }

echo "stack ok"
