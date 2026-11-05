//! The speaker and the microphone. One thread owns both cpal streams (they cannot
//! move between threads on macOS); the runtime switches the microphone through
//! [`AudioMic`], and the output follows the default device when it changes.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use nidoo_satellite::sendspin::clock::now_us;
use nidoo_satellite::{MicSwitch, SharedPlayout};

/// Sendspin's rate; the playout is 48 kHz stereo.
const OUT_RATE: u32 = 48_000;
const MIC_RATE: u32 = 16_000;
/// How often the default output device is checked.
const DEVICE_CHECK: Duration = Duration::from_secs(2);

pub enum Cmd {
    Mic(bool, mpsc::SyncSender<anyhow::Result<()>>),
}

/// The microphone switch the runtime holds.
pub struct AudioMic {
    cmd: mpsc::Sender<Cmd>,
}

impl AudioMic {
    /// The switch, and the receiving end for [`spawn`].
    pub fn pair() -> (AudioMic, mpsc::Receiver<Cmd>) {
        let (cmd, rx) = mpsc::channel();
        (AudioMic { cmd }, rx)
    }
}

impl MicSwitch for AudioMic {
    fn set(&mut self, on: bool) -> anyhow::Result<()> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.cmd
            .send(Cmd::Mic(on, tx))
            .context("the audio thread is gone")?;
        rx.recv_timeout(Duration::from_secs(5))
            .context("the audio thread did not answer")?
    }
}

/// Starts the audio thread; the speaker plays `playout` from now on.
pub fn spawn(
    rx: mpsc::Receiver<Cmd>,
    playout: SharedPlayout,
    mic_tx: tokio::sync::mpsc::Sender<Vec<i16>>,
) -> anyhow::Result<()> {
    std::thread::Builder::new()
        .name("audio".into())
        .spawn(move || audio_thread(&rx, &playout, &mic_tx))?;
    Ok(())
}

