#![forbid(unsafe_code)]

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use nidoo::cli::Cli;
use nidoo::config::{AclSource, Config, LogConfig};
use nidoo::history::History;
use nidoo::hub::Hub;
use nidoo::setup::boot::{Boot, resolve, spawn_location_lookup};
use nidoo::setup::bundled::{self, InitOutcome};
use nidoo::setup::{HubFile, SetupCtx, banner};
use nidoo::weather::open_meteo::Endpoints;
use nidoo::{http, logging, mcp, mqtt};
use tokio::sync::mpsc;

/// Publishes waiting for the MQTT task before the hub refuses new ones.
const PUBLISH_QUEUE: usize = 64;
/// Log lines waiting for the ACL watchdog.
const ACL_LINES: usize = 256;
/// How long the Disconnect packet gets to reach the wire before the task is dropped.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Feeds the ACL watchdog from the Mosquitto log file when that is its source: the tail task and
/// the task that hands its lines to the hub.
fn spawn_acl_tail(cfg: &Config, hub: &Arc<Hub>) -> Vec<tokio::task::JoinHandle<()>> {
    let acl = &cfg.watchdog.acl;
    let (true, AclSource::File, Some(path)) = (acl.enabled, acl.source, acl.log_file.clone())
    else {
        return Vec::new();
    };
    tracing::info!(path = %path.display(), "watching the broker log for ACL denials");
    let (tx, mut rx) = mpsc::channel::<String>(ACL_LINES);
    let tail = nidoo_watchdog::tail(path, tx);
    let hub = Arc::clone(hub);
    let feed = tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            hub.on_acl_line(&line);
        }
    });
    vec![tail, feed]
}

/// The log settings, read before anything else so the boot itself can log: `hub.json` when it
/// exists, else the defaults, under the `NIDOO_LOG__*` overrides.
fn early_log_config(data_dir: &Path) -> LogConfig {
    // A broken file is reported by `resolve`.
    HubFile::load(data_dir)
        .unwrap_or_default()
        .unwrap_or_default()
        .with_bootstrap_env()
        .map(|f| f.log)
        .unwrap_or_default()
}

/// Setup mode: only HTTP, serving the setup API and the web UI.
async fn run_setup(ctx: Arc<SetupCtx>) -> anyhow::Result<ExitCode> {
    let http_cfg = ctx.file.lock().await.http.clone();
    tracing::info!("setup mode: the hub is not configured yet; MQTT and the engine are off");
    let finished = ctx.file.lock().await.finished;
    let http_task = http::serve(&http_cfg, http::setup_router(ctx)).await?;
    banner::log_banner(&http_cfg.bind, finished);
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");
    http_task.abort();
    tracing::info!("stopped");
    Ok(ExitCode::SUCCESS)
}

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse_args();
    match cli {
        Cli::Run { data_dir } => {
            logging::init(&early_log_config(&data_dir));
            let (cfg, setup_ctx) = match resolve(&data_dir).await? {
                Boot::Setup(ctx) => return run_setup(ctx).await,
                Boot::Run { cfg, ctx } => (*cfg, ctx),
            };
            cfg.validate_all()
                .map_err(|e| anyhow::anyhow!("invalid config: {e}"))?;
            // Nothing leaves the LAN while weather is off.
            if cfg.weather.enabled && setup_ctx.file.lock().await.pending_city.is_some() {
                spawn_location_lookup(Arc::clone(&setup_ctx), Endpoints::default());
            }
            tracing::info!(host = %cfg.mqtt.host, port = cfg.mqtt.port, "starting nidoo");

            let (publish_tx, publish_rx) = mpsc::channel(PUBLISH_QUEUE);
            let hub = Hub::new(&cfg, publish_tx);
            hub.set_setup(Arc::clone(&setup_ctx));
            if cfg.history.enabled() {
                match History::start(&cfg.history, hub.zone()) {
                    Ok(history) => {
                        hub.set_history(history);
                        tokio::spawn(nidoo::history::restore::run(Arc::clone(&hub)));
                    }
                    Err(e) => tracing::warn!(
                        error = %e,
                        "history disabled: lazy pool could not be built; the house keeps running"
                    ),
                }
            }
            let engine = hub.start_engine();
            let climate = hub.start_climate();
            let alarm = hub.start_alarm();
            let network = hub.start_network();
            let push_retry = hub.push().spawn_retry_worker();
            let acl_tail = spawn_acl_tail(&cfg, &hub);
            let (link, mut link_task) = mqtt::start(
                cfg.mqtt.clone(),
                cfg.topics.clone(),
                Arc::clone(&hub),
                publish_rx,
            );
            let weather = hub.start_weather();
            let solar = hub.start_solar();
            tokio::spawn(nidoo::groups::migrate::spawn(Arc::clone(&hub)));
            let finished = setup_ctx.file.lock().await.finished;
            let http_task =
                http::serve(&cfg.http, http::router(Arc::clone(&hub), setup_ctx)).await?;
            banner::log_banner(&cfg.http.bind, finished);

            tokio::signal::ctrl_c().await?;
            tracing::info!("shutting down");
            if link.disconnect() {
                if tokio::time::timeout(SHUTDOWN_GRACE, &mut link_task)
                    .await
                    .is_err()
                {
                    tracing::warn!("the broker did not take the disconnect in time");
                    link_task.abort();
                }
            } else {
                link_task.abort();
            }
            engine.abort();
            if let Some(task) = &climate {
                task.abort();
            }
            alarm.abort();
            if let Some(task) = &network {
                task.abort();
            }
            if let Some(task) = &weather {
                task.abort();
            }
            if let Some(task) = &solar {
                task.abort();
            }
            push_retry.abort();
            for task in &acl_tail {
                task.abort();
            }
            http_task.abort();
            tracing::info!("stopped");
            Ok(ExitCode::SUCCESS)
        }
        Cli::InitBundled { data_dir } => {
            logging::init(&LogConfig::default());
            match bundled::init_bundled(&data_dir)? {
                InitOutcome::Created => println!(
                    "bundled credentials created in {}",
                    data_dir.join(bundled::BUNDLED_FILE).display()
                ),
                InitOutcome::Kept => println!("bundled credentials already exist; nothing changed"),
                InitOutcome::HubConfigured => {
                    println!("this hub is already set up; no bundled credentials created")
                }
                InitOutcome::DatabaseWithoutCredentials => {
                    eprintln!(
                        "data/timescaledb already holds a database but data/bundled.json is gone; \
                         its password may still be in data/bundled/db-password; restore \
                         bundled.json or delete data/timescaledb to start over"
                    );
                    return Ok(ExitCode::from(1));
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Cli::McpStdio { server, data_dir } => {
            let (log, load_err) = match HubFile::load(&data_dir) {
                Ok(file) => match file.unwrap_or_default().with_bootstrap_env() {
                    Ok(file) => (file.log, None),
                    Err(e) => (LogConfig::default(), Some(e.to_string())),
                },
                Err(e) => (LogConfig::default(), Some(e.to_string())),
            };
            logging::init(&log);
            if let Some(e) = load_err {
                tracing::warn!(error = %e, "ignoring invalid hub.json, using defaults for logging");
            }
            mcp::run_stdio_proxy(server).await?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
