---
title: History
sidebar_position: 11
description: Record every entity state change in TimescaleDB, connect it in the setup wizard or Settings, query it and back it up with pg_dump.
---

The hub records every entity state change (and availability change) in TimescaleDB for years, so the web UI and MCP can answer "what did this sensor read" and "how much energy today". The database is a required step of the setup wizard, because the hub keeps its settings there too: the Docker install starts a TimescaleDB for you and pre-fills the step. To point the hub at another one, use **Settings → Database** (host, port, database, user, password) and **Test connection**, which also prepares the tables. The password is write-only, and changing the host or port needs it entered again. Saving restarts the hub. See the [Settings reference](../reference/configuration.md#database-and-broker).

The hub keeps up to 100000 states in memory while the database is down (the `history_buffer_rows` field of the advanced settings, changed through `PUT /api/v1/settings/advanced` or `set_settings`).

Any TimescaleDB 2.17+ on Postgres 17 works, as long as the role can log in and `CREATE EXTENSION timescaledb` has been run inside the target database (the extension itself must be in `shared_preload_libraries` on the server). The hub applies its own schema migrations on first connect, writes every state change inline from `Hub::ingest` (never on the hot MQTT path — the writer runs in a bounded, try-send queue) and rolls each hour up into `history_hourly` every five minutes. A down database never blocks the house: up to 100000 states wait, past that the oldest are dropped with one warning per outage.

## Querying

Routes under `/api/v1/history/*` (see the [HTTP API](../reference/http-api.md)): `series` for charts (`range` is `1h`, `6h`, `24h`, `7d`, `30d` or `1y`), `daily` for kWh per day over 7, 30 or 365 days, `today` for kWh since local midnight (`daily` and `today` take `kind=counter` or `kind=power`), `last_change` for 1 to 50 entities, and `grid_day` for what crossed the grid meter on one day in half hours (bought off-peak and peak, sold; `date=YYYY-MM-DD`, today when left out; with an off-peak device the split follows its recorded readings, so keep history on for it). Errors are `400 bad_request`, `404 not_recorded` (the entity has no rows) and `503 history_unavailable` (the database is not connected, or is down). MCP exposes the same through `get_history` and `get_grid_day`; `grid_day` also answers `404 no_grid_meter` when no entity with the `grid` role has an energy counter.

## In the web UI

Every entity sheet has a **View history →** link to `/history/<id>`, a full-screen chart with range chips (Last hour, Last 6 hours, Last 24 hours, Last 7 days, Last 30 days, Last year; the range is kept in the URL as `?range=`). A sensor draws a smooth line you can hover for the value, with the min and max band on the 7-day and longer ranges. An energy counter (kWh and similar) draws its paired power sensor over the short ranges, or kWh bars per day over 7, 30 or 365 days (a single day's bar when there is no power sensor). An entity with no rows shows "No data recorded for this entity yet", and a database that is down shows "History is unavailable right now".

## Retention

Nothing is deleted by age, except the [system health](system-health.md) sensors (`system_*`) and the hub's restart log, which are kept 90 days. The rest of the history is kept as long as you keep the database.

## Backup

Back up with `pg_dump` from any client that can reach the database:

```sh
pg_dump -h <host> -p <port> -U <db-user> -Fc <db-name> > history-$(date +%F).dump
```
