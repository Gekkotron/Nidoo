//! Database-backed tests for the `settings` table. They skip without `TEST_DATABASE_URL`,
//! like `history_db.rs`.

use crate::common;

use common::db::fresh_db;
use nidoo::setup::sections::Sections;
use nidoo::setup::store::{load_sections, save_sections};

fn sample() -> Sections {
    let mut s = Sections::default();
    s.solcast.enabled = true;
    // Solcast on needs a site, or every PUT fails the boot validation.
    s.solcast.sites = vec!["site-1".into()];
    s.solcast.api_key = Some("solcast-key".into());
    s.frigate.url = Some("http://192.0.2.9:5000".into());
    s.frigate.password = Some("frigate-pass".into());
    s
}

#[tokio::test]
async fn empty_table_loads_defaults() {
    let Some(tdb) = fresh_db().await else { return };
    assert_eq!(load_sections(&tdb.db).await.unwrap(), Sections::default());
    tdb.drop_db().await;
}

#[tokio::test]
async fn save_then_load_round_trips() {
    let Some(tdb) = fresh_db().await else { return };
    let s = sample();
    save_sections(&tdb.db, &s).await.unwrap();
    let back = load_sections(&tdb.db).await.unwrap();
    assert_eq!(back, s);
    assert_eq!(back.solcast.api_key.as_deref(), Some("solcast-key"));
    assert_eq!(back.frigate.password.as_deref(), Some("frigate-pass"));
    tdb.drop_db().await;
}

#[tokio::test]
async fn save_twice_overwrites() {
    let Some(tdb) = fresh_db().await else { return };
    let mut s = sample();
    save_sections(&tdb.db, &s).await.unwrap();
    s.weather.enabled = !s.weather.enabled;
    save_sections(&tdb.db, &s).await.unwrap();
    let back = load_sections(&tdb.db).await.unwrap();
    assert_eq!(back, s);
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM settings")
        .fetch_one(tdb.db.pool())
        .await
        .unwrap();
    assert_eq!(n as usize, nidoo::setup::sections::SECTION_KEYS.len());
    tdb.drop_db().await;
}

#[tokio::test]
async fn database_probe_reports_timescale_and_history_since() {
    use nidoo::setup::probe;

    let Some(tdb) = fresh_db().await else { return };
    let result = probe::database(&tdb.url).await;
    assert!(result.ok, "{}", result.message);
    let detail = result.detail.expect("detail");
    assert!(
        detail["history_since"].is_null(),
        "an empty database has no history"
    );

    sqlx::query(
        "INSERT INTO entity_state (time, entity_id, value) \
         VALUES ('2026-01-02T03:04:05Z', 'e', '1'::jsonb)",
    )
    .execute(tdb.db.pool())
    .await
    .unwrap();
    let result = probe::database(&tdb.url).await;
    assert!(result.ok, "{}", result.message);
    let since = result.detail.unwrap()["history_since"]
        .as_str()
        .map(str::to_owned)
        .expect("history_since");
    assert!(since.starts_with("2026-01-02T03:04:05"), "{since}");
    tdb.drop_db().await;
}

// ---- PUT /api/v1/settings/{section}, over a real database ------------------------------

mod put {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use nidoo::config::Config;
    use nidoo::history::db::Db;
    use nidoo::hub::Hub;
    use nidoo::setup::{HubFile, Mode, RestartHook, SetupCtx};
    use nidoo::weather::open_meteo::Endpoints;
    use reqwest::StatusCode;
    use serde_json::{Value, json};

    use super::*;

    struct Served {
        addr: std::net::SocketAddr,
        dir: tempfile::TempDir,
        ctx: Arc<SetupCtx>,
        restarts: Arc<AtomicUsize>,
    }

    impl Served {
        async fn put(&self, section: &str, body: Value) -> (StatusCode, Value) {
            let r = reqwest::Client::new()
                .put(format!("http://{}/api/v1/settings/{section}", self.addr))
                .json(&body)
                .send()
                .await
                .unwrap();
            (r.status(), r.json().await.unwrap_or(Value::Null))
        }
        async fn get(&self, section: &str) -> Value {
            reqwest::get(format!("http://{}/api/v1/settings/{section}", self.addr))
                .await
                .unwrap()
                .json()
                .await
                .unwrap()
        }
    }

