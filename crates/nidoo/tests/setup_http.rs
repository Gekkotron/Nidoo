//! The setup API, served on an ephemeral port over a temp data directory. Brokers and
//! geocoding are closed local ports or local stubs: nothing leaves the machine.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::routing::get;
use nidoo::config::MqttConfig;
use nidoo::http::{router_with, setup_router, setup_router_with};
use nidoo::hub::Hub;
use nidoo::setup::sections::Sections;
use nidoo::setup::{HubFile, Mode, RestartHook, SetupCtx};
use nidoo::weather::open_meteo::Endpoints;
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::net::TcpListener;

struct Server {
    addr: SocketAddr,
    dir: tempfile::TempDir,
    ctx: Arc<SetupCtx>,
    restarts: Arc<AtomicUsize>,
    client: reqwest::Client,
}

impl Server {
    fn url(&self, path: &str) -> String {
        format!("http://{}/api/v1/setup{path}", self.addr)
    }
    async fn get(&self, path: &str) -> (StatusCode, Value) {
        let r = self.client.get(self.url(path)).send().await.unwrap();
        (r.status(), r.json().await.unwrap_or(Value::Null))
    }
    async fn send(&self, method: reqwest::Method, path: &str, body: Value) -> (StatusCode, Value) {
        let r = self
            .client
            .request(method, self.url(path))
            .json(&body)
            .send()
            .await
            .unwrap();
        (r.status(), r.json().await.unwrap_or(Value::Null))
    }
    async fn put(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send(reqwest::Method::PUT, path, body).await
    }
    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send(reqwest::Method::POST, path, body).await
    }
    fn hub_json(&self) -> Option<Vec<u8>> {
        std::fs::read(self.dir.path().join("hub.json")).ok()
    }
}

fn ctx_for(dir: &Path, mode: Mode, file: HubFile) -> (Arc<SetupCtx>, Arc<AtomicUsize>) {
    let restarts = Arc::new(AtomicUsize::new(0));
    let ctx = Arc::new(SetupCtx::new(
        dir.to_path_buf(),
        mode,
        file,
        None,
        Sections::default(),
        RestartHook::Record(Arc::clone(&restarts)),
    ));
    (ctx, restarts)
}

async fn serve(mode: Mode, file: HubFile, geocoding: Option<Endpoints>, with_hub: bool) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let (ctx, restarts) = ctx_for(dir.path(), mode, file);
    let geo = geocoding.unwrap_or_default();
    let app = if with_hub {
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let hub = Hub::new(&nidoo::config::Config::default(), tx);
        router_with(hub, Arc::clone(&ctx), geo)
    } else {
        setup_router_with(Arc::clone(&ctx), geo)
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, app).into_future());
    Server {
        addr,
        dir,
        ctx,
        restarts,
        client: reqwest::Client::new(),
    }
}

async fn setup_server() -> Server {
    serve(Mode::Setup, HubFile::default(), None, false).await
}

/// A local port nothing listens on.
async fn closed_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    l.local_addr().unwrap().port()
}

fn mqtt_file(host: &str) -> HubFile {
    HubFile {
        mqtt: Some(MqttConfig {
            host: host.into(),
            port: 1,
            user: Some("u".into()),
            password: Some("stored-pass".into()),
            ..MqttConfig::default()
        }),
        ..HubFile::default()
    }
}

fn write_bundled(dir: &Path) {
    std::fs::write(
        dir.join("bundled.json"),
        json!({
            "mqtt_user": "oikos", "mqtt_password": "bundled-mqtt-secret",
            "db_user": "oikos", "db_password": "bundled-db-secret", "db_name": "oikos"
        })
        .to_string(),
    )
    .unwrap();
}

#[tokio::test]
async fn status_in_empty_dir() {
    let s = setup_server().await;
    let (code, body) = s.get("").await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(body["mode"], "setup");
    assert_eq!(body["step"], "welcome");
    assert_eq!(body["bundled"], false);
    assert_eq!(body["finished"], false);
    assert!(body["mqtt"].is_null() && body["database"].is_null());
}

