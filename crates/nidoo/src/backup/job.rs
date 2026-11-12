//! The backup job: one at a time, to the FTP server or to the browser that asked for it.
//!
//! The file is written into an in-memory pipe whose other end feeds the FTP upload or the HTTP
//! body. Whatever the outcome, the job ends by writing `<data>/backup-last.json`, the record the
//! page and the `system_last_backup` sensor read. A failed FTP backup (the unattended one) also
//! raises a push, when the section's switch is on.

use std::io;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, ready};
use std::time::Duration;

use bytes::Bytes;
use chrono::Utc;
use futures_util::Stream;
use nidoo_api::{
    BackupDestination, BackupDone, BackupInput, BackupPart, BackupProgress, BackupState, FtpCheck,
    FtpInput, FtpStep, LastBackup, Secret,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader, DuplexStream, ReadBuf};
use tokio::sync::{oneshot, watch};
use tokio_util::io::ReaderStream;

use super::BackupError;
use super::archive::{Sources, write_backup};
use super::ftp::FtpStore;
use super::name;
use super::retained::{Layout, MqttRetained, RetainedBroker};
use super::settings;
use crate::config::{Config, MqttConfig};
use crate::hub::Hub;
use crate::setup::SetupCtx;

/// The last-backup record, in the data directory.
pub const RECORD_FILE: &str = "backup-last.json";
/// The pipe between the archive writer and the upload or the HTTP body.
const PIPE: usize = 256 * 1024;
const UPLOAD_BUFFER: usize = 64 * 1024;
/// The chunks the HTTP body is sent in.
const BODY_CHUNK: usize = 64 * 1024;
/// A download whose browser reads nothing for this long, with bytes waiting, is cancelled.
pub const DOWNLOAD_IDLE: Duration = Duration::from_secs(10 * 60);

type Outcome = Option<Result<BackupDone, BackupError>>;

struct Job {
    progress: BackupProgress,
    cancel: watch::Sender<bool>,
    done: watch::Receiver<Outcome>,
}

type Slot = Arc<Mutex<Option<Job>>>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The hub's one backup slot and what a backup reads the retained documents from.
pub struct Backups {
    job: Slot,
    layout: Layout,
    mqtt: MqttConfig,
    /// The broker the retained documents are read from; the hub's own broker (over a
    /// connection of its own) unless a test set another.
    broker: Mutex<Option<Arc<dyn RetainedBroker>>>,
    download_idle: Mutex<Duration>,
}

impl Backups {
    pub fn new(cfg: &Config) -> Backups {
        Backups {
            job: Slot::default(),
            layout: Layout::from_config(cfg),
            mqtt: cfg.mqtt.clone(),
            broker: Mutex::new(None),
            download_idle: Mutex::new(DOWNLOAD_IDLE),
        }
    }

    pub(crate) fn set_broker(&self, broker: Arc<dyn RetainedBroker>) {
        *lock(&self.broker) = Some(broker);
    }

    fn broker(&self) -> Arc<dyn RetainedBroker> {
        lock(&self.broker)
            .get_or_insert_with(|| Arc::new(MqttRetained::new(self.mqtt.clone())))
            .clone()
    }
}

/// One job's fixed facts, and its way to the slot.
struct Run {
    slot: Slot,
    destination: BackupDestination,
    started_at: i64,
    file: String,
    /// Bytes of the file handed on so far.
    sent: Arc<AtomicU64>,
    /// Bytes put into the pipe so far.
    written: Arc<AtomicU64>,
}

impl Run {
    fn update(&self, f: impl FnOnce(&mut BackupProgress)) {
        if let Some(job) = lock(&self.slot).as_mut() {
            f(&mut job.progress);
        }
    }
}

