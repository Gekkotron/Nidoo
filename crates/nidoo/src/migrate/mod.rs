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

use crate::config::mask_url_password;
use crate::setup::HubFile;
use crate::setup::fs::write_private_atomic;
use crate::setup::store::CACHE_FILE;
use broker::{BrokerOpts, DEFAULT_QUIET};
use rewrite::{Action, Change, Retained, shown};

/// The names the switch-over moves from and to.
const FROM: &str = "oikos";
const TO: &str = "nidoo";

/// The subcommand name: `nidoo migrate-oikos`. It lives here because only this module may name
/// the old brand.
pub const COMMAND: &str = "migrate-oikos";
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

/// Stands in for `hub.json`'s database URL to tell whether the environment replaces it.
const URL_PROBE: &str = "postgresql://probe@probe.invalid/probe";

/// True when an environment variable (`NIDOO_HISTORY__URL`) sets the hub's database URL. The
/// command renames the database and then rewrites `hub.json`, so such an override would keep
/// pointing the hub (and the command itself) at another URL than the one it switched over.
#[allow(clippy::result_large_err)] // figment::Error, as `with_bootstrap_env` returns it
fn env_sets_database_url(raw: &HubFile) -> Result<bool, figment::Error> {
    let mut probe = raw.clone();
    probe.database_url = Some(URL_PROBE.to_string());
    Ok(probe.with_bootstrap_env()?.database_url.as_deref() != Some(URL_PROBE))
}

/// The old default `mqtt.client_id`, saved into `hub.json` by the old setup.
const OLD_CLIENT_ID: &str = "oikos-os";

/// `hub.json` with the old names renamed, and the changes. Every value goes through
/// [`rewrite::rename_json`] except the broker's address and login, which this switch-over does
/// not move, and the database URL, whose user and database name are swapped field by field
/// (its password and host are kept; the change shows it with the password masked). The old
/// default client id becomes the new default.
fn rename_hub_file(file: &HubFile) -> Result<(HubFile, Vec<Change>)> {
    const KEPT: [&str; 4] = [
        "/database_url",
        "/mqtt/host",
        "/mqtt/user",
        "/mqtt/password",
    ];
    let mut v = serde_json::to_value(file).context("hub.json")?;
    let kept: Vec<(&str, serde_json::Value)> = KEPT
        .iter()
        .filter_map(|p| v.pointer_mut(p).map(|slot| (*p, slot.take())))
        .collect();
    let (mut v, mut changes) = rewrite::rename_json(v);
    for (p, value) in kept {
        if let Some(slot) = v.pointer_mut(p) {
            *slot = value;
        }
    }
    let mut new: HubFile = serde_json::from_value(v).context("renamed hub.json")?;
    if let (Some(old), Some(mqtt)) = (&file.mqtt, &mut new.mqtt)
        && old.client_id == OLD_CLIENT_ID
    {
        mqtt.client_id = crate::config::MqttConfig::default().client_id;
        if let Some(c) = changes.iter_mut().find(|c| c.path == "/mqtt/client_id") {
            c.new = mqtt.client_id.clone();
        }
    }
    if let Some(old) = &file.database_url {
        let renamed = db::renamed_url(old, FROM, TO);
        if renamed != *old {
            changes.push(Change {
                path: "/database_url".into(),
                old: mask_url_password(old),
                new: mask_url_password(&renamed),
            });
            new.database_url = Some(renamed);
        }
    }
    Ok((new, changes))
}

fn cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join(CACHE_FILE)
}

/// `settings-cache.json` (the settings copy the hub boots from when the database is slow)
/// with the old names renamed, and the changes; `None` when there is no cache.
fn rename_cache(data_dir: &Path) -> Result<Option<(serde_json::Value, Vec<Change>)>> {
    let path = cache_path(data_dir);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let v: serde_json::Value = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "{} is not valid JSON; move it away (the hub writes it again from the database)",
            path.display()
        )
    })?;
    Ok(Some(rewrite::rename_json(v)))
}

