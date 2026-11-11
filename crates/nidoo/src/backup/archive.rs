//! The backup file: a tar of JSON documents and `COPY` text, sealed by [`crypt::seal`].
//!
//! Its entries, in this order:
//! - `manifest.json`: the [`Manifest`], first so a restore can check a file from its head;
//! - `hub.json`: the [`HubFields`];
//! - `settings.json`: the `settings` rows, `key → value`;
//! - `retained/<n>.json`: the backed-up retained documents, as compact JSON arrays of
//!   `{"topic", "payload_b64"}` of at most [`MAX_ENTRY`] bytes each (`n` from `000001`; there is
//!   always a first one, `[]` when there are no documents);
//! - `db/<table>/<n>.copy`: each table's `COPY` text, cut into entries of at most
//!   [`MAX_ENTRY`] bytes (`n` from `000001`); a table without rows has none;
//! - `end.json`: `{"tables": {name: rows}}`, the rows each table's entries hold.

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use futures_util::TryStreamExt;
use futures_util::stream::BoxStream;
use nidoo_api::BackupPart;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};

use super::BackupError;
use super::crypt;
use super::db::{self, CopyIn, Snapshot, TableInfo};
use super::retained::{Family, Layout, QUIET, RetainedBroker, classify, filters};
use super::tar::{MAX_ENTRY, TarReader, TarWriter};
use crate::setup::hub_file::HubFields;

/// The file format this hub writes, and the newest it reads.
pub const FORMAT: u32 = 1;

const MANIFEST: &str = "manifest.json";
const HUB: &str = "hub.json";
const SETTINGS: &str = "settings.json";
const END: &str = "end.json";

const BLOCK: usize = 512;

/// One table of the backup: the columns its `COPY` text holds, in order, and its row count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableManifest {
    pub columns: Vec<String>,
    pub rows: u64,
}

/// What a backup holds, readable before the rest of the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub nidoo_version: String,
    /// The source database's migration level.
    pub migration: i64,
    /// RFC 3339, UTC.
    pub created_at: String,
    /// The source hub's host name.
    pub host: String,
    pub tables: BTreeMap<String, TableManifest>,
    /// Retained documents carried, flows and dashboards included.
    pub retained: u32,
    pub flows: u32,
    pub dashboards: u32,
    /// The history's size on disk in the source database: an estimate, not the file's size.
    pub history_bytes: u64,
}

/// What a backup reads from.
pub struct Sources<'a> {
    pub pool: &'a PgPool,
    pub broker: &'a dyn RetainedBroker,
    pub layout: Layout,
    pub hub: HubFields,
}

/// What a restore hands back for its caller to apply: `hub.json` is the caller's to write.
#[derive(Clone, PartialEq)]
pub struct Restored {
    pub hub: HubFields,
    pub settings: BTreeMap<String, Value>,
    pub manifest: Manifest,
}

/// Settings rows may hold secrets: only their keys are shown.
impl fmt::Debug for Restored {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Restored")
            .field("hub", &self.hub)
            .field("settings", &self.settings.keys().collect::<Vec<_>>())
            .field("manifest", &self.manifest)
            .finish()
    }
}

#[derive(Serialize, Deserialize)]
struct RetainedDoc {
    topic: String,
    payload_b64: String,
}

#[derive(Serialize, Deserialize)]
struct End {
    tables: BTreeMap<String, u64>,
}

/// Writes a backup of `src` into `out`, encrypted with `password`, and finalizes `out` (its
/// `shutdown` is called).
///
/// The retained documents are collected first, then the database is read in one snapshot.
/// `progress` gets the part being written and the plaintext bytes written so far.
pub async fn write_backup<W: AsyncWrite + Unpin + Send>(
    src: Sources<'_>,
    out: W,
    password: &str,
    progress: &(dyn Fn(BackupPart, u64) + Sync),
) -> Result<Manifest, BackupError> {
    let docs = retained_docs(src.broker, &src.layout).await?;
    // Before the snapshot: it holds a connection of its own until the last table is read.
    let migration = db::migration_level(src.pool).await?;
    let history_bytes = db::history_bytes(src.pool).await?;
    let mut snapshot = Snapshot::begin(src.pool).await?;
    let tables = snapshot.tables().await?;
    let settings = snapshot.settings_rows().await?;
    let count = |family| {
        docs.iter()
            .filter(|(topic, _)| classify(topic, &src.layout) == Some(family))
            .count() as u32
    };
    let manifest = Manifest {
        format: FORMAT,
        nidoo_version: env!("CARGO_PKG_VERSION").to_owned(),
        migration,
        created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        host: sysinfo::System::host_name()
            .ok()
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| "nidoo".to_owned()),
        tables: tables
            .iter()
            .map(|t| {
                let table = TableManifest {
                    columns: t.columns.clone(),
                    rows: t.rows,
                };
                (t.name.clone(), table)
            })
            .collect(),
        retained: docs.len() as u32,
        flows: count(Family::Flow),
        dashboards: count(Family::Dashboard),
        history_bytes,
    };
    let contents = Contents {
        manifest: &manifest,
        hub: &src.hub,
        settings: &settings,
        docs: &docs,
        tables: &tables,
    };
    write_archive(crypt::seal(out, password), contents, snapshot, progress).await?;
    Ok(manifest)
}

/// The backed-up retained documents (empty payloads left out).
async fn retained_docs(
    broker: &dyn RetainedBroker,
    layout: &Layout,
) -> Result<Vec<(String, Bytes)>, BackupError> {
    let docs = broker
        .collect(filters(layout), QUIET)
        .await
        .map_err(BackupError::Broker)?;
    Ok(docs
        .into_iter()
        .filter(|(topic, payload)| {
            !payload.is_empty() && classify(topic, layout).is_some_and(Family::backed_up)
        })
        .collect())
}

/// Everything but the tables' rows.
struct Contents<'a> {
    manifest: &'a Manifest,
    hub: &'a HubFields,
    settings: &'a BTreeMap<String, Value>,
    docs: &'a [(String, Bytes)],
    tables: &'a [TableInfo],
}

