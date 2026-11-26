#![allow(clippy::result_large_err)] // figment::Jail closures return figment::Error

use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::routing::get;
use tokio::net::TcpListener;

use super::*;
use crate::setup::hub_file::RestoreState;

/// `resolve` reads `NIDOO_*`, so each call runs inside figment's lock on a runtime of its
/// own.
fn resolve_locked(dir: &Path) -> anyhow::Result<Boot> {
    let mut out = None;
    figment::Jail::expect_with(|_| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        out = Some(rt.block_on(resolve(dir)));
        Ok(())
    });
    out.unwrap()
}

fn write(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

const DOWN: &str = "postgres://u:p@127.0.0.1:1/nidoo";

#[test]
fn empty_dir_is_setup_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let boot = resolve_locked(tmp.path()).unwrap();
    let Boot::Setup(ctx) = boot else {
        panic!("expected setup mode")
    };
    assert_eq!(ctx.mode, Mode::Setup);
    assert!(ctx.db.is_none());
}

#[test]
fn complete_file_with_restore_pending_boots_restore_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let file = HubFile {
        mqtt: Some(crate::config::MqttConfig::default()),
        database_url: Some(DOWN.into()),
        restore: Some(RestoreState::default()),
        ..HubFile::default()
    };
    file.save(tmp.path()).unwrap();
    let Boot::Restore(ctx) = resolve_locked(tmp.path()).unwrap() else {
        panic!("expected restore mode")
    };
    assert_eq!(ctx.mode, Mode::Restore);
    assert!(ctx.db.is_some());
}

#[test]
fn an_empty_table_boots_from_the_cache_and_refills_it() {
    let mut cached = Sections::default();
    cached.weather.enabled = true;
    let (got, refill) = from_table(None, Some(cached.clone()));
    assert_eq!(got, cached);
    assert!(refill);
    // No cache either: the defaults, nothing to put back.
    let (got, refill) = from_table(None, None);
    assert_eq!(got, Sections::default());
    assert!(!refill);
    // Rows win over the cache, even rows at their defaults.
    let (got, refill) = from_table(Some(Sections::default()), Some(cached));
    assert_eq!(got, Sections::default());
    assert!(!refill);
}

#[test]
fn complete_file_and_db_down_uses_the_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let file = HubFile {
        mqtt: Some(crate::config::MqttConfig::default()),
        database_url: Some(DOWN.into()),
        finished: true,
        ..HubFile::default()
    };
    file.save(tmp.path()).unwrap();
    let mut cached = Sections::default();
    cached.weather.enabled = true;
    write_cache(tmp.path(), &cached).unwrap();
    let Boot::Run { cfg, ctx } = resolve_locked(tmp.path()).unwrap() else {
        panic!("expected normal mode")
    };
    assert!(cfg.weather.enabled);
    assert_eq!(ctx.mode, Mode::Run);
}

#[test]
fn complete_file_db_down_and_no_cache_uses_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let file = HubFile {
        mqtt: Some(crate::config::MqttConfig::default()),
        database_url: Some(DOWN.into()),
        ..HubFile::default()
    };
    file.save(tmp.path()).unwrap();
    let Boot::Run { cfg, .. } = resolve_locked(tmp.path()).unwrap() else {
        panic!("expected normal mode")
    };
    assert_eq!(cfg.weather.enabled, Sections::default().weather.enabled);
}

#[test]
fn corrupt_hub_file_is_an_error_naming_it() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "hub.json", "{\"mqtt\":");
    let err = match resolve_locked(tmp.path()) {
        Ok(_) => panic!("a corrupt hub.json must not boot"),
        Err(e) => format!("{e:#}"),
    };
    assert!(err.contains("hub.json"), "{err}");
}

#[test]
fn env_alone_completes_the_bootstrap() {
    let tmp = tempfile::tempdir().unwrap();
    let mut out = None;
    figment::Jail::expect_with(|jail| {
        jail.set_env("NIDOO_MQTT__HOST", "192.0.2.7");
        jail.set_env("NIDOO_HISTORY__URL", DOWN);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        out = Some(rt.block_on(resolve(tmp.path())));
        Ok(())
    });
    let Boot::Run { cfg, .. } = out.unwrap().unwrap() else {
        panic!("expected normal mode")
    };
    assert_eq!(cfg.mqtt.host, "192.0.2.7");
}

fn ctx_with(file: HubFile, dir: &Path) -> (Arc<SetupCtx>, Arc<AtomicUsize>) {
    let count = Arc::new(AtomicUsize::new(0));
    let ctx = Arc::new(SetupCtx::new(
        dir.to_path_buf(),
        Mode::Run,
        file,
        None,
        Sections::default(),
        RestartHook::Record(Arc::clone(&count)),
    ));
    (ctx, count)
}

async fn geocoder(body: &'static str) -> Endpoints {
    let app = Router::new().route("/geocode", get(move || async move { body }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Endpoints {
        geocoding: format!("{base}/geocode"),
        forecast: format!("{base}/forecast"),
    }
}

const LYON: &str = r#"{"results":[{"name":"Lyon","latitude":45.75,"longitude":4.85,
        "country_code":"FR","admin1":"Auvergne-Rhone-Alpes","timezone":"Europe/Paris"}]}"#;

async fn wait_for(mut done: impl FnMut() -> bool) {
    for _ in 0..200 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out");
}

fn pending(tz: Option<&str>) -> HubFile {
    HubFile {
        pending_city: Some(("Lyon".into(), Some("FR".into()))),
        timezone: tz.map(Into::into),
        ..HubFile::default()
    }
}