/// Rewrites `settings-cache.json` when it names the old prefixes (mode 0600, atomic); returns
/// how many values changed. A cache with nothing to rename keeps its bytes.
fn rewrite_cache(data_dir: &Path) -> Result<usize> {
    let Some((v, changes)) = rename_cache(data_dir)? else {
        return Ok(0);
    };
    if !changes.is_empty() {
        let json = serde_json::to_vec_pretty(&v).context("settings cache")?;
        write_private_atomic(&cache_path(data_dir), &json)
            .with_context(|| format!("writing {}", cache_path(data_dir).display()))?;
    }
    Ok(changes.len())
}

/// A leftover line when `settings-cache.json` still says `oikos` in any casing. Names the file
/// only, never its content (it holds secrets).
fn verify_cache(data_dir: &Path) -> Result<Vec<String>> {
    let path = cache_path(data_dir);
    match std::fs::read(&path) {
        Ok(b) if b.to_ascii_lowercase().windows(5).any(|w| w == b"oikos") => {
            Ok(vec![format!("{CACHE_FILE}: still names oikos")])
        }
        Ok(_) => Ok(Vec::new()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn print_changes(changes: &[Change]) {
    for c in changes {
        println!("        {}", rewrite::change_line(c));
    }
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
        println!(
            "restored {n} of the {} retained messages in {} (the old and new names' topics only)",
            msgs.len(),
            path.display()
        );
        return Ok(());
    }

    match args.mode {
        Mode::Verify => verify(&opts, &url, data_dir).await,
        Mode::DryRun | Mode::Apply => {
            let apply = args.mode == Mode::Apply;
            if apply && args.backup.is_none() {
                bail!("--apply needs --backup <file>");
            }
            // Even a dry run runs the real UPDATEs (rolled back), which contend with a live hub.
            refuse_if_hub_running(&file.http.bind)?;
            if env_sets_database_url(&raw)? {
                bail!(
                    "an environment variable (NIDOO_HISTORY__URL, in .env or the compose file) sets \
                     the hub's database URL; remove it from the environment for this run, so the \
                     command acts on the database_url in hub.json, then put it back afterwards \
                     with the new user and database name ({TO})"
                );
            }
            // Before any write, broker included: a wrong URL fails here, with nothing changed.
            let pool = pool(&url).await?;
            if let Some(path) = &args.backup {
                claim_backup(path)?;
            }
            let msgs = broker::read_retained(&opts, DEFAULT_QUIET).await?;
            broker::require_retained(&msgs)?;
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
                    pool,
                    &url,
                    &raw,
                    data_dir,
                    args.backup.as_deref(),
                )
                .await
            } else {
                dry_run(&msgs, &actions, &pool, &raw, data_dir).await
            }
        }
    }
}