    async fn serve(url: &str, sections: Sections) -> Served {
        let dir = tempfile::tempdir().unwrap();
        let restarts = Arc::new(AtomicUsize::new(0));
        let file = HubFile {
            location: serde_json::from_value(json!({"city":"Paris","country":"FR",
                "latitude":48.85,"longitude":2.35,"timezone":"Europe/Paris"}))
            .unwrap(),
            ..HubFile::default()
        };
        let ctx = Arc::new(SetupCtx::new(
            dir.path().to_path_buf(),
            Mode::Run,
            file,
            Some(Db::connect_lazy(url).unwrap()),
            sections,
            RestartHook::Record(Arc::clone(&restarts)),
        ));
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let hub = Hub::new(&Config::default(), tx);
        hub.set_setup(Arc::clone(&ctx));
        let app = nidoo::http::router_with(hub, Arc::clone(&ctx), Endpoints::default());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(axum::serve(listener, app).into_future());
        Served {
            addr,
            dir,
            ctx,
            restarts,
        }
    }

    #[tokio::test]
    async fn mcp_set_settings_saves_and_restarts() {
        use rmcp::ServiceExt;
        use rmcp::model::CallToolRequestParams;
        use rmcp::transport::StreamableHttpClientTransport;
        let Some(tdb) = fresh_db().await else { return };
        let s = serve(&tdb.url, sample()).await;
        let client = ()
            .serve(StreamableHttpClientTransport::from_uri(format!(
                "http://{}/mcp",
                s.addr
            )))
            .await
            .unwrap();
        let args = json!({"section": "solcast",
            "value": {"enabled": true, "sites": ["site-1"], "daily_limit": 5}});
        let result = client
            .call_tool(
                CallToolRequestParams::new("set_settings")
                    .with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
        assert_eq!(result.structured_content, Some(json!({"restarting": true})));
        assert_eq!(s.restarts.load(Ordering::SeqCst), 1);
        let stored = load_sections(&tdb.db).await.unwrap();
        assert_eq!(stored.solcast.sites, ["site-1"]);
        assert_eq!(stored.solcast.api_key.as_deref(), Some("solcast-key"));
        drop(client);
        tdb.drop_db().await;
    }

    #[tokio::test]
    async fn backup_section_is_live_and_write_only() {
        let Some(tdb) = fresh_db().await else { return };
        let s = serve(&tdb.url, Sections::default()).await;
        let body = json!({"password": "correct-horse", "keep": 3,
            "ftp": {"host": "nas.local", "user": "u", "password": "ftp-pw", "folder": "/backups"},
            "notify_failure": true});
        let (code, view) = s.put("backup", body).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(view["password"], json!({"set": true}));
        assert_eq!(view["ftp"]["password"], json!({"set": true}));
        assert_eq!(view["ftp"]["port"], 21);
        assert_eq!(s.restarts.load(Ordering::SeqCst), 0, "no restart");
        let got = s.get("backup").await;
        assert_eq!(got["password"], json!({"set": true}));
        for text in [view.to_string(), got.to_string()] {
            assert!(!text.contains("correct-horse") && !text.contains("ftp-pw"));
        }
        let stored = load_sections(&tdb.db).await.unwrap();
        assert_eq!(stored.backup.password.as_deref(), Some("correct-horse"));
        assert_eq!(stored.backup.keep, 3);
        let (code, _) = s.put("backup", json!({"password": null})).await;
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
        drop(s.dir);
        tdb.drop_db().await;
    }

    const GW: &str = "https://house.example.ts.net";

    #[tokio::test]
    async fn mobile_put_round_trips_without_restart() {
        let Some(tdb) = fresh_db().await else { return };
        let s = serve(&tdb.url, Sections::default()).await;
        let key = "ab".repeat(32);
        let (code, body) = s
            .put("mobile", json!({"gateway_url": GW, "gateway_key": key}))
            .await;
        assert_eq!(code, StatusCode::OK, "{body}");
        assert_eq!(body["gateway_ready"], true);
        assert_eq!(s.restarts.load(Ordering::SeqCst), 0);
        let stored = load_sections(&tdb.db).await.unwrap();
        assert_eq!(stored.mobile.gateway_url.as_deref(), Some(GW));
        assert_eq!(stored.mobile.gateway_key.as_deref(), Some(key.as_str()));
        tdb.drop_db().await;
    }

    #[tokio::test]
    async fn mobile_put_keeps_key_on_set_view_and_empty() {
        let Some(tdb) = fresh_db().await else { return };
        let s = serve(&tdb.url, Sections::default()).await;
        let key = "ab".repeat(32);
        s.put("mobile", json!({"gateway_url": GW, "gateway_key": key}))
            .await;
        let (code, _) = s
            .put(
                "mobile",
                json!({"gateway_url": GW, "gateway_key": {"set": true}}),
            )
            .await;
        assert_eq!(code, StatusCode::OK);
        let (code, _) = s
            .put("mobile", json!({"gateway_url": GW, "gateway_key": ""}))
            .await;
        assert_eq!(code, StatusCode::OK);
        let stored = load_sections(&tdb.db).await.unwrap();
        assert_eq!(stored.mobile.gateway_key.as_deref(), Some(key.as_str()));
        tdb.drop_db().await;
    }

    #[tokio::test]
    async fn mobile_put_refuses_bad_input_without_echo() {
        let Some(tdb) = fresh_db().await else { return };
        let s = serve(&tdb.url, Sections::default()).await;
        let short = "cd".repeat(31) + "c";
        for body in [
            json!({"gateway_url": "http://house.example.ts.net"}),
            json!({"gateway_url": GW, "gateway_key": short}),
        ] {
            let (code, resp) = s.put("mobile", body).await;
            assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
            assert!(!resp.to_string().contains(&short), "{resp}");
        }
        tdb.drop_db().await;
    }

    #[tokio::test]
    async fn mobile_code_includes_the_gateway() {
        let Some(tdb) = fresh_db().await else { return };
        let s = serve(&tdb.url, Sections::default()).await;
        s.ctx.file.lock().await.mqtt = Some(nidoo::config::MqttConfig {
            host: "192.0.2.7".into(),
            ..Default::default()
        });
        let key = "ab".repeat(32);
        s.put("mobile", json!({"gateway_url": GW, "gateway_key": key}))
            .await;
        let body: Value = reqwest::get(format!("http://{}/api/v1/setup/mobile-code", s.addr))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let p: Value = serde_json::from_str(body["payload"].as_str().unwrap()).unwrap();
        assert_eq!(p["gateway"]["url"], GW);
        tdb.drop_db().await;
    }

    #[tokio::test]
    async fn put_saves_db_and_cache_and_keeps_omitted_secret() {
        let Some(tdb) = fresh_db().await else { return };
        save_sections(&tdb.db, &sample()).await.unwrap();
        let s = serve(&tdb.url, sample()).await;
        let (code, body) = s
            .put(
                "solcast",
                json!({"enabled": true, "sites": ["site-1", "site-2"], "daily_limit": 5}),
            )
            .await;
        assert_eq!(code, StatusCode::OK, "{body}");
        assert_eq!(body, json!({"restarting": true}));
        assert_eq!(s.restarts.load(Ordering::SeqCst), 1);
        let stored = load_sections(&tdb.db).await.unwrap();
        assert_eq!(stored.solcast.sites, ["site-1", "site-2"]);
        assert_eq!(stored.solcast.daily_limit, 5);
        assert_eq!(stored.solcast.api_key.as_deref(), Some("solcast-key"));
        // The other sections are saved as they were.
        assert_eq!(stored.frigate.password.as_deref(), Some("frigate-pass"));
        let cached = nidoo::setup::store::read_cache(s.dir.path()).unwrap();
        assert_eq!(cached, stored);
        // A GET before the restart already shows the new values, without the key.
        let view = s.get("solcast").await;
        assert_eq!(view["sites"], json!(["site-1", "site-2"]));
        assert_eq!(view["api_key"], json!({"set": true}));
        assert_eq!(s.ctx.sections.lock().await.solcast, stored.solcast);
        tdb.drop_db().await;
    }

    #[tokio::test]
    async fn null_clears_a_secret_and_a_string_sets_it() {
        let Some(tdb) = fresh_db().await else { return };
        let s = serve(&tdb.url, sample()).await;
        let (code, body) = s
            .put(
                "solcast",
                json!({"enabled": false, "sites": [], "api_key": null}),
            )
            .await;
        assert_eq!(code, StatusCode::OK, "{body}");
        assert_eq!(load_sections(&tdb.db).await.unwrap().solcast.api_key, None);
        tdb.drop_db().await;

        let Some(tdb) = fresh_db().await else { return };
        let s = serve(&tdb.url, sample()).await;
        let (code, body) = s
            .put(
                "frigate",
                json!({"url": "http://frigate.example.com:5000", "password": "new-pass"}),
            )
            .await;
        assert_eq!(code, StatusCode::OK, "{body}");
        let stored = load_sections(&tdb.db).await.unwrap();
        assert_eq!(stored.frigate.password.as_deref(), Some("new-pass"));
        assert_eq!(
            stored.frigate.url.as_deref(),
            Some("http://frigate.example.com:5000")
        );
        tdb.drop_db().await;
    }

    #[tokio::test]
    async fn frigate_path_only_change_keeps_the_password() {
        let Some(tdb) = fresh_db().await else { return };
        let s = serve(&tdb.url, sample()).await;
        let (code, body) = s
            .put("frigate", json!({"url": "http://192.0.2.9:5000/frigate"}))
            .await;
        assert_eq!(code, StatusCode::OK, "{body}");
        let stored = load_sections(&tdb.db).await.unwrap();
        assert_eq!(stored.frigate.password.as_deref(), Some("frigate-pass"));
        assert_eq!(
            stored.frigate.url.as_deref(),
            Some("http://192.0.2.9:5000/frigate")
        );
        tdb.drop_db().await;
    }

    #[tokio::test]
    async fn invalid_solcast_saves_nothing() {
        let Some(tdb) = fresh_db().await else { return };
        save_sections(&tdb.db, &sample()).await.unwrap();
        let s = serve(&tdb.url, sample()).await;
        let (code, body) = s
            .put("solcast", json!({"enabled": true, "sites": []}))
            .await;
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body["error"].as_str().unwrap().contains("[solcast] sites"));
        assert_eq!(load_sections(&tdb.db).await.unwrap(), sample());
        assert!(!s.dir.path().join("settings-cache.json").exists());
        assert_eq!(s.restarts.load(Ordering::SeqCst), 0);
        tdb.drop_db().await;
    }

