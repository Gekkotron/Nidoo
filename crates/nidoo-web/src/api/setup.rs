//! The hub's setup routes (`/api/v1/setup/*`) and the settings sections the wizard edits.

use gloo_net::http::{Request, Response};
use nidoo_api::{
    ApiError, BackupSummary, FtpSource, GeoPlace, Location, MobileCode, MobileView, RemoteBackup,
    RestoreFromFtp, RestoreProgress, SetupStatus, SetupStep, TestResult,
};
use serde_json::{Value, json};

use super::core::{WebError, check, check_unless_502, encode_id, read_json, read_json_or_none};
use crate::i18n::Lang;
use crate::i18n::setup::{SetupText, setup_text};

/// Why a save did not go through.
#[derive(Debug, Clone, PartialEq)]
pub enum SaveError {
    /// 422 with a test result: the hub tried the new values and they failed.
    Failed(TestResult),
    /// 422 with the hub's own message: it refused the values without trying them.
    Refused(String),
    /// 503: the hub is restarting.
    Restarting,
    Api(ApiError),
}

impl SaveError {
    /// The text for a toast or an inline note. The hub's wording stays as received.
    pub fn message(&self, lang: Lang) -> String {
        match self {
            SaveError::Failed(r) => r.message.clone(),
            SaveError::Refused(m) => m.clone(),
            SaveError::Restarting => setup_text(lang, SetupText::Applying).to_owned(),
            SaveError::Api(e) => e.error.clone(),
        }
    }
}

/// The error for a non-2xx reply to a save, told apart by status and body.
pub fn decode_save_error(status: u16, body: &str) -> SaveError {
    if status == 503 {
        // The hub also answers 503 when the database is unreachable; only "restarting" restarts.
        return match serde_json::from_str::<ApiError>(body) {
            Ok(e) if e.error != "restarting" => SaveError::Api(e),
            _ => SaveError::Restarting,
        };
    }
    if status == 422 {
        if let Ok(result) = serde_json::from_str::<TestResult>(body) {
            return SaveError::Failed(result);
        }
        if let Ok(e) = serde_json::from_str::<ApiError>(body) {
            return SaveError::Refused(e.error);
        }
    }
    SaveError::Api(
        serde_json::from_str::<ApiError>(body).unwrap_or_else(|_| WebError::Http(status).into()),
    )
}

/// Sends a write and turns a failure into a [`SaveError`].
async fn save(request: Result<Request, gloo_net::Error>) -> Result<Response, SaveError> {
    let response = request
        .map_err(|e| SaveError::Api(WebError::Network(e.to_string()).into()))?
        .send()
        .await
        .map_err(|e| SaveError::Api(WebError::Network(e.to_string()).into()))?;
    let status = response.status();
    if status >= 400 {
        let body = response.text().await.unwrap_or_default();
        return Err(decode_save_error(status, &body));
    }
    Ok(response)
}

async fn save_status(request: Result<Request, gloo_net::Error>) -> Result<SetupStatus, SaveError> {
    let response = save(request).await?;
    read_json(response).await.map_err(SaveError::Api)
}

pub async fn get_setup() -> Result<SetupStatus, ApiError> {
    let response = check(Request::get("/api/v1/setup").build(), None).await?;
    read_json(response).await
}

/// Candidate places for `city`, optionally narrowed to a country.
pub async fn geocode(city: &str, country: Option<&str>) -> Result<Vec<GeoPlace>, ApiError> {
    let mut url = format!("/api/v1/setup/geocode?city={}", encode_id(city));
    if let Some(country) = country {
        url.push_str(&format!("&country={}", encode_id(country)));
    }
    let response = check(Request::get(&url).build(), None).await?;
    read_json(response).await
}

pub async fn put_location(loc: &Location) -> Result<SetupStatus, SaveError> {
    save_status(Request::put("/api/v1/setup/location").json(loc)).await
}

async fn test(url: &str, body: &Value) -> Result<TestResult, ApiError> {
    let response = check(Request::post(url).json(body), None).await?;
    read_json(response).await
}

pub async fn test_mqtt(body: &Value) -> Result<TestResult, ApiError> {
    test("/api/v1/setup/test/mqtt", body).await
}

pub async fn put_mqtt(body: &Value) -> Result<SetupStatus, SaveError> {
    save_status(Request::put("/api/v1/setup/mqtt").json(body)).await
}

pub async fn test_database(body: &Value) -> Result<TestResult, ApiError> {
    test("/api/v1/setup/test/database", body).await
}

pub async fn put_database(body: &Value) -> Result<SetupStatus, SaveError> {
    save_status(Request::put("/api/v1/setup/database").json(body)).await
}

/// Stores the step the browser resumes at.
pub async fn put_step(step: SetupStep) -> Result<SetupStatus, SaveError> {
    save_status(Request::put("/api/v1/setup/step").json(&json!({ "step": step }))).await
}

pub async fn finish() -> Result<SetupStatus, SaveError> {
    save_status(Request::post("/api/v1/setup/finish").build()).await
}

/// The header that carries an upload's backup password, percent-encoded, beside the file.
const PASSWORD_HEADER: &str = "x-backup-password";

/// How much of a backup file's head an upload inspect sends.
const INSPECT_BYTES: f64 = 1_048_576.0;

/// Turns the restore path on or off while setting up. Turning it off while a restore is pending
/// sends the hub back to the devices step.
pub async fn restore_pending(pending: bool) -> Result<SetupStatus, SaveError> {
    save_status(Request::put("/api/v1/setup/restore").json(&json!({ "pending": pending }))).await
}

/// Why a call that reaches the FTP server did not go through.
#[derive(Debug, Clone, PartialEq)]
pub enum FtpError {
    /// 502: the hub could not reach the FTP server.
    Unreachable,
    Api(ApiError),
}

