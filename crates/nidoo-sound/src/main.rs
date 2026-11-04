use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use nidoo_sound::clock::{Clock, MonotonicClock};
use nidoo_sound::config::{self, SoundConfig};
use nidoo_sound::discover::{self, DiscoveryEvent};
use nidoo_sound::group::{Group, sendspin_connector};
use nidoo_sound::input::{InputEvent, run_pcm_input};
use nidoo_sound::listen::accept_players;
use nidoo_sound::mqtt::run_mqtt;
use nidoo_sound::player::{Liveness, PlayerCmd, PlayerEvent, run_player};
use nidoo_sound::proto::messages::PCM_48K_STEREO_16;
use nidoo_sound::proto::noise::ServerIdentity;
use nidoo_sound::state::StateStore;
use nidoo_sound::tone;
use tokio::sync::mpsc;

/// A Sendspin-compatible server: plays one PCM input on every satellite, in sync.
#[derive(Debug, Parser)]
#[command(name = "nidoo-sound", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the sound service.
    Run {
        /// Configuration file (default: `nidoo-sound.toml` if present).
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Hardware check: play a 440 Hz tone on every player found, then change
    /// its volume, mute and unmute it, and end the stream.
    #[command(hide = true)]
    Tone {
        #[arg(long, default_value_t = 10)]
        seconds: u64,
        #[arg(long, default_value_t = 30)]
        volume: u8,
        /// How far ahead of playback each chunk is timestamped.
        #[arg(long, default_value_t = 800)]
        lead_ms: u32,
        /// Holds `sound/identity.key`.
        #[arg(long, default_value = "./data")]
        data_dir: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,nidoo_sound=debug".into()),
        )
        .init();
    let cli = Cli::parse();
    let rt = tokio::runtime::Runtime::new()?;
    match cli.command {
        Command::Run { config } => rt.block_on(run(config)),
        Command::Tone {
            seconds,
            volume,
            lead_ms,
            data_dir,
        } => rt.block_on(tone_check(seconds, volume, lead_ms, data_dir)),
    }
}

async fn run(config: Option<PathBuf>) -> anyhow::Result<()> {
    let cfg = SoundConfig::load(config.as_deref())?;
    let sound_dir = cfg.data_dir.join("sound");
    let identity = Arc::new(ServerIdentity::load_or_create(
        &sound_dir.join("identity.key"),
    )?);
    let store = StateStore::load(sound_dir.join("players.json"))?;
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::new());
    let shutdown = shutdown_listener()?;

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", cfg.input_port)).await?;
    tracing::info!(
        server_id = %identity.server_id,
        input_port = cfg.input_port,
        broker = %format!("{}:{}", cfg.mqtt.host, cfg.mqtt.port),
        broker_user = cfg.mqtt.user.as_deref().unwrap_or("(none)"),
        login_from = if cfg.broker_from_hub { "hub.json" } else { "settings" },
        "nidoo-sound started"
    );
    if cfg.mqtt.user.is_none() {
        // A broker with access rules accepts an anonymous login and then
        // drops its messages without an error: the satellites never show up
        // in Nidoo.
        tracing::warn!(
            hub_file = %cfg.data_dir.join(config::HUB_FILE).display(),
            "no broker login: if the satellites do not show up in Nidoo, check that the \
             hub's settings file is in the data folder, or set NIDOO_SOUND_MQTT__USER and \
             NIDOO_SOUND_MQTT__PASSWORD"
        );
    }
    let discovery = discover::browse()?;
    let (input_tx, input_rx) = mpsc::channel(256);
    let idle = Duration::from_millis(u64::from(cfg.idle_stop_ms));
    tokio::spawn(run_pcm_input(listener, idle, input_tx.clone()));

    let (control_tx, control_rx) = mpsc::channel(64);
    // Requests wait here while one clip is fetched: no more than the sentences the mixer queues.
    let (speak_tx, speak_rx) = mpsc::channel(nidoo_sound::mixer::MAX_QUEUED);
    let (speech_tx, speech_rx) = mpsc::channel(4);
    tracing::info!(hub = %cfg.hub_base_url(), "speech clips come from the hub");
    tokio::spawn(nidoo_sound::speech::run_speech_fetcher(
        cfg.hub_base_url(),
        speak_rx,
        speech_tx,
    ));
    let (publish_tx, publish_rx) = mpsc::channel(256);
    let (hold_tx, hold_rx) = mpsc::channel(16);
    let lead_us = i64::from(cfg.lead_ms) * 1_000;
    let group = Group::new(
        store,
        clock.clone(),
        lead_us,
        sendspin_connector(identity.clone(), clock.clone()),
    )
    .with_input_buffer(i64::from(cfg.input_buffer_ms) * 1_000);
    if cfg.listen_port != 0 {
        let players = tokio::net::TcpListener::bind(("0.0.0.0", cfg.listen_port)).await?;
        tracing::info!(
            listen_port = cfg.listen_port,
            path = nidoo_sound::listen::PATH,
            "players can connect"
        );
        tokio::spawn(accept_players(
            players,
            identity,
            clock,
            group.players(),
            Liveness::INBOUND,
        ));
    }
    let group = tokio::spawn(group.run(
        discovery, input_rx, control_rx, speech_rx, hold_rx, publish_tx,
    ));
    let mqtt = {
        let cfg = cfg.clone();
        tokio::spawn(async move {
            run_mqtt(
                &cfg.mqtt,
                &cfg.discovery_prefix,
                publish_rx,
                control_tx,
                speak_tx,
                hold_tx,
            )
            .await
        })
    };

    tokio::select! {
        _ = shutdown => tracing::info!("shutting down"),
        r = mqtt => anyhow::bail!("MQTT link ended: {r:?}"),
    }
    // End every stream, give the connections a moment to send it, then stop
    // the group: the MQTT link sees its channel close, publishes `offline`
    // and disconnects.
    input_tx.send(InputEvent::Stopped).await.ok();
    tokio::time::sleep(Duration::from_millis(300)).await;
    group.abort();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    Ok(())
}