#[tokio::test]
async fn health_and_unknown_api_in_setup_mode() {
    let s = setup_server().await;
    let r = reqwest::get(format!("http://{}/health", s.addr))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let r = reqwest::get(format!("http://{}/api/v1/nope", s.addr))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn bundled_prefills_views_without_leaking() {
    let s = setup_server().await;
    write_bundled(s.dir.path());
    let r = s.client.get(s.url("")).send().await.unwrap();
    let text = r.text().await.unwrap();
    assert!(!text.contains("bundled-mqtt-secret") && !text.contains("bundled-db-secret"));
    let body: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(body["bundled"], true);
    assert_eq!(body["mqtt"]["user"], "oikos");
    assert_eq!(body["mqtt"]["password"], json!({"set": true}));
    assert_eq!(body["database"]["database"], "oikos");
    assert_eq!(body["database"]["password"], json!({"set": true}));
}

#[tokio::test]
async fn stored_passwords_are_never_returned() {
    let mut file = mqtt_file("192.0.2.1");
    file.database_url = Some("postgres://dbu:db-secret@192.0.2.2:5433/oikos".into());
    let s = serve(Mode::Run, file, None, true).await;
    let r = s.client.get(s.url("")).send().await.unwrap();
    let text = r.text().await.unwrap();
    assert!(
        !text.contains("stored-pass") && !text.contains("db-secret"),
        "{text}"
    );
    let body: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(body["mode"], "running");
    assert_eq!(body["mqtt"]["password"], json!({"set": true}));
    assert_eq!(body["database"]["host"], "192.0.2.2");
    assert_eq!(body["database"]["port"], 5433);
    assert_eq!(body["database"]["user"], "dbu");
}

fn paris() -> Value {
    json!({"city":"Paris","country":"FR","latitude":48.85,"longitude":2.35,"timezone":"Europe/Paris"})
}

#[tokio::test]
async fn location_put_advances_to_mqtt() {
    let s = setup_server().await;
    let (code, _) = s.put("/location", paris()).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(s.get("").await.1["step"], "mqtt");
    assert_eq!(s.get("").await.1["location"]["city"], "Paris");
    let file = HubFile::load(s.dir.path()).unwrap().unwrap();
    assert_eq!(file.location.unwrap().city, "Paris");
    assert_eq!(
        s.restarts.load(Ordering::SeqCst),
        0,
        "setup mode does not restart"
    );
}

#[tokio::test]
async fn location_put_clears_the_imported_fallbacks() {
    let file = HubFile {
        pending_city: Some(("Lyon".into(), None)),
        timezone: Some("Europe/Paris".into()),
        ..HubFile::default()
    };
    let s = serve(Mode::Setup, file, None, false).await;
    assert_eq!(s.put("/location", paris()).await.0, StatusCode::OK);
    let saved = HubFile::load(s.dir.path()).unwrap().unwrap();
    assert!(saved.pending_city.is_none() && saved.timezone.is_none());
}

#[tokio::test]
async fn location_with_a_bad_timezone_is_422() {
    let s = setup_server().await;
    let mut bad = paris();
    bad["timezone"] = json!("Mars/Olympus");
    assert_eq!(
        s.put("/location", bad).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert!(s.hub_json().is_none());
}

#[tokio::test]
async fn location_with_a_bad_country_is_422() {
    let s = setup_server().await;
    for country in [json!("France"), json!("F1"), json!("")] {
        let mut bad = paris();
        bad["country"] = country.clone();
        let (code, body) = s.put("/location", bad).await;
        if country == json!("") {
            // An empty field is no country at all.
            assert_eq!(code, StatusCode::OK, "{body}");
            continue;
        }
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{country}");
        assert_eq!(
            body,
            json!({"error": "The country must be two letters, such as FR"})
        );
    }
    let saved = HubFile::load(s.dir.path()).unwrap().unwrap();
    assert_eq!(saved.location.unwrap().country, None);
    let mut lower = paris();
    lower["country"] = json!("fr");
    assert_eq!(s.put("/location", lower).await.0, StatusCode::OK);
    let saved = HubFile::load(s.dir.path()).unwrap().unwrap();
    assert_eq!(saved.location.unwrap().country.as_deref(), Some("FR"));
}

#[tokio::test]
async fn a_bad_country_saves_nothing() {
    let s = setup_server().await;
    let mut bad = paris();
    bad["country"] = json!("France");
    assert_eq!(
        s.put("/location", bad).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert!(s.hub_json().is_none());
}

/// Sections the boot would refuse: Solcast on with no site.
fn unbootable() -> Sections {
    let mut s = Sections::default();
    s.solcast.enabled = true;
    s
}

#[tokio::test]
async fn a_running_hub_never_saves_a_location_it_could_not_boot_with() {
    let s = serve_settings(Mode::Run, HubFile::default(), unbootable(), None, None).await;
    s.ctx.update_file(|_| {}).await.unwrap();
    let before = s.hub_json().unwrap();
    let (code, body) = s.put("/location", paris()).await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(
        body,
        json!({"error": "[solcast] sites needs at least one rooftop resource id"})
    );
    assert_eq!(s.hub_json().unwrap(), before, "hub.json is byte-identical");
    assert_eq!(s.restarts.load(Ordering::SeqCst), 0);
    assert!(s.ctx.file.lock().await.location.is_none());
}

#[tokio::test]
async fn a_running_hub_never_saves_a_broker_it_could_not_boot_with() {
    let port = fake_broker(0).await;
    let s = serve_settings(Mode::Run, mqtt_file("192.0.2.1"), unbootable(), None, None).await;
    s.ctx.update_file(|_| {}).await.unwrap();
    let before = s.hub_json().unwrap();
    let (code, body) = s
        .put(
            "/mqtt",
            json!({"host":"127.0.0.1","port":port,"user":"u","password":"pw","z2m_base":"z"}),
        )
        .await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("[solcast] sites"),
        "{body}"
    );
    assert_eq!(s.hub_json().unwrap(), before, "hub.json is byte-identical");
    assert_eq!(s.restarts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn setup_mode_writes_are_not_held_to_the_running_rules() {
    // Settings can only be fixed once the hub runs, so setup never refuses on them.
    let s = serve_settings(Mode::Setup, HubFile::default(), unbootable(), None, None).await;
    assert_eq!(s.put("/location", paris()).await.0, StatusCode::OK);
}

#[tokio::test]
async fn location_in_run_mode_restarts_once() {
    let s = serve(Mode::Run, HubFile::default(), None, true).await;
    assert_eq!(s.put("/location", paris()).await.0, StatusCode::OK);
    assert_eq!(s.restarts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_mqtt_probe_saves_nothing() {
    let s = setup_server().await;
    let before = s.hub_json();
    let port = closed_port().await;
    let (code, body) = s
        .put(
            "/mqtt",
            json!({"host":"127.0.0.1","port":port,"user":"u","password":"pw"}),
        )
        .await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["ok"], false);
    assert_eq!(s.hub_json(), before);
    assert!(s.hub_json().is_none());
}

#[tokio::test]
async fn failed_mqtt_probe_on_a_running_hub_leaves_the_file_and_does_not_restart() {
    let s = serve(Mode::Run, mqtt_file("192.0.2.1"), None, true).await;
    s.ctx.update_file(|_| {}).await.unwrap();
    let before = s.hub_json().unwrap();
    let (code, body) = s
        .put(
            "/mqtt",
            json!({"host":"127.0.0.1","port":1,"user":"u","password":"wrong"}),
        )
        .await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["ok"], false);
    assert_eq!(s.hub_json().unwrap(), before);
    assert_eq!(s.restarts.load(Ordering::SeqCst), 0);
    // Nothing reached the 422 body either.
    assert!(!body.to_string().contains("wrong"));
}

#[tokio::test]
async fn mqtt_test_reports_without_saving() {
    let s = setup_server().await;
    let port = closed_port().await;
    let (code, body) = s
        .post("/test/mqtt", json!({"host":"127.0.0.1","port":port}))
        .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(body["ok"], false);
    assert!(body["message"].is_string());
    assert!(s.hub_json().is_none());
}

#[tokio::test]
async fn mqtt_new_host_without_password_is_422() {
    let s = serve(Mode::Setup, mqtt_file("a.example.com"), None, false).await;
    for path in ["/mqtt", "/test/mqtt"] {
        let (code, body) = s
            .send(
                if path == "/mqtt" {
                    reqwest::Method::PUT
                } else {
                    reqwest::Method::POST
                },
                path,
                json!({"host":"b.example.com","port":1883}),
            )
            .await;
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{path}");
        assert_eq!(body["error"], "Enter the password for the new address");
    }
    assert!(s.hub_json().is_none());
}

#[tokio::test]
async fn bundled_password_does_not_follow_a_new_host() {
    let s = setup_server().await;
    write_bundled(s.dir.path());
    let (code, body) = s
        .post(
            "/test/mqtt",
            json!({"host":"attacker.example.com","port":1883}),
        )
        .await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "Enter the password for the new address");
}

#[tokio::test]
async fn database_new_host_without_password_is_422() {
    let file = HubFile {
        database_url: Some("postgres://u:p@a.example.com:5432/oikos".into()),
        ..HubFile::default()
    };
    let s = serve(Mode::Setup, file, None, false).await;
    let (code, body) = s
        .put(
            "/database",
            json!({"host":"b.example.com","port":5432,"database":"oikos","user":"u"}),
        )
        .await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "Enter the password for the new address");
}

#[tokio::test]
async fn failed_database_probe_saves_nothing() {
    let s = setup_server().await;
    let port = closed_port().await;
    let (code, body) = s
        .put(
            "/database",
            json!({"host":"127.0.0.1","port":port,"database":"oikos","user":"u","password":"pw-secret-9"}),
        )
        .await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["ok"], false);
    assert!(!body.to_string().contains("pw-secret-9"));
    assert!(s.hub_json().is_none());
}

#[tokio::test]
async fn step_put_never_finishes() {
    let s = setup_server().await;
    let (code, body) = s.put("/step", json!({"step":"done"})).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(body["step"], "done");
    assert_eq!(body["finished"], false);
    assert!(!HubFile::load(s.dir.path()).unwrap().unwrap().finished);
}

#[tokio::test]
async fn finish_sets_finished_and_done() {
    let file = HubFile {
        database_url: Some("postgres://u:p@192.0.2.2/oikos".into()),
        ..mqtt_file("192.0.2.1")
    };
    let s = serve(Mode::Setup, file, None, false).await;
    let (code, body) = s.post("/finish", json!({})).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(body["finished"], true);
    assert_eq!(body["step"], "done");
}

#[tokio::test]
async fn finish_before_mqtt_and_database_is_409() {
    let s = serve(Mode::Setup, mqtt_file("192.0.2.1"), None, false).await;
    let (code, body) = s.post("/finish", json!({})).await;
    assert_eq!(code, StatusCode::CONFLICT);
    assert_eq!(
        body,
        json!({"error": "finish MQTT and database setup first"})
    );
    assert!(s.hub_json().is_none(), "nothing saved");
}

#[tokio::test]
async fn second_write_during_a_pending_restart_is_503() {
    let s = serve(Mode::Run, HubFile::default(), None, true).await;
    assert_eq!(s.put("/location", paris()).await.0, StatusCode::OK);
    let after_first = s.hub_json().unwrap();
    for (method, path, body) in [
        (reqwest::Method::PUT, "/location", paris()),
        (reqwest::Method::PUT, "/step", json!({"step":"mqtt"})),
        (reqwest::Method::POST, "/finish", json!({})),
        (
            reqwest::Method::PUT,
            "/mqtt",
            json!({"host":"127.0.0.1","port":1}),
        ),
    ] {
        let (code, body) = s.send(method, path, body).await;
        assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE, "{path}");
        assert_eq!(body, json!({"error":"restarting"}));
    }
    assert_eq!(s.hub_json().unwrap(), after_first);
    assert_eq!(s.restarts.load(Ordering::SeqCst), 1);
    assert!(HubFile::load(s.dir.path()).is_ok());
}