/// Sends a call that reaches the FTP server; a 502 is [`FtpError::Unreachable`] whatever its
/// body says, so the page can word it in the language in use.
async fn check_ftp(request: Result<Request, gloo_net::Error>) -> Result<Response, FtpError> {
    match check_unless_502(request).await {
        Ok(Some(response)) => Ok(response),
        Ok(None) => Err(FtpError::Unreachable),
        Err(e) => Err(FtpError::Api(e)),
    }
}

/// The backup files on an FTP server.
pub async fn restore_ftp_list(source: &FtpSource) -> Result<Vec<RemoteBackup>, FtpError> {
    let response = check_ftp(Request::post("/api/v1/setup/restore/ftp/list").json(source)).await?;
    read_json(response).await.map_err(FtpError::Api)
}

/// What a backup file on an FTP server holds; a wrong password is refused here.
pub async fn restore_ftp_inspect(request: &RestoreFromFtp) -> Result<BackupSummary, FtpError> {
    let response =
        check_ftp(Request::post("/api/v1/setup/restore/ftp/inspect").json(request)).await?;
    read_json(response).await.map_err(FtpError::Api)
}

/// What an uploaded backup holds, read from the first megabyte of `file`.
pub async fn restore_upload_inspect(
    file: &web_sys::File,
    password: &str,
) -> Result<BackupSummary, ApiError> {
    let head = file
        .slice_with_f64_and_f64(0.0, INSPECT_BYTES)
        .map_err(|e| WebError::Network(format!("{e:?}")))?;
    let request = Request::post("/api/v1/setup/restore/upload/inspect")
        .header(PASSWORD_HEADER, &encode_id(password))
        .body(head);
    let response = check(request, None).await?;
    read_json(response).await
}

/// Starts restoring a backup from an FTP server; the hub answers 202 and
/// [`restore_progress`] follows it.
pub async fn restore_ftp(request: &RestoreFromFtp) -> Result<RestoreProgress, ApiError> {
    let response = check(
        Request::post("/api/v1/setup/restore/ftp").json(request),
        None,
    )
    .await?;
    read_json(response).await
}

/// Restores the uploaded backup `file`, sent whole as the request body. The reply comes when the
/// restore ended: its last progress, if the hub has one.
pub async fn restore_upload(
    file: &web_sys::File,
    password: &str,
) -> Result<Option<RestoreProgress>, ApiError> {
    let request = Request::put("/api/v1/setup/restore/upload")
        .header(PASSWORD_HEADER, &encode_id(password))
        .body(file);
    let response = check(request, None).await?;
    read_json_or_none(response).await
}

/// The running or last restore, `None` when there has been none.
pub async fn restore_progress() -> Result<Option<RestoreProgress>, ApiError> {
    let response = check(Request::get("/api/v1/setup/restore/progress").build(), None).await?;
    read_json_or_none(response).await
}

/// One settings section as the hub reads it (secrets as `{"set": bool}`).
pub async fn get_section(section: &str) -> Result<Value, ApiError> {
    let url = format!("/api/v1/settings/{}", encode_id(section));
    let response = check(Request::get(&url).build(), None).await?;
    read_json(response).await
}

/// Replaces a whole section. The reply (`{"restarting":true}`) is dropped: every successful
/// section save restarts the hub.
pub async fn put_section(section: &str, body: &Value) -> Result<(), SaveError> {
    let url = format!("/api/v1/settings/{}", encode_id(section));
    save(Request::put(&url).json(body)).await?;
    Ok(())
}

/// The phone app's section: the gateway address and whether the key is set.
pub async fn get_mobile() -> Result<MobileView, ApiError> {
    let response = check(Request::get("/api/v1/settings/mobile").build(), None).await?;
    read_json(response).await
}

/// Saves the phone app's section and returns the new view. An empty `gateway_key` keeps the
/// stored key.
pub async fn put_mobile(input: &Value) -> Result<MobileView, SaveError> {
    let response = save(Request::put("/api/v1/settings/mobile").json(input)).await?;
    read_json(response).await.map_err(SaveError::Api)
}

/// The text the setup QR code encodes (it holds the passwords: never log it).
pub async fn get_mobile_code() -> Result<MobileCode, ApiError> {
    let response = check(Request::get("/api/v1/setup/mobile-code").build(), None).await?;
    read_json(response).await
}

/// Tries a service's values (`solcast` or `frigate`) without saving them.
pub async fn test_service(kind: &str, body: &Value) -> Result<TestResult, ApiError> {
    test(&format!("/api/v1/setup/test/{}", encode_id(kind)), body).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_errors_decode_by_status_and_body() {
        assert!(
            matches!(decode_save_error(422, r#"{"ok":false,"message":"bad credentials","detail":null}"#),
            SaveError::Failed(r) if r.message == "bad credentials")
        );
        assert!(
            matches!(decode_save_error(422, r#"{"error":"Enter the password for the new address"}"#),
            SaveError::Refused(m) if m == "Enter the password for the new address")
        );
        assert!(matches!(
            decode_save_error(503, r#"{"error":"restarting"}"#),
            SaveError::Restarting
        ));
        assert!(matches!(
            decode_save_error(503, r#"{"error":"database unreachable"}"#),
            SaveError::Api(_)
        ));
        assert!(matches!(decode_save_error(500, "oops"), SaveError::Api(_)));
    }

    #[test]
    fn restarting_reads_applying_in_each_language() {
        assert_eq!(SaveError::Restarting.message(Lang::En), "Applying…");
        assert_eq!(SaveError::Restarting.message(Lang::Fr), "Application…");
    }
}