/// Where a backup reads each table's `COPY` text from.
trait TableSource: Send {
    fn copy_out(&mut self, t: &TableInfo) -> BoxStream<'_, Result<Bytes, sqlx::Error>>;
}

impl TableSource for Snapshot {
    fn copy_out(&mut self, t: &TableInfo) -> BoxStream<'_, Result<Bytes, sqlx::Error>> {
        Snapshot::copy_out(self, t)
    }
}

async fn write_archive<W: AsyncWrite + Unpin + Send, S: TableSource>(
    out: W,
    c: Contents<'_>,
    mut source: S,
    progress: &(dyn Fn(BackupPart, u64) + Sync),
) -> Result<(), BackupError> {
    let mut w = ArchiveWriter {
        tar: TarWriter::new(out),
        written: 0,
        progress,
    };
    w.json(BackupPart::Settings, MANIFEST, c.manifest).await?;
    w.json(BackupPart::Settings, HUB, c.hub).await?;
    w.json(BackupPart::Settings, SETTINGS, c.settings).await?;
    for (i, data) in retained_entries(c.docs, MAX_ENTRY)?.iter().enumerate() {
        w.entry(BackupPart::Retained, &retained_path(i as u32 + 1), data)
            .await?;
    }
    let mut end = BTreeMap::new();
    for t in c.tables {
        let rows = w.table(&t.name, source.copy_out(t)).await?;
        // Same snapshot: a difference means the export missed or invented rows.
        if rows != t.rows {
            return Err(BackupError::Database(format!(
                "table {}: copied {rows} rows, counted {}",
                t.name, t.rows
            )));
        }
        end.insert(t.name.clone(), rows);
    }
    // Ends the snapshot (and its table locks) before the last writes.
    drop(source);
    w.json(BackupPart::History, END, &End { tables: end })
        .await?;
    let mut out = w.tar.finish().await.map_err(write_error)?;
    out.shutdown().await.map_err(write_error)
}

struct ArchiveWriter<'p, W: AsyncWrite + Unpin> {
    tar: TarWriter<W>,
    /// Plaintext bytes written so far.
    written: u64,
    progress: &'p (dyn Fn(BackupPart, u64) + Sync),
}

impl<W: AsyncWrite + Unpin> ArchiveWriter<'_, W> {
    async fn entry(
        &mut self,
        part: BackupPart,
        path: &str,
        data: &[u8],
    ) -> Result<(), BackupError> {
        self.tar.entry(path, data).await.map_err(write_error)?;
        self.written += (BLOCK + data.len().div_ceil(BLOCK) * BLOCK) as u64;
        (self.progress)(part, self.written);
        Ok(())
    }

    async fn json<T: Serialize + ?Sized>(
        &mut self,
        part: BackupPart,
        path: &str,
        value: &T,
    ) -> Result<(), BackupError> {
        let data = serde_json::to_vec_pretty(value)
            .map_err(|e| BackupError::Io(format!("{path}: {e}")))?;
        self.entry(part, path, &data).await
    }

    /// Writes one table's `COPY` text as entries of at most [`MAX_ENTRY`] bytes (a row may span
    /// two); the rows written.
    async fn table(
        &mut self,
        name: &str,
        mut data: BoxStream<'_, Result<Bytes, sqlx::Error>>,
    ) -> Result<u64, BackupError> {
        let mut buf: Vec<u8> = Vec::new();
        let mut n = 0u32;
        let mut rows = 0u64;
        while let Some(chunk) = data.try_next().await? {
            // In COPY text every row ends with a newline; one inside a value is escaped.
            rows += chunk.iter().filter(|b| **b == b'\n').count() as u64;
            let mut rest = &chunk[..];
            while !rest.is_empty() {
                let take = (MAX_ENTRY - buf.len()).min(rest.len());
                buf.extend_from_slice(&rest[..take]);
                rest = &rest[take..];
                if buf.len() == MAX_ENTRY {
                    n += 1;
                    self.entry(BackupPart::History, &copy_path(name, n), &buf)
                        .await?;
                    buf.clear();
                }
            }
        }
        if !buf.is_empty() {
            n += 1;
            self.entry(BackupPart::History, &copy_path(name, n), &buf)
                .await?;
        }
        Ok(rows)
    }
}

/// The retained documents as compact JSON arrays of at most `limit` bytes each; at least one
/// (`[]` when there are none). A document too large for an entry of its own fails the backup.
fn retained_entries(docs: &[(String, Bytes)], limit: usize) -> Result<Vec<Vec<u8>>, BackupError> {
    let mut entries = Vec::new();
    let mut buf = b"[".to_vec();
    for (topic, payload) in docs {
        let doc = serde_json::to_vec(&RetainedDoc {
            topic: topic.clone(),
            payload_b64: STANDARD.encode(payload),
        })
        .map_err(|e| BackupError::Io(e.to_string()))?;
        if doc.len() + 2 > limit {
            return Err(BackupError::Broker(format!(
                "retained topic {topic} is too large to back up ({} bytes)",
                payload.len()
            )));
        }
        // The array so far, a comma, this document and the closing bracket.
        if buf.len() > 1 && buf.len() + doc.len() + 2 > limit {
            buf.push(b']');
            entries.push(std::mem::replace(&mut buf, b"[".to_vec()));
        }
        if buf.len() > 1 {
            buf.push(b',');
        }
        buf.extend_from_slice(&doc);
    }
    buf.push(b']');
    entries.push(buf);
    Ok(entries)
}

fn write_error(e: io::Error) -> BackupError {
    BackupError::Io(e.to_string())
}

fn copy_path(table: &str, n: u32) -> String {
    format!("db/{table}/{n:06}.copy")
}

fn retained_path(n: u32) -> String {
    format!("retained/{n:06}.json")
}