#[tokio::test]
async fn geocode_returns_at_most_five_places_without_secrets() {
    let results: Vec<Value> = (0..8)
        .map(|i| {
            json!({"name": format!("Paris {i}"), "admin1": "Ile", "country_code": "FR",
                   "latitude": 48.0 + f64::from(i), "longitude": 2.0, "timezone": "Europe/Paris"})
        })
        .collect();
    let body = json!({ "results": results });
    let app = Router::new().route(
        "/geocode",
        get(move || {
            let body = body.clone();
            async move { axum::Json(body) }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, app).into_future());
    let ep = Endpoints {
        geocoding: format!("http://{base}/geocode"),
        forecast: "http://127.0.0.1:1/unused".into(),
    };
    let s = serve(Mode::Setup, HubFile::default(), Some(ep), false).await;
    let (code, body) = s.get("/geocode?city=Paris&country=FR").await;
    assert_eq!(code, StatusCode::OK);
    let places = body.as_array().unwrap();
    assert_eq!(places.len(), 5);
    assert_eq!(places[0]["name"], "Paris 0");
    assert_eq!(places[0]["timezone"], "Europe/Paris");
    assert_eq!(s.get("/geocode?city=").await.1, json!([]));
}

#[tokio::test]
async fn geocode_failure_is_502() {
    let ep = Endpoints {
        geocoding: format!("http://127.0.0.1:{}/geocode", closed_port().await),
        forecast: String::new(),
    };
    let s = serve(Mode::Setup, HubFile::default(), Some(ep), false).await;
    assert_eq!(
        s.get("/geocode?city=Paris").await.0,
        StatusCode::BAD_GATEWAY
    );
}

#[tokio::test]
async fn setup_router_default_builds() {
    let (ctx, _) = ctx_for(
        tempfile::tempdir().unwrap().path(),
        Mode::Setup,
        HubFile::default(),
    );
    let _ = setup_router(ctx);
}

/// A one-shot broker: reads the CONNECT, answers a CONNACK with `code`, keeps the socket open.
async fn fake_broker(code: u8) -> u16 {
    fake_broker_with(code, None).await
}

/// Like [`fake_broker`]; `on_connect` runs the moment a CONNECT arrives, before the answer is
/// written, so whatever it does is ordered before the probe returns.
async fn fake_broker_with(code: u8, on_connect: Option<Arc<dyn Fn() + Send + Sync>>) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let on_connect = on_connect.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 512];
                let _ = sock.read(&mut buf).await;
                if let Some(f) = on_connect {
                    f();
                }
                let _ = sock.write_all(&[0x20, 0x02, 0x00, code]).await;
                let _ = sock.read(&mut buf).await;
            });
        }
    });
    port
}

