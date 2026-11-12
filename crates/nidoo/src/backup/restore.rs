//! The restore job, on a hub booted in restore mode: a backup read from an FTP server or an
//! upload into the database and the broker, then `hub.json` and a restart into Run.
//!
//! The first attempt refuses a hub that already holds data; once it has started, a retry on
//! the same database and broker clears whatever a failed attempt left. A failure wipes what it wrote and keeps the restore
//! pending, so `hub.json` only changes when the restore is done.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use nidoo_api::{
    BackupPart, BackupState, BackupSummary, FtpSource, RemoteBackup, RestoreProgress, SetupStep,
};
use sqlx::PgPool;
use tokio::io::{AsyncRead, ReadBuf};

use super::BackupError;
use super::archive::{Manifest, check_compatible, read_backup, read_manifest};
use super::db;
use super::ftp::{FtpSettings, FtpStore};
use super::retained::{Layout, QUIET, RetainedBroker, filters};
use super::settings::FtpSection;
use crate::history::db::Db;
use crate::setup::HubFile;
use crate::setup::SetupCtx;
use crate::setup::hub_file::RestoreTarget;
use crate::setup::restart::schedule_restart;
use crate::setup::routes::RESTART_DELAY;
use crate::setup::sections::{Sections, assemble};
use crate::setup::store::{load_stored, save_section, write_cache};

fn no_database() -> BackupError {
    BackupError::Database("database unreachable".into())
}

fn io_error(e: io::Error) -> BackupError {
    BackupError::Io(e.to_string())
}

/// The FTP server a typed source names: port 21 and folder `/` when left out.
pub fn ftp_settings(s: &FtpSource) -> FtpSettings {
    let defaults = FtpSection::default();
    FtpSettings {
        host: s.host.trim().to_owned(),
        port: s.port.unwrap_or(defaults.port),
        user: s.user.clone(),
        password: s.password.clone(),
        folder: s
            .folder
            .clone()
            .filter(|f| !f.trim().is_empty())
            .unwrap_or(defaults.folder),
    }
}

/// What the setup wizard shows of a backup before restoring it.
pub fn summary(m: &Manifest) -> BackupSummary {
    BackupSummary {
        created_at: m.created_at.clone(),
        nidoo_version: m.nidoo_version.clone(),
        host: m.host.clone(),
        flows: m.flows,
        dashboards: m.dashboards,
        history_bytes: m.history_bytes,
    }
}

/// Reads a backup's manifest from `input` (its head is enough) and checks this hub can restore
/// it. The migration level is checked only when the database is there; the restore checks it
/// again in any case.
pub async fn inspect(
    ctx: &SetupCtx,
    input: impl AsyncRead + Unpin + Send,
    password: &str,
) -> Result<BackupSummary, BackupError> {
    let manifest = read_manifest(input, password).await?;
    if let Some(db) = &ctx.db {
        check_compatible(&manifest, db::migration_level(db.pool()).await?)?;
    }
    Ok(summary(&manifest))
}

/// The backups on the FTP server, newest first.
pub async fn list_ftp(source: &FtpSettings) -> Result<Vec<RemoteBackup>, BackupError> {
    FtpStore::connect(source).await?.list().await
}

/// Reads the manifest of the backup `file` on the FTP server.
pub async fn inspect_ftp(
    ctx: &SetupCtx,
    source: &FtpSettings,
    file: &str,
    password: &str,
) -> Result<BackupSummary, BackupError> {
    let mut store = FtpStore::connect(source).await?;
    let mut download = Watched::new(store.download(file).await?);
    let result = inspect(ctx, &mut download, password).await;
    download.explain(result)
}

/// Takes the restore slot: `Busy` while a restore runs. A finished or failed one is replaced.
pub async fn claim(ctx: &SetupCtx, total: Option<u64>) -> Result<RestoreProgress, BackupError> {
    let mut held = ctx.restore.lock().await;
    if held
        .as_ref()
        .is_some_and(|p| p.state == BackupState::Running)
    {
        return Err(BackupError::Busy);
    }
    let progress = RestoreProgress {
        state: BackupState::Running,
        part: BackupPart::Settings,
        bytes: 0,
        total,
        error: None,
    };
    *held = Some(progress.clone());
    Ok(progress)
}

/// Updates the running restore's part and bytes. A progress read holding the lock at that
/// moment skips one update; the next one carries the count on.
fn report(ctx: &SetupCtx, part: BackupPart, bytes: u64) {
    if let Ok(mut held) = ctx.restore.try_lock()
        && let Some(p) = held.as_mut()
    {
        p.part = part;
        p.bytes = bytes;
    }
}