/// Puts a new running job in the (empty) slot.
fn claim(
    slot: &Slot,
    held: &mut Option<Job>,
    destination: BackupDestination,
) -> (Run, watch::Receiver<bool>, watch::Sender<Outcome>) {
    let now = Utc::now();
    let progress = BackupProgress {
        state: BackupState::Running,
        destination,
        part: BackupPart::Settings,
        bytes: 0,
        started_at: now.timestamp(),
        file: name::file_name(now),
        error: None,
    };
    let (cancel, cancelled) = watch::channel(false);
    let (done_tx, done) = watch::channel(None);
    let run = Run {
        slot: Arc::clone(slot),
        destination,
        started_at: progress.started_at,
        file: progress.file.clone(),
        sent: Arc::default(),
        written: Arc::default(),
    };
    *held = Some(Job {
        progress,
        cancel,
        done,
    });
    (run, cancelled, done_tx)
}

/// The writing end of the pipe. Its shutdown, the archive's very last step, marks the file
/// complete before the reader can see the end of it.
struct Head {
    inner: DuplexStream,
    complete: Arc<AtomicBool>,
    written: Arc<AtomicU64>,
}

impl AsyncWrite for Head {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let n = ready!(Pin::new(&mut self.inner).poll_write(cx, buf))?;
        self.written.fetch_add(n as u64, Ordering::Relaxed);
        Poll::Ready(Ok(n))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.complete.store(true, Ordering::SeqCst);
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// The reading end of the pipe. Its end of file is a clean one only once the archive is
/// complete; before that it is an error, so a cut backup is never stored or served as whole.
/// Reaching the clean end is reported on `end`; dropping the reader before it drops `end`.
struct Tail {
    inner: DuplexStream,
    complete: Arc<AtomicBool>,
    sent: Arc<AtomicU64>,
    end: Option<oneshot::Sender<()>>,
}

impl AsyncRead for Tail {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        let before = buf.filled().len();
        ready!(Pin::new(&mut this.inner).poll_read(cx, buf))?;
        let n = buf.filled().len() - before;
        if n > 0 {
            this.sent.fetch_add(n as u64, Ordering::Relaxed);
            return Poll::Ready(Ok(()));
        }
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if !this.complete.load(Ordering::SeqCst) {
            return Poll::Ready(Err(io::Error::other("the backup stopped")));
        }
        if let Some(end) = this.end.take() {
            let _ = end.send(());
        }
        Poll::Ready(Ok(()))
    }
}

/// A pipe for one backup: the writer's end, the reader's end and the signal the reader sends
/// when it read the whole file. `written` and `sent` count the bytes in and out.
fn pipe(written: &Arc<AtomicU64>, sent: &Arc<AtomicU64>) -> (Head, Tail, oneshot::Receiver<()>) {
    let (w, r) = tokio::io::duplex(PIPE);
    let complete = Arc::new(AtomicBool::new(false));
    let (end_tx, end_rx) = oneshot::channel();
    let head = Head {
        inner: w,
        complete: Arc::clone(&complete),
        written: Arc::clone(written),
    };
    let tail = Tail {
        inner: r,
        complete,
        sent: Arc::clone(sent),
        end: Some(end_tx),
    };
    (head, tail, end_rx)
}

/// Resolves once the reader has taken nothing for `limit` while bytes wait in the pipe. A slow
/// reader that keeps taking, or a writer with nothing to give yet, never trips it.
async fn stalled(read: &AtomicU64, written: &AtomicU64, limit: Duration) {
    let step = (limit / 10).max(Duration::from_millis(10));
    let mut last = read.load(Ordering::Relaxed);
    let mut since = tokio::time::Instant::now();
    loop {
        tokio::time::sleep(step).await;
        let now = read.load(Ordering::Relaxed);
        if now != last || written.load(Ordering::Relaxed) <= now {
            last = now;
            since = tokio::time::Instant::now();
        } else if since.elapsed() >= limit {
            return;
        }
    }
}

/// The data directory's last-backup record; `None` when there is none or it can't be read.
pub fn read_record(dir: &Path) -> Option<LastBackup> {
    let text = std::fs::read(dir.join(RECORD_FILE)).ok()?;
    match serde_json::from_slice(&text) {
        Ok(record) => Some(record),
        Err(error) => {
            tracing::warn!(%error, file = RECORD_FILE, "unreadable last-backup record");
            None
        }
    }
}

