//! The `backup` settings section: the backup password, the FTP destination and the retention.
//! Stored live (no restart), like `mobile`; the job reads it from the hub's live sections.

use std::fmt;

use nidoo_api::{BackupInput, BackupView, FtpView, SecretView};
use serde::{Deserialize, Serialize};

use crate::backup::BackupError;
use crate::backup::ftp::FtpSettings;
use crate::setup::secret;

const SHORT_PASSWORD: &str = "The backup password needs at least 8 characters";
const NO_REMOVE: &str = "The backup password cannot be removed";
const NO_KEEP: &str = "Keep at least one backup";
const MIN_PASSWORD_CHARS: usize = 8;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupSection {
    pub password: Option<String>,
    pub ftp: FtpSection,
    pub keep: u32,
    pub notify_failure: bool,
}

impl Default for BackupSection {
    fn default() -> Self {
        Self {
            password: None,
            ftp: FtpSection::default(),
            keep: 7,
            notify_failure: true,
        }
    }
}

impl fmt::Debug for BackupSection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let password = self.password.as_ref().map(|_| "***");
        f.debug_struct("BackupSection")
            .field("password", &password)
            .field("ftp", &self.ftp)
            .field("keep", &self.keep)
            .field("notify_failure", &self.notify_failure)
            .finish()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FtpSection {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<String>,
    pub folder: String,
}

impl Default for FtpSection {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 21,
            user: String::new(),
            password: None,
            folder: "/".into(),
        }
    }
}

impl fmt::Debug for FtpSection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let password = self.password.as_ref().map(|_| "***");
        f.debug_struct("FtpSection")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &password)
            .field("folder", &self.folder)
            .finish()
    }
}

impl BackupSection {
    /// The FTP destination, or `FtpNotConfigured` when no host is set.
    pub fn ftp_settings(&self) -> Result<FtpSettings, BackupError> {
        if self.ftp.host.is_empty() {
            return Err(BackupError::FtpNotConfigured);
        }
        Ok(FtpSettings {
            host: self.ftp.host.clone(),
            port: self.ftp.port,
            user: self.ftp.user.clone(),
            password: self.ftp.password.clone().unwrap_or_default(),
            folder: self.ftp.folder.clone(),
        })
    }
}

/// A read of the section: whether a password is stored, never the password.
pub fn view(s: &BackupSection) -> BackupView {
    BackupView {
        password: SecretView {
            set: s.password.is_some(),
        },
        ftp: FtpView {
            host: s.ftp.host.clone(),
            port: s.ftp.port,
            user: s.ftp.user.clone(),
            password: SecretView {
                set: s.ftp.password.is_some(),
            },
            folder: s.ftp.folder.clone(),
        },
        keep: s.keep,
        notify_failure: s.notify_failure,
    }
}

/// The section after a `PUT`, or the fixed copy saying what is wrong.
pub fn apply_input(old: &BackupSection, input: BackupInput) -> Result<BackupSection, String> {
    if matches!(&input.password, nidoo_api::Secret::Clear) && old.password.is_some() {
        return Err(NO_REMOVE.into());
    }
    if let nidoo_api::Secret::Set(p) = &input.password
        && p.chars().count() < MIN_PASSWORD_CHARS
    {
        return Err(SHORT_PASSWORD.into());
    }
    let keep = input.keep.unwrap_or(7);
    if keep == 0 {
        return Err(NO_KEEP.into());
    }
    let folder = input.ftp.folder.as_deref().map(str::trim).unwrap_or("");
    Ok(BackupSection {
        password: secret::apply(old.password.clone(), input.password),
        ftp: FtpSection {
            host: input.ftp.host.trim().to_owned(),
            port: input.ftp.port.unwrap_or(21),
            user: input.ftp.user.trim().to_owned(),
            password: secret::apply(old.ftp.password.clone(), input.ftp.password),
            folder: if folder.is_empty() { "/" } else { folder }.to_owned(),
        },
        keep,
        notify_failure: input.notify_failure,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_api::{FtpInput, Secret};

    fn with_password() -> BackupSection {
        BackupSection {
            password: Some("old-password".into()),
            ..BackupSection::default()
        }
    }

    fn input() -> BackupInput {
        BackupInput {
            notify_failure: true,
            ..BackupInput::default()
        }
    }

    #[test]
    fn apply_input_rules() {
        let old = BackupSection::default();
        let short = BackupInput {
            password: Secret::Set("short".into()),
            ..input()
        };
        assert_eq!(apply_input(&old, short).unwrap_err(), SHORT_PASSWORD);
        let clear = BackupInput {
            password: Secret::Clear,
            ..input()
        };
        assert_eq!(
            apply_input(&with_password(), clear.clone()).unwrap_err(),
            NO_REMOVE
        );
        // Nothing to remove yet: clearing is a no-op.
        assert_eq!(apply_input(&old, clear).unwrap().password, None);
        let zero = BackupInput {
            keep: Some(0),
            ..input()
        };
        assert_eq!(apply_input(&old, zero).unwrap_err(), NO_KEEP);
        // Keep keeps; Set sets.
        let kept = apply_input(&with_password(), input()).unwrap();
        assert_eq!(kept.password.as_deref(), Some("old-password"));
        let set = BackupInput {
            password: Secret::Set("long-enough".into()),
            ..input()
        };
        let set = apply_input(&old, set).unwrap();
        assert_eq!(set.password.as_deref(), Some("long-enough"));
        // Defaults.
        let d = apply_input(&old, input()).unwrap();
        assert_eq!((d.ftp.port, d.ftp.folder.as_str(), d.keep), (21, "/", 7));
        assert!(d.notify_failure && d.ftp.host.is_empty());
    }

    #[test]
    fn ftp_password_follows_the_secret_rules() {
        let mut old = BackupSection::default();
        old.ftp.password = Some("ftp-pass".into());
        let with = |password| BackupInput {
            ftp: FtpInput {
                host: "  nas.local ".into(),
                port: Some(2121),
                user: "u".into(),
                password,
                folder: Some("/backups".into()),
            },
            ..input()
        };
        let kept = apply_input(&old, with(Secret::Keep)).unwrap();
        assert_eq!(kept.ftp.password.as_deref(), Some("ftp-pass"));
        assert_eq!(kept.ftp.host, "nas.local");
        assert_eq!(
            (kept.ftp.port, kept.ftp.folder.as_str()),
            (2121, "/backups")
        );
        let set = apply_input(&old, with(Secret::Set("new".into()))).unwrap();
        assert_eq!(set.ftp.password.as_deref(), Some("new"));
        assert_eq!(
            apply_input(&old, with(Secret::Clear)).unwrap().ftp.password,
            None
        );
    }

    #[test]
    fn backup_section_defaults_from_empty_json() {
        let s: BackupSection = serde_json::from_str("{}").unwrap();
        assert_eq!(s, BackupSection::default());
        assert_eq!((s.keep, s.ftp.port, s.ftp.folder.as_str()), (7, 21, "/"));
        assert!(s.notify_failure);
        assert!(matches!(
            s.ftp_settings(),
            Err(BackupError::FtpNotConfigured)
        ));
    }

    #[test]
    fn view_and_debug_hide_the_passwords() {
        let mut s = with_password();
        s.ftp.host = "nas.local".into();
        s.ftp.password = Some("ftp-secret".into());
        let v = serde_json::to_string(&view(&s)).unwrap();
        let dbg = format!("{s:?}");
        for text in [v, dbg] {
            assert!(!text.contains("old-password") && !text.contains("ftp-secret"));
        }
        assert_eq!(s.ftp_settings().unwrap().password, "ftp-secret");
    }
}