/// `n` of a `retained/<n>.json` entry.
fn parse_retained_path(path: &str) -> Option<u32> {
    let n: u32 = path
        .strip_prefix("retained/")?
        .strip_suffix(".json")?
        .parse()
        .ok()?;
    (retained_path(n) == path).then_some(n)
}

/// `(table, n)` of a `db/<table>/<n>.copy` entry.
fn parse_copy_path(path: &str) -> Option<(&str, u32)> {
    let (table, file) = path.strip_prefix("db/")?.split_once('/')?;
    let n: u32 = file.strip_suffix(".copy")?.parse().ok()?;
    (!table.is_empty() && copy_path(table, n) == path).then_some((table, n))
}

/// Reads a backup's manifest: the first entry, so the head of the file is enough.
pub async fn read_manifest<R: AsyncRead + Unpin + Send>(
    input: R,
    password: &str,
) -> Result<Manifest, BackupError> {
    open_archive(input, password).await.map(|(_, m)| m)
}

/// Refuses a backup this hub can't restore: a newer file format, or a database migrated further
/// than this one.
pub fn check_compatible(m: &Manifest, target_migration: i64) -> Result<(), BackupError> {
    if m.format > FORMAT || m.migration > target_migration {
        return Err(BackupError::TooNew);
    }
    Ok(())
}

/// Restores a backup into `pool` and `broker`: the settings rows (each `INSERT … ON CONFLICT
/// UPDATE`), the retained documents, then every table with `COPY`, each table's row count checked
/// against the manifest, then the serial counters moved past the restored rows.
///
/// A wrong password, a file that is not a backup, one from a newer Nidoo, or a table the target
/// can no longer hold is refused before anything is written. Damage found later (a file cut
/// short, a row count that differs) returns an error with part of the data already written:
/// clearing it is the caller's job (the restore job wipes the database and the topics).
///
/// `progress` gets the part being read and the bytes of `input` read so far.
pub async fn read_backup<R: AsyncRead + Unpin + Send>(
    input: R,
    password: &str,
    pool: &PgPool,
    broker: &dyn RetainedBroker,
    progress: &(dyn Fn(BackupPart, u64) + Sync),
) -> Result<Restored, BackupError> {
    let read = AtomicU64::new(0);
    let counted = Counted {
        inner: input,
        n: &read,
    };
    let (tar, manifest) = open_archive(counted, password).await?;
    check_compatible(&manifest, db::migration_level(pool).await?)?;
    let mut target = DbTarget::prepare(pool, broker, &manifest).await?;
    let report = |part| progress(part, read.load(Ordering::Relaxed));
    let (hub, settings) = restore_into(tar, &manifest, &mut target, &report).await?;
    db::fix_sequences(pool).await?;
    Ok(Restored {
        hub,
        settings,
        manifest,
    })
}

/// Opens the file and reads its manifest; the reader is left at the second entry.
async fn open_archive<R: AsyncRead + Unpin + Send>(
    input: R,
    password: &str,
) -> Result<(TarReader<impl AsyncRead + Unpin + Send>, Manifest), BackupError> {
    let mut tar = TarReader::new(crypt::open(input, password).await?);
    let first = tar.next().await.map_err(|e| {
        if is_data_error(&e) {
            BackupError::NotABackup
        } else {
            BackupError::Io(e.to_string())
        }
    })?;
    match first {
        Some((path, data)) if path == MANIFEST => Ok((tar, parse_manifest(&data)?)),
        _ => Err(BackupError::NotABackup),
    }
}

/// A manifest this hub can't parse is from a newer format when it says so.
fn parse_manifest(data: &[u8]) -> Result<Manifest, BackupError> {
    #[derive(Deserialize)]
    struct Format {
        format: u32,
    }
    serde_json::from_slice(data).map_err(|_| match serde_json::from_slice::<Format>(data) {
        Ok(f) if f.format > FORMAT => BackupError::TooNew,
        _ => BackupError::NotABackup,
    })
}

/// Where a restore writes to.
trait Target {
    async fn settings(&mut self, rows: &BTreeMap<String, Value>) -> Result<(), BackupError>;
    async fn retained(&mut self, docs: Vec<(String, Bytes)>) -> Result<(), BackupError>;
    async fn begin_table(&mut self, name: &str) -> Result<(), BackupError>;
    async fn send(&mut self, data: Bytes) -> Result<(), BackupError>;
    /// Ends the table begun last; the rows it received.
    async fn end_table(&mut self) -> Result<u64, BackupError>;
}

struct DbTarget<'a> {
    pool: &'a PgPool,
    broker: &'a dyn RetainedBroker,
    /// The target columns of each backup table.
    columns: BTreeMap<String, Vec<String>>,
    copy: Option<CopyIn>,
}

impl<'a> DbTarget<'a> {
    /// Maps every table's columns before anything is written. A table that still exists but is
    /// not one a backup carries (`settings`, `_sqlx_migrations`, a view) is refused.
    async fn prepare(
        pool: &'a PgPool,
        broker: &'a dyn RetainedBroker,
        m: &Manifest,
    ) -> Result<DbTarget<'a>, BackupError> {
        let exported = db::exported_table_names(pool).await?;
        let mut columns = BTreeMap::new();
        for (name, t) in &m.tables {
            let target = db::target_columns(pool, name).await?;
            let mapped =
                db::map_columns(name, &t.columns, &target).map_err(BackupError::Database)?;
            if !exported.contains(name) {
                return Err(BackupError::Corrupt(format!(
                    "table {name} can't be restored"
                )));
            }
            columns.insert(name.clone(), mapped);
        }
        Ok(DbTarget {
            pool,
            broker,
            columns,
            copy: None,
        })
    }
}