/// Replaces the record (atomically, mode 0600).
pub fn write_record(dir: &Path, record: &LastBackup) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(record).expect("a record serializes");
    crate::setup::fs::write_private_atomic(&dir.join(RECORD_FILE), &bytes)
}

/// The push raised when an FTP backup fails.
pub fn failure_alert(started_at: i64, reason: &str) -> Value {
    json!({
        "v": 1,
        "id": format!("backup-failed-{started_at}"),
        "channel": "system",
        "severity": "warning",
        "title": "Backup failed",
        "body": format!("Backup failed: {reason}"),
        "silent": false,
        "loc": {"key": "hub.backup_failed", "args": {"reason": reason}},
    })
}

/// `512 B`, `12.3 MB`, `1.4 GB`: powers of 1000, one decimal.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["kB", "MB", "GB", "TB", "PB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    // 999.95 would print as 1000.0 of the smaller unit.
    while value >= 999.95 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

fn no_database() -> BackupError {
    BackupError::Database("no database".into())
}

fn hub_gone() -> BackupError {
    BackupError::Io("the hub is stopping".into())
}

enum Kind {
    Ftp,
    Download {
        head: Head,
        end: oneshot::Receiver<()>,
    },
}

impl Hub {
    /// The setup context with a database, which every backup needs.
    fn backup_ctx(&self) -> Result<Arc<SetupCtx>, BackupError> {
        let ctx = self.setup.get().ok_or_else(no_database)?;
        if ctx.db.is_none() {
            return Err(no_database());
        }
        Ok(Arc::clone(ctx))
    }

    /// Starts a backup to the FTP server, ended by `limit` if still running then. `Busy` while
    /// another runs; `NoPassword` and `FtpNotConfigured` are checked before it starts.
    pub fn start_backup(&self, limit: Duration) -> Result<BackupProgress, BackupError> {
        let mut held = lock(&self.backups.job);
        if held.is_some() {
            return Err(BackupError::Busy);
        }
        self.launch_ftp(&mut held, limit)
            .map(|(progress, _)| progress)
    }

    /// Joins the running FTP backup, or starts one with `limit`, and waits for its end.
    /// Dropping the future leaves the job running.
    pub async fn backup_and_wait(&self, limit: Duration) -> Result<BackupDone, BackupError> {
        let mut done = {
            let mut held = lock(&self.backups.job);
            match held.as_ref() {
                Some(job) if job.progress.destination == BackupDestination::Ftp => job.done.clone(),
                Some(_) => return Err(BackupError::Busy),
                None => self.launch_ftp(&mut held, limit)?.1,
            }
        };
        match done.wait_for(Option::is_some).await {
            Ok(outcome) => outcome.clone().expect("waited for an outcome"),
            Err(_) => Err(BackupError::Io("the backup stopped".into())),
        }
    }

    fn launch_ftp(
        &self,
        held: &mut Option<Job>,
        limit: Duration,
    ) -> Result<(BackupProgress, watch::Receiver<Outcome>), BackupError> {
        let ctx = self.backup_ctx()?;
        // A settings write holding the lock right now: the job checks again when it starts.
        if let Ok(sections) = ctx.sections.try_lock() {
            if sections.backup.password.is_none() {
                return Err(BackupError::NoPassword);
            }
            sections.backup.ftp_settings()?;
        }
        let hub = self.me.upgrade().ok_or_else(hub_gone)?;
        let (run, cancelled, done) = claim(&self.backups.job, held, BackupDestination::Ftp);
        let job = held.as_ref().expect("just claimed");
        let started = (job.progress.clone(), job.done.clone());
        tokio::spawn(async move {
            hub.run_backup(ctx, Arc::new(run), Kind::Ftp, Some(limit), cancelled, done)
                .await;
        });
        Ok(started)
    }

