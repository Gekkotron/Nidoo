//! The FTP side of a backup: upload, list, download and prune over plain passive FTP.
//!
//! An upload goes to `<name>.partial` and is renamed only after its last byte, so a listed
//! backup is always complete. Nothing here puts the password in an error.

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use nidoo_api::{FtpCheck, FtpStep, RemoteBackup};
use suppaftp::tokio::{AsyncDataStream, AsyncFtpStream, AsyncNoTlsStream};
use suppaftp::types::FileType;
use suppaftp::{FtpError, Mode};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, ReadBuf};

use super::BackupError;
use super::name;

/// A `.partial` file untouched for this long is an abandoned upload.
const PARTIAL_TTL: chrono::Duration = chrono::Duration::hours(24);
const PARTIAL: &str = ".partial";
const PROBE_FILE: &str = ".nidoo-probe";
const CHUNK: usize = 64 * 1024;

/// How long a stalled NAS may keep us waiting. suppaftp's tokio client has no timeouts of its
/// own, so every await on the server goes through one of these.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    /// The first connection and the greeting.
    pub connect: Duration,
    /// One control command (login, cwd, PASV and opening the data connection, SIZE, rm, ...).
    pub control: Duration,
    /// A data transfer, or the server's final answer to one, making no progress at all. A slow
    /// transfer that keeps moving is never cut.
    pub stall: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Timeouts {
            connect: Duration::from_secs(15),
            control: Duration::from_secs(30),
            stall: Duration::from_secs(60),
        }
    }
}

fn timed_out() -> FtpError {
    FtpError::ConnectionError(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "timed out",
    ))
}

/// Runs one server exchange, giving up after `limit`.
async fn within<T>(
    limit: Duration,
    op: impl std::future::Future<Output = Result<T, FtpError>>,
) -> Result<T, FtpError> {
    tokio::time::timeout(limit, op)
        .await
        .unwrap_or_else(|_| Err(timed_out()))
}

fn is_timeout(e: &FtpError) -> bool {
    matches!(e, FtpError::ConnectionError(io) if io.kind() == std::io::ErrorKind::TimedOut)
}

/// Where the backups live. `password` is never logged or put in an error.
#[derive(Clone)]
pub struct FtpSettings {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub folder: String,
}

impl std::fmt::Debug for FtpSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FtpSettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("folder", &self.folder)
            .finish_non_exhaustive()
    }
}

/// The data connection of a running download; hand it back to [`FtpStore::finish_download`].
/// Reading it fails with `TimedOut` when the server sends nothing for the stall timeout.
pub struct Download {
    inner: AsyncDataStream<AsyncNoTlsStream>,
    stall: Duration,
    idle: Pin<Box<tokio::time::Sleep>>,
}

impl AsyncRead for Download {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(r) => {
                this.idle
                    .as_mut()
                    .reset(tokio::time::Instant::now() + this.stall);
                Poll::Ready(r)
            }
            Poll::Pending => match this.idle.as_mut().poll(cx) {
                Poll::Ready(()) => Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "timed out",
                ))),
                Poll::Pending => Poll::Pending,
            },
        }
    }
}

/// A logged-in connection, already in the backup folder.
pub struct FtpStore {
    ftp: AsyncFtpStream,
    t: Timeouts,
}

fn unreachable(e: FtpError) -> BackupError {
    if is_timeout(&e) {
        BackupError::Unreachable("timed out".into())
    } else {
        BackupError::Unreachable(e.to_string())
    }
}

fn io(e: FtpError) -> BackupError {
    if is_timeout(&e) {
        BackupError::Unreachable("timed out".into())
    } else {
        BackupError::Io(e.to_string())
    }
}

fn io_err(e: std::io::Error) -> BackupError {
    if e.kind() == std::io::ErrorKind::TimedOut {
        BackupError::Unreachable("timed out".into())
    } else {
        BackupError::Io(e.to_string())
    }
}

async fn open(s: &FtpSettings, t: Timeouts) -> Result<AsyncFtpStream, BackupError> {
    let addr = (s.host.as_str(), s.port);
    let mut ftp = within(t.connect, AsyncFtpStream::connect(addr))
        .await
        .map_err(unreachable)?;
    ftp.set_mode(Mode::Passive);
    Ok(ftp)
}

async fn log_in(ftp: &mut AsyncFtpStream, s: &FtpSettings, t: Timeouts) -> Result<(), BackupError> {
    match within(t.control, ftp.login(s.user.as_str(), s.password.as_str())).await {
        Ok(()) => Ok(()),
        Err(FtpError::UnexpectedResponse(_)) => Err(BackupError::LoginRefused),
        Err(e) => Err(unreachable(e)),
    }
}

