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

#[cfg(test)]
mod tests {
    use super::*;

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
