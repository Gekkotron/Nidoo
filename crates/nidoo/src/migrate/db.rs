//! Database side of the switch-over: rename the `oikos_` entity ids in every table, rewrite
//! the settings values, then rename the database and role. Throwaway like the rest of the
//! module.

use anyhow::{Context, Result, bail};
use percent_encoding::percent_decode_str;
use sqlx::postgres::{PgConnectOptions, PgConnection};
use sqlx::{Connection, PgPool, Row};
use url::Url;

use super::rewrite::{Change, rename_json};

/// The temporary superuser that renames the database and role from outside.
const TMP_ROLE: &str = "nidoo_migrate_tmp";

/// How many times the background workers are asked to start, a second apart, while the
/// launcher has no free worker slot.
const START_ATTEMPTS: u32 = 10;

/// What became of TimescaleDB's background workers (the scheduler that runs the compression
/// and retention jobs) on the renamed database. `ALTER DATABASE ... RENAME` stops them, and
/// TimescaleDB only warns that they must be restarted by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Workers {
    /// Running again.
    Started,
    /// The database has no TimescaleDB, so it has no workers.
    NotInstalled,
    /// Still stopped; the reason says why.
    NotStarted(String),
}

/// Asks the launcher to start the scheduler of the database `conn` is on. Needs a superuser
/// session. Never fails: the outcome is reported instead, after the rename it cannot undo.
async fn start_workers(conn: &mut PgConnection) -> Workers {
    // TimescaleDB 2.12 moved the function from `_timescaledb_internal` to
    // `_timescaledb_functions`; ask the catalog rather than guess.
    let schema: Option<String> = match sqlx::query_scalar(
        "SELECT n.nspname::text FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
         WHERE p.proname = 'start_background_workers' \
           AND n.nspname IN ('_timescaledb_functions', '_timescaledb_internal') \
         ORDER BY n.nspname = '_timescaledb_functions' DESC LIMIT 1",
    )
    .fetch_optional(&mut *conn)
    .await
    {
        Ok(s) => s,
        Err(e) => return Workers::NotStarted(format!("{e}")),
    };
    let Some(schema) = schema else {
        return Workers::NotInstalled;
    };
    let sql = format!("SELECT {}.start_background_workers()", quote_ident(&schema));
    for attempt in 1..=START_ATTEMPTS {
        match sqlx::query_scalar::<_, bool>(&sql)
            .fetch_one(&mut *conn)
            .await
        {
            Ok(true) => return Workers::Started,
            Ok(false) if attempt < START_ATTEMPTS => {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            Ok(false) => break,
            Err(e) => return Workers::NotStarted(format!("{e}")),
        }
    }
    Workers::NotStarted(format!(
        "the launcher had no free background worker slot after {START_ATTEMPTS} tries"
    ))
}

/// `"name"` with embedded quotes doubled, safe to splice into SQL.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// `'text'` with embedded quotes doubled, for the places a bind parameter is not allowed.
fn quote_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Every `public` base table with an `entity_id text` column. Views are left out.
pub async fn entity_id_tables(pool: &PgPool) -> sqlx::Result<Vec<String>> {
    let rows = sqlx::query(
        "SELECT c.table_name FROM information_schema.columns c \
         JOIN information_schema.tables t \
           ON t.table_schema = c.table_schema AND t.table_name = c.table_name \
         WHERE c.table_schema = 'public' AND c.column_name = 'entity_id' \
           AND c.data_type = 'text' AND t.table_type = 'BASE TABLE' \
         ORDER BY c.table_name",
    )
    .fetch_all(pool)
    .await?;
    rows.iter().map(|r| r.try_get(0)).collect()
}

/// The columns, besides `entity_id`, that make a row unique within its table.
///
/// Read from the primary key (or else the smallest unique index) that covers `entity_id`.
/// A table with no such key (the hypertables only carry non-unique indexes) falls back to
/// `time` when it has one, which is the key the history writer treats as identity. A table
/// with neither is renamed with a plain UPDATE: no collision can be detected there.
async fn key_columns(conn: &mut PgConnection, table: &str) -> sqlx::Result<Vec<String>> {
    let regclass = format!("public.{}", quote_ident(table));
    let rows = sqlx::query(
        "SELECT array_agg(a.attname::text ORDER BY array_position(i.indkey::int2[], a.attnum)) \
         FROM pg_index i \
         JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY (i.indkey::int2[]) \
         WHERE i.indrelid = $1::regclass AND i.indisunique AND i.indpred IS NULL \
         GROUP BY i.indexrelid, i.indisprimary, i.indnatts \
         ORDER BY i.indisprimary DESC, i.indnatts, i.indexrelid",
    )
    .bind(&regclass)
    .fetch_all(&mut *conn)
    .await?;
    for row in rows {
        let cols: Vec<String> = row.try_get(0)?;
        if cols.iter().any(|c| c == "entity_id") {
            return Ok(cols.into_iter().filter(|c| c != "entity_id").collect());
        }
    }
    let has_time: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = $1 AND column_name = 'time')",
    )
    .bind(table)
    .fetch_one(&mut *conn)
    .await?;
    Ok(if has_time {
        vec!["time".into()]
    } else {
        vec![]
    })
}