#[tokio::test]
async fn mqtt_put_saves_after_a_good_probe() {
    let port = fake_broker(0).await;
    let s = setup_server().await;
    let (code, body) = s
        .put(
            "/mqtt",
            json!({"host":"127.0.0.1","port":port,"user":"u","password":"pw","z2m_base":"z2m"}),
        )
        .await;
    assert_eq!(code, StatusCode::OK, "{body}");
    assert_eq!(body["step"], "database");
    assert_eq!(body["mqtt"]["password"], json!({"set": true}));
    assert!(!body.to_string().contains("\"pw\""));
    let file = HubFile::load(s.dir.path()).unwrap().unwrap();
    assert_eq!(file.mqtt.unwrap().password.as_deref(), Some("pw"));
    assert_eq!(file.z2m_base.as_deref(), Some("z2m"));
    assert_eq!(s.restarts.load(Ordering::SeqCst), 0, "not complete yet");
}

#[tokio::test]
async fn mqtt_put_that_completes_the_file_restarts() {
    let port = fake_broker(0).await;
    let file = HubFile {
        database_url: Some("postgres://u:p@192.0.2.2/oikos".into()),
        ..HubFile::default()
    };
    let s = serve(Mode::Setup, file, None, false).await;
    let (code, _) = s
        .put("/mqtt", json!({"host":"127.0.0.1","port":port}))
        .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(s.restarts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn mqtt_bad_credentials_are_reported() {
    let port = fake_broker(4).await;
    let s = setup_server().await;
    let (code, body) = s
        .post(
            "/test/mqtt",
            json!({"host":"127.0.0.1","port":port,"user":"u","password":"x"}),
        )
        .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(body["ok"], false);
    assert_eq!(body["message"], "bad credentials");
    let port = fake_broker(0).await;
    let (_, body) = s
        .post("/test/mqtt", json!({"host":"127.0.0.1","port":port}))
        .await;
    assert_eq!(body, json!({"ok":true,"message":"connected","detail":null}));
}

#[tokio::test]
async fn bad_broker_credentials_on_a_running_hub_are_never_saved() {
    let port = fake_broker(4).await;
    let s = serve(Mode::Run, mqtt_file("192.0.2.1"), None, true).await;
    s.ctx.update_file(|_| {}).await.unwrap();
    let before = s.hub_json().unwrap();
    let (code, body) = s
        .put(
            "/mqtt",
            json!({"host":"127.0.0.1","port":port,"user":"u","password":"typo-pass"}),
        )
        .await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["ok"], false);
    assert_eq!(body["message"], "bad credentials");
    assert!(!body.to_string().contains("typo-pass"));
    assert_eq!(s.hub_json().unwrap(), before);
    assert_eq!(s.restarts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_restart_scheduled_during_the_probe_refuses_the_save() {
    let slot: Arc<std::sync::OnceLock<Arc<SetupCtx>>> = Arc::new(std::sync::OnceLock::new());
    // The broker raises `restarting` (as a racing write would) after the gate, during the
    // probe, and does so before it answers: no scheduling can let the save slip in first.
    let hook = Arc::clone(&slot);
    let port = fake_broker_with(
        0,
        Some(Arc::new(move || {
            hook.get().unwrap().restarting.store(true, Ordering::SeqCst);
        })),
    )
    .await;
    let s = serve(Mode::Setup, HubFile::default(), None, false).await;
    slot.set(Arc::clone(&s.ctx)).ok().unwrap();
    let (code, body) = s
        .put("/mqtt", json!({"host":"127.0.0.1","port":port}))
        .await;
    assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body, json!({"error":"restarting"}));
    assert!(s.hub_json().is_none(), "nothing saved");
}

#[tokio::test]
async fn password_without_a_user_is_refused() {
    let s = setup_server().await;
    let port = closed_port().await;
    for path in ["/mqtt", "/test/mqtt"] {
        let (code, body) = s
            .send(
                if path == "/mqtt" {
                    reqwest::Method::PUT
                } else {
                    reqwest::Method::POST
                },
                path,
                json!({"host":"127.0.0.1","port":port,"password":"pw"}),
            )
            .await;
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{path}");
        assert_eq!(body["error"], "Enter a user name for the password");
    }
    assert!(s.hub_json().is_none());
}

const NEW_ADDRESS: &str = "Enter the password for the new address";

#[tokio::test]
async fn mqtt_same_host_other_port_needs_the_password() {
    let s = serve(Mode::Setup, mqtt_file("127.0.0.1"), None, false).await; // stored port 1
    for (method, path) in [
        (reqwest::Method::PUT, "/mqtt"),
        (reqwest::Method::POST, "/test/mqtt"),
    ] {
        let (code, body) = s
            .send(
                method,
                path,
                json!({"host":"127.0.0.1","port":2,"user":"u"}),
            )
            .await;
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{path}");
        assert_eq!(body["error"], NEW_ADDRESS);
    }
    assert!(s.hub_json().is_none());
}

#[tokio::test]
async fn bundled_mqtt_password_stays_on_the_bundled_port() {
    let s = setup_server().await;
    write_bundled(s.dir.path());
    for (method, path) in [
        (reqwest::Method::PUT, "/mqtt"),
        (reqwest::Method::POST, "/test/mqtt"),
    ] {
        let (code, body) = s
            .send(
                method,
                path,
                json!({"host":"localhost","port":1884,"user":"oikos"}),
            )
            .await;
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{path}");
        assert_eq!(body["error"], NEW_ADDRESS);
    }
    assert!(s.hub_json().is_none());
}

#[tokio::test]
async fn database_same_host_other_port_needs_the_password() {
    let file = HubFile {
        database_url: Some("postgres://u:p@127.0.0.1:5432/oikos".into()),
        ..HubFile::default()
    };
    let s = serve(Mode::Setup, file, None, false).await;
    for (method, path) in [
        (reqwest::Method::PUT, "/database"),
        (reqwest::Method::POST, "/test/database"),
    ] {
        let (code, body) = s
            .send(
                method,
                path,
                json!({"host":"127.0.0.1","port":5433,"database":"oikos","user":"u"}),
            )
            .await;
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{path}");
        assert_eq!(body["error"], NEW_ADDRESS);
    }
}

#[tokio::test]
async fn bundled_database_password_stays_on_the_bundled_port() {
    let s = setup_server().await;
    write_bundled(s.dir.path());
    for (method, path) in [
        (reqwest::Method::PUT, "/database"),
        (reqwest::Method::POST, "/test/database"),
    ] {
        let (code, body) = s
            .send(
                method,
                path,
                json!({"host":"localhost","port":5433,"database":"oikos","user":"oikos"}),
            )
            .await;
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{path}");
        assert_eq!(body["error"], NEW_ADDRESS);
    }
}

#[tokio::test]
async fn database_name_or_user_change_alone_is_not_a_new_address() {
    let port = closed_port().await;
    let file = HubFile {
        database_url: Some(format!("postgres://u:p@127.0.0.1:{port}/oikos")),
        ..HubFile::default()
    };
    let s = serve(Mode::Setup, file, None, false).await;
    let (code, body) = s
        .post(
            "/test/database",
            json!({"host":"127.0.0.1","port":port,"database":"other","user":"v"}),
        )
        .await;
    assert_eq!(code, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], false, "the probe ran (and found no server)");
}

// ---- the settings API (services) -------------------------------------------------------

const SOLCAST_KEY: &str = "solcast-secret-key";
const FRIGATE_PASS: &str = "frigate-secret-pass";
const PUSH_JSON: &str = r#"{"type":"service_account","project_id":"proj-42","private_key":"-----BEGIN PRIVATE KEY-----secret-pem"}"#;

impl Server {
    fn surl(&self, path: &str) -> String {
        format!("http://{}/api/v1/settings{path}", self.addr)
    }
    async fn sget(&self, path: &str) -> (StatusCode, String) {
        let r = self.client.get(self.surl(path)).send().await.unwrap();
        (r.status(), r.text().await.unwrap())
    }
    async fn sput(&self, path: &str, body: Value) -> (StatusCode, Value) {
        let r = self
            .client
            .put(self.surl(path))
            .json(&body)
            .send()
            .await
            .unwrap();
        (r.status(), r.json().await.unwrap_or(Value::Null))
    }
}

fn secret_sections() -> Sections {
    let mut s = Sections::default();
    s.solcast.api_key = Some(SOLCAST_KEY.into());
    s.frigate.url = Some("http://192.0.2.9:5000/".into());
    s.frigate.user = Some("u".into());
    s.frigate.password = Some(FRIGATE_PASS.into());
    s.push.service_account = Some(PUSH_JSON.into());
    s
}

fn located() -> HubFile {
    HubFile {
        location: serde_json::from_value(paris()).unwrap(),
        ..HubFile::default()
    }
}

/// A server over a ready-made context. `db_url` gives the context a (lazy) database.
async fn serve_settings(
    mode: Mode,
    file: HubFile,
    sections: Sections,
    db_url: Option<&str>,
    solcast_base: Option<String>,
) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let restarts = Arc::new(AtomicUsize::new(0));
    let ctx = Arc::new(SetupCtx::new(
        dir.path().to_path_buf(),
        mode,
        file,
        db_url.map(|u| nidoo::history::db::Db::connect_lazy(u).unwrap()),
        sections,
        RestartHook::Record(Arc::clone(&restarts)),
    ));
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let hub = Hub::new(&nidoo::config::Config::default(), tx);
    let base = solcast_base.unwrap_or_else(|| "http://127.0.0.1:1".into());
    let app = nidoo::http::router_with_solcast(hub, Arc::clone(&ctx), Endpoints::default(), &base);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, app).into_future());
    Server {
        addr,
        dir,
        ctx,
        restarts,
        client: reqwest::Client::new(),
    }
}

