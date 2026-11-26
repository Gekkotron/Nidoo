//! Speech from the hub: the `nidoo-sound/speak` message, fetching the hub's
//! WAV clip and turning it into stream audio (s16le, 48 kHz, stereo).
//!
//! The hub resolves which satellites speak and makes the clip
//! (`crates/nidoo/src/sound.rs`); this side only fetches and plays it.

use std::time::Duration;

use anyhow::{Context, bail, ensure};
use bytes::Bytes;
use serde::Deserialize;

/// Where the hub asks us to speak (not retained).
pub const SPEAK_TOPIC: &str = "nidoo-sound/speak";
/// Where the hub serves its clips; a speak message naming anything else is ignored.
pub const SPEECH_PATH: &str = "/api/v1/assist/speech/";
/// A request that waited longer than this is dropped unfetched: the hub keeps a clip 60 s from
/// when it made it, and the fetch itself needs some of that time.
pub const MAX_WAIT: Duration = Duration::from_secs(45);

/// Stream rate every player gets.
const STREAM_RATE: u32 = 48_000;
/// How long a clip may take to come from the hub.
#[derive(Debug, Clone, Copy)]
pub struct FetchLimits {
    /// The longest wait for the next piece of the clip (the hub speaks it as it sends it).
    pub stall: Duration,
    /// The longest the whole clip may take: the hub keeps a clip 60 s.
    pub total: Duration,
}

/// The limits [`fetch_speech`] uses.
pub const FETCH_LIMITS: FetchLimits = FetchLimits {
    stall: Duration::from_secs(10),
    total: Duration::from_secs(60),
};
/// Longest clip we play, in seconds of audio.
const MAX_CLIP_SECONDS: usize = 60;

/// One sentence to say: the hub's clip and the satellites (short ids) that say it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SpeakRequest {
    pub id: String,
    pub url: String,
    pub players: Vec<String>,
}

/// A speak request and when it came in, waiting for the fetcher.
pub type Waiting = (tokio::time::Instant, SpeakRequest);

/// A sentence ready to play: stream-format audio for these satellites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Speech {
    /// The hub's clip id, reported back once played.
    pub id: String,
    pub players: Vec<String>,
    pub pcm: Bytes,
}

/// The request in a `nidoo-sound/speak` payload; `None` for bad JSON, no satellite, or a clip
/// address that is not one of the hub's clips.
pub fn parse_speak(payload: &[u8]) -> Option<SpeakRequest> {
    serde_json::from_slice::<SpeakRequest>(payload)
        .ok()
        .filter(|r| !r.players.is_empty() && is_clip_path(&r.url))
}

/// `/api/v1/assist/speech/<file>`, the file name a plain one (no `/`, query or `..`).
fn is_clip_path(url: &str) -> bool {
    url.strip_prefix(SPEECH_PATH).is_some_and(|file| {
        !file.is_empty()
            && !file.starts_with('.')
            && file
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    })
}

/// A RIFF/WAVE clip (PCM, 16-bit, mono or stereo, any rate) as stream audio.
pub fn wav_to_stream_pcm(wav: &[u8]) -> anyhow::Result<Bytes> {
    let (rate, channels, data) = pcm16_data(wav)?;
    let frames = data.len() / (2 * usize::from(channels));
    ensure!(
        frames <= MAX_CLIP_SECONDS * rate as usize,
        "the clip is longer than {MAX_CLIP_SECONDS} s"
    );
    // One (left, right) pair per source frame; mono goes to both sides.
    let sample = |i: usize, ch: usize| {
        let at = (i * usize::from(channels) + ch.min(usize::from(channels) - 1)) * 2;
        f32::from(i16::from_le_bytes([data[at], data[at + 1]]))
    };
    let out_frames = (frames as u64 * u64::from(STREAM_RATE)).div_ceil(u64::from(rate)) as usize;
    let mut out = Vec::with_capacity(out_frames * 4);
    for i in 0..out_frames {
        // Position in the source; linear between the two nearest frames,
        // the last frame held.
        let pos = i as f64 * f64::from(rate) / f64::from(STREAM_RATE);
        let a = (pos.floor() as usize).min(frames - 1);
        let b = (a + 1).min(frames - 1);
        let frac = (pos - pos.floor()) as f32;
        for ch in 0..2 {
            let v = sample(a, ch) + (sample(b, ch) - sample(a, ch)) * frac;
            out.extend_from_slice(&(v.round() as i16).to_le_bytes());
        }
    }
    Ok(Bytes::from(out))
}