/// Renames the `oikos_` entity ids to `nidoo_` in every table, in one transaction. A row whose
/// renamed key already exists is deleted first: the new row wins. Returns each table with the
/// rows renamed. A dry run does the same work and rolls back.
pub async fn rename_entity_ids(pool: &PgPool, dry_run: bool) -> sqlx::Result<Vec<(String, u64)>> {
    let tables = entity_id_tables(pool).await?;
    let mut tx = pool.begin().await?;
    // Updating compressed chunks decompresses rows; the default cap (100000 per transaction)
    // would abort a live switch-over. 0 lifts it. Every target runs TimescaleDB, so no guard.
    sqlx::query("SET LOCAL timescaledb.max_tuples_decompressed_per_dml_transaction = 0")
        .execute(&mut *tx)
        .await?;
    let mut out = Vec::new();
    for table in tables {
        let keys = key_columns(&mut tx, &table).await?;
        let t = quote_ident(&table);
        // A collision needs a `nidoo_` row. With none in the table (the usual case: the old hub
        // never wrote one) there is nothing to delete or decompress, and the self-join below
        // must not run: on a large compressed hypertable it decompresses the whole table to
        // build its `n` side. The probe is cheap where `entity_id` is the segmentby column.
        let has_new: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS (SELECT 1 FROM {t} WHERE entity_id LIKE 'nidoo\\_%')"
        ))
        .fetch_one(&mut *tx)
        .await?;
        if !has_new {
            let n = rename_rows(&mut tx, &t).await?;
            out.push((table, n));
            continue;
        }
        // A DELETE ... USING self-join cannot see rows inside a compressed chunk: the planner
        // has no filter to decompress the target batches with, so the collision rows survive
        // and the UPDATE below then renames into a duplicate. Decompress first, but only where a
        // collision is possible; the compression policy recompresses after the commit.
        let compressed: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM timescaledb_information.hypertables \
             WHERE hypertable_schema = 'public' AND hypertable_name = $1 \
               AND compression_enabled)",
        )
        .bind(&table)
        .fetch_one(&mut *tx)
        .await?;
        if compressed {
            // A colliding pair shares its time key, so both rows sit in one chunk: only a chunk
            // that already holds a `nidoo_` row can hold a collision. That holds when the key
            // contains the hypertable's partitioning column; if it does not, every compressed
            // chunk is decompressed (the safe fallback). The probe is cheap where `entity_id`
            // is the segmentby column and a plain scan elsewhere (e.g. `climate_decision`,
            // segmented by `zone`). On a house with no `nidoo_` rows nothing is decompressed.
            let key_has_partitioning: bool = sqlx::query_scalar(
                "SELECT NOT EXISTS (SELECT 1 FROM timescaledb_information.dimensions \
                 WHERE hypertable_schema = 'public' AND hypertable_name = $1 \
                   AND dimension_number = 1 AND NOT (column_name = ANY ($2)))",
            )
            .bind(&table)
            .bind(&keys)
            .fetch_one(&mut *tx)
            .await?;
            let chunks: Vec<String> = sqlx::query_scalar(
                "SELECT format('%I.%I', chunk_schema, chunk_name) \
                 FROM timescaledb_information.chunks \
                 WHERE hypertable_schema = 'public' AND hypertable_name = $1 AND is_compressed",
            )
            .bind(&table)
            .fetch_all(&mut *tx)
            .await?;
            for chunk in chunks {
                let needs_decompress = if key_has_partitioning {
                    sqlx::query_scalar(&format!(
                        "SELECT EXISTS (SELECT 1 FROM {chunk} WHERE entity_id LIKE 'nidoo\\_%')"
                    ))
                    .fetch_one(&mut *tx)
                    .await?
                } else {
                    true
                };
                if needs_decompress {
                    sqlx::query(
                        "SELECT decompress_chunk($1::text::regclass, if_compressed => true)",
                    )
                    .bind(&chunk)
                    .execute(&mut *tx)
                    .await?;
                }
            }
        }
        // `n.entity_id LIKE 'nidoo\_%'` is implied by the equality, but as a constant filter it
        // lets TimescaleDB scan only the `nidoo_` segments for the `n` side.
        let mut delete = format!(
            "DELETE FROM {t} o USING {t} n \
             WHERE o.entity_id LIKE 'oikos\\_%' \
               AND n.entity_id LIKE 'nidoo\\_%' \
               AND n.entity_id = 'nidoo_' || substr(o.entity_id, 7)"
        );
        for k in &keys {
            let k = quote_ident(k);
            delete.push_str(&format!(" AND n.{k} = o.{k}"));
        }
        sqlx::query(&delete).execute(&mut *tx).await?;
        let n = rename_rows(&mut tx, &t).await?;
        out.push((table, n));
    }
    if dry_run {
        tx.rollback().await?;
    } else {
        tx.commit().await?;
    }
    Ok(out)
}