/// A database address nothing answers on.
const DEAD_DB: &str = "postgres://u:p@127.0.0.1:1/oikos";

#[tokio::test]
async fn get_never_returns_secrets() {
    let s = serve_settings(Mode::Run, located(), secret_sections(), None, None).await;
    for key in ["solcast", "frigate", "push"] {
        let (code, text) = s.sget(&format!("/{key}")).await;
        assert_eq!(code, StatusCode::OK);
        for secret in [SOLCAST_KEY, FRIGATE_PASS, "secret-pem", "private_key"] {
            assert!(!text.contains(secret), "{key} leaks {secret}: {text}");
        }
    }
    let solcast: Value = serde_json::from_str(&s.sget("/solcast").await.1).unwrap();
    assert_eq!(solcast["api_key"], json!({"set": true}));
    let frigate: Value = serde_json::from_str(&s.sget("/frigate").await.1).unwrap();
    assert_eq!(frigate["password"], json!({"set": true}));
    assert_eq!(frigate["user"], "u");
    assert_eq!(frigate["topic_prefix"], "frigate");
    let push: Value = serde_json::from_str(&s.sget("/push").await.1).unwrap();
    assert_eq!(push["service_account"], json!({"set": true}));
    let empty = serve_settings(Mode::Run, located(), Sections::default(), None, None).await;
    let solcast: Value = serde_json::from_str(&empty.sget("/solcast").await.1).unwrap();
    assert_eq!(solcast["api_key"], json!({"set": false}));
}

