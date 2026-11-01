//! One-off `oikos` to `nidoo` switch-over of the retained MQTT messages and
//! database rows. Throwaway: this module is deleted after the switch-over.

pub mod broker;
pub mod db;
pub mod rewrite;

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use sqlx::PgPool;

use crate::setup::HubFile;
use broker::{BrokerOpts, DEFAULT_QUIET};
use rewrite::{Action, Retained};

/// The names the switch-over moves from and to.
const FROM: &str = "oikos";
const TO: &str = "nidoo";
/// How long the "is the hub still running" probe waits.
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    DryRun,
    Apply,
    Verify,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrateArgs {
    pub mode: Mode,
    pub backup: Option<PathBuf>,
    pub restore: Option<PathBuf>,
}

/// Where the hub would answer: its bind address, with a wildcard probed on loopback.
fn probe_addr(bind: &str) -> Option<SocketAddr> {
    let mut addr = bind.to_socket_addrs().ok()?.next()?;
    if addr.ip().is_unspecified() {
        addr.set_ip(match addr.ip() {
            IpAddr::V4(_) => IpAddr::from([127, 0, 0, 1]),
            IpAddr::V6(_) => IpAddr::from([0, 0, 0, 0, 0, 0, 0, 1]),
        });
    }
    Some(addr)
}

fn refuse_if_hub_running(bind: &str) -> Result<()> {
    let Some(addr) = probe_addr(bind) else {
        return Ok(());
    };
    if std::net::TcpStream::connect_timeout(&addr, PROBE_TIMEOUT).is_ok() {
        bail!("the hub answers on {addr}; stop it before --apply");
    }
    Ok(())
}

fn text(p: &[u8]) -> String {
    String::from_utf8_lossy(p).into_owned()
}

async fn pool(url: &str) -> Result<PgPool> {
    PgPool::connect(url)
        .await
        .context("connecting to the database")
}

pub async fn run(args: MigrateArgs, data_dir: &Path) -> Result<()> {
    let file = HubFile::load(data_dir)?
        .with_context(|| format!("no hub.json in {}", data_dir.display()))?;
    let mqtt = file.mqtt.clone().context("hub.json has no mqtt section")?;
    let url = file
        .database_url
        .clone()
        .context("hub.json has no database_url")?;
    let opts = BrokerOpts {
        host: mqtt.host,
        port: mqtt.port,
        user: mqtt.user,
        password: mqtt.password,
    };

    if let Some(path) = &args.restore {
        let msgs = broker::read_backup(path)?;
        let n = broker::restore(&opts, &msgs).await?;
        println!("restored {n} retained messages from {}", path.display());
        return Ok(());
    }

    match args.mode {
        Mode::Verify => verify(&opts, &url).await,
        Mode::DryRun | Mode::Apply => {
            let apply = args.mode == Mode::Apply;
            if apply {
                if args.backup.is_none() {
                    bail!("--apply needs --backup <file>");
                }
                refuse_if_hub_running(&file.http.bind)?;
            }
            let msgs = broker::read_retained(&opts, DEFAULT_QUIET).await?;
            if let Some(path) = &args.backup {
                broker::write_backup(path, &msgs)?;
                println!(
                    "backed up {} retained messages to {}",
                    msgs.len(),
                    path.display()
                );
            }
            let actions = rewrite::plan(&msgs);
            if apply {
                apply_all(&opts, &actions, &url, &file, data_dir).await
            } else {
                dry_run(&msgs, &actions, &url).await
            }
        }
    }
}

async fn dry_run(msgs: &[Retained], actions: &[Action], url: &str) -> Result<()> {
    println!("== broker ({} retained messages read)", msgs.len());
    for a in actions {
        match a {
            Action::Move { from, to, changes } => {
                if *from != to.topic {
                    println!("move  {from} -> {}", to.topic);
                } else {
                    println!("keep  {from} (payload rewritten)");
                }
                for c in changes {
                    println!("        {}: {:?} -> {:?}", c.path, c.old, c.new);
                }
                // A text payload records no `Change`: show both sides instead.
                if changes.is_empty() {
                    let old = msgs.iter().find(|m| m.topic == *from);
                    println!(
                        "        was: {}",
                        old.map(|m| text(&m.payload)).unwrap_or_default()
                    );
                    println!("        now: {}", text(&to.payload));
                }
            }
            Action::Drop { topic, reason } => println!("drop  {topic} ({reason})"),
            Action::Refuse { topic, reason } => println!("REFUSE {topic}: {reason}"),
        }
    }
    let pool = pool(url).await?;
    println!("== database");
    for (table, rows) in db::rename_entity_ids(&pool, true).await? {
        println!("{table}: {rows} rows");
    }
    for (key, changes) in db::rename_settings(&pool, true).await? {
        println!("settings {key}");
        for c in changes {
            println!("        {}: {:?} -> {:?}", c.path, c.old, c.new);
        }
    }
    println!("dry run: nothing was written");
    Ok(())
}

async fn apply_all(
    opts: &BrokerOpts,
    actions: &[Action],
    url: &str,
    file: &HubFile,
    data_dir: &Path,
) -> Result<()> {
    let mut done: Vec<&str> = Vec::new();
    let result = async {
        let report = broker::apply(opts, actions, DEFAULT_QUIET).await?;
        println!(
            "broker: published {}, cleared {}",
            report.published, report.cleared
        );
        done.push("broker");
        let pool = pool(url).await?;
        for (table, rows) in db::rename_entity_ids(&pool, false).await? {
            println!("{table}: {rows} rows");
        }
        done.push("entity ids");
        let settings = db::rename_settings(&pool, false).await?;
        println!("settings: {} keys rewritten", settings.len());
        done.push("settings");
        pool.close().await;
        db::rename_db_and_role(url, FROM, TO).await?;
        println!("database and role renamed to {TO}");
        done.push("database and role");
        let mut new_file = file.clone();
        new_file.database_url = Some(db::renamed_url(url, FROM, TO));
        new_file.save(data_dir)?;
        println!("hub.json database_url updated");
        done.push("hub.json");
        Ok::<(), anyhow::Error>(())
    }
    .await;
    if let Err(e) = result {
        eprintln!(
            "stopped after: {}",
            if done.is_empty() {
                "nothing".to_string()
            } else {
                done.join(", ")
            }
        );
        return Err(e);
    }
    Ok(())
}

async fn verify(opts: &BrokerOpts, url: &str) -> Result<()> {
    let msgs = broker::read_retained(opts, DEFAULT_QUIET).await?;
    let mut lines = broker::verify_retained(&msgs);
    // After the switch-over `hub.json` points at the renamed database already.
    let pool = pool(url).await?;
    lines.extend(db::verify_db(&pool).await?);
    for l in &lines {
        println!("{l}");
    }
    if !lines.is_empty() {
        bail!("{} leftovers still carry the old name", lines.len());
    }
    println!("verify: nothing left with the old name");
    Ok(())
}