/// Renames the `oikos_` entity ids of the (quoted) table `t`; returns the rows renamed.
async fn rename_rows(conn: &mut PgConnection, t: &str) -> sqlx::Result<u64> {
    let update = format!(
        "UPDATE {t} SET entity_id = 'nidoo_' || substr(entity_id, 7) \
         WHERE entity_id LIKE 'oikos\\_%'"
    );
    Ok(sqlx::query(&update)
        .execute(&mut *conn)
        .await?
        .rows_affected())
}

/// Rewrites every `settings` value that mentions `oikos` (any casing) with
/// [`rename_json`]. Returns each changed key with its changes. A dry run rolls back.
pub async fn rename_settings(
    pool: &PgPool,
    dry_run: bool,
) -> sqlx::Result<Vec<(String, Vec<Change>)>> {
    let mut tx = pool.begin().await?;
    let rows = sqlx::query(
        "SELECT key, value FROM settings WHERE value::text ILIKE '%oikos%' ORDER BY key",
    )
    .fetch_all(&mut *tx)
    .await?;
    let mut out = Vec::new();
    for row in rows {
        let key: String = row.try_get("key")?;
        let value: serde_json::Value = row.try_get("value")?;
        let (renamed, changes) = rename_json(value);
        if changes.is_empty() {
            continue;
        }
        sqlx::query("UPDATE settings SET value = $2 WHERE key = $1")
            .bind(&key)
            .bind(&renamed)
            .execute(&mut *tx)
            .await?;
        out.push((key, changes));
    }
    if dry_run {
        tx.rollback().await?;
    } else {
        tx.commit().await?;
    }
    Ok(out)
}

/// The URL with the user and the database path segment swapped when they equal `from`.
pub fn renamed_url(url: &str, from: &str, to: &str) -> String {
    let Ok(mut parsed) = Url::parse(url) else {
        return url.to_string();
    };
    if parsed.username() == from {
        let _ = parsed.set_username(to);
    }
    if parsed.path().trim_start_matches('/') == from {
        parsed.set_path(&format!("/{to}"));
    }
    parsed.to_string()
}

/// Connection options for `admin_url`'s server with another user, password and database.
fn options(
    admin_url: &str,
    user: &str,
    password: &str,
    database: &str,
) -> Result<PgConnectOptions> {
    let url = Url::parse(admin_url).context("database url")?;
    Ok(PgConnectOptions::new()
        .host(url.host_str().context("database url has no host")?)
        .port(url.port().unwrap_or(5432))
        .username(user)
        .password(password)
        .database(database))
}

fn random_password() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|e| anyhow::anyhow!("random bytes: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

async fn role_exists(conn: &mut PgConnection, role: &str) -> sqlx::Result<bool> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)")
        .bind(role)
        .fetch_one(conn)
        .await
}