#[tokio::test]
async fn get_works_in_setup_mode_and_for_plain_sections() {
    let s = serve_settings(
        Mode::Setup,
        HubFile::default(),
        Sections::default(),
        None,
        None,
    )
    .await;
    for key in [
        "weather", "solcast", "frigate", "push", "climate", "advanced",
    ] {
        assert_eq!(s.sget(&format!("/{key}")).await.0, StatusCode::OK, "{key}");
    }
    let weather: Value = serde_json::from_str(&s.sget("/weather").await.1).unwrap();
    assert_eq!(weather["enabled"], false);
    assert_eq!(weather["refresh_minutes"], 15);
}

#[tokio::test]
async fn unknown_section_is_404() {
    let s = serve_settings(Mode::Run, located(), Sections::default(), None, None).await;
    assert_eq!(s.sget("/nope").await.0, StatusCode::NOT_FOUND);
    assert_eq!(s.sput("/nope", json!({})).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn put_in_setup_mode_is_409() {
    let s = serve_settings(
        Mode::Setup,
        HubFile::default(),
        Sections::default(),
        None,
        None,
    )
    .await;
    let (code, body) = s.sput("/weather", json!({"enabled": false})).await;
    assert_eq!(code, StatusCode::CONFLICT);
    assert_eq!(
        body,
        json!({"error": "finish MQTT and database setup first"})
    );
    assert!(!s.dir.path().join("settings-cache.json").exists());
}

#[tokio::test]
async fn put_during_a_pending_restart_is_503() {
    let s = serve_settings(
        Mode::Run,
        located(),
        Sections::default(),
        Some(DEAD_DB),
        None,
    )
    .await;
    s.ctx.restarting.store(true, Ordering::SeqCst);
    let (code, body) = s.sput("/weather", json!({"enabled": false})).await;
    assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body, json!({"error": "restarting"}));
    assert_eq!(s.restarts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn frigate_new_url_without_password_is_422() {
    let s = serve_settings(Mode::Run, located(), secret_sections(), Some(DEAD_DB), None).await;
    let (code, body) = s
        .sput(
            "/frigate",
            json!({"url": "http://frigate.example.com:5000"}),
        )
        .await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "Enter the password for the new address");
    // A new port, or a new scheme, is a new address as well.
    for url in ["http://192.0.2.9:5001", "https://192.0.2.9:5000"] {
        let (code, _) = s.sput("/frigate", json!({ "url": url })).await;
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{url}");
    }
    assert!(!s.dir.path().join("settings-cache.json").exists());
}

#[tokio::test]
async fn frigate_path_only_change_is_not_a_new_address() {
    let s = serve_settings(Mode::Run, located(), secret_sections(), Some(DEAD_DB), None).await;
    // It gets past the address rule and fails only at the (dead) database.
    let (code, body) = s
        .sput("/frigate", json!({"url": "http://192.0.2.9:5000/frigate/"}))
        .await;
    assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body, json!({"error": "database unreachable"}));
}

#[tokio::test]
async fn invalid_solcast_is_422_and_nothing_saved() {
    let s = serve_settings(
        Mode::Run,
        located(),
        Sections::default(),
        Some(DEAD_DB),
        None,
    )
    .await;
    let (code, body) = s
        .sput(
            "/solcast",
            json!({"enabled": true, "sites": [], "api_key": SOLCAST_KEY}),
        )
        .await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body["error"].as_str().unwrap().contains("[solcast] sites"),
        "{body}"
    );
    assert!(!body.to_string().contains(SOLCAST_KEY));
    assert!(!s.dir.path().join("settings-cache.json").exists());
    assert_eq!(*s.ctx.sections.lock().await, Sections::default());
    assert_eq!(s.restarts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn weather_without_location_is_422() {
    let s = serve_settings(
        Mode::Run,
        HubFile::default(),
        Sections::default(),
        Some(DEAD_DB),
        None,
    )
    .await;
    let (code, body) = s.sput("/weather", json!({"enabled": true})).await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "Set the home location first");
}

#[tokio::test]
async fn weather_with_an_imported_city_waiting_for_its_lookup_is_accepted() {
    let file = HubFile {
        pending_city: Some(("Lyon".into(), Some("FR".into()))),
        ..HubFile::default()
    };
    let s = serve_settings(Mode::Run, file, Sections::default(), Some(DEAD_DB), None).await;
    let (code, body) = s
        .sput("/weather", json!({"enabled": true, "refresh_minutes": 30}))
        .await;
    // Past the location rule and the validation; only the dead database stops it.
    assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body, json!({"error": "database unreachable"}));
}

