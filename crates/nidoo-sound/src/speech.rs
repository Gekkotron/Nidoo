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

/// A sentence ready to play: stream-format audio for these satellites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Speech {
    pub players: Vec<String>,
    pub pcm: Bytes,
}

/// The request in a `nidoo-sound/speak` payload; `None` for bad JSON or no satellite.
pub fn parse_speak(payload: &[u8]) -> Option<SpeakRequest> {
    serde_json::from_slice::<SpeakRequest>(payload)
        .ok()
        .filter(|r| !r.players.is_empty())
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
        players: req.players.clone(),
        pcm,
    })
}

/// Fetches each speak request in turn and hands the sentence to the group;
/// a clip that cannot be fetched is logged and skipped.
pub async fn run_speech_fetcher(
    hub_url: String,
    mut requests: tokio::sync::mpsc::Receiver<SpeakRequest>,
    to_group: tokio::sync::mpsc::Sender<Speech>,
) {
    let client = reqwest::Client::new();
    while let Some(req) = requests.recv().await {
        match fetch_speech(&client, &hub_url, &req).await {
            Ok(speech) => {
                if to_group.send(speech).await.is_err() {
                    return;
                }
            }
            Err(error) => {
                tracing::warn!(id = %req.id, error = format!("{error:#}"), "speech not played")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A WAV file as the hub writes it (`data` size given, or the streamed marker).
    pub(crate) fn wav_with_size(
        rate: u32,
        channels: u16,
        samples: &[i16],
        data_size: u32,
    ) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + samples.len() as u32 * 2).to_le_bytes());
        v.extend_from_slice(b"WAVE");
        v.extend_from_slice(b"fmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
        v.extend_from_slice(&(channels * 2).to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data_size.to_le_bytes());
        for s in samples {
            v.extend_from_slice(&s.to_le_bytes());
        }
        v
    }

    pub(crate) fn wav(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
        wav_with_size(rate, channels, samples, samples.len() as u32 * 2)
    }

    fn frames(pcm: &[u8]) -> Vec<(i16, i16)> {
        pcm.as_chunks::<4>()
            .0
            .iter()
            .map(|f| {
                (
                    i16::from_le_bytes([f[0], f[1]]),
                    i16::from_le_bytes([f[2], f[3]]),
                )
            })
            .collect()
    }

    #[test]
    fn parse_speak_reads_the_hub_message() {
        let msg = br#"{"id":"a","url":"/api/v1/assist/speech/a.wav","players":["afd56455c914"]}"#;
        assert_eq!(
            parse_speak(msg),
            Some(SpeakRequest {
                id: "a".into(),
                url: "/api/v1/assist/speech/a.wav".into(),
                players: vec!["afd56455c914".into()],
            })
        );
        assert_eq!(parse_speak(br#"{"id":"a","url":"/x","players":[]}"#), None);
        assert_eq!(parse_speak(b"nope"), None);
    }

    #[test]
    fn mono_24k_is_doubled_and_duplicated() {
        let pcm = wav_to_stream_pcm(&wav(24_000, 1, &[0, 1000])).unwrap();
        assert_eq!(
            frames(&pcm),
            [(0, 0), (500, 500), (1000, 1000), (1000, 1000)]
        );
    }

    #[test]
    fn stereo_48k_passes_through() {
        let pcm = wav_to_stream_pcm(&wav(48_000, 2, &[1, 2, 3, 4])).unwrap();
        assert_eq!(frames(&pcm), [(1, 2), (3, 4)]);
    }

    #[test]
    fn streamed_header_reads_to_the_end() {
        let pcm = wav_to_stream_pcm(&wav_with_size(48_000, 1, &[5, 6, 7, 8], u32::MAX)).unwrap();
        assert_eq!(frames(&pcm), [(5, 5), (6, 6), (7, 7), (8, 8)]);
    }

    #[test]
    fn rejects_non_pcm_and_8_bit() {
        let mut float = wav(48_000, 1, &[0, 0]);
        float[20] = 3; // format tag: IEEE float
        assert!(wav_to_stream_pcm(&float).is_err());
        let mut eight = wav(48_000, 1, &[0, 0]);
        eight[34] = 8; // bits per sample
        assert!(wav_to_stream_pcm(&eight).is_err());
        assert!(wav_to_stream_pcm(b"not a wav").is_err());
    }

    #[tokio::test]
    async fn fetch_reads_a_clip_from_the_hub() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = wav(48_000, 2, &[9, 10]);
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let n = sock.read(&mut buf).await.unwrap();
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            assert!(
                head.starts_with("GET /api/v1/assist/speech/a.wav "),
                "{head}"
            );
            let reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: audio/wav\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            sock.write_all(reply.as_bytes()).await.unwrap();
            sock.write_all(&body).await.unwrap();
        });
        let req = SpeakRequest {
            id: "a".into(),
            url: "/api/v1/assist/speech/a.wav".into(),
            players: vec!["p1".into()],
        };
        let speech = fetch_speech(&reqwest::Client::new(), &format!("http://{addr}"), &req)
            .await
            .unwrap();
        assert_eq!(speech.players, ["p1"]);
        assert_eq!(frames(&speech.pcm), [(9, 10)]);
    }

    /// A hub that sends the clip's header, then `pieces` pieces of it `gap` apart.
    async fn slow_hub(pieces: usize, gap: Duration) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let samples: Vec<i16> = (0..2 * pieces as i16).collect();
        let body = wav_with_size(48_000, 2, &samples, u32::MAX);
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await.unwrap();
            let head = "HTTP/1.1 200 OK\r\ncontent-type: audio/wav\r\nconnection: close\r\n\r\n";
            sock.write_all(head.as_bytes()).await.unwrap();
            sock.write_all(&body[..44]).await.unwrap();
            for piece in body[44..].chunks(4) {
                tokio::time::sleep(gap).await;
                if sock.write_all(piece).await.is_err() {
                    return;
                }
            }
        });
        addr
    }

    fn clip_request() -> SpeakRequest {
        SpeakRequest {
            id: "a".into(),
            url: "/api/v1/assist/speech/a.wav".into(),
            players: vec!["p1".into()],
        }
    }

    #[tokio::test]
    async fn a_clip_that_keeps_coming_is_read_past_the_stall_limit() {
        // Six pieces 100 ms apart: 600 ms in all, never 300 ms without data.
        let addr = slow_hub(6, Duration::from_millis(100)).await;
        let limits = FetchLimits {
            stall: Duration::from_millis(300),
            total: Duration::from_secs(5),
        };
        let speech = fetch_speech_with(
            &reqwest::Client::new(),
            &format!("http://{addr}"),
            &clip_request(),
            limits,
        )
        .await
        .unwrap();
        assert_eq!(frames(&speech.pcm).len(), 6);
    }

    #[tokio::test]
    async fn a_stalled_clip_fails_at_the_stall_limit() {
        let addr = slow_hub(2, Duration::from_secs(2)).await;
        let limits = FetchLimits {
            stall: Duration::from_millis(200),
            total: Duration::from_secs(5),
        };
        let started = std::time::Instant::now();
        let got = fetch_speech_with(
            &reqwest::Client::new(),
            &format!("http://{addr}"),
            &clip_request(),
            limits,
        )
        .await;
        assert!(got.is_err());
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn a_clip_slower_than_the_total_limit_fails() {
        let addr = slow_hub(20, Duration::from_millis(100)).await;
        let limits = FetchLimits {
            stall: Duration::from_millis(300),
            total: Duration::from_millis(500),
        };
        let got = fetch_speech_with(
            &reqwest::Client::new(),
            &format!("http://{addr}"),
            &clip_request(),
            limits,
        )
        .await;
        assert!(got.is_err());
    }
}
