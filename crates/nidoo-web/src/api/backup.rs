//! The backup routes: the backup section of the settings, and the backup job under
//! `/api/v1/backup`. A backup is downloaded with a plain link to [`DOWNLOAD_PATH`].

use gloo_net::http::Request;
use nidoo_api::{
    ApiError, BackupInput, BackupProgress, BackupView, FtpCheck, FtpInput, LastBackup, Secret,
};
use serde_json::{Map, Value, json};

use super::core::{check, read_json, read_json_or_none};

/// Where a backup file is downloaded from; the page links to it.
pub const DOWNLOAD_PATH: &str = "/api/v1/backup/download";

const SETTINGS: &str = "/api/v1/settings/backup";

/// A secret as the hub reads it: left out keeps the stored one, `null` clears it, a string sets it.
fn put_secret(out: &mut Map<String, Value>, key: &str, secret: &Secret) {
    match secret {
        Secret::Keep => {}
        Secret::Clear => {
            out.insert(key.to_owned(), Value::Null);
        }
        Secret::Set(value) => {
            out.insert(key.to_owned(), json!(value));
        }
    }
}

/// The FTP part of a `PUT`, or the body of a connection test. `FtpInput` only deserialises (its
/// `Secret` never serialises), so the body is built by hand.
fn ftp_json(ftp: &FtpInput) -> Value {
    let mut out = Map::new();
    out.insert("host".to_owned(), json!(ftp.host));
    if let Some(port) = ftp.port {
        out.insert("port".to_owned(), json!(port));
    }
    out.insert("user".to_owned(), json!(ftp.user));
    put_secret(&mut out, "password", &ftp.password);
    if let Some(folder) = &ftp.folder {
        out.insert("folder".to_owned(), json!(folder));
    }
    Value::Object(out)
}

/// The body of a `PUT` of the backup section. A left-out `keep` takes the hub's default.
fn input_json(input: &BackupInput) -> Value {
    let mut out = Map::new();
    put_secret(&mut out, "password", &input.password);
    out.insert("ftp".to_owned(), ftp_json(&input.ftp));
    if let Some(keep) = input.keep {
        out.insert("keep".to_owned(), json!(keep));
    }
    out.insert("notify_failure".to_owned(), json!(input.notify_failure));
    out.insert("include_z2m".to_owned(), json!(input.include_z2m));
    Value::Object(out)
}

/// The backup section: neither password comes back, only whether one is stored.
pub async fn get_backup_settings() -> Result<BackupView, ApiError> {
    let response = check(Request::get(SETTINGS).build(), None).await?;
    read_json(response).await
}

/// Replaces the backup section; the reply is the section as the hub now has it.
pub async fn put_backup_settings(input: &BackupInput) -> Result<BackupView, ApiError> {
    let response = check(Request::put(SETTINGS).json(&input_json(input)), None).await?;
    read_json(response).await
}

/// Tries the FTP server in `ftp`, or the stored one with `None`, step by step.
pub async fn test_ftp(ftp: Option<&FtpInput>) -> Result<Vec<FtpCheck>, ApiError> {
    let request = match ftp {
        Some(ftp) => Request::post("/api/v1/backup/test").json(&ftp_json(ftp)),
        None => Request::post("/api/v1/backup/test").build(),
    };
    let response = check(request, None).await?;
    read_json(response).await
}

/// Starts a backup to the FTP server; the hub answers 202 with the new job.
pub async fn start_backup() -> Result<BackupProgress, ApiError> {
    let response = check(Request::post("/api/v1/backup/runs").build(), None).await?;
    read_json(response).await
}

/// The running backup, `None` when there is none.
pub async fn backup_current() -> Result<Option<BackupProgress>, ApiError> {
    let response = check(Request::get("/api/v1/backup/current").build(), None).await?;
    read_json_or_none(response).await
}

/// Cancels the running backup.
pub async fn cancel_backup() -> Result<(), ApiError> {
    check(Request::delete("/api/v1/backup/current").build(), None).await?;
    Ok(())
}

/// The last backup that ended, `None` when there has been none.
pub async fn last_backup() -> Result<Option<LastBackup>, ApiError> {
    let response = check(Request::get("/api/v1/backup/last").build(), None).await?;
    read_json_or_none(response).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ftp() -> FtpInput {
        FtpInput {
            host: "nas.local".into(),
            port: Some(2121),
            user: "nidoo".into(),
            password: Secret::Keep,
            folder: Some("/backups".into()),
        }
    }

    #[test]
    fn a_kept_secret_is_left_out() {
        assert_eq!(
            ftp_json(&ftp()),
            json!({"host": "nas.local", "port": 2121, "user": "nidoo", "folder": "/backups"})
        );
    }

    #[test]
    fn a_secret_sets_or_clears() {
        let mut input = ftp();
        input.password = Secret::Set("ftp-pass".into());
        assert_eq!(ftp_json(&input)["password"], "ftp-pass");
        input.password = Secret::Clear;
        assert_eq!(ftp_json(&input)["password"], Value::Null);
        assert!(
            ftp_json(&input)
                .as_object()
                .unwrap()
                .contains_key("password")
        );
    }

    #[test]
    fn the_section_carries_both_passwords_and_the_switches() {
        let input = BackupInput {
            password: Secret::Set("backup-pass".into()),
            ftp: ftp(),
            keep: Some(3),
            notify_failure: false,
            include_z2m: false,
        };
        let body = input_json(&input);
        assert_eq!(body["password"], "backup-pass");
        assert_eq!(body["ftp"]["host"], "nas.local");
        assert_eq!(body["keep"], 3);
        assert_eq!(body["notify_failure"], false);
        assert_eq!(body["include_z2m"], false);
    }

    #[test]
    fn a_left_out_keep_and_port_stay_out() {
        let mut input = BackupInput::default();
        input.ftp.port = None;
        let body = input_json(&input);
        assert!(body.get("keep").is_none());
        assert!(body["ftp"].get("port").is_none());
        assert!(body.get("password").is_none());
    }
}