/// Puts the session in binary mode. suppaftp does not, and FTP starts in ASCII, where a server
/// may rewrite the line ends of a transfer (corrupting the encrypted file) or refuse `SIZE`.
async fn binary(ftp: &mut AsyncFtpStream, t: Timeouts) -> Result<(), BackupError> {
    within(t.control, ftp.transfer_type(FileType::Binary))
        .await
        .map_err(io)
}

async fn enter(ftp: &mut AsyncFtpStream, s: &FtpSettings, t: Timeouts) -> Result<(), BackupError> {
    within(t.control, ftp.cwd(s.folder.as_str()))
        .await
        .map_err(|e| {
            if is_timeout(&e) {
                BackupError::Unreachable("timed out".into())
            } else {
                BackupError::Folder(e.to_string())
            }
        })
}

/// The bare file name of a listing entry (some servers return it with its folder).
fn base(entry: &str) -> &str {
    entry.rsplit('/').next().unwrap_or(entry)
}

fn unix(naive: NaiveDateTime) -> DateTime<Utc> {
    naive.and_utc()
}

impl FtpStore {
    pub async fn connect(s: &FtpSettings) -> Result<FtpStore, BackupError> {
        Self::connect_with(s, Timeouts::default()).await
    }

    /// [`Self::connect`] with chosen timeouts (the tests use short ones).
    pub async fn connect_with(s: &FtpSettings, t: Timeouts) -> Result<FtpStore, BackupError> {
        let mut ftp = open(s, t).await?;
        log_in(&mut ftp, s, t).await?;
        binary(&mut ftp, t).await?;
        enter(&mut ftp, s, t).await?;
        Ok(FtpStore { ftp, t })
    }

    /// Checks the control connection still answers (a `NOOP`) and connects again once if it does
    /// not. A backup connects before it collects its data, which can outlast the server's idle
    /// timeout (300 s on vsftpd).
    pub async fn keep_alive(&mut self, s: &FtpSettings) -> Result<(), BackupError> {
        if within(self.t.control, self.ftp.noop()).await.is_ok() {
            return Ok(());
        }
        *self = Self::connect_with(s, self.t).await?;
        Ok(())
    }

