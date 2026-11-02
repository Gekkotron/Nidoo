//! The `mcp-stdio` proxy: the real binary spawned as a child process and driven over its
//! stdio by an rmcp client, forwarding to an in-process test HTTP server fed with fixtures
//! (no broker).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use nidoo::config::Config;
use nidoo::http::router;
use nidoo::hub::Hub;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, ReadResourceRequestParams};
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::mpsc;

/// A recorded ZBMINI switch: the `rooms/stair/light` device, with its diagnostic sensor.
const SWITCH_FIXTURE: &str = "discovery/switch__0x847127fffe24e322__switch.json";
const SWITCH_TOPIC: &str = "nidoo/switch/0x847127fffe24e322/switch/config";
const LINKQUALITY_FIXTURE: &str = "discovery/sensor__0x847127fffe24e322__linkquality.json";
const LINKQUALITY_TOPIC: &str = "nidoo/sensor/0x847127fffe24e322/linkquality/config";

const EXPECTED_TOOLS: &[&str] = &[
    "bridge_info",
    "get_home_settings",
    "set_home_settings",
    "get_settings",
    "set_settings",
    "list_areas",
    "set_areas",
    "list_devices",
    "list_entities",
    "get_entity",
    "set_override",
    "clear_override",
    "send_command",
    "search",
    "list_ignored",
    "list_flows",
    "get_flow",
    "set_flow",
    "delete_flow",
    "set_flow_enabled",
    "run_flow",
    "set_virtual",
    "delete_virtual",
    "list_network_watches",
    "set_network_watch",
    "delete_network_watch",
    "list_groups",
    "set_group",
    "delete_group",
    "list_ambiances",
    "set_ambiance",
    "delete_ambiance",
    "apply_ambiance",
    "get_history",
    "get_grid_day",
    "send_notification",
    "list_alerts",
    "climate_status",
    "get_climate_zone",
    "set_climate_settings",
    "set_climate_zone",
    "approve_climate_proposal",
    "reject_climate_proposal",
    "set_climate_override",
    "clear_climate_override",
    "set_climate_eco",
    "clear_climate_eco",
    "learn_climate_zone",
    "get_weather",
    "get_solar",
    "alarm_status",
    "set_alarm_settings",
];

fn fixtures() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../nidoo-core/tests/fixtures/"
    ))
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixtures().join(name)).expect("fixture")
}

fn config() -> Config {
    let mut cfg = Config::default();
    cfg.commands.confirm_timeout_ms = 50;
    cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
    cfg
}

struct Server {
    _hub: Arc<Hub>,
    addr: SocketAddr,
}

impl Server {
    /// A hub with the switch and its linkquality sensor discovered, served on `127.0.0.1:0`.
    async fn start() -> Server {
        let (tx, _publisher) = mpsc::channel(64);
        let hub = Hub::new(&config(), tx);
        hub.ingest(SWITCH_TOPIC, &fixture(SWITCH_FIXTURE));
        hub.ingest(LINKQUALITY_TOPIC, &fixture(LINKQUALITY_FIXTURE));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(
            axum::serve(
                listener,
                router(Arc::clone(&hub), nidoo::setup::SetupCtx::inert()),
            )
            .into_future(),
        );
        Server { _hub: hub, addr }
    }
}

/// The binary started as `nidoo mcp-stdio --server <url>`, stdin and stdout piped; its
/// stderr is the test's, kept to errors so a passing run stays quiet.
fn spawn_proxy(server_url: String) -> TokioChildProcess {
    TokioChildProcess::new(
        Command::new(assert_cmd::cargo::cargo_bin("nidoo")).configure(|cmd| {
            cmd.args(["mcp-stdio", "--server", &server_url])
                .env("RUST_LOG", "error");
        }),
    )
    .expect("spawn the proxy")
}

/// The JSON a successful tool call returned.
fn json_of(name: &str, result: &CallToolResult) -> Value {
    let text: String = result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect::<Vec<_>>()
        .join("\n");
    assert_ne!(result.is_error, Some(true), "{name} failed: {text}");
    if let Some(structured) = &result.structured_content {
        return structured.clone();
    }
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{name}: not json: {err}"))
}

#[tokio::test]
async fn stdio_proxy_forwards_list_tools() {
    let server = Server::start().await;
    let client =
        ().serve(spawn_proxy(format!("http://{}", server.addr)))
            .await
            .expect("initialize over stdio");

    let info = client.peer_info().expect("server info");
    let implementation = info.server_info.as_ref().expect("implementation");
    assert_eq!(implementation.name, "nidoo");
    let instructions = info.instructions.as_deref().expect("instructions");
    assert!(instructions.contains("list_entities"), "{instructions}");
    assert!(info.capabilities.tools.is_some(), "tools capability");
    assert!(
        info.capabilities.resources.is_some(),
        "resources capability"
    );

    let tools = client.list_all_tools().await.expect("list tools");
    let mut names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    names.sort_unstable();
    let mut expected = EXPECTED_TOOLS.to_vec();
    expected.sort_unstable();
    assert_eq!(names, expected);

    let result = client
        .call_tool(CallToolRequestParams::new("bridge_info"))
        .await
        .expect("bridge_info");
    let bridge = json_of("bridge_info", &result);
    assert_eq!(bridge["connection"]["type"], "connecting");
    assert!(bridge.get("bridge").is_some(), "{bridge}");

    let resources = client.list_all_resources().await.expect("list resources");
    assert!(
        resources.iter().any(|r| r.uri == "nidoo://snapshot"),
        "{resources:?}"
    );
    let read = client
        .read_resource(ReadResourceRequestParams::new("nidoo://snapshot"))
        .await
        .expect("read resource");
    let text = match read.contents.first().expect("one content") {
        rmcp::model::ResourceContents::TextResourceContents { text, .. } => text.clone(),
        other => panic!("expected text contents, got {other:?}"),
    };
    let snapshot: Value = serde_json::from_str(&text).expect("json snapshot");
    assert_eq!(snapshot["entities"].as_array().map(Vec::len), Some(2));

    // An upstream error crosses the proxy as an error, not as a fabricated answer.
    let missing = client
        .read_resource(ReadResourceRequestParams::new("nidoo://nope"))
        .await;
    assert!(missing.is_err(), "unknown resource must be an error");

    // Closing the session closes the child's stdin and kills the process.
    client.cancel().await.expect("close");
}

#[test]
fn stdio_proxy_reports_an_unreachable_server() {
    let output = assert_cmd::Command::cargo_bin("nidoo")
        .expect("binary")
        .args(["mcp-stdio", "--server", "http://127.0.0.1:1"])
        .output()
        .expect("run the proxy");
    assert!(!output.status.success(), "{:?}", output.status);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot reach"), "{stderr}");
    assert!(
        stderr.contains("http://127.0.0.1:1/mcp"),
        "the message names the URL: {stderr}"
    );
    assert!(
        output.stdout.is_empty(),
        "stdout is the protocol channel: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