/// Runs `work` (a claimed restore) in a task of its own and records how it ended in
/// `ctx.restore`, a panic included. The work goes on when the caller stops waiting.
pub fn spawn_tracked(
    ctx: Arc<SetupCtx>,
    work: impl Future<Output = Result<(), BackupError>> + Send + 'static,
) -> tokio::task::JoinHandle<Result<(), BackupError>> {
    tokio::spawn(async move {
        let result = tokio::spawn(work)
            .await
            .unwrap_or_else(|_| Err(BackupError::Io("the restore stopped".into())));
        if let Some(p) = ctx.restore.lock().await.as_mut() {
            match &result {
                Ok(()) => {
                    // The whole file was read, its end checked.
                    p.state = BackupState::Done;
                    p.bytes = p.total.unwrap_or(p.bytes);
                }
                Err(e) => {
                    p.state = BackupState::Failed;
                    p.error = Some(e.to_string());
                }
            }
        }
        result
    })
}

/// Restores the backup `file` from the FTP server `source` (a claimed restore). The file's size
/// becomes the progress total.
pub async fn restore_from_ftp(
    ctx: &Arc<SetupCtx>,
    source: FtpSettings,
    file: &str,
    password: &str,
) -> Result<(), BackupError> {
    let mut store = FtpStore::connect(&source).await?;
    let total = store
        .list()
        .await?
        .into_iter()
        .find(|b| b.file == file)
        .map(|b| b.size);
    if let Some(p) = ctx.restore.lock().await.as_mut() {
        p.total = total;
    }
    let mut download = Watched::new(store.download(file).await?);
    let progress = |part, bytes| report(ctx, part, bytes);
    let result = run_restore(ctx, &mut download, password, Some(source), &progress).await;
    let result = download.explain(result);
    if result.is_ok()
        && let Err(error) = store.finish_download(download.inner).await
    {
        // Every byte was read and checked: the restore stands.
        tracing::warn!(%error, "restore: the FTP server did not close the download cleanly");
    }
    result
}

/// Restores an uploaded backup (a claimed restore).
pub async fn restore_upload(
    ctx: &Arc<SetupCtx>,
    input: impl AsyncRead + Unpin + Send,
    password: &str,
) -> Result<(), BackupError> {
    let progress = |part, bytes| report(ctx, part, bytes);
    run_restore(ctx, input, password, None, &progress).await
}

/// Restores the backup `input` into the hub `ctx` boots: the checks (first attempt) or the
/// clean-up (a retry), the backup itself, the settings cache, then `hub.json` and the restart.
/// `ftp_used` is the FTP server the file came from; when the backup's own FTP settings differ,
/// it takes them (they just worked). On a failure after the checks, what was written is wiped.
pub async fn run_restore(
    ctx: &Arc<SetupCtx>,
    input: impl AsyncRead + Unpin + Send,
    password: &str,
    ftp_used: Option<FtpSettings>,
    progress: &(dyn Fn(BackupPart, u64) + Sync),
) -> Result<(), BackupError> {
    let db = ctx.db.as_ref().ok_or_else(no_database)?;
    let broker = ctx.restore_broker().await;
    let sections = ctx.sections.lock().await.clone();
    let file = ctx.file.lock().await.clone();
    let layout = Layout::from_config(&assemble(&file, &sections));
    let Some(state) = &file.restore else {
        return Err(BackupError::Io("no restore is pending".into()));
    };
    if state.retry_clears(&file) {
        clear(db, broker.as_ref(), &file, &layout).await?;
    } else {
        // A first attempt, or a retry on a target nobody checked (changed since by an env
        // override or a hand edit): refuse data, and record what was found empty.
        refuse_data(db.pool(), broker.as_ref(), &layout).await?;
        let checked = RestoreTarget::of(&file);
        ctx.update_file(|f| {
            if let Some(r) = f.restore.as_mut() {
                r.attempted = true;
                r.target = checked;
            }
        })
        .await
        .map_err(io_error)?;
    }
    let result = restore(
        ctx,
        db,
        broker.as_ref(),
        input,
        password,
        ftp_used,
        progress,
    )
    .await;
    if result.is_err()
        && let Err(error) = clear(db, broker.as_ref(), &file, &layout).await
    {
        tracing::warn!(%error, "restore: could not clear the failed attempt");
    }
    result
}

/// The steps that write.
async fn restore(
    ctx: &Arc<SetupCtx>,
    db: &Db,
    broker: &dyn RetainedBroker,
    input: impl AsyncRead + Unpin + Send,
    password: &str,
    ftp_used: Option<FtpSettings>,
    progress: &(dyn Fn(BackupPart, u64) + Sync),
) -> Result<(), BackupError> {
    let restored = read_backup(input, password, db.pool(), broker, progress).await?;
    let mut sections = Sections::from_rows(restored.settings);
    if let Some(used) = ftp_used {
        let typed = FtpSection {
            host: used.host,
            port: used.port,
            user: used.user,
            password: (!used.password.is_empty()).then_some(used.password),
            folder: used.folder,
        };
        if sections.backup.ftp != typed {
            sections.backup.ftp = typed;
            save_section(db, "backup", &sections).await?;
        }
    }
    write_cache(&ctx.data_dir, &sections).map_err(io_error)?;
    let hub = restored.hub;
    ctx.update_file(|f| {
        f.apply_fields(hub);
        f.restore = None;
        f.finished = true;
        f.step = SetupStep::Done;
    })
    .await
    .map_err(io_error)?;
    schedule_restart(ctx, RESTART_DELAY);
    Ok(())
}

