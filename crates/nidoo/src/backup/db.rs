//! Copying the database out for a backup and back in for a restore.
//!
//! A backup reads every table of schema `public` except `_sqlx_migrations` and `settings`
//! (settings travel as their own rows) with `COPY (SELECT …) TO STDOUT`, all inside one
//! read-only `REPEATABLE READ` transaction so the tables agree with each other and with the
//! row counts taken alongside. A restore writes each table back with `COPY … FROM STDIN`,
//! then moves every serial counter past the restored rows.

use std::collections::BTreeMap;

use bytes::Bytes;
use futures_util::stream::{self, BoxStream};
use futures_util::{StreamExt, TryStreamExt};
use serde_json::Value;
use sqlx::pool::PoolConnection;
use sqlx::{Executor, PgPool, Postgres, Row};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// Columns renamed by a migration since backups were first written: `(table, old, new)`.
/// A backup column listed here is restored into its new name.
const RENAMES: &[(&str, &str, &str)] = &[];

/// Tables a backup leaves out of `db/`.
const SKIPPED: [&str; 2] = ["_sqlx_migrations", "settings"];

/// One exported table: its name, the columns copied (in table order, generated columns left
/// out) and its row count in the snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub rows: u64,
}

/// The highest migration the database has applied.
pub async fn migration_level(pool: &PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT coalesce(max(version), 0) FROM _sqlx_migrations WHERE success")
        .fetch_one(pool)
        .await
}

/// A consistent, read-only view of the database for the length of one backup.
///
/// It holds its own connection inside an open read-only transaction, and closes that connection
/// when dropped instead of handing it back to the pool: a [`Snapshot::copy_out`] stream dropped
/// half-read (a cancelled backup) leaves `COPY` data on the wire, which would otherwise reach the
/// next query run on that connection. Closing the connection also ends the transaction.
pub struct Snapshot {
    tx: PoolConnection<Postgres>,
}

impl Snapshot {
    pub async fn begin(pool: &PgPool) -> Result<Snapshot, sqlx::Error> {
        let names: Vec<String> = exported_tables(pool)
            .await?
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        let mut tx = pool.acquire().await?;
        tx.close_on_drop();
        tx.execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        // Lock the tables before the snapshot is taken. TimescaleDB's compression (the hub's
        // policy runs it on its own) ends by truncating the chunk it compressed, and TRUNCATE is
        // not MVCC-safe: a chunk truncated after the snapshot is taken reads as empty to it, and
        // the backup would silently miss those rows. ACCESS SHARE on a hypertable also locks its
        // chunks (they inherit from it), so a compression in progress waits for the backup to
        // end before it truncates. LOCK itself takes no snapshot.
        if !names.is_empty() {
            let list = ident_list(&names);
            tx.execute(format!("LOCK TABLE {list} IN ACCESS SHARE MODE").as_str())
                .await?;
        }
        // A REPEATABLE READ snapshot is taken by the first statement that needs one, not by BEGIN
        // or LOCK: take it now so writes made after `begin` returns are not seen.
        tx.execute("SELECT 1").await?;
        Ok(Snapshot { tx })
    }

    /// The exported tables, sorted by name, each with its row count.
    pub async fn tables(&mut self) -> Result<Vec<TableInfo>, sqlx::Error> {
        let listed = exported_tables(&mut *self.tx).await?;
        let mut out = Vec::with_capacity(listed.len());
        for (name, columns) in listed {
            let rows: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {}", ident(&name)))
                .fetch_one(&mut *self.tx)
                .await?;
            out.push(TableInfo {
                name,
                columns,
                rows: rows as u64,
            });
        }
        Ok(out)
    }