    /// Starts a backup streamed to the caller: the file name and the file. No time limit;
    /// dropping the stream before its end cancels the job.
    pub fn backup_download(
        &self,
    ) -> Result<
        (
            String,
            impl Stream<Item = io::Result<Bytes>> + Send + 'static,
        ),
        BackupError,
    > {
        let ctx = self.backup_ctx()?;
        let mut held = lock(&self.backups.job);
        if held.is_some() {
            return Err(BackupError::Busy);
        }
        if let Ok(sections) = ctx.sections.try_lock()
            && sections.backup.password.is_none()
        {
            return Err(BackupError::NoPassword);
        }
        let hub = self.me.upgrade().ok_or_else(hub_gone)?;
        let (run, cancelled, done) =
            claim(&self.backups.job, &mut held, BackupDestination::Download);
        let (head, tail, end) = pipe(&run.written, &run.sent);
        let file = run.file.clone();
        tokio::spawn(async move {
            let kind = Kind::Download { head, end };
            hub.run_backup(ctx, Arc::new(run), kind, None, cancelled, done)
                .await;
        });
        Ok((file, ReaderStream::with_capacity(tail, BODY_CHUNK)))
    }

    /// The running backup, if any.
    pub fn backup_progress(&self) -> Option<BackupProgress> {
        lock(&self.backups.job).as_ref().map(|j| j.progress.clone())
    }

    /// Asks the running backup to stop; `false` when none runs.
    pub fn cancel_backup(&self) -> bool {
        match lock(&self.backups.job).as_ref() {
            Some(job) => {
                job.cancel.send_replace(true);
                true
            }
            None => false,
        }
    }

    /// The last backup that ended, from `data/backup-last.json`.
    pub fn last_backup(&self) -> Option<LastBackup> {
        read_record(&self.setup.get()?.data_dir)
    }

    /// Tries the FTP server step by step: the stored settings, or `input` with its kept
    /// password taken from the stored ones. An input the section would refuse (a new address
    /// with the password kept) fails the first step with the reason.
    pub async fn test_ftp(&self, input: Option<FtpInput>) -> Vec<FtpCheck> {
        let refused = |detail: String| {
            vec![FtpCheck {
                step: FtpStep::Connect,
                ok: false,
                detail: Some(detail),
            }]
        };
        let Some(ctx) = self.setup.get() else {
            return refused(BackupError::FtpNotConfigured.to_string());
        };
        let stored = ctx.sections.lock().await.backup.clone();
        let section = match input {
            None => stored,
            Some(ftp) => {
                let input = BackupInput {
                    password: Secret::Keep,
                    ftp,
                    keep: Some(stored.keep),
                    notify_failure: stored.notify_failure,
                };
                match settings::apply_input(&stored, input) {
                    Ok(section) => section,
                    Err(why) => return refused(why),
                }
            }
        };
        match section.ftp_settings() {
            Ok(s) => FtpStore::probe(&s).await,
            Err(e) => refused(e.to_string()),
        }
    }

    /// For tests: how long a download may sit unread before it is cancelled.
    #[doc(hidden)]
    pub fn set_backup_download_idle(&self, idle: Duration) {
        *lock(&self.backups.download_idle) = idle;
    }

    /// The whole job, from its first step to the record. The work runs in a task of its own, so
    /// the record is written and the slot freed however it ends, a panic included.
    async fn run_backup(
        self: Arc<Self>,
        ctx: Arc<SetupCtx>,
        run: Arc<Run>,
        kind: Kind,
        limit: Option<Duration>,
        mut cancelled: watch::Receiver<bool>,
        done: watch::Sender<Outcome>,
    ) {
        let mut work = {
            let (hub, ctx, run) = (Arc::clone(&self), Arc::clone(&ctx), Arc::clone(&run));
            tokio::spawn(async move {
                match kind {
                    Kind::Ftp => hub.ftp_job(&ctx, &run).await,
                    Kind::Download { head, end } => hub.download_job(&ctx, &run, head, end).await,
                }
            })
        };
        let mut joined = false;
        let outcome = {
            let finished = async {
                let outcome = (&mut work).await;
                joined = true;
                outcome.unwrap_or_else(|error| {
                    tracing::error!(%error, "the backup job died");
                    Err(BackupError::Io("the backup stopped".into()))
                })
            };
            let limited = async {
                match limit {
                    Some(limit) => tokio::time::timeout(limit, finished)
                        .await
                        .unwrap_or(Err(BackupError::TimeLimit)),
                    None => finished.await,
                }
            };
            tokio::select! {
                outcome = limited => outcome,
                Ok(_) = cancelled.wait_for(|c| *c) => Err(BackupError::Cancelled),
            }
        };
        if !joined {
            // Stopped from outside: end the work (its pipe, connection and snapshot) first.
            work.abort();
            let _ = work.await;
        }
        self.end_backup(&ctx, &run, outcome, done).await;
    }