#[tokio::test]
async fn pending_city_is_geocoded_then_restarts() {
    let tmp = tempfile::tempdir().unwrap();
    let (ctx, count) = ctx_with(pending(None), tmp.path());
    let ep = geocoder(LYON).await;
    spawn_location_lookup(Arc::clone(&ctx), ep);
    wait_for(|| count.load(Ordering::SeqCst) == 1).await;
    let file = ctx.file.lock().await.clone();
    let loc = file.location.clone().unwrap();
    assert_eq!(loc.city, "Lyon");
    assert_eq!(loc.country.as_deref(), Some("FR"));
    assert_eq!((loc.latitude, loc.longitude), (45.75, 4.85));
    assert_eq!(loc.timezone, "Europe/Paris");
    assert!(file.pending_city.is_none() && file.timezone.is_none());
    // Saved to disk too.
    assert_eq!(HubFile::load(tmp.path()).unwrap().unwrap(), file);
}

#[tokio::test]
async fn an_imported_timezone_beats_the_geocoded_one() {
    let tmp = tempfile::tempdir().unwrap();
    let (ctx, count) = ctx_with(pending(Some("America/New_York")), tmp.path());
    let ep = geocoder(LYON).await;
    spawn_location_lookup(Arc::clone(&ctx), ep);
    wait_for(|| count.load(Ordering::SeqCst) == 1).await;
    let file = ctx.file.lock().await.clone();
    assert_eq!(file.location.unwrap().timezone, "America/New_York");
    assert!(file.timezone.is_none());
}

#[tokio::test]
async fn a_result_without_a_timezone_falls_back_to_a_real_zone() {
    let tmp = tempfile::tempdir().unwrap();
    let (ctx, count) = ctx_with(pending(None), tmp.path());
    let ep = geocoder(r#"{"results":[{"name":"Lyon","latitude":45.75,"longitude":4.85}]}"#).await;
    spawn_location_lookup(Arc::clone(&ctx), ep);
    wait_for(|| count.load(Ordering::SeqCst) == 1).await;
    let tz = ctx.file.lock().await.location.clone().unwrap().timezone;
    assert!(tz.parse::<chrono_tz::Tz>().is_ok(), "{tz}");
}

#[tokio::test]
async fn no_match_keeps_the_pending_city_without_restarting() {
    let tmp = tempfile::tempdir().unwrap();
    pending(None).save(tmp.path()).unwrap();
    let (ctx, count) = ctx_with(pending(None), tmp.path());
    let ep = geocoder("{}").await;
    let task = spawn_location_lookup(Arc::clone(&ctx), ep);
    task.await.unwrap();
    let file = ctx.file.lock().await.clone();
    assert!(file.pending_city.is_some() && file.location.is_none());
    assert_eq!(count.load(Ordering::SeqCst), 0);
    let disk = HubFile::load(tmp.path()).unwrap().unwrap();
    assert!(disk.pending_city.is_some() && disk.location.is_none());
    // The kept city is what lets weather boot: the config still validates.
    let mut sections = Sections::default();
    sections.weather.enabled = true;
    assert_eq!(
        crate::setup::sections::assemble(&file, &sections).validate_all(),
        Ok(())
    );
}

#[tokio::test]
async fn a_location_set_during_the_lookup_is_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let (ctx, count) = ctx_with(pending(None), tmp.path());
    // The geocoder answers only once the user has picked a place.
    let go = Arc::new(tokio::sync::Notify::new());
    let asked = Arc::new(tokio::sync::Notify::new());
    let (wait, ask) = (Arc::clone(&go), Arc::clone(&asked));
    let app = Router::new().route(
        "/geocode",
        get(move || {
            let (wait, ask) = (Arc::clone(&wait), Arc::clone(&ask));
            async move {
                ask.notify_one();
                wait.notified().await;
                LYON
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let ep = Endpoints {
        geocoding: format!("{base}/geocode"),
        forecast: format!("{base}/forecast"),
    };
    let task = spawn_location_lookup(Arc::clone(&ctx), ep);
    // The lookup has read the pending city and is waiting for the answer.
    asked.notified().await;
    let chosen = Location {
        city: "Paris".into(),
        country: Some("FR".into()),
        latitude: 48.85,
        longitude: 2.35,
        timezone: "Europe/Paris".into(),
    };
    let picked = chosen.clone();
    ctx.update_file(move |f| {
        f.location = Some(picked);
        f.pending_city = None;
    })
    .await
    .unwrap();
    go.notify_one();
    task.await.unwrap();
    assert_eq!(ctx.file.lock().await.location, Some(chosen.clone()));
    assert_eq!(
        HubFile::load(tmp.path()).unwrap().unwrap().location,
        Some(chosen)
    );
    assert_eq!(
        count.load(Ordering::SeqCst),
        0,
        "no restart for a stale lookup"
    );
}

#[tokio::test]
async fn concurrent_file_updates_do_not_collide() {
    let tmp = tempfile::tempdir().unwrap();
    let (ctx, _) = ctx_with(HubFile::default(), tmp.path());
    let mut tasks = Vec::new();
    for i in 0..20 {
        let ctx = Arc::clone(&ctx);
        tasks.push(tokio::spawn(async move {
            ctx.update_file(|f| f.z2m_base = Some(format!("base{i}")))
                .await
        }));
    }
    for t in tasks {
        t.await.unwrap().unwrap();
    }
    let held = ctx.file.lock().await.clone();
    assert_eq!(HubFile::load(tmp.path()).unwrap().unwrap(), held);
}
