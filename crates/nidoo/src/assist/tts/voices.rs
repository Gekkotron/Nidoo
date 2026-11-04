//! Voices: the model's own, plus the public catalogue's voices in the model's languages.

use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use nidoo_api::search::fold;
use nidoo_api::{DEFAULT_VOICE, VoiceSource, VoiceView, VoicesView};
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

use super::TtsError;
use super::hf::HfClient;

/// The public repository that lists the catalogue voices.
pub const VOICE_CATALOG_REPO: &str = "gradium/phonon-voices";
/// The catalogue commit `ptts` pins.
pub const VOICE_CATALOG_REV: &str = "da6c9ec0d47be8df5c87c730a3aa97187f3229c3";
/// The largest voice file accepted.
pub const MAX_VOICE_BYTES: u64 = 2 * 1024 * 1024;
/// The largest `voices.jsonl` accepted.
const MAX_LIST_BYTES: u64 = 8 * 1024 * 1024;

/// One line of the catalogue's `voices.jsonl`; unknown fields are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CatalogVoice {
    /// The voice name.
    pub name: String,
    /// The file path inside the catalogue repository.
    pub filename: String,
    /// The catalogue's id for the voice.
    #[serde(default)]
    pub voice_id: Option<String>,
    /// The language code.
    #[serde(default)]
    pub language: Option<String>,
    /// The speaker's gender.
    #[serde(default)]
    pub gender: Option<String>,
    /// The speaker's age group.
    #[serde(default)]
    pub age: Option<String>,
    /// A short description.
    #[serde(default)]
    pub description: Option<String>,
}

/// The catalogue cache and voice files in one directory.
pub struct VoiceCatalog {
    dir: PathBuf,
}

/// The file stem of a catalogue `filename`, when it is exactly `voices/<stem>.safetensors`.
fn valid_stem(filename: &str) -> Option<&str> {
    let stem = filename
        .strip_prefix("voices/")?
        .strip_suffix(".safetensors")?;
    let plain = !stem.is_empty()
        && stem != "."
        && stem != ".."
        && !stem.contains(['/', '\\', '\0'])
        && !stem.contains("..");
    plain.then_some(stem)
}

fn io(e: std::io::Error) -> TtsError {
    TtsError::Io(e.to_string())
}

impl VoiceCatalog {
    /// A catalogue cached under `dir` (created on first write).
    pub fn new(dir: PathBuf) -> VoiceCatalog {
        VoiceCatalog { dir }
    }

    fn parse(text: &str) -> Vec<CatalogVoice> {
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| match serde_json::from_str::<CatalogVoice>(l) {
                Ok(v) => Some(v),
                Err(_) => {
                    tracing::warn!("skipped an unreadable voice catalogue line");
                    None
                }
            })
            .collect()
    }

    /// The catalogue voices; the cached list when present, else fetched once (no token).
    /// The flag is true when nothing could be read.
    pub async fn list(&self, client: &HfClient) -> (Vec<CatalogVoice>, bool) {
        let cache = self.dir.join("voices.jsonl");
        if let Ok(text) = tokio::fs::read_to_string(&cache).await {
            return (Self::parse(&text), false);
        }
        match self.download_list(client, &cache).await {
            Ok(text) => (Self::parse(&text), false),
            Err(e) => {
                tracing::warn!("voice catalogue unavailable: {e}");
                (vec![], true)
            }
        }
    }

    async fn download_list(&self, client: &HfClient, cache: &Path) -> Result<String, TtsError> {
        let bytes = self.stream(client, "voices.jsonl", MAX_LIST_BYTES).await?;
        let text = String::from_utf8(bytes)
            .map_err(|_| TtsError::Invalid("the voice list is not text".into()))?;
        tokio::fs::create_dir_all(&self.dir).await.map_err(io)?;
        let tmp = self.dir.join("voices.jsonl.part");
        tokio::fs::write(&tmp, &text).await.map_err(io)?;
        tokio::fs::rename(&tmp, cache).await.map_err(io)?;
        Ok(text)
    }

    async fn stream(&self, client: &HfClient, path: &str, max: u64) -> Result<Vec<u8>, TtsError> {
        let resp = client
            .without_token()
            .get(VOICE_CATALOG_REPO, VOICE_CATALOG_REV, path)
            .await?;
        let mut stream = resp.bytes_stream();
        let mut out = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk
                .map_err(|_| TtsError::Upstream("the connection to Hugging Face broke".into()))?;
            if (out.len() + chunk.len()) as u64 > max {
                return Err(TtsError::TooLarge(path.to_string()));
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }

    /// The path a catalogue voice would have on disk, when its `filename` is acceptable.
    pub fn path_of(&self, voice: &CatalogVoice) -> Result<PathBuf, TtsError> {
        let stem = valid_stem(&voice.filename)
            .ok_or_else(|| TtsError::Invalid("not a catalogue voice file".into()))?;
        Ok(self.dir.join(format!("{stem}.safetensors")))
    }

    /// Fetches a voice file (no token), or reuses the one on disk.
    pub async fn fetch(
        &self,
        client: &HfClient,
        voice: &CatalogVoice,
    ) -> Result<PathBuf, TtsError> {
        let target = self.path_of(voice)?;
        if tokio::fs::metadata(&target)
            .await
            .is_ok_and(|m| m.is_file())
        {
            return Ok(target);
        }
        tokio::fs::create_dir_all(&self.dir).await.map_err(io)?;
        let part = PathBuf::from(format!("{}.part", target.display()));
        let result = self.write_part(client, &voice.filename, &part).await;
        if let Err(e) = result {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(e);
        }
        if let Err(e) = tokio::fs::rename(&part, &target).await {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(io(e));
        }
        Ok(target)
    }

    async fn write_part(
        &self,
        client: &HfClient,
        filename: &str,
        part: &Path,
    ) -> Result<(), TtsError> {
        let resp = client
            .without_token()
            .get(VOICE_CATALOG_REPO, VOICE_CATALOG_REV, filename)
            .await?;
        let mut stream = resp.bytes_stream();
        let mut out = tokio::fs::File::create(part).await.map_err(io)?;
        let mut got = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk
                .map_err(|_| TtsError::Upstream("the connection to Hugging Face broke".into()))?;
            got += chunk.len() as u64;
            if got > MAX_VOICE_BYTES {
                return Err(TtsError::TooLarge("voice file".into()));
            }
            out.write_all(&chunk).await.map_err(io)?;
        }
        out.flush().await.map_err(io)?;
        if got == 0 {
            return Err(TtsError::Invalid("the voice file is empty".into()));
        }
        Ok(())
    }
}