async fn dry_run(
    msgs: &[Retained],
    actions: &[Action],
    pool: &PgPool,
    file: &HubFile,
    data_dir: &Path,
) -> Result<()> {
    println!("== broker ({} retained messages read)", msgs.len());
    for a in actions {
        match a {
            Action::Move { from, to, changes } => {
                let overwrites = if rewrite::overwrites_existing(msgs, from, to) {
                    " (overwrites existing)"
                } else {
                    ""
                };
                if *from != to.topic {
                    println!("move  {from} -> {}{overwrites}", to.topic);
                } else {
                    println!("keep  {from} (payload rewritten)");
                }
                print_changes(changes);
                // A text payload records no `Change`: show both sides instead.
                if changes.is_empty() {
                    let old = msgs.iter().find(|m| m.topic == *from);
                    println!(
                        "        was: {}",
                        shown(&old.map(|m| text(&m.payload)).unwrap_or_default())
                    );
                    println!("        now: {}", shown(&text(&to.payload)));
                }
            }
            Action::Drop { topic, reason } => println!("drop  {topic} ({reason})"),
            Action::Refuse { topic, reason } => println!("REFUSE {topic}: {reason}"),
        }
    }
    println!("== database");
    for (table, rows) in db::rename_entity_ids(pool, true).await? {
        println!("{table}: {rows} rows");
    }
    for (key, changes) in db::rename_settings(pool, true).await? {
        println!("settings {key}");
        print_changes(&changes);
    }
    println!("== files");
    let (_, changes) = rename_hub_file(file)?;
    println!("hub.json: {} values", changes.len());
    print_changes(&changes);
    match rename_cache(data_dir)? {
        None => println!("{CACHE_FILE}: none"),
        Some((_, changes)) => {
            println!("{CACHE_FILE}: {} values", changes.len());
            print_changes(&changes);
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
    pool: PgPool,
    url: &str,
    file: &HubFile,
    data_dir: &Path,
    backup: Option<&Path>,
) -> Result<()> {
    let mut done: Vec<&str> = Vec::new();
    let new_url = db::renamed_url(url, FROM, TO);
    let result = async {
        let report = broker::apply(opts, actions, DEFAULT_QUIET).await?;
        println!(
            "broker: published {}, cleared {}",
            report.published, report.cleared
        );
        done.push("broker");
        for (table, rows) in db::rename_entity_ids(&pool, false).await? {
            println!("{table}: {rows} rows");
        }
        done.push("entity ids");
        let settings = db::rename_settings(&pool, false).await?;
        println!("settings: {} keys rewritten", settings.len());
        done.push("settings");
        pool.close().await;
        let n = rewrite_cache(data_dir)?;
        println!("{CACHE_FILE}: {n} values rewritten");
        done.push(CACHE_FILE);
        let workers = db::rename_db_and_role(url, FROM, TO).await?;
        println!("database and role renamed to {TO}");
        done.push("database and role");
        match &workers {
            db::Workers::Started => println!("timescaledb background workers started on {TO}"),
            db::Workers::NotInstalled => println!("no timescaledb in {TO}: no workers to start"),
            db::Workers::NotStarted(why) => {
                eprintln!("timescaledb background workers NOT started on {TO}: {why}")
            }
        }
        let (new_file, changes) = rename_hub_file(file)?;
        new_file.save(data_dir)?;
        println!("hub.json: {} values rewritten", changes.len());
        print_changes(&changes);
        done.push("hub.json");
        if let db::Workers::NotStarted(why) = workers {
            bail!(
                "the compression and retention jobs are stopped on {TO} ({why}); restart the \
                 database server, or run this command again"
            );
        }
        // The spec's last step of --apply: the same check as --verify.
        verify(opts, &new_url, data_dir).await
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
                "the retained messages are backed up in {0}; to put them back: nidoo {COMMAND} --restore {0}",
                b.display()
            );
        }
        if done.contains(&"database and role") && !done.contains(&"hub.json") {
            eprintln!(
                "the database is already renamed: set hub.json database_url to {} (same password)",
                mask_url_password(&new_url)
            );
        }
        return Err(e);
    }
    Ok(())
}