#[tokio::test]
async fn frigate_url_with_a_login_in_it_is_refused() {
    let s = serve_settings(Mode::Run, located(), secret_sections(), Some(DEAD_DB), None).await;
    let refused = json!({"error": "Put the user and password in their own fields"});
    for body in [
        json!({"url": "http://u:inline-pass@192.0.2.9:5000", "password": "p"}),
        json!({"url": "http://192.0.2.9:5000", "media_url": "http://u:inline-pass@192.0.2.9:8971"}),
    ] {
        let (code, got) = s.sput("/frigate", body.clone()).await;
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(got, refused);
        let (code, got) = s.post("/test/frigate", body.clone()).await;
        assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(got, refused);
    }
    assert!(!s.dir.path().join("settings-cache.json").exists());
}

#[tokio::test]
async fn frigate_view_never_shows_a_login_in_the_url() {
    let mut sections = Sections::default();
    sections.frigate.url = Some("http://fu:inline-pass@192.0.2.9:5000".into());
    sections.frigate.media_url = Some("http://mu:media-pass@192.0.2.9:8971/m".into());
    let s = serve_settings(Mode::Run, located(), sections, None, None).await;
    let (code, text) = s.sget("/frigate").await;
    assert_eq!(code, StatusCode::OK);
    for leak in ["inline-pass", "media-pass", "fu:", "mu:"] {
        assert!(!text.contains(leak), "{leak} in {text}");
    }
    let view: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(view["url"], "http://192.0.2.9:5000");
    assert_eq!(view["media_url"], "http://192.0.2.9:8971/m");
}

#[tokio::test]
async fn an_unreachable_database_is_503_and_nothing_is_saved() {
    let s = serve_settings(
        Mode::Run,
        located(),
        Sections::default(),
        Some(DEAD_DB),
        None,
    )
    .await;
    let (code, body) = s
        .sput("/weather", json!({"enabled": true, "refresh_minutes": 30}))
        .await;
    assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body, json!({"error": "database unreachable"}));
    assert!(!s.dir.path().join("settings-cache.json").exists());
    assert_eq!(*s.ctx.sections.lock().await, Sections::default());
    assert_eq!(s.restarts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn push_view_shows_project_id() {
    let s = serve_settings(Mode::Run, located(), secret_sections(), None, None).await;
    let push: Value = serde_json::from_str(&s.sget("/push").await.1).unwrap();
    assert_eq!(push["project_id"], "proj-42");
    assert_eq!(push["prefix"], "oikos/push");
    // A key that is not JSON shows no project id and is never echoed.
    let mut sections = Sections::default();
    sections.push.service_account = Some("not json, secret-pem".into());
    let s = serve_settings(Mode::Run, located(), sections, None, None).await;
    let (code, text) = s.sget("/push").await;
    assert_eq!(code, StatusCode::OK);
    assert!(!text.contains("secret-pem"));
    let push: Value = serde_json::from_str(&text).unwrap();
    assert!(push["project_id"].is_null());
    assert_eq!(push["service_account"], json!({"set": true}));
}

#[tokio::test]
async fn push_key_that_is_not_a_service_account_is_422() {
    let s = serve_settings(
        Mode::Run,
        located(),
        Sections::default(),
        Some(DEAD_DB),
        None,
    )
    .await;
    let (code, body) = s
        .sput("/push", json!({"service_account": "not json, secret-pem"}))
        .await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!body.to_string().contains("secret-pem"));
}

/// A Solcast stand-in: `/rooftop_sites` wants `Bearer <SOLCAST_KEY>`.
async fn fake_solcast() -> String {
    use axum::http::{HeaderMap, header};
    async fn sites(headers: HeaderMap) -> (StatusCode, axum::Json<Value>) {
        let ok = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            == Some(&format!("Bearer {SOLCAST_KEY}")[..]);
        if ok {
            (
                StatusCode::OK,
                axum::Json(json!({"sites": [
                    {"resource_id": "aaaa-1111", "name": "Roof south"},
                    {"resource_id": "bbbb-2222", "name": "Roof west"}
                ]})),
            )
        } else {
            (StatusCode::UNAUTHORIZED, axum::Json(json!({})))
        }
    }
    let app = Router::new().route("/rooftop_sites", get(sites));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, app).into_future());
    format!("http://{addr}")
}

#[tokio::test]
async fn solcast_test_lists_the_sites() {
    let base = fake_solcast().await;
    let s = serve_settings(Mode::Run, located(), secret_sections(), None, Some(base)).await;
    // The key in the body.
    let (code, body) = s
        .post("/test/solcast", json!({"api_key": SOLCAST_KEY}))
        .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(body["ok"], true, "{body}");
    assert_eq!(body["detail"]["sites"][0]["name"], "Roof south");
    assert_eq!(body["detail"]["sites"][1]["id"], "bbbb-2222");
    // No key in the body: the stored one is used.
    let (_, body) = s.post("/test/solcast", json!({})).await;
    assert_eq!(body["ok"], true, "{body}");
    // A wrong key is reported, not echoed.
    let (code, body) = s
        .post("/test/solcast", json!({"api_key": "wrong-key-7"}))
        .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(body["ok"], false);
    assert_eq!(body["message"], "bad credentials");
    assert!(!body.to_string().contains("wrong-key-7"));
    // No key anywhere.
    let empty = serve_settings(
        Mode::Run,
        located(),
        Sections::default(),
        None,
        Some(fake_solcast().await),
    )
    .await;
    let (_, body) = empty.post("/test/solcast", json!({})).await;
    assert_eq!(body["ok"], false);
}