async fn drop_tmp_role(conn: &mut PgConnection) -> Result<()> {
    if role_exists(conn, TMP_ROLE).await? {
        sqlx::query(&format!("DROP ROLE {}", quote_ident(TMP_ROLE)))
            .execute(conn)
            .await?;
    }
    Ok(())
}

/// Renames the database and the role `from` to `to`. Run with the hub stopped, since a
/// database cannot be renamed while connected to. `admin_url` is the hub's URL (user `from`,
/// database `from`), so the same password is kept for `to`. Safe to run again: only "`from`
/// gone, `to` present" counts as already switched over. If both exist, nothing is renamed and
/// it fails. The temporary superuser is dropped on every path.
///
/// Then, on every path, TimescaleDB's background workers are started again on `to` (the
/// rename stopped them) and the outcome is returned.
pub async fn rename_db_and_role(admin_url: &str, from: &str, to: &str) -> Result<Workers> {
    let url = Url::parse(admin_url).context("database url")?;
    let password = percent_decode_str(url.password().unwrap_or(""))
        .decode_utf8()
        .context("database password")?
        .into_owned();

    // 1. A temporary superuser, because neither the role nor the database can be renamed
    //    from a session that uses them.
    let tmp_password = random_password()?;
    match PgConnection::connect_with(&options(admin_url, from, &password, "postgres")?).await {
        Ok(mut conn) => {
            let verb = if role_exists(&mut conn, TMP_ROLE).await? {
                "ALTER"
            } else {
                "CREATE"
            };
            sqlx::query(&format!(
                "{verb} ROLE {} SUPERUSER LOGIN PASSWORD {}",
                quote_ident(TMP_ROLE),
                quote_literal(&tmp_password)
            ))
            .execute(&mut conn)
            .await?;
            conn.close().await?;
        }
        Err(first) => {
            // Already switched over: the new role works, so only a leftover temp role remains.
            let mut conn =
                match PgConnection::connect_with(&options(admin_url, to, &password, "postgres")?)
                    .await
                {
                    Ok(c) => c,
                    Err(_) => {
                        return Err(first).context(format!("connecting as {from} or {to}"));
                    }
                };
            drop_tmp_role(&mut conn).await?;
            conn.close().await?;
            // A run stopped between the rename and the restart left them stopped. This needs
            // `to` to be a superuser; the outcome says so when it is not.
            let workers =
                match PgConnection::connect_with(&options(admin_url, to, &password, to)?).await {
                    Ok(mut conn) => {
                        let w = start_workers(&mut conn).await;
                        let _ = conn.close().await;
                        w
                    }
                    Err(e) => Workers::NotStarted(format!("connecting as {to} to {to}: {e}")),
                };
            return Ok(workers);
        }
    }

    // 2 and 3, then always drop the temporary role.
    let result = switch(admin_url, from, to, &password, &tmp_password).await;
    let dropped = drop_tmp_role_after(admin_url, from, to, &password).await;
    match (result, dropped) {
        (Ok(w), Ok(())) => Ok(w),
        (Err(e), Ok(())) => Err(e),
        (Ok(_), Err(d)) => Err(d),
        (Err(e), Err(d)) => {
            Err(e.context(format!("and the temporary role was left behind: {d:#}")))
        }
    }
}

/// Renames as the temporary role, then checks the new role logs in with the old password, then
/// starts the background workers on the renamed database, still as the temporary superuser.
async fn switch(
    admin_url: &str,
    from: &str,
    to: &str,
    password: &str,
    tmp_password: &str,
) -> Result<Workers> {
    let mut tmp =
        PgConnection::connect_with(&options(admin_url, TMP_ROLE, tmp_password, "postgres")?)
            .await
            .context("connecting as the temporary role")?;
    let result = rename_as_tmp(&mut tmp, admin_url, from, to, password).await;
    let _ = tmp.close().await;
    result?;
    Ok(
        match PgConnection::connect_with(&options(admin_url, TMP_ROLE, tmp_password, to)?).await {
            Ok(mut conn) => {
                let w = start_workers(&mut conn).await;
                let _ = conn.close().await;
                w
            }
            Err(e) => Workers::NotStarted(format!("connecting to {to} as {TMP_ROLE}: {e}")),
        },
    )
}

