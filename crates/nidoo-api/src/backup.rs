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
}
