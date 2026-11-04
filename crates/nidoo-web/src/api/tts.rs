//! The speech settings' REST routes: status, languages, models, downloads, uploads, voices, the
//! Hugging Face token and the speech endpoint.

use gloo_net::http::Request;
use nidoo_api::{
    ApiError, AvailableLanguage, DownloadRequest, DownloadView, HouseRequest, InspectRequest,
    LanguageRequest, ModelView, RepoListing, SoundSpeakReply, SoundSpeakRequest, SpeechRequest,
    SpeechTicket, TtsStatus, UploadSession, VoicesView,
};

use super::core::{check, check_unless_404, encode_id, read_json};

const BASE: &str = "/api/v1/assist/tts";

/// The languages spoken (each with its model, voice and engine), the token and the download.
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

/// The languages the installed models speak that the hub does not speak yet.
pub async fn get_available_languages() -> Result<Vec<AvailableLanguage>, ApiError> {
    let url = format!("{BASE}/languages/available");
    let response = check(Request::get(&url).build(), None).await?;
    read_json(response).await
}

/// Adds the language `code`, or changes its model or voice.
pub async fn set_language(code: &str, req: &LanguageRequest) -> Result<TtsStatus, ApiError> {
    let url = format!("{BASE}/languages/{}", encode_id(code));
    let response = check(Request::put(&url).json(req), None).await?;
    read_json(response).await
}

/// Stops speaking the language `code`.
pub async fn remove_language(code: &str) -> Result<TtsStatus, ApiError> {
    let url = format!("{BASE}/languages/{}", encode_id(code));
    let response = check(Request::delete(&url).build(), None).await?;
    read_json(response).await
}

/// Makes `code` the house language, the one speech uses when no language is asked.
pub async fn set_house(code: &str) -> Result<TtsStatus, ApiError> {
    let body = HouseRequest {
        code: code.to_owned(),
    };
    let url = format!("{BASE}/languages/house");
    let response = check(Request::put(&url).json(&body), None).await?;
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

/// The voices on offer for the language `code` and the one it uses.
pub async fn get_voices(language: &str) -> Result<VoicesView, ApiError> {
    let url = format!("{BASE}/voices?language={}", encode_id(language));
    let response = check(Request::get(&url).build(), None).await?;
    read_json(response).await
}

/// Stores the Hugging Face token, or clears it with `None`.
pub async fn set_token(token: Option<&str>) -> Result<(), ApiError> {
    // `TokenInput` only deserialises (its `Secret` never serialises), so the body is built by hand.
    let body = serde_json::json!({ "token": token });
    check(Request::put(&format!("{BASE}/token")).json(&body), None).await?;
    Ok(())
}

/// Says a sentence on the speaker satellites of a room, of every room or of one satellite.
pub async fn sound_speak(req: &SoundSpeakRequest) -> Result<SoundSpeakReply, ApiError> {
    let response = check(Request::post("/api/v1/sound/speak").json(req), None).await?;
    read_json(response).await
}

/// Asks the hub to speak a text; the ticket's URL streams the audio.
pub async fn speak(req: &SpeechRequest) -> Result<SpeechTicket, ApiError> {
    let response = check(Request::post("/api/v1/assist/speech").json(req), None).await?;
    read_json(response).await
}