/// `(rate, channels, sample bytes)` of a PCM 16-bit WAV. A `data` size of 0 or
/// `u32::MAX` (a header written before the length was known) reads to the end.
fn pcm16_data(wav: &[u8]) -> anyhow::Result<(u32, u16, &[u8])> {
    ensure!(
        wav.len() >= 12 && &wav[0..4] == b"RIFF" && &wav[8..12] == b"WAVE",
        "not a WAV file"
    );
    let u16_at = |at: usize| u16::from_le_bytes([wav[at], wav[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes([wav[at], wav[at + 1], wav[at + 2], wav[at + 3]]);
    let mut format = None;
    let mut at = 12;
    while at + 8 <= wav.len() {
        let id = &wav[at..at + 4];
        let size = u32_at(at + 4);
        let body = at + 8;
        match id {
            b"fmt " => {
                ensure!(body + 16 <= wav.len(), "short fmt chunk");
                format = Some((
                    u16_at(body),
                    u16_at(body + 2),
                    u32_at(body + 4),
                    u16_at(body + 14),
                ));
            }
            b"data" => {
                let Some((tag, channels, rate, bits)) = format else {
                    bail!("data before fmt");
                };
                ensure!(tag == 1, "not PCM (format {tag})");
                ensure!(bits == 16, "{bits}-bit audio, only 16-bit is played");
                ensure!((1..=2).contains(&channels), "{channels} channels");
                ensure!(rate > 0, "rate 0");
                let end = if size == 0 || size == u32::MAX {
                    wav.len()
                } else {
                    (body + size as usize).min(wav.len())
                };
                let frame = 2 * usize::from(channels);
                let data = &wav[body..end];
                let data = &data[..data.len() / frame * frame];
                ensure!(!data.is_empty(), "the clip is empty");
                return Ok((rate, channels, data));
            }
            _ => {}
        }
        // Chunks are padded to an even size.
        at = body.saturating_add(size as usize + (size as usize & 1));
    }
    bail!("no data chunk")
}

/// Fetches `req.url` from the hub at `hub_url` and turns it into a [`Speech`].
pub async fn fetch_speech(
    client: &reqwest::Client,
    hub_url: &str,
    req: &SpeakRequest,
) -> anyhow::Result<Speech> {
    fetch_speech_with(client, hub_url, req, FETCH_LIMITS).await
}

/// [`fetch_speech`] with the given limits.
pub async fn fetch_speech_with(
    client: &reqwest::Client,
    hub_url: &str,
    req: &SpeakRequest,
    limits: FetchLimits,
) -> anyhow::Result<Speech> {
    let url = format!("{}{}", hub_url.trim_end_matches('/'), req.url);
    // The clip streams while the hub speaks it: read it to the end, as long as it keeps coming.
    let read = async {
        let mut response = tokio::time::timeout(limits.stall, client.get(&url).send())
            .await
            .map_err(|_| anyhow::anyhow!("no answer within {:?}", limits.stall))?
            .and_then(reqwest::Response::error_for_status)?;
        let mut body = Vec::new();
        loop {
            let piece = tokio::time::timeout(limits.stall, response.chunk())
                .await
                .map_err(|_| anyhow::anyhow!("no data for {:?}", limits.stall))??;
            match piece {
                Some(piece) => body.extend_from_slice(&piece),
                None => return anyhow::Ok(body),
            }
        }
    };
    let body = tokio::time::timeout(limits.total, read)
        .await
        .map_err(|_| anyhow::anyhow!("longer than {:?}", limits.total))
        .and_then(|r| r)
        .with_context(|| format!("fetching {url}"))?;
    let pcm = wav_to_stream_pcm(&body).with_context(|| format!("decoding {url}"))?;
    Ok(Speech {
        id: req.id.clone(),
        players: req.players.clone(),
        pcm,
    })
}

/// Fetches each speak request in turn and hands the sentence to the group;
/// a clip that cannot be fetched, or waited past [`MAX_WAIT`], is logged and skipped.
pub async fn run_speech_fetcher(
    hub_url: String,
    mut requests: tokio::sync::mpsc::Receiver<Waiting>,
    to_group: tokio::sync::mpsc::Sender<Speech>,
    publish: tokio::sync::mpsc::Sender<crate::group::Published>,
) {
    let not_played = |id: String| crate::group::Published::Spoken { id, played: false };
    let client = reqwest::Client::new();
    while let Some((received, req)) = requests.recv().await {
        let waited = received.elapsed();
        if waited > MAX_WAIT {
            tracing::warn!(id = %req.id, waited_s = waited.as_secs(), "speech waited too long, dropped");
            publish.send(not_played(req.id)).await.ok();
            continue;
        }
        match fetch_speech(&client, &hub_url, &req).await {
            Ok(speech) => {
                if to_group.send(speech).await.is_err() {
                    return;
                }
            }
            Err(error) => {
                tracing::warn!(id = %req.id, error = format!("{error:#}"), "speech not played");
                publish.send(not_played(req.id)).await.ok();
            }
        }
    }
}

#[cfg(test)]
mod tests;