    /// The stored settings rows, `key → value`.
    pub async fn settings_rows(&mut self) -> Result<BTreeMap<String, Value>, sqlx::Error> {
        let rows = sqlx::query("SELECT key, value FROM settings")
            .fetch_all(&mut *self.tx)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.get::<String, _>(0), r.get::<Value, _>(1)))
            .collect())
    }

    /// The table's rows in `COPY` text format, in chunks as the server sends them.
    ///
    /// Read the stream to its end before the next call on this `Snapshot`. A stream dropped
    /// half-read leaves the rest of the `COPY` on the connection, so the `Snapshot` must not be
    /// used again: drop it (its connection is closed, not returned to the pool) and begin anew.
    pub fn copy_out(&mut self, t: &TableInfo) -> BoxStream<'_, Result<Bytes, sqlx::Error>> {
        let sql = format!(
            "COPY (SELECT {} FROM {}) TO STDOUT",
            ident_list(&t.columns),
            ident(&t.name)
        );
        let conn: &mut sqlx::PgConnection = &mut self.tx;
        stream::once(async move { conn.copy_out_raw(&sql).await })
            .try_flatten()
            .boxed()
    }
}

/// The target table's columns, in table order, generated columns left out; empty when the
/// table does not exist.
pub async fn target_columns(pool: &PgPool, table: &str) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns
         WHERE table_schema = 'public' AND table_name = $1 AND is_generated = 'NEVER'
         ORDER BY ordinal_position",
    )
    .bind(table)
    .fetch_all(pool)
    .await
}

/// The target columns a backup table's columns go into, in the backup's order. Every backup
/// column must still exist (under its [`RENAMES`] name); target-only columns take their defaults.
pub fn map_columns(
    table: &str,
    backup: &[String],
    target: &[String],
) -> Result<Vec<String>, String> {
    map_with(RENAMES, table, backup, target)
}

fn map_with(
    renames: &[(&str, &str, &str)],
    table: &str,
    backup: &[String],
    target: &[String],
) -> Result<Vec<String>, String> {
    if target.is_empty() {
        return Err(format!("table `{table}` of the backup no longer exists"));
    }
    backup
        .iter()
        .map(|c| {
            let name = renames
                .iter()
                .find(|(t, old, _)| *t == table && *old == c)
                .map_or(c.as_str(), |(_, _, new)| new);
            if target.iter().any(|t| t == name) {
                Ok(name.to_owned())
            } else {
                Err(format!(
                    "column `{table}.{c}` of the backup no longer exists"
                ))
            }
        })
        .collect()
}

/// One table's `COPY … FROM STDIN`, on a connection the copy holds until it ends.
///
/// sqlx only offers a copy that borrows its connection, so the copy runs in its own task that
/// owns the connection; [`CopyIn::send`] hands it the data. Dropping a `CopyIn` without
/// [`CopyIn::finish`] aborts the copy, and the server keeps none of its rows.
pub struct CopyIn {
    tx: mpsc::Sender<Msg>,
    /// `None` once a failed [`CopyIn::send`] has collected the task's result.
    task: Option<JoinHandle<Result<u64, sqlx::Error>>>,
}

enum Msg {
    Data(Bytes),
    Finish,
}