async fn database_exists(conn: &mut PgConnection, name: &str) -> sqlx::Result<bool> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
        .bind(name)
        .fetch_one(conn)
        .await
}

async fn rename_as_tmp(
    tmp: &mut PgConnection,
    admin_url: &str,
    from: &str,
    to: &str,
    password: &str,
) -> Result<()> {
    let db_from = database_exists(tmp, from).await?;
    let db_to = database_exists(tmp, to).await?;
    let role_from = role_exists(tmp, from).await?;
    let role_to = role_exists(tmp, to).await?;
    if db_from && db_to {
        bail!("database {to} already exists next to {from}: nothing was renamed");
    }
    if role_from && role_to {
        bail!("role {to} already exists next to {from}: nothing was renamed");
    }
    if db_from {
        sqlx::query(&format!(
            "ALTER DATABASE {} RENAME TO {}",
            quote_ident(from),
            quote_ident(to)
        ))
        .execute(&mut *tmp)
        .await
        .with_context(|| format!("renaming database {from} (is the hub still connected?)"))?;
    }
    if role_from {
        sqlx::query(&format!(
            "ALTER ROLE {} RENAME TO {}",
            quote_ident(from),
            quote_ident(to)
        ))
        .execute(&mut *tmp)
        .await
        .with_context(|| format!("renaming role {from}"))?;
    }

    let target = options(admin_url, to, password, to)?;
    let conn = match PgConnection::connect_with(&target).await {
        Ok(c) => c,
        Err(_) => {
            sqlx::query(&format!(
                "ALTER ROLE {} PASSWORD {}",
                quote_ident(to),
                quote_literal(password)
            ))
            .execute(&mut *tmp)
            .await?;
            PgConnection::connect_with(&target)
                .await
                .context("connecting as the renamed role")?
        }
    };
    conn.close().await?;
    Ok(())
}

/// Drops the temporary role from a session that is not the temporary role: as `from` while it
/// still exists, else as `to`.
async fn drop_tmp_role_after(admin_url: &str, from: &str, to: &str, password: &str) -> Result<()> {
    let mut last = None;
    for user in [from, to] {
        match PgConnection::connect_with(&options(admin_url, user, password, "postgres")?).await {
            Ok(mut conn) => {
                let r = drop_tmp_role(&mut conn).await;
                let _ = conn.close().await;
                return r;
            }
            Err(e) => last = Some(e),
        }
    }
    Err(anyhow::Error::from(last.expect("two attempts"))).context(format!(
        "dropping role {TMP_ROLE}: no login as {from} or {to}"
    ))
}

/// Everything that still says `oikos` in any casing: database names, role names, entity ids
/// and settings values. A settings row is named by its key only: its value may hold a secret.
pub async fn verify_db(pool: &PgPool) -> sqlx::Result<Vec<String>> {
    let mut left = Vec::new();
    for (what, sql) in [
        (
            "database",
            "SELECT datname::text FROM pg_database WHERE datname ILIKE '%oikos%'",
        ),
        (
            "role",
            "SELECT rolname::text FROM pg_roles WHERE rolname ILIKE '%oikos%'",
        ),
        (
            "settings",
            "SELECT key FROM settings WHERE value::text ILIKE '%oikos%' ORDER BY key",
        ),
    ] {
        for r in sqlx::query_scalar::<_, String>(sql).fetch_all(pool).await? {
            left.push(format!("{what}: {r}"));
        }
    }
    for table in entity_id_tables(pool).await? {
        let sql = format!(
            "SELECT DISTINCT entity_id FROM {} WHERE entity_id ILIKE '%oikos%'",
            quote_ident(&table)
        );
        for r in sqlx::query_scalar::<_, String>(&sql)
            .fetch_all(pool)
            .await?
        {
            left.push(format!("{table}.entity_id: {r}"));
        }
    }
    Ok(left)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renamed_url_swaps_user_and_db() {
        assert_eq!(
            renamed_url("postgres://oikos:pw@h:5433/oikos", "oikos", "nidoo"),
            "postgres://nidoo:pw@h:5433/nidoo"
        );
        assert_eq!(
            renamed_url("postgres://other:pw@h:5433/oikos", "oikos", "nidoo"),
            "postgres://other:pw@h:5433/nidoo"
        );
    }
}