    #[tokio::test]
    async fn a_failed_cache_write_is_a_500_and_changes_nothing() {
        let Some(tdb) = fresh_db().await else { return };
        save_sections(&tdb.db, &sample()).await.unwrap();
        let s = serve(&tdb.url, sample()).await;
        // A directory where the cache file goes makes the write fail.
        std::fs::create_dir(s.dir.path().join("settings-cache.json")).unwrap();
        let (code, body) = s
            .put("weather", json!({"enabled": true, "refresh_minutes": 45}))
            .await;
        assert_eq!(code, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, json!({"error": "could not save the settings cache"}));
        assert_eq!(s.restarts.load(Ordering::SeqCst), 0);
        assert_eq!(*s.ctx.sections.lock().await, sample());
        assert_eq!(load_sections(&tdb.db).await.unwrap(), sample());
        tdb.drop_db().await;
    }

    #[tokio::test]
    async fn a_read_can_be_sent_back_as_it_is() {
        let Some(tdb) = fresh_db().await else { return };
        let mut sections = sample();
        sections.push.service_account =
            Some(r#"{"project_id":"proj-1","private_key":"pem"}"#.into());
        let s = serve(&tdb.url, sections).await;
        let frigate = s.get("frigate").await;
        let (code, body) = s.put("frigate", frigate).await;
        assert_eq!(code, StatusCode::OK, "{body}");
        // Each PUT schedules a restart; the stand-in hook never performs it, so clear the
        // flag for the next PUT (a real hub would have restarted in between).
        s.ctx.restarting.store(false, Ordering::SeqCst);
        let push = s.get("push").await;
        assert_eq!(push["service_account"], json!({"set": true}));
        let solcast = s.get("solcast").await;
        assert_eq!(s.put("solcast", solcast).await.0, StatusCode::OK);
        s.ctx.restarting.store(false, Ordering::SeqCst);
        let stored = load_sections(&tdb.db).await.unwrap();
        assert_eq!(stored.frigate.password.as_deref(), Some("frigate-pass"));
        assert_eq!(stored.solcast.api_key.as_deref(), Some("solcast-key"));
        // The push view, project id and all, goes back too.
        assert_eq!(s.put("push", push).await.0, StatusCode::OK);
        let stored = load_sections(&tdb.db).await.unwrap();
        assert!(stored.push.service_account.unwrap().contains("pem"));
        tdb.drop_db().await;
    }

    #[tokio::test]
    async fn a_put_replaces_the_whole_section() {
        let Some(tdb) = fresh_db().await else { return };
        let mut sections = sample();
        sections.frigate.cooldown = 99;
        sections.frigate.topic_prefix = "cams".into();
        let s = serve(&tdb.url, sections).await;
        let (code, _) = s
            .put("frigate", json!({"url": "http://192.0.2.9:5000"}))
            .await;
        assert_eq!(code, StatusCode::OK);
        let stored = load_sections(&tdb.db).await.unwrap();
        assert_eq!(stored.frigate.cooldown, 0);
        assert_eq!(stored.frigate.topic_prefix, "frigate");
        assert_eq!(stored.frigate.password.as_deref(), Some("frigate-pass"));
        tdb.drop_db().await;
    }

    #[tokio::test]
    async fn a_put_rewrites_only_its_own_row() {
        let Some(tdb) = fresh_db().await else { return };
        // The table holds real settings, but the hub booted from the defaults (the database
        // was down and there was no cache).
        save_sections(&tdb.db, &sample()).await.unwrap();
        let s = serve(&tdb.url, Sections::default()).await;
        let (code, body) = s
            .put("weather", json!({"enabled": true, "refresh_minutes": 45}))
            .await;
        assert_eq!(code, StatusCode::OK, "{body}");
        let stored = load_sections(&tdb.db).await.unwrap();
        assert_eq!(stored.weather.refresh_minutes, 45);
        // Every other row is as it was.
        let expected = Sections {
            weather: stored.weather.clone(),
            ..sample()
        };
        assert_eq!(stored, expected);
        assert_eq!(stored.frigate.password.as_deref(), Some("frigate-pass"));
        assert_eq!(stored.solcast.api_key.as_deref(), Some("solcast-key"));
        tdb.drop_db().await;
    }
}

// ---- booting over an empty settings table ----------------------------------------------

mod boot {
    use nidoo::config::MqttConfig;
    use nidoo::setup::HubFile;
    use nidoo::setup::boot::{Boot, resolve};
    use nidoo::setup::store::{read_cache, save_section, write_cache};

