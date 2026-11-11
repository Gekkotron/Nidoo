//! Backup and restore: one encrypted file holding the settings, the retained definitions and the
//! whole history.

pub mod archive;
pub mod crypt;
pub mod db;
pub mod ftp;
pub mod name;
pub mod retained;
pub mod tar;

/// Why a backup or restore stopped. The text is shown to the user as is; none carries a secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BackupError {
    #[error("a backup is already running")]
    Busy,
    #[error("no backup password set")]
    NoPassword,
    #[error("FTP not configured")]
    FtpNotConfigured,
    #[error("can't reach the FTP server ({0})")]
    Unreachable(String),
    #[error("login refused")]
    LoginRefused,
    #[error("FTP folder: {0}")]
    Folder(String),
    #[error("time limit reached")]
    TimeLimit,
    #[error("cancelled")]
    Cancelled,
    #[error("Wrong password")]
    WrongPassword,
    #[error("This file is not a Nidoo backup")]
    NotABackup,
    #[error("This backup comes from a newer Nidoo; update this hub first")]
    TooNew,
    #[error("the backup is damaged: {0}")]
    Corrupt(String),
    #[error("This hub already holds data ({0})")]
    HoldsData(String),
    #[error("database: {0}")]
    Database(String),
    #[error("broker: {0}")]
    Broker(String),
    #[error("{0}")]
    Io(String),
}

impl From<crypt::OpenError> for BackupError {
    fn from(e: crypt::OpenError) -> Self {
        match e {
            crypt::OpenError::WrongPassword => Self::WrongPassword,
            crypt::OpenError::NotABackup => Self::NotABackup,
            crypt::OpenError::Io(e) => Self::Io(e),
        }
    }
}

impl From<sqlx::Error> for BackupError {
    fn from(e: sqlx::Error) -> Self {
        Self::Database(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_the_ones_shown() {
        let cases = [
            (BackupError::Busy, "a backup is already running"),
            (BackupError::NoPassword, "no backup password set"),
            (BackupError::FtpNotConfigured, "FTP not configured"),
            (
                BackupError::Unreachable("connection refused".into()),
                "can't reach the FTP server (connection refused)",
            ),
            (BackupError::LoginRefused, "login refused"),
            (
                BackupError::Folder("not found".into()),
                "FTP folder: not found",
            ),
            (BackupError::TimeLimit, "time limit reached"),
            (BackupError::Cancelled, "cancelled"),
            (BackupError::WrongPassword, "Wrong password"),
            (BackupError::NotABackup, "This file is not a Nidoo backup"),
            (
                BackupError::TooNew,
                "This backup comes from a newer Nidoo; update this hub first",
            ),
            (
                BackupError::Corrupt("table t: 10 rows, expected 12".into()),
                "the backup is damaged: table t: 10 rows, expected 12",
            ),
            (
                BackupError::HoldsData("history in entity_state".into()),
                "This hub already holds data (history in entity_state)",
            ),
            (BackupError::Database("gone".into()), "database: gone"),
            (BackupError::Broker("gone".into()), "broker: gone"),
            (BackupError::Io("broken pipe".into()), "broken pipe"),
        ];
        for (err, text) in cases {
            assert_eq!(err.to_string(), text);
        }
    }

    #[test]
    fn open_errors_map_across() {
        use crypt::OpenError;
        assert_eq!(
            BackupError::from(OpenError::WrongPassword),
            BackupError::WrongPassword
        );
        assert_eq!(
            BackupError::from(OpenError::NotABackup),
            BackupError::NotABackup
        );
        assert_eq!(
            BackupError::from(OpenError::Io("reset".into())),
            BackupError::Io("reset".into())
        );
    }
}
