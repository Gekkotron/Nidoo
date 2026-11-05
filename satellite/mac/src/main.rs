//! Nidoo Satellite: a menu-bar app that makes this Mac one of the house satellites
//! (Sendspin music in sync, "Hey Jarvis" over the signed voice protocol). The logic is
//! in `crates/nidoo-satellite`; this shell adds the microphone, the speaker, the wake
//! word, the menu and the app bundle.

mod audio;
mod features;
mod icon;
mod login;
mod menu;
mod models;
mod train;
mod wake;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Context;
use nidoo_satellite::store::Store;
use nidoo_satellite::{Config, start};

/// The voice port every satellite listens on.
const VOICE_PORT: u16 = 4961;
const STOCK_THRESHOLD: f32 = 0.5;
const PERSONAL_THRESHOLD: f32 = 0.9;

fn store_dir() -> anyhow::Result<PathBuf> {
    let home = std::env::var_os("HOME").context("no HOME")?;
    Ok(PathBuf::from(home).join("Library/Application Support/Nidoo Satellite"))
}

/// The Mac's name as Sharing shows it, else its host name.
fn computer_name() -> String {
    let from = |cmd: &str, args: &[&str]| {
        Command::new(cmd)
            .args(args)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    from("scutil", &["--get", "ComputerName"])
        .or_else(|| from("hostname", &["-s"]))
        .unwrap_or_else(|| "Mac".into())
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let store_dir = store_dir()?;
    let settings = Store::open(&store_dir)?.settings().clone();
    let models = models::Models::spawn_download(store_dir.join("models"));
    let head_changed = Arc::new(AtomicBool::new(false));
    let detector = wake::Detector::new(
        models.clone(),
        store_dir.clone(),
        wake::Thresholds {
            stock: settings.wake_threshold.unwrap_or(STOCK_THRESHOLD),
            personal: settings.personal_threshold.unwrap_or(PERSONAL_THRESHOLD),
        },
        head_changed.clone(),
    );
    let trainer = train::MacTrainer::new(models.clone(), store_dir.clone(), head_changed);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let (mic_tx, mic_rx) = tokio::sync::mpsc::channel(64);
    // The audio thread needs the playout `start` makes, and `start` needs the mic
    // switch: the switch's channel is made first and its thread started right after.
    let (mic, audio_rx) = audio::AudioMic::pair();
    let cfg = Config {
        store_dir,
        name: computer_name(),
        voice_port: VOICE_PORT,
        sendspin_port: 0,
        advertise: true,
    };
    let handles = rt.block_on(start(
        cfg,
        Box::new(mic),
        mic_rx,
        Box::new(detector),
        Box::new(trainer),
    ))?;
    audio::spawn(audio_rx, handles.playout.clone(), mic_tx)?;
    menu::run(rt, handles, models)
}
