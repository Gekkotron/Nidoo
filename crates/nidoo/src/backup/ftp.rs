//! The FTP side of a backup: upload, list, download and prune over plain passive FTP.
//!
//! An upload goes to `<name>.partial` and is renamed only after its last byte, so a listed
//! backup is always complete. Nothing here puts the password in an error.

use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use nidoo_api::{FtpCheck, FtpStep, RemoteBackup};
use suppaftp::tokio::{AsyncDataStream, AsyncFtpStream, AsyncNoTlsStream};
use suppaftp::{FtpError, Mode};
use tokio::io::{AsyncRead, AsyncWriteExt};

use super::BackupError;
use super::name;

/// How long to wait for the server to answer the first connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// A `.partial` file untouched for this long is an abandoned upload.
const PARTIAL_TTL: chrono::Duration = chrono::Duration::hours(24);
const PARTIAL: &str = ".partial";
const PROBE_FILE: &str = ".nidoo-probe";

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
pub type Download = AsyncDataStream<AsyncNoTlsStream>;

/// A logged-in connection, already in the backup folder.
pub struct FtpStore {
    ftp: AsyncFtpStream,
}

fn unreachable(e: FtpError) -> BackupError {
    BackupError::Unreachable(e.to_string())
}

fn io(e: FtpError) -> BackupError {
    BackupError::Io(e.to_string())
}

async fn open(s: &FtpSettings) -> Result<AsyncFtpStream, BackupError> {
    let addr = (s.host.as_str(), s.port);
    let mut ftp = match tokio::time::timeout(CONNECT_TIMEOUT, AsyncFtpStream::connect(addr)).await {
        Ok(Ok(ftp)) => ftp,
        Ok(Err(e)) => return Err(unreachable(e)),
        Err(_) => return Err(BackupError::Unreachable("timed out".into())),
    };
    ftp.set_mode(Mode::Passive);
    Ok(ftp)
}

async fn log_in(ftp: &mut AsyncFtpStream, s: &FtpSettings) -> Result<(), BackupError> {
    match ftp.login(s.user.as_str(), s.password.as_str()).await {
        Ok(()) => Ok(()),
        Err(FtpError::UnexpectedResponse(_)) => Err(BackupError::LoginRefused),
        Err(e) => Err(unreachable(e)),
    }
}

async fn enter(ftp: &mut AsyncFtpStream, s: &FtpSettings) -> Result<(), BackupError> {
    ftp.cwd(s.folder.as_str())
        .await
        .map_err(|e| BackupError::Folder(e.to_string()))
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
        let mut ftp = open(s).await?;
        log_in(&mut ftp, s).await?;
        enter(&mut ftp, s).await?;
        Ok(FtpStore { ftp })
    }

    /// The backups on the server, newest first. Anything not named like a backup (including
    /// `.partial` files) is ignored.
    pub async fn list(&mut self) -> Result<Vec<RemoteBackup>, BackupError> {
        let mut found = Vec::new();
        for entry in self.ftp.nlst(None).await.map_err(io)? {
            let file = base(&entry).to_string();
            let Some(at) = name::parse(&file) else {
                continue;
            };
            let size = self.ftp.size(&file).await.map_err(io)?;
            found.push(RemoteBackup {
                file,
                size: size as u64,
                created_at: at.timestamp(),
            });
        }
        found.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.file.cmp(&a.file)));
        Ok(found)
    }

    /// Stores `data` as `name`, through `<name>.partial`. Returns the bytes sent. On failure the
    /// `.partial` is removed as far as the server allows.
    pub async fn upload(
        &mut self,
        name: &str,
        mut data: impl AsyncRead + Unpin,
    ) -> Result<u64, BackupError> {
        let partial = format!("{name}{PARTIAL}");
        let mut stream = self.ftp.put_with_stream(&partial).await.map_err(io)?;
        let copied = tokio::io::copy(&mut data, &mut stream).await;
        let sent = match copied {
            Ok(sent) => sent,
            Err(e) => {
                // Close the data connection and read the server's answer so the control
                // connection is usable for the clean-up.
                let _ = stream.shutdown().await;
                let _ = self.ftp.finalize_put_stream(stream).await;
                let _ = self.ftp.rm(&partial).await;
                return Err(BackupError::Io(e.to_string()));
            }
        };
        // The server has the last byte only once it answers the end of the transfer.
        if let Err(e) = self.ftp.finalize_put_stream(stream).await {
            let _ = self.ftp.rm(&partial).await;
            return Err(io(e));
        }
        if let Err(e) = self.ftp.rename(partial.as_str(), name).await {
            let _ = self.ftp.rm(&partial).await;
            return Err(io(e));
        }
        Ok(sent)
    }

    /// Starts downloading `name`; read the stream to its end, then call [`Self::finish_download`].
    pub async fn download(&mut self, name: &str) -> Result<Download, BackupError> {
        self.ftp.retr_as_stream(name).await.map_err(io)
    }

    /// Closes a download started with [`Self::download`] and checks the server saw it through.
    pub async fn finish_download(&mut self, stream: Download) -> Result<(), BackupError> {
        self.ftp.finalize_retr_stream(stream).await.map_err(io)
    }

    /// Deletes the oldest backups beyond `keep`, and `.partial` files of backups that have sat
    /// untouched for more than 24 hours. Files that are not named like either are left alone.
    /// Returns the names deleted.
    pub async fn prune(
        &mut self,
        keep: u32,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>, BackupError> {
        let entries: Vec<String> = self
            .ftp
            .nlst(None)
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
            .skip(keep as usize)
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
            if let Ok(modified) = self.ftp.mdtm(entry).await
                && now - unix(modified) > PARTIAL_TTL
            {
                doomed.push(entry.clone());
            }
        }

        let mut deleted = Vec::new();
        for entry in doomed {
            self.ftp.rm(&entry).await.map_err(io)?;
            deleted.push(entry);
        }
        Ok(deleted)
    }

    /// Tries each step of using the server in turn (connect, log in, enter the folder, write a
    /// file and delete it) and stops at the first that fails.
    pub async fn probe(s: &FtpSettings) -> Vec<FtpCheck> {
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

        let mut ftp = match open(s).await {
            Ok(ftp) => ftp,
            Err(e) => {
                step(FtpStep::Connect, Err(e));
                return checks;
            }
        };
        step(FtpStep::Connect, Ok(()));
        if !step(FtpStep::Login, log_in(&mut ftp, s).await) {
            return checks;
        }
        if !step(FtpStep::Folder, enter(&mut ftp, s).await) {
            return checks;
        }
        let write = async {
            ftp.put_file(PROBE_FILE, &mut &b"nidoo"[..])
                .await
                .map_err(io)?;
            ftp.rm(PROBE_FILE).await.map_err(io)
        };
        step(FtpStep::Write, write.await);
        let _ = ftp.quit().await;
        checks
    }
}