/// A first attempt goes ahead only on an empty hub: no history, no backed-up documents.
async fn refuse_data(
    pool: &PgPool,
    broker: &dyn RetainedBroker,
    layout: &Layout,
) -> Result<(), BackupError> {
    if let Some(table) = db::holds_history(pool).await? {
        return Err(BackupError::HoldsData(format!("history in {table}")));
    }
    let docs = broker
        .collect(filters(layout), QUIET)
        .await
        .map_err(BackupError::Broker)?;
    if !docs.is_empty() {
        return Err(BackupError::HoldsData(
            "flows or settings on the broker".into(),
        ));
    }
    Ok(())
}

/// Empties the database and clears the backed-up topics, under both this hub's layout and the
/// one of the settings an attempt left in the table (a backup's own prefixes).
async fn clear(
    db: &Db,
    broker: &dyn RetainedBroker,
    file: &HubFile,
    layout: &Layout,
) -> Result<(), BackupError> {
    let mut wanted = filters(layout);
    if let Some(left) = load_stored(db).await? {
        for filter in filters(&Layout::from_config(&assemble(file, &left))) {
            if !wanted.contains(&filter) {
                wanted.push(filter);
            }
        }
    }
    db::wipe(db.pool()).await?;
    let docs = broker
        .collect(wanted, QUIET)
        .await
        .map_err(BackupError::Broker)?;
    if docs.is_empty() {
        return Ok(());
    }
    broker
        .publish_retained(docs.into_iter().map(|(t, _)| (t, Bytes::new())).collect())
        .await
        .map_err(BackupError::Broker)
}

/// A download that remembers whether the server went quiet, so a stalled transfer is reported
/// as the server being unreachable, whatever error the layers above made of it.
struct Watched<R> {
    inner: R,
    timed_out: bool,
}

impl<R> Watched<R> {
    fn new(inner: R) -> Self {
        Watched {
            inner,
            timed_out: false,
        }
    }

    fn explain<T>(&self, result: Result<T, BackupError>) -> Result<T, BackupError> {
        match result {
            Err(_) if self.timed_out => Err(BackupError::Unreachable("timed out".into())),
            other => other,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for Watched<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Err(e)) = &polled
            && e.kind() == io::ErrorKind::TimedOut
        {
            self.timed_out = true;
        }
        polled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    /// Reads `data`, then fails with `kind`.
    struct Failing {
        data: Vec<u8>,
        kind: io::ErrorKind,
    }

    impl AsyncRead for Failing {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            if self.data.is_empty() {
                return Poll::Ready(Err(io::Error::new(self.kind, "failed")));
            }
            let data = std::mem::take(&mut self.data);
            buf.put_slice(&data);
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn a_stalled_download_is_unreachable() {
        let mut w = Watched::new(Failing {
            data: b"abc".to_vec(),
            kind: io::ErrorKind::TimedOut,
        });
        let mut out = Vec::new();
        assert!(w.read_to_end(&mut out).await.is_err());
        assert_eq!(out, b"abc");
        let explained = w.explain::<()>(Err(BackupError::Corrupt("zstd".into())));
        assert_eq!(explained, Err(BackupError::Unreachable("timed out".into())));
    }

    #[tokio::test]
    async fn other_errors_are_kept() {
        let mut w = Watched::new(Failing {
            data: Vec::new(),
            kind: io::ErrorKind::ConnectionReset,
        });
        assert!(w.read_u8().await.is_err());
        let explained = w.explain::<()>(Err(BackupError::Io("reset".into())));
        assert_eq!(explained, Err(BackupError::Io("reset".into())));
    }

    #[test]
    fn a_typed_source_fills_the_defaults() {
        let s = ftp_settings(&FtpSource {
            host: " nas.local ".into(),
            port: None,
            user: "u".into(),
            password: "p".into(),
            folder: Some(" ".into()),
        });
        assert_eq!(s.host, "nas.local");
        assert_eq!(s.port, 21);
        assert_eq!(s.folder, "/");
        let s = ftp_settings(&FtpSource {
            host: "nas.local".into(),
            port: Some(2121),
            user: "u".into(),
            password: "p".into(),
            folder: Some("/backups".into()),
        });
        assert_eq!((s.port, s.folder.as_str()), (2121, "/backups"));
    }
}
