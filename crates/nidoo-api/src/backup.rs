//! Wire types for backup and restore: the job's progress, the last backup, the restore source.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