impl Target for DbTarget<'_> {
    async fn settings(&mut self, rows: &BTreeMap<String, Value>) -> Result<(), BackupError> {
        let mut tx = self.pool.begin().await?;
        for (key, value) in rows {
            sqlx::query(
                "INSERT INTO settings (key, value) VALUES ($1, $2)
                 ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
            )
            .bind(key)
            .bind(value)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn retained(&mut self, docs: Vec<(String, Bytes)>) -> Result<(), BackupError> {
        if docs.is_empty() {
            return Ok(());
        }
        self.broker
            .publish_retained(docs)
            .await
            .map_err(BackupError::Broker)
    }

    async fn begin_table(&mut self, name: &str) -> Result<(), BackupError> {
        let columns = self
            .columns
            .get(name)
            .ok_or_else(|| BackupError::Corrupt(format!("table {name} is not in the manifest")))?;
        let conn = self.pool.acquire().await?;
        self.copy = Some(CopyIn::start(conn, name, columns).await?);
        Ok(())
    }

    async fn send(&mut self, data: Bytes) -> Result<(), BackupError> {
        match &mut self.copy {
            Some(copy) => Ok(copy.send(data).await?),
            None => Err(BackupError::Corrupt("rows outside a table".into())),
        }
    }

    async fn end_table(&mut self) -> Result<u64, BackupError> {
        match self.copy.take() {
            Some(copy) => Ok(copy.finish().await?),
            None => Ok(0),
        }
    }
}

/// Reads every entry after the manifest into `target`, checking each table's rows against the
/// manifest, then the file's end.
async fn restore_into<R: AsyncRead + Unpin, T: Target>(
    mut tar: TarReader<R>,
    manifest: &Manifest,
    target: &mut T,
    report: &(dyn Fn(BackupPart) + Sync),
) -> Result<(HubFields, BTreeMap<String, Value>), BackupError> {
    let hub: HubFields = parse(HUB, &expect(&mut tar, HUB).await?)?;
    let settings: BTreeMap<String, Value> = parse(SETTINGS, &expect(&mut tar, SETTINGS).await?)?;
    target.settings(&settings).await?;
    report(BackupPart::Settings);

    // `retained/000001.json` onwards; the entry after them is the first table's (or the end).
    let mut docs = Vec::new();
    let mut last = 0u32;
    let after_retained = loop {
        let (path, data) = next(&mut tar).await?.ok_or_else(ends_early)?;
        match parse_retained_path(&path) {
            Some(n) if n == last + 1 => {
                last = n;
                let entry: Vec<RetainedDoc> = parse(&path, &data)?;
                for d in entry {
                    let payload = STANDARD
                        .decode(&d.payload_b64)
                        .map_err(|_| BackupError::Corrupt(format!("{path} is not valid")))?;
                    if !payload.is_empty() {
                        docs.push((d.topic, Bytes::from(payload)));
                    }
                }
            }
            None if last > 0 => break (path, data),
            _ => return Err(unexpected(&path)),
        }
    };
    target.retained(docs).await?;
    report(BackupPart::Retained);

    let mut restored: BTreeMap<String, u64> = BTreeMap::new();
    // The table being copied and the number of its last entry.
    let mut current: Option<(String, u32)> = None;
    let mut pending = Some(after_retained);
    let end = loop {
        let (path, data) = match pending.take() {
            Some(entry) => entry,
            None => next(&mut tar).await?.ok_or_else(ends_early)?,
        };
        if path == END {
            break data;
        }
        let (table, n) = parse_copy_path(&path).ok_or_else(|| unexpected(&path))?;
        let continues = matches!(&current, Some((t, last)) if t == table && n == last + 1);
        if !continues {
            if let Some((t, _)) = current.take() {
                end_table(target, manifest, &mut restored, t).await?;
            }
            if n != 1 || restored.contains_key(table) || !manifest.tables.contains_key(table) {
                return Err(unexpected(&path));
            }
            target.begin_table(table).await?;
        }
        current = Some((table.to_owned(), n));
        target.send(Bytes::from(data)).await?;
        report(BackupPart::History);
    };
    if let Some((t, _)) = current.take() {
        end_table(target, manifest, &mut restored, t).await?;
    }
    for (name, t) in &manifest.tables {
        if t.rows != 0 && !restored.contains_key(name) {
            return Err(rows_differ(name, 0, t.rows));
        }
    }
    let end: End = parse(END, &end)?;
    let expected: BTreeMap<String, u64> = manifest
        .tables
        .iter()
        .map(|(name, t)| (name.clone(), t.rows))
        .collect();
    if end.tables != expected {
        return Err(BackupError::Corrupt(format!(
            "{END} does not match {MANIFEST}"
        )));
    }
    // The tar reader stops at the first closing block. Read on to the end of the file, so that
    // the decryption checks its last chunk and zstd its frame end: a file cut after the tar's
    // end fails here.
    let mut rest = tar.into_inner();
    tokio::io::copy(&mut rest, &mut tokio::io::sink())
        .await
        .map_err(read_error)?;
    Ok((hub, settings))
}

async fn end_table<T: Target>(
    target: &mut T,
    manifest: &Manifest,
    restored: &mut BTreeMap<String, u64>,
    name: String,
) -> Result<(), BackupError> {
    let rows = target.end_table().await?;
    let expected = manifest.tables.get(&name).map_or(0, |t| t.rows);
    if rows != expected {
        return Err(rows_differ(&name, rows, expected));
    }
    restored.insert(name, rows);
    Ok(())
}

fn unexpected(path: &str) -> BackupError {
    BackupError::Corrupt(format!("unexpected entry {path}"))
}

fn rows_differ(table: &str, rows: u64, expected: u64) -> BackupError {
    BackupError::Corrupt(format!("table {table}: {rows} rows, expected {expected}"))
}

async fn next<R: AsyncRead + Unpin>(
    tar: &mut TarReader<R>,
) -> Result<Option<(String, Vec<u8>)>, BackupError> {
    tar.next().await.map_err(read_error)
}

/// The next entry, which must be `name`.
async fn expect<R: AsyncRead + Unpin>(
    tar: &mut TarReader<R>,
    name: &str,
) -> Result<Vec<u8>, BackupError> {
    match next(tar).await?.ok_or_else(ends_early)? {
        (path, data) if path == name => Ok(data),
        (path, _) => Err(BackupError::Corrupt(format!(
            "{path} where {name} was expected"
        ))),
    }
}

/// The error text never quotes the document: a settings row may hold a secret.
fn parse<T: DeserializeOwned>(name: &str, data: &[u8]) -> Result<T, BackupError> {
    serde_json::from_slice(data).map_err(|_| BackupError::Corrupt(format!("{name} is not valid")))
}

fn ends_early() -> BackupError {
    BackupError::Corrupt("the file ends early".into())
}

/// Errors the file's content causes (decryption, decompression, tar), as opposed to the
/// input's own I/O.
fn is_data_error(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::UnexpectedEof | io::ErrorKind::InvalidData | io::ErrorKind::Other
    )
}