    use super::*;

    fn complete(url: &str) -> HubFile {
        HubFile {
            mqtt: Some(MqttConfig::default()),
            database_url: Some(url.to_owned()),
            finished: true,
            ..HubFile::default()
        }
    }

    #[tokio::test]
    async fn an_empty_table_boots_from_the_cache_and_fills_the_table() {
        let Some(tdb) = fresh_db().await else { return };
        let tmp = tempfile::tempdir().unwrap();
        complete(&tdb.url).save(tmp.path()).unwrap();
        write_cache(tmp.path(), &sample()).unwrap();
        let Boot::Run { cfg, ctx } = resolve(tmp.path()).await.unwrap() else {
            panic!("expected normal mode")
        };
        assert_eq!(*ctx.sections.lock().await, sample());
        assert_eq!(cfg.frigate.password.as_deref(), Some("frigate-pass"));
        assert_eq!(load_sections(&tdb.db).await.unwrap(), sample());
        assert_eq!(read_cache(tmp.path()), Some(sample()));
        tdb.drop_db().await;
    }

    #[tokio::test]
    async fn a_table_with_some_rows_still_rules() {
        let Some(tdb) = fresh_db().await else { return };
        let tmp = tempfile::tempdir().unwrap();
        complete(&tdb.url).save(tmp.path()).unwrap();
        write_cache(tmp.path(), &sample()).unwrap();
        let mut one = Sections::default();
        one.weather.refresh_minutes = 45;
        save_section(&tdb.db, "weather", &one).await.unwrap();
        let Boot::Run { ctx, .. } = resolve(tmp.path()).await.unwrap() else {
            panic!("expected normal mode")
        };
        // The missing rows are defaults; the cache is refreshed from the table.
        assert_eq!(*ctx.sections.lock().await, one);
        assert_eq!(read_cache(tmp.path()), Some(one));
        tdb.drop_db().await;
    }
}

#[tokio::test]
async fn set_system_notify_writes_cache_without_restart() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use nidoo::config::Config;
    use nidoo::hub::Hub;
    use nidoo::setup::{HubFile, Mode, RestartHook, SetupCtx, store};

    let Some(tdb) = fresh_db().await else { return };
    let dir = tempfile::tempdir().unwrap();
    let restarts = Arc::new(AtomicUsize::new(0));
    let ctx = Arc::new(SetupCtx::new(
        dir.path().to_path_buf(),
        Mode::Run,
        HubFile::default(),
        Some(nidoo::history::db::Db::connect_lazy(&tdb.url).unwrap()),
        Sections::default(),
        RestartHook::Record(Arc::clone(&restarts)),
    ));
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let hub = Hub::new(&Config::default(), tx);
    hub.set_setup(Arc::clone(&ctx));

    assert!(!hub.system_notify());
    assert!(hub.set_system_notify(true).await.unwrap());
    assert!(hub.system_notify());
    assert!(!ctx.restarting.load(Ordering::SeqCst));
    assert_eq!(restarts.load(Ordering::SeqCst), 0);

    let cache: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join(store::CACHE_FILE)).unwrap())
            .unwrap();
    assert_eq!(cache["system"]["notify"], true);
    assert!(load_sections(&tdb.db).await.unwrap().system.notify);
    tdb.drop_db().await;
}
