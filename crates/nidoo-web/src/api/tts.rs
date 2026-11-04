//! The speech settings' REST routes: engine status, models, downloads, uploads, voices, the
//! Hugging Face token and the speech endpoint.

use gloo_net::http::Request;
use nidoo_api::{
    ActiveRequest, ApiError, DownloadRequest, DownloadView, InspectRequest, ModelView, RepoListing,
    SpeechRequest, SpeechTicket, TtsStatus, UploadSession, VoiceRequest, VoicesView,
};

use super::core::{check, check_unless_404, encode_id, read_json};

const BASE: &str = "/api/v1/assist/tts";

/// Where the engine stands, the active model, the voice and the running download.
pub async fn get_status() -> Result<TtsStatus, ApiError> {
    let response = check(Request::get(&format!("{BASE}/status")).build(), None).await?;
    read_json(response).await
}

/// The installed models.
pub async fn get_models() -> Result<Vec<ModelView>, ApiError> {
    let response = check(Request::get(&format!("{BASE}/models")).build(), None).await?;
    read_json(response).await
}

/// Removes an installed model.
pub async fn remove_model(id: &str) -> Result<(), ApiError> {
    let url = format!("{BASE}/models/{}", encode_id(id));
    check(Request::delete(&url).build(), None).await?;
    Ok(())
}

/// Makes a model the active one, or `None` to stop using a model.
pub async fn set_active(model: Option<&str>) -> Result<TtsStatus, ApiError> {
    let body = ActiveRequest {
        model: model.map(str::to_owned),
    };
    let response = check(Request::put(&format!("{BASE}/active")).json(&body), None).await?;
    read_json(response).await
}

/// Lists what a Hugging Face repository offers.
pub async fn inspect(url: &str) -> Result<RepoListing, ApiError> {
    let body = InspectRequest {
        url: url.to_owned(),
    };
    let response = check(Request::post(&format!("{BASE}/inspect")).json(&body), None).await?;
    read_json(response).await
}

/// Starts downloading one weight file of a repository.
pub async fn start_download(req: &DownloadRequest) -> Result<DownloadView, ApiError> {
    let response = check(Request::post(&format!("{BASE}/downloads")).json(req), None).await?;
    read_json(response).await
}

/// The download in progress or the last one; `None` when there has been none.
pub async fn get_download() -> Result<Option<DownloadView>, ApiError> {
    let request = Request::get(&format!("{BASE}/downloads/current")).build();
    match check_unless_404(request).await? {
        Some(response) => read_json(response).await.map(Some),
        None => Ok(None),
    }
}

/// Cancels the running download.
pub async fn cancel_download() -> Result<(), ApiError> {
    check(
        Request::delete(&format!("{BASE}/downloads/current")).build(),
        None,
    )
    .await?;
    Ok(())
}

/// Opens an upload; a new one replaces an idle one.
pub async fn upload_open() -> Result<UploadSession, ApiError> {
    let response = check(Request::post(&format!("{BASE}/uploads")).build(), None).await?;
    read_json(response).await
}

/// Sends one file of the model folder, as the raw request body.
pub async fn upload_file(id: &str, name: &str, file: &web_sys::File) -> Result<(), ApiError> {
    let url = format!("{BASE}/uploads/{}/files/{}", encode_id(id), encode_id(name));
    check(Request::put(&url).body(file), None).await?;
    Ok(())
}

/// Closes the upload; the hub checks the folder and installs the model.
pub async fn upload_finish(id: &str) -> Result<ModelView, ApiError> {
    let url = format!("{BASE}/uploads/{}/finish", encode_id(id));
    let response = check(Request::post(&url).build(), None).await?;
    read_json(response).await
}

/// Throws the upload away.
pub async fn upload_abort(id: &str) -> Result<(), ApiError> {
    let url = format!("{BASE}/uploads/{}", encode_id(id));
    check(Request::delete(&url).build(), None).await?;
    Ok(())
}

/// The voices on offer and the one in use.
pub async fn get_voices() -> Result<VoicesView, ApiError> {
    let response = check(Request::get(&format!("{BASE}/voices")).build(), None).await?;
    read_json(response).await
}

/// Chooses the voice speech uses.
pub async fn set_voice(voice: &str) -> Result<VoicesView, ApiError> {
    let body = VoiceRequest {
        voice: voice.to_owned(),
    };
    let response = check(Request::put(&format!("{BASE}/voice")).json(&body), None).await?;
    read_json(response).await
}

/// Stores the Hugging Face token, or clears it with `None`.
pub async fn set_token(token: Option<&str>) -> Result<(), ApiError> {
    // `TokenInput` only deserialises (its `Secret` never serialises), so the body is built by hand.
    let body = serde_json::json!({ "token": token });
    check(Request::put(&format!("{BASE}/token")).json(&body), None).await?;
    Ok(())
}

/// Asks the hub to speak a text; the ticket's URL streams the audio.
pub async fn speak(req: &SpeechRequest) -> Result<SpeechTicket, ApiError> {
    let response = check(Request::post("/api/v1/assist/speech").json(req), None).await?;
    read_json(response).await
}