impl CopyIn {
    pub async fn start(
        mut conn: PoolConnection<Postgres>,
        table: &str,
        columns: &[String],
    ) -> Result<CopyIn, sqlx::Error> {
        let sql = format!("COPY {} ({}) FROM STDIN", ident(table), ident_list(columns));
        let (tx, rx) = mpsc::channel::<Msg>(4);
        let (ready, started) = oneshot::channel();
        let task = tokio::spawn(async move {
            let result = run_copy(&mut conn, &sql, ready, rx).await;
            if result.is_err() {
                // Only a finished copy is known to leave the connection in step with the
                // server (an aborted one after a rejected row leaves a reply unread): close any
                // other instead of handing it back to the pool.
                conn.close_on_drop();
            }
            result
        });
        match started.await {
            Ok(Ok(())) => Ok(CopyIn {
                tx,
                task: Some(task),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(joined(task.await)),
        }
    }

    /// Hands the next piece of `COPY` text to the server (pieces need not end on a row).
    pub async fn send(&mut self, data: Bytes) -> Result<(), sqlx::Error> {
        if self.tx.send(Msg::Data(data)).await.is_ok() {
            return Ok(());
        }
        // The task ended early: its result says why.
        match self.task.take() {
            Some(task) => Err(joined(task.await)),
            None => Err(stopped()),
        }
    }

    /// Ends the copy; the number of rows the server wrote.
    pub async fn finish(self) -> Result<u64, sqlx::Error> {
        let Some(task) = self.task else {
            return Err(stopped());
        };
        let _ = self.tx.send(Msg::Finish).await;
        match task.await {
            Ok(result) => result,
            Err(e) => Err(sqlx::Error::Protocol(format!("copy task failed: {e}"))),
        }
    }
}

/// The body of a [`CopyIn`] task: start the copy, pass the data on, end it as told.
async fn run_copy(
    conn: &mut sqlx::PgConnection,
    sql: &str,
    ready: oneshot::Sender<Result<(), sqlx::Error>>,
    mut rx: mpsc::Receiver<Msg>,
) -> Result<u64, sqlx::Error> {
    let mut copy = match conn.copy_in_raw(sql).await {
        Ok(copy) => {
            let _ = ready.send(Ok(()));
            copy
        }
        Err(e) => {
            let _ = ready.send(Err(e));
            return Err(stopped());
        }
    };
    while let Some(msg) = rx.recv().await {
        match msg {
            Msg::Data(data) => {
                copy.send(data).await?;
            }
            Msg::Finish => return copy.finish().await,
        }
    }
    // Dropped without `finish`: the server discards the whole COPY.
    let _ = copy.abort("restore stopped").await;
    Err(stopped())
}

/// The error a copy task ended with, when it should not have ended yet.
fn joined(r: Result<Result<u64, sqlx::Error>, tokio::task::JoinError>) -> sqlx::Error {
    match r {
        Ok(Err(e)) => e,
        Ok(Ok(_)) => stopped(),
        Err(e) => sqlx::Error::Protocol(format!("copy task failed: {e}")),
    }
}

fn stopped() -> sqlx::Error {
    sqlx::Error::Protocol("copy stopped before it finished".into())
}

/// Moves every serial counter of schema `public` past the largest value its column holds, so the
/// next row written after a restore neither collides with nor sorts before a restored one.
/// A counter on an empty table is left alone.
pub async fn fix_sequences(pool: &PgPool) -> Result<(), sqlx::Error> {
    let serials = sqlx::query(
        "SELECT c.relname::text, a.attname::text,
                pg_get_serial_sequence(format('%I.%I', n.nspname, c.relname), a.attname)
         FROM pg_attrdef d
         JOIN pg_attribute a ON a.attrelid = d.adrelid AND a.attnum = d.adnum
         JOIN pg_class c ON c.oid = d.adrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p')
           AND NOT a.attisdropped
           AND pg_get_expr(d.adbin, d.adrelid) LIKE 'nextval(%'",
    )
    .fetch_all(pool)
    .await?;
    for row in serials {
        let table: String = row.get(0);
        let column: String = row.get(1);
        let Some(sequence) = row.get::<Option<String>, _>(2) else {
            tracing::warn!(
                table,
                column,
                "backup: no sequence owned by a nextval column"
            );
            continue;
        };
        // HAVING keeps setval from running at all on an empty table.
        sqlx::query(&format!(
            "SELECT setval($1::text::regclass, max({col}), true) FROM {table} \
             HAVING max({col}) IS NOT NULL",
            col = ident(&column),
            table = ident(&table),
        ))
        .bind(&sequence)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// The on-disk size of the history, in bytes: every table of schema `public` but
/// `_sqlx_migrations` and `settings`, with indexes and TOAST, plus TimescaleDB's chunks (which
/// live in `_timescaledb_internal`, compressed ones included). An estimate of the work a backup
/// does, not of the file it writes.
pub async fn history_bytes(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let bytes: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(pg_total_relation_size(c.oid)), 0)::bigint
         FROM pg_class c
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE c.relkind = 'r'
           AND (n.nspname = '_timescaledb_internal'
                OR (n.nspname = 'public' AND NOT (c.relname::text = ANY($1))))",
    )
    .bind(SKIPPED.as_slice())
    .fetch_one(pool)
    .await?;
    Ok(bytes.max(0) as u64)
}

/// The tables a backup carries and a restore may write, sorted by name.
pub async fn exported_table_names(pool: &PgPool) -> Result<Vec<String>, sqlx::Error> {
    Ok(exported_tables(pool)
        .await?
        .into_iter()
        .map(|(name, _)| name)
        .collect())
}

/// The first exported table that holds a row, if any.
pub async fn holds_history(pool: &PgPool) -> Result<Option<String>, sqlx::Error> {
    for (name, _) in exported_tables(pool).await? {
        let any: bool =
            sqlx::query_scalar(&format!("SELECT EXISTS (SELECT 1 FROM {})", ident(&name)))
                .fetch_one(pool)
                .await?;
        if any {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

/// Empties every exported table and the settings, all or nothing (before a restore is retried).
pub async fn wipe(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for (name, _) in exported_tables(&mut *tx).await? {
        // One table per statement: TimescaleDB handles TRUNCATE of each hypertable on its own.
        sqlx::query(&format!("TRUNCATE {}", ident(&name)))
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("DELETE FROM settings")
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

/// `(table, columns)` for every exported table, sorted by name (byte order, not collation).
async fn exported_tables<'e, E>(e: E) -> Result<Vec<(String, Vec<String>)>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        "SELECT c.table_name::text, array_agg(c.column_name::text ORDER BY c.ordinal_position)
         FROM information_schema.columns c
         JOIN information_schema.tables t
           ON t.table_schema = c.table_schema AND t.table_name = c.table_name
         WHERE c.table_schema = 'public' AND t.table_type = 'BASE TABLE'
           AND c.is_generated = 'NEVER' AND NOT (c.table_name::text = ANY($1))
         GROUP BY c.table_name",
    )
    .bind(SKIPPED.as_slice())
    .fetch_all(e)
    .await?;
    let mut tables: Vec<(String, Vec<String>)> =
        rows.into_iter().map(|r| (r.get(0), r.get(1))).collect();
    tables.sort();
    Ok(tables)
}

/// A quoted SQL identifier.
fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn ident_list(names: &[String]) -> String {
    names
        .iter()
        .map(|n| ident(n))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(c: &[&str]) -> Vec<String> {
        c.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn map_columns_refuses_a_dropped_column() {
        let err = map_columns(
            "entity_state",
            &cols(&["time", "seq", "gone"]),
            &cols(&["time", "seq"]),
        )
        .unwrap_err();
        assert_eq!(
            err,
            "column `entity_state.gone` of the backup no longer exists"
        );
    }

    #[test]
    fn map_columns_allows_new_target_columns() {
        let mapped = map_columns(
            "climate_decision",
            &cols(&["time", "zone"]),
            &cols(&["time", "seq", "zone", "adjust"]),
        )
        .unwrap();
        assert_eq!(mapped, ["time", "zone"]);
    }

    #[test]
    fn map_columns_refuses_a_dropped_table() {
        let err = map_columns("old_table", &cols(&["time"]), &[]).unwrap_err();
        assert_eq!(err, "table `old_table` of the backup no longer exists");
    }

    #[test]
    fn map_columns_follows_a_rename() {
        let renames = [("t", "old", "new")];
        let target = cols(&["a", "new"]);
        assert_eq!(
            map_with(&renames, "t", &cols(&["old", "a"]), &target).unwrap(),
            ["new", "a"]
        );
        // Only in the table it was renamed in.
        assert!(map_with(&renames, "u", &cols(&["old"]), &target).is_err());
    }

    #[test]
    fn identifiers_are_quoted() {
        assert_eq!(ident("entity_state"), "\"entity_state\"");
        assert_eq!(ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(ident_list(&cols(&["time", "seq"])), "\"time\", \"seq\"");
    }
}
