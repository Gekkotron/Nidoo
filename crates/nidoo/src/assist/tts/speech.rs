//! The speech address: text becomes a clip that streams out as a WAV while it is made.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use bytes::Bytes;
use tokio::sync::Notify;

/// The longest text one speech may hold, in characters.
pub const MAX_SPEECH_CHARS: usize = 500;
/// How long a clip can be fetched, in seconds.
pub const CLIP_TTL_S: u64 = 60;

/// A 44-byte PCM, mono, 16-bit WAV header; `None` marks a length not known yet.
pub fn wav_header(sample_rate: u32, data_len: Option<u32>) -> [u8; 44] {
    let (riff, data) = match data_len {
        Some(n) => (n.saturating_add(36), n),
        None => (u32::MAX, u32::MAX),
    };
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&riff.to_le_bytes());
    h[8..16].copy_from_slice(b"WAVEfmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    h[22..24].copy_from_slice(&1u16.to_le_bytes()); // mono
    h[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    h[28..32].copy_from_slice(&sample_rate.saturating_mul(2).to_le_bytes());
    h[32..34].copy_from_slice(&2u16.to_le_bytes()); // block align
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data.to_le_bytes());
    h
}

/// Appends samples as little-endian 16-bit PCM, clamped to [-1, 1].
pub fn to_pcm16(samples: &[f32], out: &mut Vec<u8>) {
    out.reserve(samples.len() * 2);
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
}

#[derive(Default)]
struct ClipState {
    pcm: Vec<u8>,
    done: bool,
    error: Option<String>,
}

/// One speech: its audio so far, whether it is finished, and when it was asked for.
pub struct Clip {
    state: Mutex<ClipState>,
    notify: Notify,
    created: tokio::time::Instant,
}

impl Clip {
    fn new() -> Clip {
        Clip {
            state: Mutex::new(ClipState::default()),
            notify: Notify::new(),
            created: tokio::time::Instant::now(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ClipState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Adds PCM bytes and wakes the readers.
    pub fn push(&self, pcm: &[u8]) {
        self.lock().pcm.extend_from_slice(pcm);
        self.notify.notify_waiters();
    }

    /// Marks the clip finished, with the engine's error if it failed.
    pub fn finish(&self, error: Option<String>) {
        {
            let mut st = self.lock();
            st.done = true;
            st.error = error;
        }
        self.notify.notify_waiters();
    }

    /// True once the clip is older than [`CLIP_TTL_S`].
    pub fn expired(&self) -> bool {
        self.created.elapsed() >= Duration::from_secs(CLIP_TTL_S)
    }

    /// The WAV body: header, then every byte from the start, following new chunks.
    ///
    /// The header carries exact sizes when the clip is already finished, else an
    /// unknown length. An engine error mid-way ends the body with a failure.
    pub fn body(self: Arc<Self>, sample_rate: u32) -> Body {
        let known = {
            let st = self.lock();
            st.done
                .then(|| u32::try_from(st.pcm.len()).unwrap_or(u32::MAX))
        };
        let header = Bytes::copy_from_slice(&wav_header(sample_rate, known));
        let stream = futures_util::stream::unfold(
            (self, Some(header), 0usize, false),
            |(clip, header, pos, over)| async move {
                if over {
                    return None;
                }
                if let Some(h) = header {
                    return Some((Ok(h), (clip, None, pos, false)));
                }
                match next_step(&clip, pos).await {
                    Step::Data(b) => {
                        let next = pos + b.len();
                        Some((Ok(b), (clip, None, next, false)))
                    }
                    Step::Failed(e) => Some((Err(e), (clip, None, pos, true))),
                    Step::End => None,
                }
            },
        );
        Body::from_stream(stream)
    }
}

enum Step {
    Data(Bytes),
    Failed(std::io::Error),
    End,
}

/// Waits until the clip has bytes past `pos`, failed, or finished.
async fn next_step(clip: &Clip, pos: usize) -> Step {
    loop {
        // Register for wake-ups before looking, so none is lost.
        let notified = clip.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        {
            let st = clip.lock();
            if st.pcm.len() > pos {
                return Step::Data(Bytes::copy_from_slice(&st.pcm[pos..]));
            }
            if let Some(e) = &st.error {
                return Step::Failed(std::io::Error::other(e.clone()));
            }
            if st.done {
                return Step::End;
            }
        }
        notified.await;
    }
}

/// The clips in memory.
#[derive(Default)]
pub struct Clips {
    map: Mutex<HashMap<String, Arc<Clip>>>,
}

impl Clips {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Clip>>> {
        self.map.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A new empty clip under a fresh unguessable id; clips past their life are dropped.
    pub fn create(&self) -> (String, Arc<Clip>) {
        let mut raw = [0u8; 16];
        getrandom::getrandom(&mut raw).expect("the system has a random source");
        let id: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        let clip = Arc::new(Clip::new());
        let mut map = self.lock();
        map.retain(|_, c| !c.expired());
        map.insert(id.clone(), Arc::clone(&clip));
        (id, clip)
    }

    /// A clip that is still alive.
    pub fn get(&self, id: &str) -> Option<Arc<Clip>> {
        self.lock().get(id).filter(|c| !c.expired()).cloned()
    }

    /// Forgets a clip.
    pub fn remove(&self, id: &str) {
        self.lock().remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_with_unknown_length() {
        let h = wav_header(24_000, None);
        assert_eq!(&h[0..4], b"RIFF");
        assert_eq!(&h[4..8], &[0xFF; 4]);
        assert_eq!(&h[8..16], b"WAVEfmt ");
        assert_eq!(&h[24..28], &24_000u32.to_le_bytes());
        assert_eq!(&h[34..36], &16u16.to_le_bytes());
        assert_eq!(&h[36..40], b"data");
        assert_eq!(&h[40..44], &[0xFF; 4]);
    }

    #[test]
    fn header_with_known_length() {
        let h = wav_header(24_000, Some(4_800));
        assert_eq!(&h[4..8], &4_836u32.to_le_bytes());
        assert_eq!(&h[40..44], &4_800u32.to_le_bytes());
        assert_eq!(&h[22..24], &1u16.to_le_bytes());
        assert_eq!(&h[28..32], &48_000u32.to_le_bytes());
        assert_eq!(&h[32..34], &2u16.to_le_bytes());
    }

    #[test]
    fn pcm16_clamps() {
        let mut out = Vec::new();
        to_pcm16(&[2.0, -2.0, 0.0], &mut out);
        assert_eq!(out, [0xFF, 0x7F, 0x01, 0x80, 0x00, 0x00]);
    }
}