/// A Frigate stand-in on its login port: user `u` / password `p`, camera list behind the cookie.
async fn fake_frigate() -> String {
    use axum::http::{HeaderMap, header};
    async fn login(axum::Json(body): axum::Json<Value>) -> axum::response::Response {
        use axum::response::IntoResponse;
        if body["user"] == "u" && body["password"] == "p" {
            ([(header::SET_COOKIE, "frigate_token=tok1; Path=/")], "{}").into_response()
        } else {
            StatusCode::UNAUTHORIZED.into_response()
        }
    }
    async fn config(headers: HeaderMap) -> axum::response::Response {
        use axum::response::IntoResponse;
        let cookie = headers.get(header::COOKIE).and_then(|v| v.to_str().ok());
        if cookie != Some("frigate_token=tok1") {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        axum::Json(json!({"mqtt": {"password": "frigate-mqtt-secret"},
            "cameras": {"front": {"enabled": true}, "garden": {}, "old": {"enabled": false}}}))
        .into_response()
    }
    let app = Router::new()
        .route("/api/login", axum::routing::post(login))
        .route("/api/config", get(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, app).into_future());
    format!("http://{addr}")
}

#[tokio::test]
async fn frigate_test_lists_the_cameras() {
    let base = fake_frigate().await;
    let mut sections = Sections::default();
    sections.frigate.url = Some(base.clone());
    sections.frigate.user = Some("u".into());
    sections.frigate.password = Some("p".into());
    let s = serve_settings(Mode::Run, located(), sections, None, None).await;
    // The stored password, kept by leaving it out.
    let (code, body) = s
        .post("/test/frigate", json!({"url": base, "user": "u"}))
        .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(body["ok"], true, "{body}");
    let mut cameras: Vec<String> =
        serde_json::from_value(body["detail"]["cameras"].clone()).unwrap();
    cameras.sort();
    assert_eq!(cameras, ["front", "garden"]);
    assert!(!body.to_string().contains("frigate-mqtt-secret"));
    // A wrong password is reported as such.
    let (_, body) = s
        .post(
            "/test/frigate",
            json!({"url": base, "user": "u", "password": "wrong-pw-3"}),
        )
        .await;
    assert_eq!(body["ok"], false);
    assert_eq!(body["message"], "bad credentials");
    assert!(!body.to_string().contains("wrong-pw-3"));
    // Another address needs the password again.
    let (code, body) = s
        .post(
            "/test/frigate",
            json!({"url": "http://frigate.example.com:8971", "user": "u"}),
        )
        .await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "Enter the password for the new address");
    // Nothing was saved by any of it.
    assert!(!s.dir.path().join("settings-cache.json").exists());
}

/// A server that counts every request it gets (nothing should reach it).
async fn counting_server() -> (String, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&hits);
    let app = Router::new().fallback(move || {
        let seen = Arc::clone(&seen);
        async move {
            seen.fetch_add(1, Ordering::SeqCst);
            StatusCode::UNAUTHORIZED
        }
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, app).into_future());
    (format!("http://{addr}"), hits)
}

#[tokio::test]
async fn a_stored_frigate_password_with_no_url_is_not_sent_to_a_new_address() {
    let (attacker, hits) = counting_server().await;
    let mut sections = Sections::default();
    sections.frigate.password = Some(FRIGATE_PASS.into());
    let s = serve_settings(Mode::Run, located(), sections, Some(DEAD_DB), None).await;
    let (code, body) = s
        .post("/test/frigate", json!({"url": attacker, "user": "u"}))
        .await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "Enter the password for the new address");
    let (code, body) = s
        .sput("/frigate", json!({"url": attacker, "user": "u"}))
        .await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "Enter the password for the new address");
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "the fake server was reached"
    );
    // Entering the password again is allowed (it fails later, at the dead database).
    let (code, _) = s
        .sput(
            "/frigate",
            json!({"url": attacker, "user": "u", "password": "p"}),
        )
        .await;
    assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
    // With no stored password there is nothing to protect.
    let bare = serve_settings(
        Mode::Run,
        located(),
        Sections::default(),
        Some(DEAD_DB),
        None,
    )
    .await;
    let (code, _) = bare.sput("/frigate", json!({"url": attacker})).await;
    assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn a_malformed_test_body_is_refused_without_echoing_it() {
    let s = serve_settings(Mode::Run, located(), Sections::default(), None, None).await;
    let (code, body) = s.post("/test/solcast", json!({"api_key": 4242424})).await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!body.to_string().contains("4242424"), "{body}");
    let (code, body) = s.post("/test/frigate", json!({"url": 4242424})).await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!body.to_string().contains("4242424"), "{body}");
}

#[tokio::test]
async fn a_restart_that_lands_while_a_put_waits_for_the_lock_wins() {
    let s = serve_settings(
        Mode::Run,
        located(),
        Sections::default(),
        Some(DEAD_DB),
        None,
    )
    .await;
    // Hold the settings lock: the PUT passes the early check, then waits here.
    let guard = s.ctx.sections.lock().await;
    let client = s.client.clone();
    let url = s.surl("/weather");
    let put = tokio::spawn(async move {
        let r = client
            .put(url)
            .json(&json!({"enabled": false}))
            .send()
            .await
            .unwrap();
        (r.status(), r.json::<Value>().await.unwrap())
    });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    s.ctx.restarting.store(true, Ordering::SeqCst);
    drop(guard);
    let (code, body) = put.await.unwrap();
    // The re-check under the lock answers; the database was never tried.
    assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body, json!({"error": "restarting"}));
    assert!(!s.dir.path().join("settings-cache.json").exists());
}
