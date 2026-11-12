//! Wire types for backup and restore: the job's progress, the last backup, the restore source.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::setup::{Secret, SecretView};

/// The part of the file a backup or restore is working on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BackupPart {
    Settings,
    Retained,
    History,
    Upload,
    Download,
}

/// Where a backup job is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BackupState {
    Running,
    Done,
    Failed,
    Cancelled,
}

/// Where a backup goes: the FTP server, or the browser that asked for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BackupDestination {
    Ftp,
    Download,
}

/// The running backup job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BackupProgress {
    pub state: BackupState,
    pub destination: BackupDestination,
    pub part: BackupPart,
    /// Bytes of the backup written so far, before compression.
    pub bytes: u64,
    /// Unix seconds.
    pub started_at: i64,
    pub file: String,
    pub error: Option<String>,
}

/// The last backup that ended, whatever its outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LastBackup {
    /// Unix seconds.
    pub started_at: i64,
    /// Unix seconds.
    pub ended_at: i64,
    pub destination: BackupDestination,
    pub file: String,
    /// Bytes of the file sent (all of it on success).
    pub size: u64,
    pub ok: bool,
    pub error: Option<String>,
    /// When the last successful backup ended, unix seconds; kept across failures.
    pub last_ok_at: Option<i64>,
}

/// A finished FTP backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BackupDone {
    pub file: String,
    pub size: u64,
    /// The FTP server it went to.
    pub host: String,
}

/// One backup file found on the FTP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RemoteBackup {
    pub file: String,
    pub size: u64,
    /// When the backup was taken, unix seconds (read from the file name).
    pub created_at: i64,
}

/// A step of the FTP connection test, in the order they are tried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FtpStep {
    Connect,
    Login,
    Folder,
    Write,
}

/// The outcome of one step of the FTP connection test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FtpCheck {
    pub step: FtpStep,
    pub ok: bool,
    pub detail: Option<String>,
}

/// The backup section as a read shows it: neither password is sent back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BackupView {
    pub password: SecretView,
    pub ftp: FtpView,
    pub keep: u32,
    pub notify_failure: bool,
}

/// Where the backups are copied, as a read shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FtpView {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: SecretView,
    pub folder: String,
}

/// The backup section in a `PUT`. A `PUT` replaces the whole section: a left-out `keep`,
/// `port` or `folder` takes its default. Only the passwords follow `Secret`.
#[derive(Debug, Clone, Deserialize)]
pub struct BackupInput {
    #[serde(default)]
    pub password: Secret,
    #[serde(default)]
    pub ftp: FtpInput,
    #[serde(default)]
    pub keep: Option<u32>,
    #[serde(default = "on")]
    pub notify_failure: bool,
}

fn on() -> bool {
    true
}

impl Default for BackupInput {
    fn default() -> Self {
        Self {
            password: Secret::default(),
            ftp: FtpInput::default(),
            keep: None,
            notify_failure: true,
        }
    }
}

/// The FTP part of [`BackupInput`]. An empty `host` means FTP is not configured.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FtpInput {
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: Secret,
    #[serde(default)]
    pub folder: Option<String>,
}

/// The FTP server a restore reads from, as typed in the setup wizard. A left-out `port` is 21
/// and a left-out `folder` is `/`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FtpSource {
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub folder: Option<String>,
}

impl std::fmt::Debug for FtpSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FtpSource")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("folder", &self.folder)
            .finish_non_exhaustive()
    }
}

/// One backup file on an FTP server, and the backup password that opens it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RestoreFromFtp {
    pub source: FtpSource,
    pub file: String,
    pub password: String,
}

impl std::fmt::Debug for RestoreFromFtp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestoreFromFtp")
            .field("source", &self.source)
            .field("file", &self.file)
            .finish_non_exhaustive()
    }
}

/// What a backup holds, read from its head before restoring it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BackupSummary {
    /// RFC 3339, UTC.
    pub created_at: String,
    pub nidoo_version: String,
    /// The host name of the hub the backup was taken on.
    pub host: String,
    pub flows: u32,
    pub dashboards: u32,
    /// The history's size in the source database: an estimate, not the file's size.
    pub history_bytes: u64,
}

/// The running or last restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RestoreProgress {
    pub state: BackupState,
    pub part: BackupPart,
    /// Bytes of the backup file read so far.
    pub bytes: u64,
    /// The file's size, when known.
    pub total: Option<u64>,
    pub error: Option<String>,
    /// What kind of failure `error` is, for the ones a page words itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<RestoreErrorCode>,
}

/// A failed restore's kind, beside its English `error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RestoreErrorCode {
    /// The FTP server could not be reached, or stopped answering.
    FtpUnreachable,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ftp_source_defaults_and_hides_its_password() {
        let source: FtpSource =
            serde_json::from_str(r#"{"host": "nas.local", "password": "ftp-secret"}"#).unwrap();
        assert_eq!(source.port, None);
        assert_eq!(source.folder, None);
        assert_eq!(source.user, "");
        let request = RestoreFromFtp {
            source,
            file: "f".into(),
            password: "backup-secret".into(),
        };
        let shown = format!("{request:?}");
        assert!(
            !shown.contains("ftp-secret") && !shown.contains("backup-secret"),
            "{shown}"
        );
    }

    #[test]
    fn restore_progress_code_is_optional_on_the_wire() {
        let old: RestoreProgress = serde_json::from_str(
            r#"{"state":"failed","part":"history","bytes":1,"total":null,"error":"x"}"#,
        )
        .unwrap();
        assert_eq!(old.code, None);
        assert!(!serde_json::to_string(&old).unwrap().contains("code"));
        let unreachable = RestoreProgress {
            code: Some(RestoreErrorCode::FtpUnreachable),
            ..old
        };
        assert_eq!(
            serde_json::to_value(&unreachable).unwrap()["code"],
            "ftp_unreachable"
        );
    }

    #[test]
    fn backup_part_is_snake_case() {
        assert_eq!(
            serde_json::to_string(&BackupPart::Retained).unwrap(),
            "\"retained\""
        );
        assert_eq!(
            serde_json::from_str::<BackupPart>("\"download\"").unwrap(),
            BackupPart::Download
        );
    }

    #[test]
    fn backup_input_notify_failure_defaults_on() {
        let input: BackupInput = serde_json::from_str("{}").unwrap();
        assert!(input.notify_failure);
        let off: BackupInput = serde_json::from_str(r#"{"notify_failure": false}"#).unwrap();
        assert!(!off.notify_failure);
    }

    #[test]
    fn last_backup_wire_shape() {
        let last = LastBackup {
            started_at: 10,
            ended_at: 20,
            destination: BackupDestination::Download,
            file: "f".into(),
            size: 3,
            ok: false,
            error: Some("cancelled".into()),
            last_ok_at: None,
        };
        assert_eq!(
            serde_json::to_value(&last).unwrap(),
            serde_json::json!({"started_at": 10, "ended_at": 20, "destination": "download",
                "file": "f", "size": 3, "ok": false, "error": "cancelled", "last_ok_at": null})
        );
        assert_eq!(
            serde_json::to_string(&BackupState::Cancelled).unwrap(),
            "\"cancelled\""
        );
    }

    #[test]
    fn ftp_check_wire_shape() {
        let check = FtpCheck {
            step: FtpStep::Folder,
            ok: false,
            detail: Some("not found".into()),
        };
        assert_eq!(
            serde_json::to_value(&check).unwrap(),
            serde_json::json!({"step": "folder", "ok": false, "detail": "not found"})
        );
    }
}