async fn verify(opts: &BrokerOpts, url: &str, data_dir: &Path) -> Result<()> {
    let msgs = broker::read_retained(opts, DEFAULT_QUIET).await?;
    broker::require_retained(&msgs)?;
    let mut lines = broker::verify_retained(&msgs);
    // After the switch-over `hub.json` points at the renamed database already.
    let pool = pool(url).await?;
    lines.extend(db::verify_db(&pool).await?);
    pool.close().await;
    lines.extend(verify_cache(data_dir)?);
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
    #[allow(clippy::result_large_err)] // figment::Jail closures return figment::Error
    fn env_database_url_is_detected_with_or_without_hub_json_url() {
        figment::Jail::expect_with(|jail| {
            let with_url = HubFile {
                database_url: Some("postgresql://oikos:p@h/oikos".into()),
                ..HubFile::default()
            };
            assert!(!env_sets_database_url(&with_url)?);
            assert!(!env_sets_database_url(&HubFile::default())?);
            // Whatever the env URL names, old or new: the command would act on another URL.
            for env in [
                "postgresql://oikos:p@h/oikos",
                "postgresql://nidoo:p@h/nidoo",
            ] {
                jail.set_env("NIDOO_HISTORY__URL", env);
                assert!(env_sets_database_url(&with_url)?, "{env}");
                assert!(env_sets_database_url(&HubFile::default())?, "{env}");
            }
            Ok(())
        });
    }

    fn hub_file() -> HubFile {
        HubFile {
            mqtt: Some(crate::config::MqttConfig {
                host: "oikos-mosquitto".into(),
                user: Some("oikos".into()),
                password: Some("oikos-pw".into()),
                client_id: OLD_CLIENT_ID.into(),
                ..Default::default()
            }),
            database_url: Some("postgresql://oikos:oikos-pw@oikos-db:5432/oikos".into()),
            log: crate::config::LogConfig {
                level: "info,oikos=debug".into(),
                ..Default::default()
            },
            ..HubFile::default()
        }
    }

    #[test]
    fn hub_json_follows_the_mapping_but_keeps_logins_and_addresses() {
        let (new, changes) = rename_hub_file(&hub_file()).unwrap();
        let mqtt = new.mqtt.as_ref().unwrap();
        assert_eq!(mqtt.client_id, "nidoo");
        assert_eq!(mqtt.host, "oikos-mosquitto");
        assert_eq!(mqtt.user.as_deref(), Some("oikos"));
        assert_eq!(mqtt.password.as_deref(), Some("oikos-pw"));
        assert_eq!(new.log.level, "info,nidoo=debug");
        // User and database renamed; password and host kept.
        assert_eq!(
            new.database_url.as_deref(),
            Some("postgresql://nidoo:oikos-pw@oikos-db:5432/nidoo")
        );
        let mut paths: Vec<&str> = changes.iter().map(|c| c.path.as_str()).collect();
        paths.sort();
        assert_eq!(paths, ["/database_url", "/log/level", "/mqtt/client_id"]);
        let url = changes.iter().find(|c| c.path == "/database_url").unwrap();
        assert!(!url.old.contains("oikos-pw") && !url.new.contains("oikos-pw"));
        let id = changes
            .iter()
            .find(|c| c.path == "/mqtt/client_id")
            .unwrap();
        assert_eq!((id.old.as_str(), id.new.as_str()), ("oikos-os", "nidoo"));
        // A second pass has nothing left.
        assert!(rename_hub_file(&new).unwrap().1.is_empty());
    }

    #[test]
    fn settings_cache_is_rewritten_and_verified() {
        let dir = tempfile::tempdir().unwrap();
        // No cache: nothing to do, nothing to flag.
        assert!(rename_cache(dir.path()).unwrap().is_none());
        assert_eq!(rewrite_cache(dir.path()).unwrap(), 0);
        assert!(verify_cache(dir.path()).unwrap().is_empty());

        let cache = serde_json::json!({
            "advanced": {"topics": {"ui_prefix": "oikos/ui"}},
            "push": {"prefix": "oikos/push", "service_account": "-----BEGIN PRIVATE KEY-----oikos"}
        });
        let path = dir.path().join(CACHE_FILE);
        std::fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
        assert_eq!(
            verify_cache(dir.path()).unwrap(),
            [format!("{CACHE_FILE}: still names oikos")]
        );
        let (_, changes) = rename_cache(dir.path()).unwrap().unwrap();
        assert_eq!(changes.len(), 3);
        assert_eq!(rewrite_cache(dir.path()).unwrap(), 3);
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(v["advanced"]["topics"]["ui_prefix"], "nidoo/ui");
        assert_eq!(v["push"]["prefix"], "nidoo/push");
        assert!(verify_cache(dir.path()).unwrap().is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Nothing left to rename: the bytes are kept.
        let before = std::fs::read(&path).unwrap();
        assert_eq!(rewrite_cache(dir.path()).unwrap(), 0);
        assert_eq!(std::fs::read(&path).unwrap(), before);
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