    /// Writes the record, pushes a failed FTP backup, frees the slot, then tells the waiters.
    async fn end_backup(
        &self,
        ctx: &SetupCtx,
        run: &Run,
        outcome: Result<BackupDone, BackupError>,
        done: watch::Sender<Outcome>,
    ) {
        // The job's own check of the settings, when the one before the start was skipped:
        // a refusal like a 409, so no record and no push.
        if let Err(error @ (BackupError::NoPassword | BackupError::FtpNotConfigured)) = &outcome {
            tracing::warn!(%error, "backup refused");
            *lock(&run.slot) = None;
            done.send_replace(Some(outcome));
            return;
        }
        let ended_at = Utc::now().timestamp();
        let previous = read_record(&ctx.data_dir).and_then(|r| r.last_ok_at);
        let record = LastBackup {
            started_at: run.started_at,
            ended_at,
            destination: run.destination,
            file: run.file.clone(),
            size: match &outcome {
                Ok(d) => d.size,
                Err(_) => run.sent.load(Ordering::Relaxed),
            },
            ok: outcome.is_ok(),
            error: outcome.as_ref().err().map(ToString::to_string),
            last_ok_at: if outcome.is_ok() {
                Some(ended_at)
            } else {
                previous
            },
        };
        match &outcome {
            Ok(_) => tracing::info!(file = %run.file, size = record.size, "backup done"),
            Err(error) => tracing::warn!(file = %run.file, %error, "backup ended"),
        }
        let dir = ctx.data_dir.clone();
        let written = tokio::task::spawn_blocking(move || write_record(&dir, &record)).await;
        match written {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::error!(%error, "could not write the last-backup record"),
            Err(error) => tracing::error!(%error, "could not write the last-backup record"),
        }
        if let Err(error) = &outcome
            && *error != BackupError::Cancelled
            && run.destination == BackupDestination::Ftp
            && ctx.sections.lock().await.backup.notify_failure
        {
            let alert = failure_alert(run.started_at, &error.to_string());
            if let Err(rejected) = self.notify("system", &alert) {
                tracing::warn!(reason = %rejected.reason, "backup failure push refused");
            }
        }
        *lock(&run.slot) = None;
        done.send_replace(Some(outcome));
    }

