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

/// Every address the hub could answer on: the bind address, a wildcard probed on loopback.
fn probe_addrs(bind: &str) -> Result<Vec<SocketAddr>> {
    let addrs: Vec<SocketAddr> = bind
        .to_socket_addrs()
        .with_context(|| format!("cannot resolve the hub's http bind address {bind:?}"))?
        .map(|mut addr| {
            if addr.ip().is_unspecified() {
                addr.set_ip(match addr.ip() {
                    IpAddr::V4(_) => IpAddr::from([127, 0, 0, 1]),
                    IpAddr::V6(_) => IpAddr::from([0, 0, 0, 0, 0, 0, 0, 1]),
                });
            }
            addr
        })
        .collect();
    if addrs.is_empty() {
        bail!("the hub's http bind address {bind:?} resolves to nothing");
    }
    Ok(addrs)
}

fn refuse_if_hub_running(bind: &str) -> Result<()> {
    for addr in probe_addrs(bind)? {
        if std::net::TcpStream::connect_timeout(&addr, PROBE_TIMEOUT).is_ok() {
            bail!("the hub answers on {addr}; stop it first");
        }
    }
    Ok(())
}

/// True when an environment override gives the hub another database URL than `hub.json` and
/// that URL still names the old user or database: rewriting `hub.json` alone cannot move it.
fn env_pins_old_url(raw: &str, effective: &str) -> bool {
    if raw == effective {
        return false;
    }
    let Ok(url) = url::Url::parse(effective) else {
        return false;
    };
    url.username() == FROM || url.path().trim_start_matches('/') == FROM
}

/// Claims the backup path; refuses an existing file so the first backup is never replaced.
fn claim_backup(path: &Path) -> Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(|_| ())
        .with_context(|| {
            format!(
                "backup file {} exists or cannot be created; it may hold the only pre-migration state, move it away",
                path.display()
            )
        })
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
    let raw = HubFile::load(data_dir)?
        .with_context(|| format!("no hub.json in {}", data_dir.display()))?;
    // What the hub boots with: hub.json under the NIDOO_* environment overrides.
    let file = raw.clone().with_bootstrap_env()?;
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
            if apply && args.backup.is_none() {
                bail!("--apply needs --backup <file>");
            }
            // Even a dry run runs the real UPDATEs (rolled back), which contend with a live hub.
            refuse_if_hub_running(&file.http.bind)?;
            if apply
                && let Some(raw_url) = &raw.database_url
                && env_pins_old_url(raw_url, &url)
            {
                bail!(
                    "an environment override (NIDOO_HISTORY__URL or similar) points the hub at \
                     the old database; edit .env to the new name first, hub.json alone cannot move it"
                );
            }
            if let Some(path) = &args.backup {
                claim_backup(path)?;
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
                apply_all(
                    &opts,
                    &actions,
                    &url,
                    &raw,
                    data_dir,
                    args.backup.as_deref(),
                )
                .await
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
    let refused = actions
        .iter()
        .filter(|a| matches!(a, Action::Refuse { .. }))
        .count();
    if refused > 0 {
        bail!("dry run: apply would refuse {refused} topics (nothing was written)");
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
    backup: Option<&Path>,
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
        new_file.database_url = Some(db::renamed_url(
            file.database_url.as_deref().unwrap_or(url),
            FROM,
            TO,
        ));
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
        if let Some(b) = backup {
            eprintln!(
                "the retained messages are backed up in {0}; to put them back: nidoo migrate-oikos --restore {0}",
                b.display()
            );
        }
        if done.contains(&"database and role") {
            eprintln!(
                "the database is already renamed: set hub.json database_url to {}",
                db::renamed_url(url, FROM, TO)
            );
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_backup_is_refused() {
        let dir = std::env::temp_dir().join(format!("nidoo-backup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("b.jsonl");
        claim_backup(&path).unwrap();
        assert!(claim_backup(&path).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn env_url_pinning_the_old_name_is_detected() {
        let raw = "postgresql://oikos:p@h/oikos";
        assert!(!env_pins_old_url(raw, raw));
        assert!(env_pins_old_url(raw, "postgresql://oikos:p@other/oikos"));
        assert!(!env_pins_old_url(raw, "postgresql://nidoo:p@h/nidoo"));
    }

    #[test]
    fn wildcard_bind_is_probed_on_loopback() {
        let addrs = probe_addrs("0.0.0.0:8081").unwrap();
        assert_eq!(addrs, vec!["127.0.0.1:8081".parse().unwrap()]);
        assert!(probe_addrs("not an address").is_err());
    }

    #[test]
    #[allow(clippy::result_large_err)] // figment::Jail closures return figment::Error
    fn old_env_prefix_is_ignored() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("OIKOS_MQTT__HOST", "x");
            let file = HubFile::default().with_bootstrap_env()?;
            let cfg = crate::setup::sections::assemble(&file, &Default::default());
            assert_eq!(cfg.mqtt.host, crate::config::Config::default().mqtt.host);
            Ok(())
        });
    }
}