/// The voices on offer: the model's, then the catalogue's in the model's languages.
pub fn view(
    model_voices: &[String],
    languages: &[String],
    catalog: &[CatalogVoice],
    on_disk: impl Fn(&CatalogVoice) -> bool,
    chosen: &str,
    offline: bool,
) -> VoicesView {
    let mut voices: Vec<VoiceView> = model_voices
        .iter()
        .map(|n| VoiceView {
            name: n.clone(),
            language: None,
            gender: None,
            age: None,
            description: None,
            source: VoiceSource::Model,
            on_disk: true,
        })
        .collect();
    let mut extra: Vec<&CatalogVoice> = catalog
        .iter()
        .filter(|v| {
            v.language
                .as_ref()
                .is_some_and(|l| languages.iter().any(|m| m == l))
        })
        .filter(|v| !model_voices.contains(&v.name))
        .collect();
    // Alphabetical as a reader expects: accents and case folded (Élise between Eve and Félix),
    // the exact name only breaking ties.
    extra.sort_by_cached_key(|v| (fold(&v.name), v.name.clone()));
    voices.extend(extra.into_iter().map(|v| VoiceView {
        name: v.name.clone(),
        language: v.language.clone(),
        gender: v.gender.clone(),
        age: v.age.clone(),
        description: v.description.clone(),
        source: VoiceSource::Catalog,
        on_disk: on_disk(v),
    }));
    let known = voices.iter().any(|v| v.name == chosen);
    VoicesView {
        voices,
        chosen: if known { chosen } else { DEFAULT_VOICE }.to_string(),
        fell_back: !known,
        catalog_offline: offline,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cv(name: &str, lang: &str) -> CatalogVoice {
        CatalogVoice {
            name: name.into(),
            filename: format!("voices/{name}.safetensors"),
            voice_id: None,
            language: Some(lang.into()),
            gender: None,
            age: None,
            description: None,
        }
    }

    #[test]
    fn view_filters_by_the_model_languages() {
        let cat = [
            cv("Zoe", "fr"),
            cv("Bob", "en"),
            cv("Anna", "fr"),
            cv("Hans", "de"),
        ];
        let v = view(
            &["default".to_string()],
            &["fr".to_string()],
            &cat,
            |_| false,
            "default",
            false,
        );
        let names: Vec<_> = v.voices.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["default", "Anna", "Zoe"]);
        assert!(!v.fell_back);
        assert_eq!(v.voices[0].source, VoiceSource::Model);
        assert!(v.voices[0].on_disk);
    }

    #[test]
    fn view_sorts_accented_names_with_their_letter() {
        let cat = [
            cv("Zoe", "fr"),
            cv("Élise", "fr"),
            cv("eve", "fr"),
            cv("Félix", "fr"),
            cv("Ambre", "fr"),
        ];
        let v = view(&[], &["fr".to_string()], &cat, |_| false, "default", false);
        let names: Vec<_> = v.voices.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["Ambre", "Élise", "eve", "Félix", "Zoe"]);
    }

    #[test]
    fn view_falls_back_to_default() {
        let v = view(
            &["default".to_string()],
            &["fr".to_string()],
            &[],
            |_| false,
            "Ghost",
            true,
        );
        assert_eq!(v.chosen, "default");
        assert!(v.fell_back);
        assert!(v.catalog_offline);
    }
}