    /// The backups on the server, newest first. Anything not named like a backup (including
    /// `.partial` files) is ignored.
    pub async fn list(&mut self) -> Result<Vec<RemoteBackup>, BackupError> {
        let mut found = Vec::new();
        for entry in within(self.t.control, self.ftp.nlst(None))
            .await
            .map_err(io)?
        {
            let file = base(&entry).to_string();
            let Some(at) = name::parse(&file) else {
                continue;
            };
            let size = within(self.t.control, self.ftp.size(&file))
                .await
                .map_err(io)?;
            found.push(RemoteBackup {
                file,
                size: size as u64,
                created_at: at.timestamp(),
            });
        }
        found.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.file.cmp(&a.file)));
        Ok(found)
    }

    /// Best-effort delete of a leftover; the error that matters is the caller's.
    async fn discard(&mut self, file: &str) {
        let _ = within(self.t.control, self.ftp.rm(file)).await;
    }

    /// Stores `data` as `name`, through `<name>.partial`. Returns the bytes sent. On failure the
    /// `.partial` is removed as far as the server allows.
    pub async fn upload(
        &mut self,
        name: &str,
        mut data: impl AsyncRead + Unpin,
    ) -> Result<u64, BackupError> {
        let partial = format!("{name}{PARTIAL}");
        let t = self.t;
        let mut stream = within(t.control, self.ftp.put_with_stream(&partial))
            .await
            .map_err(io)?;
        let mut sent = 0u64;
        let mut buf = vec![0u8; CHUNK];
        let copied: Result<(), BackupError> = async {
            loop {
                let n = tokio::time::timeout(t.stall, data.read(&mut buf))
                    .await
                    .map_err(|_| BackupError::Io("source stalled".into()))?
                    .map_err(|e| BackupError::Io(e.to_string()))?;
                if n == 0 {
                    return Ok(());
                }
                tokio::time::timeout(t.stall, stream.write_all(&buf[..n]))
                    .await
                    .map_err(|_| BackupError::Unreachable("timed out".into()))?
                    .map_err(io_err)?;
                sent += n as u64;
            }
        }
        .await;
        if let Err(e) = copied {
            // Close the data connection and read the server's answer so the control
            // connection is usable for the clean-up.
            let _ = within(t.control, async {
                let _ = stream.shutdown().await;
                self.ftp.finalize_put_stream(stream).await
            })
            .await;
            self.discard(&partial).await;
            return Err(e);
        }
        // The server has the last byte only once it answers the end of the transfer.
        if let Err(e) = within(t.stall, self.ftp.finalize_put_stream(stream)).await {
            self.discard(&partial).await;
            return Err(io(e));
        }
        if let Err(e) = within(t.control, self.ftp.rename(partial.as_str(), name)).await {
            self.discard(&partial).await;
            return Err(io(e));
        }
        Ok(sent)
    }

    /// Starts downloading `name`; read the stream to its end, then call [`Self::finish_download`].
    pub async fn download(&mut self, name: &str) -> Result<Download, BackupError> {
        let inner = within(self.t.control, self.ftp.retr_as_stream(name))
            .await
            .map_err(io)?;
        Ok(Download {
            inner,
            stall: self.t.stall,
            idle: Box::pin(tokio::time::sleep(self.t.stall)),
        })
    }

    /// Closes a download started with [`Self::download`] and checks the server saw it through.
    pub async fn finish_download(&mut self, stream: Download) -> Result<(), BackupError> {
        within(self.t.stall, self.ftp.finalize_retr_stream(stream.inner))
            .await
            .map_err(io)
    }

    /// Deletes the oldest backups beyond `keep` (at least one is always kept), and `.partial`
    /// files of backups that have sat untouched for more than 24 hours. Files that are not named
    /// like either are left alone. A delete that fails is logged and skipped. Returns the names
    /// actually deleted.
    pub async fn prune(
        &mut self,
        keep: u32,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>, BackupError> {
        let keep = keep.max(1) as usize;
        let c = self.t.control;
        let entries: Vec<String> = within(c, self.ftp.nlst(None))
            .await
            .map_err(io)?
            .iter()
            .map(|e| base(e).to_string())
            .collect();

        let mut backups: Vec<(DateTime<Utc>, &String)> = entries
            .iter()
            .filter_map(|e| name::parse(e).map(|at| (at, e)))
            .collect();
        backups.sort_by(|a, b| b.cmp(a));
        let mut doomed: Vec<String> = backups
            .iter()
            .skip(keep)
            .map(|(_, e)| (*e).clone())
            .collect();

        for entry in &entries {
            let Some(stem) = entry.strip_suffix(PARTIAL) else {
                continue;
            };
            if name::parse(stem).is_none() {
                continue;
            }
            // A file whose age we cannot read is not ours to guess about.
            if let Ok(modified) = within(c, self.ftp.mdtm(entry)).await
                && now - unix(modified) > PARTIAL_TTL
            {
                doomed.push(entry.clone());
            }
        }

        let mut deleted = Vec::new();
        for entry in doomed {
            match within(c, self.ftp.rm(&entry)).await {
                Ok(()) => deleted.push(entry),
                Err(e) => tracing::warn!(file = %entry, error = %e, "backup prune: delete failed"),
            }
        }
        Ok(deleted)
    }

    /// Tries each step of using the server in turn (connect, log in, enter the folder, write a
    /// file and delete it) and stops at the first that fails. Always returns.
    pub async fn probe(s: &FtpSettings) -> Vec<FtpCheck> {
        Self::probe_with(s, Timeouts::default()).await
    }

    /// [`Self::probe`] with chosen timeouts.
    pub async fn probe_with(s: &FtpSettings, t: Timeouts) -> Vec<FtpCheck> {
        let mut checks = Vec::new();
        let mut step = |step: FtpStep, result: Result<(), BackupError>| {
            let ok = result.is_ok();
            checks.push(FtpCheck {
                step,
                ok,
                detail: result.err().map(|e| e.to_string()),
            });
            ok
        };

        let mut ftp = match open(s, t).await {
            Ok(ftp) => ftp,
            Err(e) => {
                step(FtpStep::Connect, Err(e));
                return checks;
            }
        };
        step(FtpStep::Connect, Ok(()));
        let logged_in = match log_in(&mut ftp, s, t).await {
            Ok(()) => binary(&mut ftp, t).await,
            Err(e) => Err(e),
        };
        if !step(FtpStep::Login, logged_in) {
            return checks;
        }
        if !step(FtpStep::Folder, enter(&mut ftp, s, t).await) {
            return checks;
        }
        let stored = within(t.stall, ftp.put_file(PROBE_FILE, &mut &b"nidoo"[..]))
            .await
            .map_err(io);
        let write = match stored {
            Ok(_) => within(t.control, ftp.rm(PROBE_FILE)).await.map_err(io),
            Err(e) => Err(e),
        };
        if write.is_err() {
            let _ = within(t.control, ftp.rm(PROBE_FILE)).await;
        }
        step(FtpStep::Write, write);
        let _ = within(t.control, ftp.quit()).await;
        checks
    }
}