    fn backup_sources<'a>(
        &'a self,
        ctx: &'a SetupCtx,
        broker: &'a dyn RetainedBroker,
        hub: crate::setup::hub_file::HubFields,
    ) -> Result<Sources<'a>, BackupError> {
        Ok(Sources {
            pool: ctx.db.as_ref().ok_or_else(no_database)?.pool(),
            broker,
            layout: self.backups.layout.clone(),
            hub,
        })
    }

    /// Connects, writes the archive into the pipe while the upload reads it, then prunes.
    async fn ftp_job(&self, ctx: &SetupCtx, run: &Run) -> Result<BackupDone, BackupError> {
        let section = ctx.sections.lock().await.backup.clone();
        let password = section.password.clone().ok_or(BackupError::NoPassword)?;
        let ftp = section.ftp_settings()?;
        let fields = ctx.file.lock().await.fields();
        let broker = self.backups.broker();
        let sources = self.backup_sources(ctx, &*broker, fields)?;
        let mut store = FtpStore::connect(&ftp).await?;
        let (head, tail, _end) = pipe(&run.written, &run.sent);

        // Whichever side fails first holds the cause; the other then fails on the pipe.
        let first = Mutex::new(None);
        let fail = |e: BackupError| {
            lock(&first).get_or_insert(e);
        };
        let progress = |part, bytes| {
            run.update(|p| {
                p.part = part;
                p.bytes = bytes;
            })
        };
        let write = async {
            match write_backup(sources, head, &password, &progress).await {
                Ok(_) => run.update(|p| p.part = BackupPart::Upload),
                Err(e) => fail(e),
            }
        };
        let upload = async {
            let mut reader = BufReader::with_capacity(UPLOAD_BUFFER, tail);
            // The transfer starts with the first bytes: collecting the documents and counting
            // the rows come first, and the upload gives up on a source that stays silent.
            if let Err(e) = reader.fill_buf().await {
                fail(BackupError::Io(e.to_string()));
                return None;
            }
            match store.upload(&run.file, reader).await {
                Ok(sent) => Some(sent),
                Err(e) => {
                    fail(e);
                    None
                }
            }
        };
        let ((), sent) = tokio::join!(write, upload);
        if let Some(e) = first.into_inner().unwrap_or_else(PoisonError::into_inner) {
            return Err(e);
        }
        let size = sent.ok_or_else(|| BackupError::Io("the upload stopped".into()))?;
        match store.prune(section.keep, Utc::now()).await {
            Ok(deleted) if !deleted.is_empty() => {
                tracing::info!(?deleted, "backup prune: old backups deleted");
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "backup prune failed"),
        }
        Ok(BackupDone {
            file: run.file.clone(),
            size,
            host: ftp.host,
        })
    }

    /// Writes the archive into the pipe the HTTP body reads; done once the body read it all.
    async fn download_job(
        &self,
        ctx: &SetupCtx,
        run: &Run,
        head: Head,
        mut end: oneshot::Receiver<()>,
    ) -> Result<BackupDone, BackupError> {
        let section = ctx.sections.lock().await.backup.clone();
        let password = section.password.ok_or(BackupError::NoPassword)?;
        let fields = ctx.file.lock().await.fields();
        let broker = self.backups.broker();
        let sources = self.backup_sources(ctx, &*broker, fields)?;
        let progress = |part, bytes| {
            run.update(|p| {
                p.part = part;
                p.bytes = bytes;
            })
        };
        let idle = *lock(&self.backups.download_idle);
        let stall = stalled(&run.sent, &run.written, idle);
        tokio::pin!(stall);
        let write = write_backup(sources, head, &password, &progress);
        tokio::pin!(write);
        let mut read_all = false;
        let written = loop {
            tokio::select! {
                written = &mut write => break written,
                read = &mut end, if !read_all => match read {
                    Ok(()) => read_all = true,
                    // The body was dropped: nobody reads the rest.
                    Err(_) => return Err(BackupError::Cancelled),
                },
                // The browser stopped reading: free the slot and the snapshot.
                () = &mut stall => return Err(BackupError::Cancelled),
            }
        };
        if let Err(e) = written {
            // A write that broke on the pipe because the browser left is a cancel.
            let left = matches!(end.try_recv(), Err(oneshot::error::TryRecvError::Closed));
            return Err(if left { BackupError::Cancelled } else { e });
        }
        run.update(|p| p.part = BackupPart::Download);
        if !read_all {
            let read = tokio::select! {
                read = end => read.is_ok(),
                () = &mut stall => false,
            };
            if !read {
                return Err(BackupError::Cancelled);
            }
        }
        Ok(BackupDone {
            file: run.file.clone(),
            size: run.sent.load(Ordering::Relaxed),
            host: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_size_formats() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(1_000), "1.0 kB");
        assert_eq!(human_size(12_345_678), "12.3 MB");
        assert_eq!(human_size(1_400_000_000), "1.4 GB");
        assert_eq!(human_size(999_960), "1.0 MB");
    }

    fn record(ok: bool) -> LastBackup {
        LastBackup {
            started_at: 1_800_000_000,
            ended_at: 1_800_000_060,
            destination: BackupDestination::Ftp,
            file: "nidoo-backup-20270115-080000.tar.zst.age".into(),
            size: 1234,
            ok,
            error: (!ok).then(|| "time limit reached".into()),
            last_ok_at: Some(1_799_000_000),
        }
    }

    #[test]
    fn record_round_trips_privately() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_record(dir.path()), None);
        write_record(dir.path(), &record(true)).unwrap();
        assert_eq!(read_record(dir.path()), Some(record(true)));
        write_record(dir.path(), &record(false)).unwrap();
        assert_eq!(read_record(dir.path()), Some(record(false)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(RECORD_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::write(dir.path().join(RECORD_FILE), "not json").unwrap();
        assert_eq!(read_record(dir.path()), None);
    }

    #[test]
    fn failure_alert_shape() {
        let alert = failure_alert(1_800_000_000, "time limit reached");
        assert_eq!(
            alert,
            json!({"v": 1, "id": "backup-failed-1800000000", "channel": "system",
                "severity": "warning", "title": "Backup failed",
                "body": "Backup failed: time limit reached", "silent": false,
                "loc": {"key": "hub.backup_failed", "args": {"reason": "time limit reached"}}})
        );
        let normalized = nidoo_push::normalize::normalize("system", &alert, 1_800_000_000)
            .expect("a valid alert");
        assert!(normalized.loc.is_some(), "the loc hint was dropped");
    }

    #[tokio::test]
    async fn tail_ends_cleanly_only_after_the_archive_finished() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let sent = Arc::new(AtomicU64::new(0));
        let written = Arc::new(AtomicU64::new(0));
        let (mut head, mut tail, end) = pipe(&written, &sent);
        head.write_all(b"abc").await.unwrap();
        drop(head);
        let mut got = Vec::new();
        let err = tail.read_to_end(&mut got).await.unwrap_err();
        assert_eq!(err.to_string(), "the backup stopped");
        assert_eq!(got, b"abc");
        drop(tail);
        assert!(end.await.is_err(), "no clean end");

        let (mut head, mut tail, end) = pipe(&written, &sent);
        head.write_all(b"defg").await.unwrap();
        head.shutdown().await.unwrap();
        let mut got = Vec::new();
        tail.read_to_end(&mut got).await.unwrap();
        assert_eq!(got, b"defg");
        assert!(end.await.is_ok());
        assert_eq!(sent.load(Ordering::Relaxed), 7);
    }

    #[tokio::test(start_paused = true)]
    async fn stall_needs_bytes_waiting_and_no_reads() {
        let limit = Duration::from_secs(600);
        let (read, written) = (AtomicU64::new(0), AtomicU64::new(0));
        // Nothing written yet (collecting, counting): never stalled.
        let quiet = tokio::time::timeout(limit * 3, stalled(&read, &written, limit)).await;
        assert!(quiet.is_err());
        // A slow reader that keeps taking: never stalled.
        written.store(1_000, Ordering::Relaxed);
        let slow = async {
            for n in 1..=30 {
                tokio::time::sleep(Duration::from_secs(60)).await;
                read.store(n, Ordering::Relaxed);
            }
        };
        tokio::select! {
            () = stalled(&read, &written, limit) => panic!("a moving reader stalled"),
            () = slow => {}
        }
        // Bytes waiting, nothing read: stalled after the limit, not before.
        let start = tokio::time::Instant::now();
        stalled(&read, &written, limit).await;
        let waited = start.elapsed();
        assert!(waited >= limit && waited <= limit + limit / 5, "{waited:?}");
    }

    #[tokio::test]
    async fn dropped_reader_breaks_a_blocked_writer() {
        use tokio::io::AsyncWriteExt;

        let sent = Arc::new(AtomicU64::new(0));
        let written = Arc::new(AtomicU64::new(0));
        let (mut head, tail, _end) = pipe(&written, &sent);
        let writer = tokio::spawn(async move {
            let block = vec![0u8; PIPE * 2];
            head.write_all(&block).await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        drop(tail);
        let err = tokio::time::timeout(Duration::from_secs(1), writer)
            .await
            .expect("the writer noticed")
            .unwrap()
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }
}