fn read_error(e: io::Error) -> BackupError {
    match e.kind() {
        io::ErrorKind::UnexpectedEof => ends_early(),
        _ if is_data_error(&e) => BackupError::Corrupt(e.to_string()),
        _ => BackupError::Io(e.to_string()),
    }
}

/// Counts the bytes read through it.
struct Counted<'a, R> {
    inner: R,
    n: &'a AtomicU64,
}

impl<R: AsyncRead + Unpin> AsyncRead for Counted<'_, R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        let read = (buf.filled().len() - before) as u64;
        self.n.fetch_add(read, Ordering::Relaxed);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use futures_util::stream;
    use serde_json::json;
    use std::sync::Mutex;

    const PASSWORD: &str = "unit test password";

    /// Tables served from memory.
    struct Fixed(BTreeMap<String, Vec<Bytes>>);

    impl TableSource for Fixed {
        fn copy_out(&mut self, t: &TableInfo) -> BoxStream<'_, Result<Bytes, sqlx::Error>> {
            let chunks = self.0.get(&t.name).cloned().unwrap_or_default();
            stream::iter(chunks.into_iter().map(Ok)).boxed()
        }
    }

    /// A restore target that keeps what it is given.
    #[derive(Default)]
    struct Recorder {
        settings: BTreeMap<String, Value>,
        docs: Vec<(String, Bytes)>,
        tables: BTreeMap<String, Vec<u8>>,
        current: Option<String>,
    }

    impl Target for Recorder {
        async fn settings(&mut self, rows: &BTreeMap<String, Value>) -> Result<(), BackupError> {
            self.settings = rows.clone();
            Ok(())
        }

        async fn retained(&mut self, docs: Vec<(String, Bytes)>) -> Result<(), BackupError> {
            self.docs = docs;
            Ok(())
        }

        async fn begin_table(&mut self, name: &str) -> Result<(), BackupError> {
            self.current = Some(name.to_owned());
            self.tables.insert(name.to_owned(), Vec::new());
            Ok(())
        }

        async fn send(&mut self, data: Bytes) -> Result<(), BackupError> {
            let t = self.current.as_ref().unwrap();
            self.tables.get_mut(t).unwrap().extend_from_slice(&data);
            Ok(())
        }

        async fn end_table(&mut self) -> Result<u64, BackupError> {
            let t = self.current.take().unwrap();
            Ok(lines(&self.tables[&t]))
        }
    }

    fn lines(data: &[u8]) -> u64 {
        data.iter().filter(|b| **b == b'\n').count() as u64
    }

    fn random(n: usize) -> Vec<u8> {
        let mut v = vec![0u8; n];
        getrandom::getrandom(&mut v).unwrap();
        v
    }

    /// `COPY`-like text that zstd can't shrink: `rows` lines of random hex.
    fn random_rows(rows: usize, width: usize) -> Vec<u8> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = Vec::with_capacity(rows * (width * 2 + 1));
        for _ in 0..rows {
            for b in random(width) {
                out.push(HEX[usize::from(b >> 4)]);
                out.push(HEX[usize::from(b & 15)]);
            }
            out.push(b'\n');
        }
        out
    }

    struct Sample {
        manifest: Manifest,
        hub: HubFields,
        settings: BTreeMap<String, Value>,
        docs: Vec<(String, Bytes)>,
        tables: Vec<TableInfo>,
        data: BTreeMap<String, Vec<Bytes>>,
    }

    /// Three tables: `big` spans two entries, `empty` has no rows, `small` two rows sent in
    /// pieces that split a row.
    fn sample(big_bytes: usize) -> Sample {
        let width = 500;
        let big = random_rows(big_bytes / (width * 2 + 1) + 1, width);
        let small = b"1\ttab\\there\n2\t\\N\n".to_vec();
        let data: BTreeMap<String, Vec<Bytes>> = [
            (
                "big".to_owned(),
                big.chunks(100_000).map(Bytes::copy_from_slice).collect(),
            ),
            ("empty".to_owned(), vec![]),
            (
                "small".to_owned(),
                vec![
                    Bytes::copy_from_slice(&small[..5]),
                    Bytes::copy_from_slice(&small[5..]),
                ],
            ),
        ]
        .into_iter()
        .collect();
        let tables: Vec<TableInfo> = data
            .iter()
            .map(|(name, chunks)| TableInfo {
                name: name.clone(),
                columns: vec!["a".into(), "b".into()],
                rows: chunks.iter().map(|c| lines(c)).sum(),
            })
            .collect();
        let docs = vec![
            (
                "nidoo/automation/f1".to_owned(),
                Bytes::from("{\"id\":\"f1\"}"),
            ),
            (
                "nidoo/ui/dashboards/d1".to_owned(),
                Bytes::from(vec![0u8, 255, 10]),
            ),
        ];
        let manifest = Manifest {
            format: FORMAT,
            nidoo_version: "1.2.3".into(),
            migration: 7,
            created_at: "2026-11-11T10:00:00Z".into(),
            host: "hub".into(),
            tables: tables
                .iter()
                .map(|t| {
                    let m = TableManifest {
                        columns: t.columns.clone(),
                        rows: t.rows,
                    };
                    (t.name.clone(), m)
                })
                .collect(),
            retained: 2,
            flows: 1,
            dashboards: 1,
            history_bytes: 123,
        };
        Sample {
            manifest,
            hub: HubFields {
                z2m_base: Some("zigbee2mqtt".into()),
                timezone: Some("Europe/Paris".into()),
                ..HubFields::default()
            },
            settings: [("home".to_owned(), json!({"name": "Maison"}))]
                .into_iter()
                .collect(),
            docs,
            tables,
            data,
        }
    }

    async fn write_sample(s: &Sample, progress: &(dyn Fn(BackupPart, u64) + Sync)) -> Vec<u8> {
        let mut out = Vec::new();
        let contents = Contents {
            manifest: &s.manifest,
            hub: &s.hub,
            settings: &s.settings,
            docs: &s.docs,
            tables: &s.tables,
        };
        write_archive(
            crypt::seal(&mut out, PASSWORD),
            contents,
            Fixed(s.data.clone()),
            progress,
        )
        .await
        .unwrap();
        out
    }

    async fn restore(
        file: &[u8],
    ) -> Result<(Manifest, HubFields, BTreeMap<String, Value>, Recorder), BackupError> {
        let (tar, manifest) = open_archive(file, PASSWORD).await?;
        let mut rec = Recorder::default();
        let (hub, settings) = restore_into(tar, &manifest, &mut rec, &|_| {}).await?;
        Ok((manifest, hub, settings, rec))
    }

    /// The plaintext tar, decrypted and decompressed by the standard libraries.
    fn plaintext_entries(file: &[u8]) -> Vec<(String, Vec<u8>)> {
        let decryptor = age::Decryptor::new(file).unwrap();
        let identity =
            age::scrypt::Identity::new(age::secrecy::SecretString::from(PASSWORD.to_owned()));
        let plain = decryptor
            .decrypt(std::iter::once(&identity as &dyn age::Identity))
            .unwrap();
        let mut archive = ::tar::Archive::new(zstd::stream::read::Decoder::new(plain).unwrap());
        archive
            .entries()
            .unwrap()
            .map(|e| {
                let mut e = e.unwrap();
                let path = e.path().unwrap().to_string_lossy().into_owned();
                let mut data = Vec::new();
                std::io::Read::read_to_end(&mut e, &mut data).unwrap();
                (path, data)
            })
            .collect()
    }

    #[test]
    fn manifest_json_has_the_documented_fields() {
        let m = sample(10).manifest;
        let v = serde_json::to_value(&m).unwrap();
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "created_at",
                "dashboards",
                "flows",
                "format",
                "history_bytes",
                "host",
                "migration",
                "nidoo_version",
                "retained",
                "tables"
            ]
        );
        assert_eq!(
            v["tables"]["small"],
            json!({"columns": ["a", "b"], "rows": 2})
        );
        assert_eq!(serde_json::from_value::<Manifest>(v).unwrap(), m);
    }

    #[test]
    fn copy_paths_round_trip() {
        assert_eq!(copy_path("entity_state", 1), "db/entity_state/000001.copy");
        assert_eq!(
            parse_copy_path("db/entity_state/000012.copy"),
            Some(("entity_state", 12))
        );
        for bad in [
            "db/t/1.copy",
            "db//000001.copy",
            "db/t/000001.txt",
            "db/a/b/000001.copy",
            "x/t/000001.copy",
        ] {
            assert_eq!(parse_copy_path(bad), None, "{bad}");
        }
    }

    #[test]
    fn newer_backups_are_too_new() {
        let m = sample(10).manifest;
        assert_eq!(check_compatible(&m, 7), Ok(()));
        assert_eq!(check_compatible(&m, 8), Ok(()));
        assert_eq!(check_compatible(&m, 6), Err(BackupError::TooNew));
        let newer = Manifest {
            format: FORMAT + 1,
            ..m
        };
        assert_eq!(check_compatible(&newer, 7), Err(BackupError::TooNew));
        // A newer manifest this hub can't even parse.
        assert_eq!(
            parse_manifest(br#"{"format": 2, "parts": []}"#),
            Err(BackupError::TooNew)
        );
        assert_eq!(
            parse_manifest(br#"{"format": 1}"#),
            Err(BackupError::NotABackup)
        );
        assert_eq!(parse_manifest(b"[]"), Err(BackupError::NotABackup));
    }

    #[tokio::test]
    async fn entries_are_laid_out_in_order() {
        let s = sample(MAX_ENTRY + 1000);
        let file = write_sample(&s, &|_, _| {}).await;
        let entries = plaintext_entries(&file);
        let names: Vec<&str> = entries.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            names,
            [
                "manifest.json",
                "hub.json",
                "settings.json",
                "retained/000001.json",
                "db/big/000001.copy",
                "db/big/000002.copy",
                "db/small/000001.copy",
                "end.json"
            ]
        );
        assert_eq!(entries[4].1.len(), MAX_ENTRY);
        let big: Vec<u8> = s.data["big"].concat();
        assert_eq!([&entries[4].1[..], &entries[5].1[..]].concat(), big);

        let json = |i: usize| serde_json::from_slice::<Value>(&entries[i].1).unwrap();
        assert_eq!(
            serde_json::from_value::<Manifest>(json(0)).unwrap(),
            s.manifest
        );
        assert_eq!(json(1)["z2m_base"], "zigbee2mqtt");
        assert_eq!(json(2), json!({"home": {"name": "Maison"}}));
        assert_eq!(
            json(3),
            json!([
                {"topic": "nidoo/automation/f1", "payload_b64": "eyJpZCI6ImYxIn0="},
                {"topic": "nidoo/ui/dashboards/d1", "payload_b64": "AP8K"}
            ])
        );
        assert_eq!(
            json(7),
            json!({"tables": {"big": s.manifest.tables["big"].rows, "empty": 0, "small": 2}})
        );
    }

    #[tokio::test]
    async fn progress_counts_plaintext_bytes() {
        let s = sample(1000);
        let seen = Mutex::new(Vec::new());
        let file = write_sample(&s, &|part, bytes| seen.lock().unwrap().push((part, bytes))).await;
        let seen = seen.into_inner().unwrap();
        let parts: Vec<BackupPart> = seen.iter().map(|(p, _)| *p).collect();
        use BackupPart::*;
        assert_eq!(
            parts,
            [
                Settings, Settings, Settings, Retained, History, History, History
            ]
        );
        assert!(seen.windows(2).all(|w| w[0].1 < w[1].1), "{seen:?}");
        // The last report is every entry; the closing blocks follow.
        let tar_len: u64 = plaintext_entries(&file)
            .iter()
            .map(|(_, d)| (BLOCK + d.len().div_ceil(BLOCK) * BLOCK) as u64)
            .sum();
        assert_eq!(seen.last().unwrap().1, tar_len);
    }

    #[tokio::test]
    async fn a_table_whose_rows_changed_is_refused() {
        let mut s = sample(1000);
        s.tables
            .iter_mut()
            .find(|t| t.name == "small")
            .unwrap()
            .rows = 3;
        let mut out = Vec::new();
        let contents = Contents {
            manifest: &s.manifest,
            hub: &s.hub,
            settings: &s.settings,
            docs: &s.docs,
            tables: &s.tables,
        };
        let err = write_archive(
            crypt::seal(&mut out, PASSWORD),
            contents,
            Fixed(s.data.clone()),
            &|_, _| {},
        )
        .await
        .unwrap_err();
        assert_eq!(
            err,
            BackupError::Database("table small: copied 2 rows, counted 3".into())
        );
    }

    #[tokio::test]
    async fn restore_reads_back_what_was_written() {
        let s = sample(200_000);
        let file = write_sample(&s, &|_, _| {}).await;
        let (manifest, hub, settings, rec) = restore(&file).await.unwrap();
        assert_eq!(manifest, s.manifest);
        assert_eq!(hub, s.hub);
        assert_eq!(settings, s.settings);
        assert_eq!(rec.settings, s.settings);
        assert_eq!(rec.docs, s.docs);
        let expected: BTreeMap<String, Vec<u8>> = s
            .data
            .iter()
            .filter(|(_, chunks)| !chunks.is_empty())
            .map(|(name, chunks)| (name.clone(), chunks.concat()))
            .collect();
        assert_eq!(rec.tables, expected);
    }

    /// Ruling A: 256 KiB holds the first age chunk (64 KiB + tag after the header) and more.
    #[tokio::test]
    async fn manifest_reads_from_the_head() {
        let s = sample(1024 * 1024);
        let file = write_sample(&s, &|_, _| {}).await;
        assert!(file.len() > 2 * 256 * 1024, "{}", file.len());
        let head = &file[..256 * 1024];
        assert_eq!(read_manifest(head, PASSWORD).await.unwrap(), s.manifest);
        assert_eq!(
            read_manifest(head, "wrong password").await,
            Err(BackupError::WrongPassword)
        );
        assert_eq!(
            read_manifest(&random(4096)[..], PASSWORD).await,
            Err(BackupError::NotABackup)
        );
    }

    /// Where each age payload chunk (64 KiB + 16-byte tag) starts; the header ends with
    /// `--- <mac>\n` and a 16-byte nonce follows it.
    fn chunk_starts(file: &[u8]) -> Vec<usize> {
        let mac = file.windows(5).position(|w| w == b"\n--- ").unwrap() + 5;
        let header_end = mac + file[mac..].iter().position(|b| *b == b'\n').unwrap() + 1;
        (header_end + 16..file.len())
            .step_by(64 * 1024 + 16)
            .collect()
    }

    /// No cut file restores: not at 70 %, not one byte short, not without its last chunk.
    #[tokio::test]
    async fn cut_files_never_restore() {
        let s = sample(300_000);
        let file = write_sample(&s, &|_, _| {}).await;
        let starts = chunk_starts(&file);
        assert!(starts.len() > 2, "{starts:?}");
        for cut in [file.len() * 7 / 10, file.len() - 1, *starts.last().unwrap()] {
            let result = restore(&file[..cut]).await;
            assert!(result.is_err(), "cut at {cut} of {} restored", file.len());
        }
    }

    /// The tar's end can be decoded before the stream's own end: zstd flushes it in a block of
    /// its own, and the frame end that follows may fall in age's next chunk. A cut there only
    /// shows when the stream is read to its end, past the tar's closing block.
    #[tokio::test]
    async fn an_error_after_the_tar_end_fails_the_restore() {
        struct Fails;
        impl AsyncRead for Fails {
            fn poll_read(
                self: Pin<&mut Self>,
                _: &mut Context<'_>,
                _: &mut ReadBuf<'_>,
            ) -> Poll<io::Result<()>> {
                Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "age file is truncated",
                )))
            }
        }
        let s = sample(1000);
        let file = write_sample(&s, &|_, _| {}).await;
        let mut tar = Vec::new();
        {
            let mut tw = TarWriter::new(&mut tar);
            for (path, data) in plaintext_entries(&file) {
                tw.entry(&path, &data).await.unwrap();
            }
            tw.finish().await.unwrap();
        }

        async fn restore_from<R: AsyncRead + Unpin>(
            input: R,
            manifest: &Manifest,
        ) -> Result<(HubFields, BTreeMap<String, Value>), BackupError> {
            let mut reader = TarReader::new(input);
            reader.next().await.unwrap().unwrap();
            restore_into(reader, manifest, &mut Recorder::default(), &|_| {}).await
        }
        use tokio::io::AsyncReadExt;
        let whole = restore_from(tar.as_slice().chain(&[][..]), &s.manifest).await;
        assert!(whole.is_ok(), "{whole:?}");
        let err = restore_from(tar.as_slice().chain(Fails), &s.manifest)
            .await
            .unwrap_err();
        assert_eq!(err, ends_early());
    }

    /// A sealed file holding exactly `entries`.
    async fn file_with(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut tw = TarWriter::new(crypt::seal(&mut out, PASSWORD));
        for (path, data) in entries {
            tw.entry(path, data).await.unwrap();
        }
        tw.finish().await.unwrap().shutdown().await.unwrap();
        out
    }

    #[test]
    fn retained_entries_stay_under_the_limit() {
        assert_eq!(retained_entries(&[], 100).unwrap(), [b"[]".to_vec()]);

        let docs: Vec<(String, Bytes)> = (0..20)
            .map(|i| {
                (
                    format!("nidoo/automation/f{i}"),
                    Bytes::from(vec![i as u8; 40]),
                )
            })
            .collect();
        let limit = 300;
        let entries = retained_entries(&docs, limit).unwrap();
        assert!(entries.len() > 3, "{}", entries.len());
        let mut back = Vec::new();
        for e in &entries {
            assert!(e.len() <= limit, "{} > {limit}", e.len());
            let part: Vec<RetainedDoc> = serde_json::from_slice(e).unwrap();
            assert!(!part.is_empty());
            back.extend(part.into_iter().map(|d| {
                (
                    d.topic,
                    Bytes::from(STANDARD.decode(d.payload_b64).unwrap()),
                )
            }));
        }
        assert_eq!(back, docs);
        // Compact JSON, no pretty-printing.
        assert!(!entries[0].contains(&b'\n'));

        let huge = [(
            "nidoo/ui/dashboards/big".to_owned(),
            Bytes::from(vec![0u8; 400]),
        )];
        let err = retained_entries(&huge, limit).unwrap_err();
        assert_eq!(
            err.to_string(),
            "broker: retained topic nidoo/ui/dashboards/big is too large to back up (400 bytes)"
        );
    }

    #[tokio::test]
    async fn retained_entries_are_read_in_order() {
        let mut s = sample(10);
        s.manifest.tables = BTreeMap::new();
        let head = vec![
            (MANIFEST, serde_json::to_vec(&s.manifest).unwrap()),
            (HUB, b"{}".to_vec()),
            (SETTINGS, b"{}".to_vec()),
        ];
        let doc = |topic: &str| json!([{"topic": topic, "payload_b64": "e30="}]).to_string();
        let end = (END, br#"{"tables": {}}"#.to_vec());

        let mut two = head.clone();
        two.push(("retained/000001.json", doc("a/1").into_bytes()));
        two.push(("retained/000002.json", doc("a/2").into_bytes()));
        two.push(end.clone());
        let (.., rec) = restore(&file_with(&two).await).await.unwrap();
        let topics: Vec<&str> = rec.docs.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(topics, ["a/1", "a/2"]);
        assert_eq!(rec.docs[0].1, Bytes::from("{}"));

        let mut gap = head.clone();
        gap.push(("retained/000001.json", doc("a/1").into_bytes()));
        gap.push(("retained/000003.json", doc("a/3").into_bytes()));
        gap.push(end.clone());
        let err = restore(&file_with(&gap).await).await.err().unwrap();
        assert_eq!(err, unexpected("retained/000003.json"));

        let mut none = head;
        none.push(end);
        let err = restore(&file_with(&none).await).await.err().unwrap();
        assert_eq!(err, unexpected(END));
    }

    #[test]
    fn restored_debug_shows_only_setting_keys() {
        let s = sample(10);
        let restored = Restored {
            hub: s.hub,
            settings: [("backup".to_owned(), json!({"password": "hunter2-secret"}))]
                .into_iter()
                .collect(),
            manifest: s.manifest,
        };
        let text = format!("{restored:?}");
        assert!(text.contains("\"backup\""), "{text}");
        assert!(!text.contains("hunter2-secret"), "{text}");
    }

    /// A tar ending without `end.json`, or with fewer rows than the manifest says, is damaged.
    #[tokio::test]
    async fn missing_rows_are_found() {
        let mut s = sample(10);
        s.manifest.tables = [(
            "t".to_owned(),
            TableManifest {
                columns: vec!["a".into()],
                rows: 12,
            },
        )]
        .into_iter()
        .collect();
        let head = vec![
            (MANIFEST, serde_json::to_vec(&s.manifest).unwrap()),
            (HUB, b"{}".to_vec()),
            (SETTINGS, b"{}".to_vec()),
            ("retained/000001.json", b"[]".to_vec()),
        ];
        let end = (END, br#"{"tables": {"t": 12}}"#.to_vec());

        let mut short = head.clone();
        short.push(("db/t/000001.copy", b"1\n".repeat(10)));
        short.push(end.clone());
        let err = restore(&file_with(&short).await).await.err().unwrap();
        assert_eq!(
            err.to_string(),
            "the backup is damaged: table t: 10 rows, expected 12"
        );

        let mut no_rows = head.clone();
        no_rows.push(end.clone());
        let err = restore(&file_with(&no_rows).await).await.err().unwrap();
        assert_eq!(err, rows_differ("t", 0, 12));

        let mut no_end = head.clone();
        no_end.push(("db/t/000001.copy", b"1\n".repeat(12)));
        let err = restore(&file_with(&no_end).await).await.err().unwrap();
        assert_eq!(err, ends_early());

        let mut gap = head;
        gap.push(("db/t/000001.copy", b"1\n".repeat(6)));
        gap.push(("db/t/000003.copy", b"1\n".repeat(6)));
        gap.push(end);
        assert!(restore(&file_with(&gap).await).await.is_err());
    }

    /// Spawned jobs need these futures to be `Send`; this only has to compile.
    #[allow(dead_code)]
    fn futures_are_send(pool: &PgPool, broker: &dyn RetainedBroker, layout: Layout) {
        fn send<T: Send>(_: T) {}
        let sources = Sources {
            pool,
            broker,
            layout,
            hub: HubFields::default(),
        };
        send(write_backup(sources, Vec::new(), "pw", &|_, _| {}));
        send(read_backup(&b""[..], "pw", pool, broker, &|_, _| {}));
        send(read_manifest(&b""[..], "pw"));
    }
}