/// Resolves on Ctrl-C or SIGTERM (`docker stop`).
fn shutdown_listener() -> std::io::Result<impl std::future::Future<Output = ()>> {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut ctrl_c = Box::pin(tokio::signal::ctrl_c());
    Ok(async move {
        tokio::select! {
            _ = &mut ctrl_c => {}
            _ = term.recv() => {}
        }
    })
}

async fn tone_check(
    seconds: u64,
    volume: u8,
    lead_ms: u32,
    data_dir: PathBuf,
) -> anyhow::Result<()> {
    let identity = Arc::new(ServerIdentity::load_or_create(
        &data_dir.join("sound/identity.key"),
    )?);
    println!("server_id {}", identity.server_id);
    let clock: Arc<dyn Clock> = Arc::new(MonotonicClock::new());
    let (ev_tx, mut events) = mpsc::channel(256);

    // Connect to every player advertised within a few seconds.
    let mut found = discover::browse()?;
    let mut players: HashMap<String, mpsc::Sender<PlayerCmd>> = HashMap::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    while let Ok(Some(ev)) = tokio::time::timeout_at(deadline, found.recv()).await {
        if let DiscoveryEvent::Up(d) = ev {
            if players.contains_key(&d.instance) {
                continue;
            }
            println!("found {} at {}", d.instance, d.url);
            let (tx, rx) = mpsc::channel(256);
            let (id, clock, ev_tx, instance) = (
                identity.clone(),
                clock.clone(),
                ev_tx.clone(),
                d.instance.clone(),
            );
            tokio::spawn(async move {
                if let Err(e) = run_player(d.url, id, clock, ev_tx, rx).await {
                    println!("{instance}: connection ended: {e:#}");
                }
            });
            // Queued until the handshake and client/hello are done.
            tx.send(PlayerCmd::Activate { playback: true }).await.ok();
            players.insert(d.instance, tx);
        }
    }
    anyhow::ensure!(!players.is_empty(), "no Sendspin player found");
    drop(ev_tx);

    // Wait for each player's first client/state.
    let mut ready = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while ready < players.len() {
        let Ok(Some(ev)) = tokio::time::timeout_at(deadline, events.recv()).await else {
            break;
        };
        println!("{ev:?}");
        if matches!(
            ev,
            PlayerEvent::State {
                available: true,
                ..
            }
        ) {
            ready += 1;
        }
    }
    let all: Vec<mpsc::Sender<PlayerCmd>> = players.values().cloned().collect();
    println!("{ready} of {} players ready", all.len());
    let send_all = |cmd: PlayerCmd| {
        let all = all.clone();
        async move {
            for tx in &all {
                tx.send(cmd.clone()).await.ok();
            }
        }
    };

    send_all(PlayerCmd::Volume(volume)).await;
    send_all(PlayerCmd::StreamStart(PCM_48K_STEREO_16)).await;
    let lead = i64::from(lead_ms) * 1_000;
    let start = clock.now_us() + lead;
    let total = seconds * 50;
    for i in 0..total {
        let ts = start + i as i64 * 20_000;
        let wait = ts - lead - clock.now_us();
        if wait > 0 {
            let mut sleep = std::pin::pin!(tokio::time::sleep(Duration::from_micros(wait as u64)));
            loop {
                tokio::select! {
                    _ = &mut sleep => break,
                    Some(ev) = events.recv() => println!("{ev:?}"),
                }
            }
        }
        if i == total * 4 / 10 {
            println!("-> volume {}", volume / 2);
            send_all(PlayerCmd::Volume(volume / 2)).await;
        } else if i == total * 6 / 10 {
            println!("-> mute");
            send_all(PlayerCmd::Mute(true)).await;
        } else if i == total * 8 / 10 {
            println!("-> unmute");
            send_all(PlayerCmd::Mute(false)).await;
        }
        let data = bytes::Bytes::from(tone::tone_chunk(440.0, i * tone::CHUNK_FRAMES, 0.5));
        send_all(PlayerCmd::Audio {
            timestamp_us: ts,
            data,
        })
        .await;
    }
    // Let the buffered tail play, then end.
    tokio::time::sleep(Duration::from_micros(lead as u64)).await;
    send_all(PlayerCmd::StreamEnd).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    send_all(PlayerCmd::Close).await;
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_secs(2), events.recv()).await {
        println!("{ev:?}");
    }
    Ok(())
}