fn audio_thread(
    rx: &mpsc::Receiver<Cmd>,
    playout: &SharedPlayout,
    mic_tx: &tokio::sync::mpsc::Sender<Vec<i16>>,
) {
    let lost = Arc::new(AtomicBool::new(false));
    let mut output: Option<(Stream, String)> = None;
    let mut input: Option<Stream> = None;
    loop {
        let current = default_output_name();
        if lost.swap(false, Ordering::AcqRel)
            || output.as_ref().map(|(_, name)| name) != current.as_ref()
        {
            output = None;
            match open_output(playout.clone(), lost.clone()) {
                Ok(o) => {
                    tracing::info!(device = %o.1, "speaker");
                    output = Some(o);
                }
                Err(e) => tracing::warn!(error = %format!("{e:#}"), "speaker"),
            }
        }
        match rx.recv_timeout(DEVICE_CHECK) {
            Ok(Cmd::Mic(on, reply)) => {
                let result = if on {
                    open_input(mic_tx.clone()).map(|s| input = Some(s))
                } else {
                    input = None;
                    Ok(())
                };
                let _ = reply.send(result);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn default_output_name() -> Option<String> {
    cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok())
}

fn open_output(playout: SharedPlayout, lost: Arc<AtomicBool>) -> anyhow::Result<(Stream, String)> {
    let device = cpal::default_host()
        .default_output_device()
        .context("no output device")?;
    let name = device.name().unwrap_or_default();
    let channels = device.default_output_config()?.channels();
    let supported = device.supported_output_configs()?.any(|c| {
        c.channels() == channels
            && c.sample_format() == SampleFormat::F32
            && c.min_sample_rate().0 <= OUT_RATE
            && c.max_sample_rate().0 >= OUT_RATE
    });
    if !supported {
        bail!("{name} cannot play 48 kHz");
    }
    let config = StreamConfig {
        channels,
        sample_rate: cpal::SampleRate(OUT_RATE),
        buffer_size: cpal::BufferSize::Default,
    };
    let stream = device.build_output_stream(
        &config,
        move |out: &mut [f32], info: &cpal::OutputCallbackInfo| {
            let ts = info.timestamp();
            let ahead = ts.playback.duration_since(&ts.callback).unwrap_or_default();
            let at = now_us() + i64::try_from(ahead.as_micros()).unwrap_or(0);
            // Never wait in the audio callback: a busy lock is one buffer of silence.
            match playout.try_lock() {
                Ok(mut p) => p.fill(out, usize::from(channels), at),
                Err(_) => out.fill(0.0),
            }
        },
        move |e| {
            tracing::warn!(error = %e, "speaker stream");
            lost.store(true, Ordering::Release);
        },
        None,
    )?;
    stream.play()?;
    Ok((stream, name))
}

fn open_input(mic_tx: tokio::sync::mpsc::Sender<Vec<i16>>) -> anyhow::Result<Stream> {
    let device = cpal::default_host()
        .default_input_device()
        .context("no microphone")?;
    let config = device.default_input_config()?;
    if config.sample_format() != SampleFormat::F32 {
        bail!("microphone format {:?}", config.sample_format());
    }
    let channels = usize::from(config.channels());
    let mut resampler = Resampler::new(config.sample_rate().0, MIC_RATE);
    let stream = device.build_input_stream(
        &config.into(),
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            let out = resampler.push(data, channels);
            if !out.is_empty() {
                // Full means the runtime is behind: dropping is better than blocking here.
                let _ = mic_tx.try_send(out);
            }
        },
        |e| tracing::warn!(error = %e, "microphone stream"),
        None,
    )?;
    stream.play()?;
    Ok(stream)
}

/// Mono, then a box low-pass as wide as the rate ratio and linear interpolation: an
/// exact block average for 48 → 16 kHz (the spike showed it scores as well as
/// `afconvert`), and close enough for 44.1 kHz.
struct Resampler {
    ratio: f64,
    width: usize,
    window: VecDeque<f32>,
    sum: f32,
    /// Index of the next input sample, and the input position of the next output.
    n: u64,
    pos: f64,
    prev: f32,
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        let ratio = f64::from(from) / f64::from(to);
        let width = (ratio.round() as usize).max(1);
        Self {
            ratio,
            width,
            window: VecDeque::with_capacity(width + 1),
            sum: 0.0,
            n: 0,
            // The first output is the first full window.
            pos: (width - 1) as f64,
            prev: 0.0,
        }
    }

    fn push(&mut self, interleaved: &[f32], channels: usize) -> Vec<i16> {
        let mut out = Vec::with_capacity(interleaved.len() / channels.max(1) / 2 + 1);
        for frame in interleaved.chunks(channels.max(1)) {
            let x = frame.iter().sum::<f32>() / frame.len() as f32;
            self.window.push_back(x);
            self.sum += x;
            if self.window.len() > self.width {
                self.sum -= self.window.pop_front().unwrap_or(0.0);
            }
            let cur = self.sum / self.window.len() as f32;
            let n = self.n as f64;
            while self.pos <= n {
                let frac = (self.pos - (n - 1.0)) as f32;
                let y = self.prev + (cur - self.prev) * frac;
                out.push((y * 32768.0).round().clamp(-32768.0, 32767.0) as i16);
                self.pos += self.ratio;
            }
            self.prev = cur;
            self.n += 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_average_at_48k() {
        let mut r = Resampler::new(48_000, 16_000);
        let input: Vec<f32> = (0..12).map(|i| i as f32 / 32768.0).collect();
        let out = r.push(&input, 1);
        assert_eq!(out, vec![1, 4, 7, 10]);
    }

    #[test]
    fn rate_is_kept_at_44k1_and_stereo_is_mixed() {
        let mut r = Resampler::new(44_100, 16_000);
        let mut total = 0;
        for _ in 0..100 {
            let stereo: Vec<f32> = (0..882).flat_map(|_| [0.5, 0.0]).collect();
            total += r.push(&stereo, 2).len();
        }
        assert!((31_990..=32_010).contains(&total), "{total}");
        let out = r.push(&[0.5, 0.0, 0.5, 0.0, 0.5, 0.0], 2);
        assert!(out.iter().all(|&s| (8191..=8193).contains(&s)), "{out:?}");
    }
}
