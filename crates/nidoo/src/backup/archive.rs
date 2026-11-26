//! The backup file: a tar of JSON documents and `COPY` text, sealed by [`crypt::seal`].
//!
//! Its entries, in this order:
//! - `manifest.json`: the [`Manifest`], first so a restore can check a file from its head;
//! - `hub.json`: the [`HubFields`];
//! - `settings.json`: the `settings` rows, `key → value`;
//! - `z2m/<n>.zip`: Zigbee2MQTT's own backup zip, cut into entries of at most [`MAX_ENTRY`]
//!   bytes (`n` from `000001`); none when the backup carries no Zigbee2MQTT data (and in every
//!   format 1 file);
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
pub const FORMAT: u32 = 2;

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

/// Zigbee2MQTT's backup zip as the file carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Z2mManifest {
    /// The zip's size.
    pub bytes: u64,
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
    /// Zigbee2MQTT's zip, when the file carries it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z2m: Option<Z2mManifest>,
}

/// What a backup reads from.
pub struct Sources<'a> {
    pub pool: &'a PgPool,
    pub broker: &'a dyn RetainedBroker,
    pub layout: Layout,
    pub hub: HubFields,
    /// Zigbee2MQTT's backup zip, when there is one to carry (it holds the Zigbee network key).
    pub z2m: Option<Bytes>,
}

/// What a restore hands back for its caller to apply: `hub.json` is the caller's to write.
#[derive(Clone, PartialEq)]
pub struct Restored {
    pub hub: HubFields,
    pub settings: BTreeMap<String, Value>,
    pub manifest: Manifest,
    /// Zigbee2MQTT's backup zip, when the file carried one.
    pub z2m: Option<Bytes>,
}

/// Settings rows may hold secrets: only their keys are shown, and the Zigbee2MQTT zip (it holds
/// the network key) only by its length.
impl fmt::Debug for Restored {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Restored")
            .field("hub", &self.hub)
            .field("settings", &self.settings.keys().collect::<Vec<_>>())
            .field("manifest", &self.manifest)
            .field("z2m", &self.z2m.as_ref().map(Bytes::len))
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
        z2m: src.z2m.as_ref().map(|z| Z2mManifest {
            bytes: z.len() as u64,
        }),
    };
    let contents = Contents {
        manifest: &manifest,
        hub: &src.hub,
        settings: &settings,
        z2m: src.z2m.as_ref(),
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
    z2m: Option<&'a Bytes>,
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
    if let Some(z2m) = c.z2m {
        for (i, data) in z2m.chunks(MAX_ENTRY).enumerate() {
            w.entry(BackupPart::Z2m, &z2m_path(i as u32 + 1), data)
                .await?;
        }
    }
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

fn z2m_path(n: u32) -> String {
    format!("z2m/{n:06}.zip")
}

/// `n` of a `z2m/<n>.zip` entry.
fn parse_z2m_path(path: &str) -> Option<u32> {
    let n: u32 = path
        .strip_prefix("z2m/")?
        .strip_suffix(".zip")?
        .parse()
        .ok()?;
    (z2m_path(n) == path).then_some(n)
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
    let (hub, settings, z2m) = restore_into(tar, &manifest, &mut target, &report).await?;
    db::fix_sequences(pool).await?;
    Ok(Restored {
        hub,
        settings,
        manifest,
        z2m,
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
) -> Result<(HubFields, BTreeMap<String, Value>, Option<Bytes>), BackupError> {
    let hub: HubFields = parse(HUB, &expect(&mut tar, HUB).await?)?;
    let settings: BTreeMap<String, Value> = parse(SETTINGS, &expect(&mut tar, SETTINGS).await?)?;
    target.settings(&settings).await?;
    report(BackupPart::Settings);

    // `z2m/000001.zip` onwards, when there are any; the entry after them is the first retained one.
    let mut zip: Vec<u8> = Vec::new();
    let mut parts = 0u32;
    let first_retained = loop {
        let (path, data) = next(&mut tar).await?.ok_or_else(ends_early)?;
        match parse_z2m_path(&path) {
            Some(n) if n == parts + 1 => {
                parts = n;
                zip.extend_from_slice(&data);
            }
            Some(_) => return Err(unexpected(&path)),
            None => break (path, data),
        }
    };
    let expected_zip = manifest.z2m.as_ref().map_or(0, |z| z.bytes);
    if zip.len() as u64 != expected_zip {
        return Err(BackupError::Corrupt(
            "the Zigbee2MQTT data does not match the manifest".into(),
        ));
    }
    let z2m = (parts > 0).then(|| Bytes::from(zip));
    if z2m.is_some() {
        report(BackupPart::Z2m);
    }

    // `retained/000001.json` onwards; the entry after them is the first table's (or the end).
    let mut docs = Vec::new();
    let mut last = 0u32;
    let mut first = Some(first_retained);
    let after_retained = loop {
        let (path, data) = match first.take() {
            Some(entry) => entry,
            None => next(&mut tar).await?.ok_or_else(ends_early)?,
        };
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
    Ok((hub, settings, z2m))
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
mod tests;
