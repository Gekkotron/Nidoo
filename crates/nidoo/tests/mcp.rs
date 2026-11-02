//! The MCP server at `/mcp`, driven by rmcp's streamable HTTP client over a hub fed with
//! fixtures (no broker).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use nidoo::config::Config;
use nidoo::http::router;
use nidoo::hub::Hub;
use nidoo_api::ConnectionStatus;
use nidoo_core::command::Publish;
use rmcp::model::{CallToolRequestParams, CallToolResult, ReadResourceRequestParams};
use rmcp::service::RunningService;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

type Client = RunningService<RoleClient, ()>;

/// A recorded ZBMINI switch: the `rooms/stair/light` device, with its diagnostic sensor.
const SWITCH_FIXTURE: &str = "discovery/switch__0x847127fffe24e322__switch.json";
const SWITCH_ID: &str = "0x847127fffe24e322_switch_zigbee2mqtt";
const SWITCH_TOPIC: &str = "nidoo/switch/0x847127fffe24e322/switch/config";
const LINKQUALITY_FIXTURE: &str = "discovery/sensor__0x847127fffe24e322__linkquality.json";
const LINKQUALITY_ID: &str = "0x847127fffe24e322_linkquality_zigbee2mqtt";
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
    "list_groups",
    "set_group",
    "delete_group",
    "list_network_watches",
    "set_network_watch",
    "delete_network_watch",
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

const LED_ID: &str = "0x60a423fffe94d1c6_light_zigbee2mqtt";
const LED_TOPIC: &str = "nidoo/light/0x60a423fffe94d1c6/light/config";
const LED_FIXTURE: &str = "discovery/light__0x60a423fffe94d1c6__light.json";
const MOTION_ID: &str = "0x00158d008b5a2d65_occupancy_zigbee2mqtt";
const MOTION_TOPIC: &str = "nidoo/binary_sensor/0x00158d008b5a2d65/occupancy/config";
const MOTION_FIXTURE: &str = "discovery/binary_sensor__0x00158d008b5a2d65__occupancy.json";

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
    hub: Arc<Hub>,
    publisher: mpsc::Receiver<Publish>,
    addr: SocketAddr,
}

impl Server {
    /// A hub with the switch and its linkquality sensor discovered, served on `127.0.0.1:0`.
    async fn start() -> Server {
        Server::start_with(config()).await
    }

    /// [`Server::start`] with the given config.
    async fn start_with(cfg: Config) -> Server {
        let (tx, publisher) = mpsc::channel(64);
        let hub = Hub::new(&cfg, tx);
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
        Server {
            hub,
            publisher,
            addr,
        }
    }

    /// A [`Server::start`] hub that also knows the LED and the motion sensor, with the broker
    /// link up, so flows can be written. (A separate constructor: the other tests count entities.)
    async fn start_with_flows() -> Server {
        let server = Server::start().await;
        server.hub.ingest(LED_TOPIC, &fixture(LED_FIXTURE));
        server.hub.ingest(MOTION_TOPIC, &fixture(MOTION_FIXTURE));
        server.online();
        server
    }

    /// Marks the broker link up, so the hub's writers publish.
    fn online(&self) {
        self.hub.set_connection(ConnectionStatus::Online);
    }

    /// [`Server::start`] with `ctx` as the hub's setup context, so the settings tools work.
    async fn start_with_setup(ctx: Arc<nidoo::setup::SetupCtx>) -> Server {
        let (tx, publisher) = mpsc::channel(64);
        let hub = Hub::new(&config(), tx);
        hub.set_setup(Arc::clone(&ctx));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(axum::serve(listener, router(Arc::clone(&hub), ctx)).into_future());
        Server {
            hub,
            publisher,
            addr,
        }
    }

    /// An initialized MCP client session against `/mcp`.
    async fn client(&self) -> Client {
        let transport =
            StreamableHttpClientTransport::from_uri(format!("http://{}/mcp", self.addr));
        ().serve(transport).await.expect("mcp initialize")
    }
}

async fn call(client: &Client, name: &'static str, args: Value) -> CallToolResult {
    let arguments = match args {
        Value::Object(map) => map,
        other => panic!("tool arguments must be an object, got {other}"),
    };
    client
        .call_tool(CallToolRequestParams::new(name).with_arguments(arguments))
        .await
        .unwrap_or_else(|err| panic!("calling {name}: {err}"))
}

/// The JSON a successful tool call returned.
fn json_of(name: &str, result: &CallToolResult) -> Value {
    assert_ne!(
        result.is_error,
        Some(true),
        "{name} failed: {}",
        text_of(result)
    );
    if let Some(structured) = &result.structured_content {
        return structured.clone();
    }
    serde_json::from_str(&text_of(result)).unwrap_or_else(|err| panic!("{name}: not json: {err}"))
}

/// The text content of a tool result, concatenated.
fn text_of(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn lists_the_expected_tools() {
    let server = Server::start().await;
    let client = server.client().await;

    let info = client.peer_info().expect("server info");
    let implementation = info.server_info.as_ref().expect("implementation");
    assert_eq!(implementation.name, "nidoo");
    assert_eq!(implementation.version, env!("CARGO_PKG_VERSION"));
    let instructions = info.instructions.as_deref().expect("instructions");
    assert!(instructions.contains("list_entities"), "{instructions}");
    assert!(instructions.contains("set_override"), "{instructions}");
    assert!(instructions.contains("send_command"), "{instructions}");
    assert!(instructions.contains("send_notification"), "{instructions}");
    assert!(instructions.contains("optional `field`"), "{instructions}");
    assert!(instructions.contains("list_alerts"), "{instructions}");
    assert!(instructions.contains("`notify`"), "{instructions}");
    assert!(instructions.contains("climate_status"), "{instructions}");
    assert!(instructions.contains("nidoo_alarm_house"), "{instructions}");

    let tools = client.list_all_tools().await.expect("list tools");
    let mut names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    names.sort_unstable();
    let mut expected = EXPECTED_TOOLS.to_vec();
    expected.sort_unstable();
    assert_eq!(names, expected);
    for tool in &tools {
        assert!(
            tool.description.as_deref().is_some_and(|d| !d.is_empty()),
            "{} has no description",
            tool.name
        );
    }
    let list_entities = tools
        .iter()
        .find(|tool| tool.name == "list_entities")
        .expect("list_entities");
    let properties = list_entities.input_schema["properties"]
        .as_object()
        .expect("properties");
    for field in [
        "area",
        "kind",
        "device_id",
        "query",
        "changed_since",
        "include",
        "limit",
        "offset",
    ] {
        let property = &properties[field];
        assert!(
            property.get("description").is_some(),
            "list_entities.{field} is undocumented: {property}"
        );
    }

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn list_entities_primary_hides_diagnostics_and_all_shows_them() {
    let server = Server::start().await;
    let client = server.client().await;

    let primary = json_of(
        "list_entities",
        &call(&client, "list_entities", json!({})).await,
    );
    assert_eq!(primary["total"], 1);
    assert_eq!(primary["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(primary["items"][0]["id"], SWITCH_ID);

    let all = json_of(
        "list_entities",
        &call(&client, "list_entities", json!({"include": "all"})).await,
    );
    assert_eq!(all["total"], 2);
    let ids: Vec<&str> = all["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert!(ids.contains(&SWITCH_ID), "{ids:?}");
    assert!(ids.contains(&LINKQUALITY_ID), "{ids:?}");

    let paged = json_of(
        "list_entities",
        &call(
            &client,
            "list_entities",
            json!({"include": "all", "limit": 1, "offset": 1}),
        )
        .await,
    );
    assert_eq!(paged["total"], 2);
    assert_eq!(paged["items"].as_array().map(Vec::len), Some(1));

    let by_kind = json_of(
        "list_entities",
        &call(
            &client,
            "list_entities",
            json!({"include": "all", "kind": "sensor"}),
        )
        .await,
    );
    assert_eq!(by_kind["total"], 1);
    assert_eq!(by_kind["items"][0]["id"], LINKQUALITY_ID);

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn set_override_round_trip_through_tool() {
    let mut server = Server::start().await;
    let client = server.client().await;

    // The broker link is still connecting: the write is refused in-band, nothing is queued.
    let refused = call(
        &client,
        "set_override",
        json!({"id": SWITCH_ID, "patch": {"showAs": "light"}}),
    )
    .await;
    assert_eq!(refused.is_error, Some(true));
    assert_eq!(text_of(&refused), "publish: broker offline");
    assert!(server.publisher.try_recv().is_err());
    server.online();

    let view = json_of(
        "set_override",
        &call(
            &client,
            "set_override",
            json!({"id": SWITCH_ID, "patch": {"showAs": "light"}}),
        )
        .await,
    );
    assert_eq!(view["id"], SWITCH_ID);
    assert_eq!(view["kind"], "light");
    let published = server.publisher.try_recv().expect("the override publish");
    assert_eq!(published.topic, format!("nidoo/ui/{SWITCH_ID}"));
    assert!(published.retain);

    let detail = json_of(
        "get_entity",
        &call(&client, "get_entity", json!({"id": SWITCH_ID})).await,
    );
    assert_eq!(detail["id"], SWITCH_ID);
    assert_eq!(detail["kind"], "light");
    assert_eq!(detail["raw_override"]["showAs"], "light");
    assert_eq!(
        detail["bindings"]["state_topic"],
        "zigbee2mqtt/rooms/stair/light"
    );

    let view = json_of(
        "clear_override",
        &call(&client, "clear_override", json!({"id": SWITCH_ID})).await,
    );
    assert_eq!(view["kind"], "switch");
    let published = server.publisher.try_recv().expect("the clearing publish");
    assert_eq!(published.payload, "");

    let detail = json_of(
        "get_entity",
        &call(&client, "get_entity", json!({"id": SWITCH_ID})).await,
    );
    assert_eq!(detail["kind"], "switch");
    assert_eq!(detail["raw_override"], Value::Null);

    let missing = call(&client, "get_entity", json!({"id": "nope"})).await;
    assert_eq!(missing.is_error, Some(true));
    assert!(text_of(&missing).contains("unknown entity `nope`"));

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn set_override_rejects_the_retired_as_light() {
    let mut server = Server::start().await;
    server.online();
    let client = server.client().await;

    let result = call(
        &client,
        "set_override",
        json!({"id": SWITCH_ID, "patch": {"asLight": true}}),
    )
    .await;
    assert_eq!(result.is_error, Some(true));
    let message = text_of(&result);
    assert!(
        message.contains("unknown override field `asLight`"),
        "{message}"
    );
    assert!(message.contains("showAs"), "{message}");
    assert!(server.publisher.try_recv().is_err());

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn set_override_unknown_field_is_a_tool_error_listing_fields() {
    let mut server = Server::start().await;
    let client = server.client().await;

    let result = call(
        &client,
        "set_override",
        json!({"id": SWITCH_ID, "patch": {"nope": true}}),
    )
    .await;
    assert_eq!(result.is_error, Some(true));
    let message = text_of(&result);
    assert!(
        message.contains("unknown override field `nope`"),
        "{message}"
    );
    assert!(message.contains("showAs"), "{message}");
    assert!(message.contains("lightOnBrightness"), "{message}");
    assert!(server.publisher.try_recv().is_err());

    let result = call(
        &client,
        "set_override",
        json!({"id": SWITCH_ID, "patch": {"energyRole": "nope"}}),
    )
    .await;
    assert_eq!(result.is_error, Some(true));
    let message = text_of(&result);
    assert!(message.contains("energyRole"), "{message}");
    assert!(message.contains("solar"), "{message}");
    assert!(message.contains("grid_export"), "{message}");
    assert!(server.publisher.try_recv().is_err());

    let result = call(
        &client,
        "set_override",
        json!({"id": "nope", "patch": {"favorite": true}}),
    )
    .await;
    assert_eq!(result.is_error, Some(true));
    assert!(text_of(&result).contains("unknown entity `nope`"));

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn snapshot_resource_reads() {
    let server = Server::start().await;
    let client = server.client().await;

    let resources = client.list_all_resources().await.expect("list resources");
    let snapshot = resources
        .iter()
        .find(|resource| resource.uri == "nidoo://snapshot")
        .expect("the snapshot resource");
    assert_eq!(snapshot.mime_type.as_deref(), Some("application/json"));

    let read = client
        .read_resource(ReadResourceRequestParams::new("nidoo://snapshot"))
        .await
        .expect("read resource");
    let (uri, mime_type, text) = match read.contents.first().expect("one content") {
        rmcp::model::ResourceContents::TextResourceContents {
            uri,
            mime_type,
            text,
            ..
        } => (uri.clone(), mime_type.clone(), text.clone()),
        other => panic!("expected text contents, got {other:?}"),
    };
    assert_eq!(uri, "nidoo://snapshot");
    assert_eq!(mime_type.as_deref(), Some("application/json"));
    let snapshot: Value = serde_json::from_str(&text).expect("json snapshot");
    assert_eq!(snapshot["connection"]["type"], "connecting");
    assert_eq!(snapshot["entities"].as_array().map(Vec::len), Some(2));

    let missing = client
        .read_resource(ReadResourceRequestParams::new("nidoo://nope"))
        .await;
    assert!(missing.is_err(), "unknown resource must be an error");

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn send_command_search_areas_and_the_small_tools() {
    let mut server = Server::start().await;
    let client = server.client().await;

    server
        .hub
        .ingest("zigbee2mqtt/bridge/state", br#"{"state":"offline"}"#);
    let rejected = call(
        &client,
        "send_command",
        json!({"id": SWITCH_ID, "command": {"type": "turn_off"}}),
    )
    .await;
    assert_eq!(rejected.is_error, Some(true));
    assert_eq!(text_of(&rejected), "rejected: entity is offline");
    assert!(server.publisher.try_recv().is_err());

    server
        .hub
        .ingest("zigbee2mqtt/bridge/state", br#"{"state":"online"}"#);
    server.online();
    let response = json_of(
        "send_command",
        &call(
            &client,
            "send_command",
            json!({"id": SWITCH_ID, "command": {"type": "turn_on"}}),
        )
        .await,
    );
    assert_eq!(response["accepted"], false);
    assert_eq!(response["reason"], "no state change within 50 ms");
    let published = server.publisher.try_recv().expect("the command publish");
    assert_eq!(published.topic, "zigbee2mqtt/rooms/stair/light/set");
    assert_eq!(published.payload, "ON");

    let unknown = call(
        &client,
        "send_command",
        json!({"id": "nope", "command": {"type": "turn_on"}}),
    )
    .await;
    assert_eq!(unknown.is_error, Some(true));
    assert_eq!(text_of(&unknown), "unknown entity `nope`");

    let areas = json_of(
        "set_areas",
        &call(
            &client,
            "set_areas",
            json!({"areas": [{"id": "stair", "name": "Stair", "order": 2}]}),
        )
        .await,
    );
    let stair = areas["areas"]
        .as_array()
        .expect("areas")
        .iter()
        .find(|a| a["id"] == "stair")
        .expect("the declared area");
    assert_eq!(stair["name"], "Stair");
    let published = server.publisher.try_recv().expect("the areas publish");
    assert_eq!(published.topic, "nidoo/ui/_areas");
    let listed = json_of("list_areas", &call(&client, "list_areas", json!({})).await);
    assert_eq!(listed, areas);

    // The device `rooms/stair/light` names both entities "light …"; the search is
    // case-insensitive and no `bridge/devices` was ingested, so no device matches.
    let found = json_of(
        "search",
        &call(&client, "search", json!({"query": "LIGHT"})).await,
    );
    let ids: Vec<&str> = found["entities"]
        .as_array()
        .expect("entities")
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert!(ids.contains(&SWITCH_ID), "{ids:?}");
    assert!(ids.contains(&LINKQUALITY_ID), "{ids:?}");
    assert_eq!(found["devices"].as_array().map(Vec::len), Some(0));
    let limited = json_of(
        "search",
        &call(&client, "search", json!({"query": "light", "limit": 1})).await,
    );
    assert_eq!(limited["entities"].as_array().map(Vec::len), Some(1));
    let none = json_of(
        "search",
        &call(&client, "search", json!({"query": "kitchen"})).await,
    );
    assert_eq!(none["entities"].as_array().map(Vec::len), Some(0));

    // A switch shown as a siren is found by its type word too.
    call(
        &client,
        "set_override",
        json!({"id": SWITCH_ID, "patch": {"showAs": "siren"}}),
    )
    .await;
    server.publisher.try_recv().expect("the override publish");
    let sirens = json_of(
        "search",
        &call(&client, "search", json!({"query": "Siren"})).await,
    );
    let ids: Vec<&str> = sirens["entities"]
        .as_array()
        .expect("entities")
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert_eq!(ids, [SWITCH_ID]);

    // Once the devices are known, a query matching a device (`rooms/stair/light`) also
    // returns that device's entities, although their names do not contain it.
    server.hub.ingest(
        "zigbee2mqtt/bridge/devices",
        &fixture("bridge_devices.json"),
    );
    let by_device = json_of(
        "search",
        &call(&client, "search", json!({"query": "stair"})).await,
    );
    assert!(
        by_device["devices"]
            .as_array()
            .expect("devices")
            .iter()
            .any(|d| d["friendly_name"] == "rooms/stair/light"),
        "{by_device}"
    );
    let ids: Vec<&str> = by_device["entities"]
        .as_array()
        .expect("entities")
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert!(ids.contains(&SWITCH_ID), "{ids:?}");
    assert!(ids.contains(&LINKQUALITY_ID), "{ids:?}");

    // A room is found by its name, accents, case and articles aside, and every word must
    // match: the switch moved to the room is found, the link quality left behind is not.
    call(
        &client,
        "set_areas",
        json!({"areas": [
            {"id": "stair", "name": "Stair", "order": 2},
            {"id": "living", "name": "Séjour", "order": 0},
        ]}),
    )
    .await;
    server.publisher.try_recv().expect("the areas publish");
    call(
        &client,
        "set_override",
        json!({"id": SWITCH_ID, "patch": {"area": "living"}}),
    )
    .await;
    server.publisher.try_recv().expect("the override publish");
    let in_room = json_of(
        "search",
        &call(&client, "search", json!({"query": "stair du SEJOUR"})).await,
    );
    let ids: Vec<&str> = in_room["entities"]
        .as_array()
        .expect("entities")
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert_eq!(ids, [SWITCH_ID]);

    let devices = json_of(
        "list_devices",
        &call(&client, "list_devices", json!({"query": "zbmini"})).await,
    );
    assert!(devices["devices"].is_array());

    let bridge = json_of(
        "bridge_info",
        &call(&client, "bridge_info", json!({})).await,
    );
    assert_eq!(bridge["connection"]["type"], "online");
    assert_eq!(bridge["bridge"]["state"], "online");

    let ignored = json_of(
        "list_ignored",
        &call(&client, "list_ignored", json!({})).await,
    );
    assert!(ignored["ignored"].is_array());

    client.cancel().await.expect("close");
}

/// The reference automation: LED on at motion, off five minutes after the last motion.
fn reference_flow() -> Value {
    json!({
        "id": "living", "name": "Living LED on motion", "enabled": true, "mode": "restart",
        "nodes": [
            { "id": "motion", "type": "trigger.state", "params": { "entity": MOTION_ID } },
            { "id": "moving", "type": "if", "params": { "op": "eq", "value": true } },
            { "id": "on", "type": "command",
              "params": { "entity": LED_ID, "command": { "type": "turn_on" } } },
            { "id": "wait", "type": "delay", "params": { "for": "5m" } },
            { "id": "off", "type": "command",
              "params": { "entity": LED_ID, "command": { "type": "turn_off" } } }
        ],
        "edges": [
            { "from": "motion", "to": "moving" },
            { "from": "moving", "port": "true", "to": "on" },
            { "from": "moving", "port": "false", "to": "wait" },
            { "from": "wait", "to": "off" }
        ]
    })
}

#[tokio::test]
async fn set_flow_then_get_and_list() {
    let server = Server::start_with_flows().await;
    let client = server.client().await;

    let set = json_of(
        "set_flow",
        &call(&client, "set_flow", json!({ "flow": reference_flow() })).await,
    );
    assert_eq!(set["flow"]["definition"]["id"], "living");
    assert_eq!(set["flow"]["invalid"], Value::Null);

    let got = json_of(
        "get_flow",
        &call(&client, "get_flow", json!({ "id": "living" })).await,
    );
    assert_eq!(got["definition"]["name"], "Living LED on motion");

    let list = json_of("list_flows", &call(&client, "list_flows", json!({})).await);
    let flows = list["flows"].as_array().expect("flows");
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0]["id"], "living");
    assert_eq!(flows[0]["enabled"], true);
    assert_eq!(flows[0]["nodes"], 5);
    assert_eq!(flows[0]["last_run"], Value::Null);
    assert_eq!(flows[0]["area"], Value::Null);

    let missing = call(&client, "get_flow", json!({ "id": "nope" })).await;
    assert_eq!(missing.is_error, Some(true));
    assert!(
        text_of(&missing).contains("unknown flow"),
        "{}",
        text_of(&missing)
    );

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn set_flow_with_area_lists_it() {
    let server = Server::start_with_flows().await;
    let client = server.client().await;

    call(
        &client,
        "set_areas",
        json!({"areas": [{"id": "living", "name": "Living room"}]}),
    )
    .await;

    let mut flow = reference_flow();
    flow["area"] = json!("living");
    let set = call(&client, "set_flow", json!({ "flow": flow.clone() })).await;
    assert_ne!(set.is_error, Some(true), "{}", text_of(&set));

    let list = json_of("list_flows", &call(&client, "list_flows", json!({})).await);
    assert_eq!(list["flows"][0]["area"], "living");

    flow["area"] = json!("ghost");
    let bad = call(&client, "set_flow", json!({ "flow": flow })).await;
    assert_eq!(bad.is_error, Some(true));
    assert!(text_of(&bad).contains("unknown area"), "{}", text_of(&bad));

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn mcp_set_flow_returns_wrapper_and_warning() {
    let server = Server::start_with_flows().await;
    let client = server.client().await;

    let flow = json!({ "id": "s", "name": "S", "nodes": [{
        "id": "ev", "type": "trigger.sun", "params": { "which": "sunset" }
    }] });
    let out = json_of(
        "set_flow",
        &call(&client, "set_flow", json!({ "flow": flow })).await,
    );
    assert!(
        out["warning"]
            .as_str()
            .is_some_and(|w| w.contains("Paris fallback")),
        "{out}"
    );
    assert_eq!(out["flow"]["definition"]["id"], "s");

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn set_flow_reports_validation_in_band() {
    let server = Server::start_with_flows().await;
    let client = server.client().await;

    let mut flow = reference_flow();
    flow["edges"][1] = json!({ "from": "moving", "to": "on" });
    let result = call(&client, "set_flow", json!({ "flow": flow })).await;
    assert_eq!(result.is_error, Some(true));
    assert!(text_of(&result).contains("port"), "{}", text_of(&result));
    assert!(server.hub.flow("living").is_none());

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn set_flow_accepts_a_math_card() {
    let server = Server::start_with_flows().await;
    let client = server.client().await;
    json_of(
        "set_virtual",
        &call(
            &client,
            "set_virtual",
            json!({ "id": "heater_target", "spec": { "kind": "number", "name": "Heater target",
                "min": 0, "max": 2200, "step": 1 } }),
        )
        .await,
    );

    let flow = json!({ "id": "water", "name": "Heater target from grid", "nodes": [
        { "id": "grid", "type": "trigger.state", "params": { "entity": LINKQUALITY_ID } },
        { "id": "target", "type": "math", "params": {
            "inputs": [{ "name": "grid", "entity": LINKQUALITY_ID }],
            "formula": "clamp(0 - grid, 0, 2200)",
            "name": "desired",
            "save_to": "virtual_heater_target" } }
    ], "edges": [{ "from": "grid", "to": "target" }] });
    let set = json_of(
        "set_flow",
        &call(&client, "set_flow", json!({ "flow": flow.clone() })).await,
    );
    assert_eq!(set["flow"]["invalid"], Value::Null, "{set}");

    let got = json_of(
        "get_flow",
        &call(&client, "get_flow", json!({ "id": "water" })).await,
    );
    let math = &got["definition"]["nodes"][1];
    assert_eq!(math["type"], "math", "{got}");
    assert_eq!(math["params"]["name"], "desired");
    assert_eq!(math["params"]["save_to"], "virtual_heater_target");

    // A saved result must go to a number entity: a sensor is refused in band.
    let mut bad = flow;
    bad["nodes"][1]["params"]["save_to"] = json!(LINKQUALITY_ID);
    let refused = call(&client, "set_flow", json!({ "flow": bad })).await;
    assert_eq!(refused.is_error, Some(true));
    assert!(
        text_of(&refused).contains("is not a number entity"),
        "{}",
        text_of(&refused)
    );

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn set_flow_enabled_and_run_flow() {
    let server = Server::start_with_flows().await;
    let client = server.client().await;
    json_of(
        "set_flow",
        &call(&client, "set_flow", json!({ "flow": reference_flow() })).await,
    );

    let off = json_of(
        "set_flow_enabled",
        &call(
            &client,
            "set_flow_enabled",
            json!({ "id": "living", "enabled": false }),
        )
        .await,
    );
    assert_eq!(off["definition"]["enabled"], false);
    let refused = call(&client, "run_flow", json!({ "id": "living" })).await;
    assert_eq!(refused.is_error, Some(true));
    assert!(
        text_of(&refused).contains("disabled"),
        "{}",
        text_of(&refused)
    );

    let on = json_of(
        "set_flow_enabled",
        &call(
            &client,
            "set_flow_enabled",
            json!({ "id": "living", "enabled": true }),
        )
        .await,
    );
    assert_eq!(on["definition"]["enabled"], true);
    let run = json_of(
        "run_flow",
        &call(&client, "run_flow", json!({ "id": "living" })).await,
    );
    assert_eq!(run["flow_id"], "living");
    assert_eq!(run["status"], "running");

    let list = json_of("list_flows", &call(&client, "list_flows", json!({})).await);
    assert_eq!(list["flows"][0]["last_run"]["status"], "running");
    assert!(list["flows"][0]["last_run"]["started_at"].is_u64());

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn run_flow_takes_a_manual_card() {
    let server = Server::start_with_flows().await;
    let client = server.client().await;
    let mut flow = reference_flow();
    flow["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "id": "test_on", "type": "trigger.manual", "params": { "value": true } }));
    flow["edges"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "from": "test_on", "to": "on" }));
    json_of(
        "set_flow",
        &call(&client, "set_flow", json!({ "flow": flow })).await,
    );

    let refused = call(
        &client,
        "run_flow",
        json!({ "id": "living", "node": "motion" }),
    )
    .await;
    assert_eq!(refused.is_error, Some(true));
    assert!(
        text_of(&refused).contains("not a manual trigger"),
        "{}",
        text_of(&refused)
    );

    let run = json_of(
        "run_flow",
        &call(
            &client,
            "run_flow",
            json!({ "id": "living", "node": "test_on" }),
        )
        .await,
    );
    assert_eq!(run["flow_id"], "living");
    assert_eq!(run["status"], "running");
    assert_eq!(run["trigger"]["value"], json!(true), "{run}");

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn delete_flow() {
    let mut server = Server::start_with_flows().await;
    let client = server.client().await;
    json_of(
        "set_flow",
        &call(&client, "set_flow", json!({ "flow": reference_flow() })).await,
    );
    while server.publisher.try_recv().is_ok() {}

    let deleted = json_of(
        "delete_flow",
        &call(&client, "delete_flow", json!({ "id": "living" })).await,
    );
    assert_eq!(deleted, json!({ "id": "living" }));
    let published = server.publisher.try_recv().expect("retained clear");
    assert_eq!(published.topic, "nidoo/automation/living");
    assert!(published.payload.is_empty() && published.retain);
    let list = json_of("list_flows", &call(&client, "list_flows", json!({})).await);
    assert_eq!(list["flows"].as_array().map(Vec::len), Some(0));

    let again = call(&client, "delete_flow", json!({ "id": "living" })).await;
    assert_eq!(again.is_error, Some(true));

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn mcp_set_areas_keeps_group_fields() {
    let mut server = Server::start().await;
    server.online();
    server.hub.ingest(
        "nidoo/ui/_areas",
        br#"{"areas":[{"id":"dehors","name":"Dehors","icon":"tree","groupNames":{"cover":"Volets"},"groupFavorites":["cover"],"newKey":true},{"id":"gone","name":"Gone"}]}"#,
    );
    let client = server.client().await;

    let result = call(
        &client,
        "set_areas",
        json!({"areas": [{"id": "dehors", "name": "Outdoor", "order": 3}]}),
    )
    .await;
    json_of("set_areas", &result);
    let published = server.publisher.try_recv().expect("the areas publish");
    let doc: Value = serde_json::from_str(&published.payload).expect("json");
    assert_eq!(
        doc,
        json!({"areas": [{
            "id": "dehors", "name": "Outdoor", "order": 3, "icon": "tree",
            "groupNames": {"cover": "Volets"}, "groupFavorites": ["cover"], "newKey": true
        }]})
    );
}

#[tokio::test]
async fn mcp_list_then_set_areas_round_trips() {
    let mut server = Server::start().await;
    server.online();
    server.hub.ingest(
        "nidoo/ui/_areas",
        br#"{"areas":[{"id":"dehors","name":"Dehors","icon":"tree","groupNames":{"cover":"Volets"},"groupFavorites":["cover"],"newKey":true}]}"#,
    );
    let client = server.client().await;

    let mut listed = json_of("list_areas", &call(&client, "list_areas", json!({})).await);
    // The fixture house also lists `stair`, derived from its entities; sending it back would
    // declare it, which is not what this test is about.
    listed["areas"]
        .as_array_mut()
        .expect("areas")
        .retain(|a| a["id"] == "dehors");
    listed["areas"][0]["name"] = json!("Outdoor");
    json_of(
        "set_areas",
        &call(&client, "set_areas", listed.clone()).await,
    );
    let published = server.publisher.try_recv().expect("the areas publish");
    let doc: Value = serde_json::from_str(&published.payload).expect("json");
    assert_eq!(
        doc,
        json!({"areas": [{
            "id": "dehors", "name": "Outdoor", "icon": "tree",
            "groupNames": {"cover": "Volets"}, "groupFavorites": ["cover"], "newKey": true
        }]})
    );

    // Editing the listed `group_names` is applied.
    listed["areas"][0]["group_names"] = json!({"cover": "Stores"});
    json_of("set_areas", &call(&client, "set_areas", listed).await);
    let published = server.publisher.try_recv().expect("the second publish");
    let doc: Value = serde_json::from_str(&published.payload).expect("json");
    assert_eq!(
        doc["areas"][0]["groupNames"],
        json!({"cover": "Stores"}),
        "{doc}"
    );
    assert!(doc["areas"][0].get("group_names").is_none(), "{doc}");
    assert_eq!(doc["areas"][0]["newKey"], json!(true));

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn mcp_set_virtual_lists_the_entity() {
    let mut server = Server::start().await;
    server.online();
    let client = server.client().await;
    let view = json_of(
        "set_virtual",
        &call(
            &client,
            "set_virtual",
            json!({ "id": "force_heating", "spec": { "kind": "switch", "name": "Force heating" } }),
        )
        .await,
    );
    assert_eq!(view["id"], "virtual_force_heating");

    let list = json_of(
        "list_entities",
        &call(&client, "list_entities", json!({ "query": "force" })).await,
    );
    assert_eq!(list["total"], 1);

    while server.publisher.try_recv().is_ok() {}
    let deleted = json_of(
        "delete_virtual",
        &call(&client, "delete_virtual", json!({ "id": "force_heating" })).await,
    );
    assert_eq!(deleted, json!({ "id": "force_heating" }));
    assert_eq!(
        server.publisher.try_recv().expect("config clear").topic,
        "nidoo/switch/virtual/force_heating/config"
    );
    let again = call(&client, "delete_virtual", json!({ "id": "force_heating" })).await;
    assert_eq!(again.is_error, Some(true));

    client.cancel().await.expect("close");
}

/// Flexible groups switched on, with the broker link up so the group writers publish.
async fn start_with_groups() -> Server {
    let mut cfg = config();
    cfg.groups.enabled = true;
    let server = Server::start_with(cfg).await;
    server.online();
    server
}

fn switch_group_spec() -> Value {
    json!({ "kind": "switch", "name": "Stair", "members": [SWITCH_ID], "hideMembers": true })
}

#[tokio::test]
async fn mcp_set_then_list_ambiance() {
    let server = start_with_groups().await;
    let client = server.client().await;

    let view = json_of(
        "set_ambiance",
        &call(
            &client,
            "set_ambiance",
            json!({ "ambiance": {
                "id": "night",
                "name": "Night",
                "actions": [{ "type": "category", "what": "lights", "to": "off" }]
            } }),
        )
        .await,
    );
    assert_eq!(view["id"], "night");

    let list = json_of(
        "list_ambiances",
        &call(&client, "list_ambiances", json!({})).await,
    );
    assert_eq!(list["ambiances"].as_array().map(Vec::len), Some(1));
    assert_eq!(list["ambiances"][0]["ambiance"]["name"], "Night");

    let deleted = json_of(
        "delete_ambiance",
        &call(&client, "delete_ambiance", json!({ "id": "night" })).await,
    );
    assert_eq!(deleted, json!({ "id": "night" }));

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn mcp_apply_unknown_ambiance_errors() {
    let server = start_with_groups().await;
    let client = server.client().await;

    let result = call(&client, "apply_ambiance", json!({ "id": "nope" })).await;
    assert_eq!(result.is_error, Some(true));
    assert!(text_of(&result).contains("unknown"), "{}", text_of(&result));

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn mcp_list_groups_is_empty_before_any_set_group() {
    let server = start_with_groups().await;
    let client = server.client().await;

    let list = json_of(
        "list_groups",
        &call(&client, "list_groups", json!({})).await,
    );
    assert_eq!(list, json!({ "groups": [] }));

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn mcp_set_group_round_trips_to_the_hub() {
    let server = start_with_groups().await;
    let client = server.client().await;

    let view = json_of(
        "set_group",
        &call(
            &client,
            "set_group",
            json!({ "id": "stair", "spec": switch_group_spec() }),
        )
        .await,
    );
    assert_eq!(view["id"], "stair");
    assert_eq!(view["entity"]["id"], "group_stair");
    assert_eq!(view["members"].as_array().map(Vec::len), Some(1));

    let stored = server.hub.list_groups().expect("groups");
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].id, "stair");

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn mcp_set_network_watch_round_trips() {
    let server = Server::start().await;
    server.online();
    let client = server.client().await;
    let view = json_of(
        "set_network_watch",
        &call(
            &client,
            "set_network_watch",
            json!({ "id": "nas", "spec": { "name": "NAS", "host": "nas.local" } }),
        )
        .await,
    );
    assert_eq!(view["entity"], "network_nas");

    let list = json_of(
        "list_network_watches",
        &call(&client, "list_network_watches", json!({})).await,
    );
    assert_eq!(list["watches"].as_array().map(Vec::len), Some(1));

    let deleted = json_of(
        "delete_network_watch",
        &call(&client, "delete_network_watch", json!({ "id": "nas" })).await,
    );
    assert_eq!(deleted, json!({ "id": "nas" }));

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn mcp_delete_group_unknown_id_is_a_tool_error() {
    let server = start_with_groups().await;
    let client = server.client().await;

    let result = call(&client, "delete_group", json!({ "id": "nope" })).await;
    assert_eq!(result.is_error, Some(true));
    assert!(text_of(&result).contains("unknown"), "{}", text_of(&result));

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn get_history_without_history_reports_unavailable() {
    let server = Server::start().await;
    let client = server.client().await;

    let result = call(
        &client,
        "get_history",
        json!({ "entity": "x", "what": "series", "range": "24h" }),
    )
    .await;
    assert_eq!(result.is_error, Some(true));
    let text = text_of(&result);
    assert!(text.contains("history_unavailable"), "{text}");

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn get_history_is_listed() {
    let server = Server::start().await;
    let client = server.client().await;

    let tools = client.list_all_tools().await.expect("list tools");
    let tool = tools
        .iter()
        .find(|tool| tool.name == "get_history")
        .expect("get_history is listed");
    assert_eq!(
        tool.annotations.as_ref().and_then(|a| a.read_only_hint),
        Some(true)
    );

    client.cancel().await.expect("close");
}

#[tokio::test]
async fn get_grid_day_is_listed_read_only() {
    let server = Server::start().await;
    let client = server.client().await;

    let tools = client.list_all_tools().await.expect("list tools");
    let tool = tools
        .iter()
        .find(|tool| tool.name == "get_grid_day")
        .expect("get_grid_day is listed");
    assert_eq!(
        tool.annotations.as_ref().and_then(|a| a.read_only_hint),
        Some(true)
    );

    let result = call(&client, "get_grid_day", json!({})).await;
    assert_eq!(result.is_error, Some(true));
    let text = text_of(&result);
    assert!(text.contains("history_unavailable"), "{text}");

    client.cancel().await.expect("close");
}

const HISTORY_TOPIC: &str = "nidoo/push/history";

fn history_entry(id: &str, channel: &str, sent_at: u64) -> String {
    format!(
        r#"{{"v":1,"id":"{id}","channel":"{channel}","severity":"info","title":"T {id}","body":"","silent":false,"sent_at":{sent_at},"source":"test"}}"#
    )
}

#[tokio::test]
async fn send_notification_tool_returns_id() {
    let server = Server::start().await;
    let client = server.client().await;

    let result = call(
        &client,
        "send_notification",
        json!({"title": "Washer done", "channel": "energy", "severity": "warning"}),
    )
    .await;
    let id = json_of("send_notification", &result)["id"]
        .as_str()
        .expect("id")
        .to_owned();
    assert!(id.starts_with("mcp-"), "{id}");

    let fixed = call(
        &client,
        "send_notification",
        json!({"title": "Dryer", "id": "dryer"}),
    )
    .await;
    assert_eq!(json_of("send_notification", &fixed)["id"], "dryer");
}

#[tokio::test]
async fn send_notification_rejects_missing_title() {
    let server = Server::start().await;
    let client = server.client().await;

    let blank = call(&client, "send_notification", json!({"title": "  "})).await;
    assert_eq!(blank.is_error, Some(true));
    assert!(text_of(&blank).contains("title"), "{}", text_of(&blank));

    let channel = call(
        &client,
        "send_notification",
        json!({"title": "T", "channel": "pager"}),
    )
    .await;
    assert_eq!(channel.is_error, Some(true));
    assert!(
        text_of(&channel).contains("unknown channel `pager`"),
        "{}",
        text_of(&channel)
    );

    let severity = call(
        &client,
        "send_notification",
        json!({"title": "T", "severity": "loud"}),
    )
    .await;
    assert_eq!(severity.is_error, Some(true));
    assert!(
        text_of(&severity).contains("unknown severity `loud`"),
        "{}",
        text_of(&severity)
    );

    let absent = client
        .call_tool(
            CallToolRequestParams::new("send_notification").with_arguments(serde_json::Map::new()),
        )
        .await;
    let refused = match absent {
        Err(_) => true,
        Ok(result) => result.is_error == Some(true),
    };
    assert!(refused, "a call without title must be refused");
}

#[tokio::test]
async fn list_alerts_filters_by_channel_and_limits() {
    let server = Server::start().await;
    let client = server.client().await;
    let history = format!(
        "[{},{},{}]",
        history_entry("c", "camera", 300),
        history_entry("b", "energy", 200),
        history_entry("a", "camera", 100)
    );
    server.hub.ingest(HISTORY_TOPIC, history.as_bytes());

    let all = json_of(
        "list_alerts",
        &call(&client, "list_alerts", json!({})).await,
    );
    let ids: Vec<&str> = all["alerts"]
        .as_array()
        .expect("alerts")
        .iter()
        .map(|a| a["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, ["c", "b", "a"]);

    let camera = json_of(
        "list_alerts",
        &call(&client, "list_alerts", json!({"channel": "camera"})).await,
    );
    let ids: Vec<&str> = camera["alerts"]
        .as_array()
        .expect("alerts")
        .iter()
        .map(|a| a["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, ["c", "a"]);

    let one = json_of(
        "list_alerts",
        &call(&client, "list_alerts", json!({"limit": 1})).await,
    );
    assert_eq!(one["alerts"].as_array().map(Vec::len), Some(1));
    assert_eq!(one["alerts"][0]["id"], "c");
}

#[tokio::test]
async fn climate_status_disabled_error() {
    let server = Server::start().await;
    let client = server.client().await;
    let result = call(&client, "climate_status", json!({})).await;
    assert_eq!(result.is_error, Some(true));
    assert!(
        text_of(&result).contains("climate disabled"),
        "{}",
        text_of(&result)
    );
    let result = call(&client, "learn_climate_zone", json!({"zone": "living"})).await;
    assert_eq!(result.is_error, Some(true));
}

#[tokio::test]
async fn climate_status_enabled_returns_zones() {
    let mut cfg = config();
    cfg.climate.enabled = true;
    let server = Server::start_with(cfg).await;
    server.online();
    server.hub.ingest(
        "nidoo/climate/daikin_192_168_1_150/climate/config",
        &fixture("discovery/climate__daikin_192_168_1_150__climate.json"),
    );
    let id = server
        .hub
        .snapshot()
        .entities
        .into_iter()
        .find(|e| e.id.starts_with("daikin_192_168_1_150"))
        .expect("daikin")
        .id;
    let mut patch = serde_json::Map::new();
    patch.insert("area".to_owned(), Value::from("living"));
    server.hub.set_override(&id, &patch).await.expect("area");

    let client = server.client().await;
    let status = json_of(
        "climate_status",
        &call(&client, "climate_status", json!({})).await,
    );
    assert_eq!(status["zones"][0]["zone"], "living");

    let zone = json_of(
        "get_climate_zone",
        &call(&client, "get_climate_zone", json!({"zone": "living"})).await,
    );
    assert_eq!(zone["zone"]["zone"], "living");
    assert!(zone["decisions"].is_array());

    // A pending zone does not follow overrides: an in-band refusal.
    let pending = call(
        &client,
        "set_climate_override",
        json!({"zone": "living", "setpoint": 21.0, "minutes": 30}),
    )
    .await;
    assert_eq!(pending.is_error, Some(true));
    assert!(
        text_of(&pending).contains("zone is not active"),
        "{}",
        text_of(&pending)
    );
    json_of(
        "set_climate_zone",
        &call(
            &client,
            "set_climate_zone",
            json!({"zone": "living", "settings": {"state": "active"}}),
        )
        .await,
    );

    let held = call(
        &client,
        "set_climate_override",
        json!({"zone": "living", "setpoint": 21.0, "minutes": 30}),
    )
    .await;
    json_of("set_climate_override", &held);
    let both = call(
        &client,
        "set_climate_override",
        json!({"zone": "living", "setpoint": 21.0, "minutes": 30, "until": 4_000_000_000u64}),
    )
    .await;
    assert_eq!(both.is_error, Some(true));
    json_of(
        "clear_climate_override",
        &call(&client, "clear_climate_override", json!({"zone": "living"})).await,
    );
    json_of(
        "set_climate_eco",
        &call(
            &client,
            "set_climate_eco",
            json!({"zone": "living", "until": 4_000_000_000u64}),
        )
        .await,
    );
    let eco_of = |server: &Server| {
        server
            .hub
            .climate_view()
            .unwrap()
            .zones
            .into_iter()
            .find(|z| z.zone == "living")
            .and_then(|z| z.eco)
    };
    assert_eq!(eco_of(&server).map(|e| e.until), Some(4_000_000_000));
    let past = call(
        &client,
        "set_climate_eco",
        json!({"zone": "living", "until": 1}),
    )
    .await;
    assert_eq!(past.is_error, Some(true));
    json_of(
        "clear_climate_eco",
        &call(&client, "clear_climate_eco", json!({"zone": "living"})).await,
    );
    assert_eq!(eco_of(&server), None);
    let nothing = call(
        &client,
        "approve_climate_proposal",
        json!({"zone": "living"}),
    )
    .await;
    assert_eq!(nothing.is_error, Some(true));
}

#[tokio::test]
async fn set_climate_override_schema_accepts_zone_with_the_fields() {
    let server = Server::start().await;
    let client = server.client().await;
    let tools = client.list_all_tools().await.expect("list tools");
    let tool = tools
        .iter()
        .find(|t| t.name == "set_climate_override")
        .expect("tool");
    let schema = Value::Object((*tool.input_schema).clone());
    let props = schema["properties"].as_object().expect("properties");
    for field in ["zone", "setpoint", "mode", "minutes", "until"] {
        assert!(props.contains_key(field), "{field} missing: {schema}");
    }
    // `additionalProperties: false` is fine only because every field is listed above.
    assert_eq!(schema["required"], json!(["zone"]), "{schema}");
}

#[tokio::test]
async fn alarm_tools_read_and_replace_settings() {
    let server = Server::start().await;
    let client = server.client().await;
    let status = call(&client, "alarm_status", json!({})).await;
    let view = json_of("alarm_status", &status);
    assert_eq!(view["enabled"], false);
    assert_eq!(view["entity_id"], "nidoo_alarm_house");

    let bad = call(
        &client,
        "set_alarm_settings",
        json!({"settings": {"siren_s": 5}}),
    )
    .await;
    assert_eq!(bad.is_error, Some(true));
    assert!(text_of(&bad).starts_with("rejected:"), "{}", text_of(&bad));
}

const FRIGATE_PASS: &str = "frigate-secret-pass";

/// A Run-mode setup context holding a Frigate password, with a lazy (unreachable) database.
fn settings_ctx(mode: nidoo::setup::Mode) -> Arc<nidoo::setup::SetupCtx> {
    use nidoo::setup::{HubFile, RestartHook, SetupCtx, sections::Sections};
    let mut sections = Sections::default();
    sections.frigate.url = Some("http://192.0.2.9:5000".into());
    sections.frigate.user = Some("u".into());
    sections.frigate.password = Some(FRIGATE_PASS.into());
    Arc::new(SetupCtx::new(
        std::env::temp_dir().join(format!("nidoo-mcp-settings-{}", std::process::id())),
        mode,
        HubFile::default(),
        Some(nidoo::history::db::Db::connect_lazy("postgres://u:p@127.0.0.1:1/x").unwrap()),
        sections,
        RestartHook::Record(Arc::default()),
    ))
}

#[tokio::test]
async fn mcp_get_settings_hides_secrets() {
    let server = Server::start_with_setup(settings_ctx(nidoo::setup::Mode::Run)).await;
    let client = server.client().await;
    let result = call(&client, "get_settings", json!({"section": "frigate"})).await;
    assert!(
        !text_of(&result).contains(FRIGATE_PASS),
        "{}",
        text_of(&result)
    );
    let view = json_of("get_settings", &result);
    assert_eq!(view["password"], json!({"set": true}));
    assert_eq!(view["user"], "u");
    assert_eq!(view["url"], "http://192.0.2.9:5000");
    assert!(!view.to_string().contains(FRIGATE_PASS));
}

#[tokio::test]
async fn mcp_set_settings_refuses_new_frigate_url_without_password() {
    let server = Server::start_with_setup(settings_ctx(nidoo::setup::Mode::Run)).await;
    let client = server.client().await;
    let result = call(
        &client,
        "set_settings",
        json!({"section": "frigate", "value": {"url": "http://frigate.example.com:5000"}}),
    )
    .await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(text_of(&result), "Enter the password for the new address");
}

#[tokio::test]
async fn mcp_settings_unknown_section_is_an_error() {
    let server = Server::start_with_setup(settings_ctx(nidoo::setup::Mode::Run)).await;
    let client = server.client().await;
    let got = call(&client, "get_settings", json!({"section": "nope"})).await;
    assert_eq!(got.is_error, Some(true));
    assert_eq!(text_of(&got), "unknown section");
    let put = call(
        &client,
        "set_settings",
        json!({"section": "nope", "value": {}}),
    )
    .await;
    assert_eq!(put.is_error, Some(true));
}

#[tokio::test]
async fn mcp_set_settings_refuses_in_setup_mode() {
    let server = Server::start_with_setup(settings_ctx(nidoo::setup::Mode::Setup)).await;
    let client = server.client().await;
    let result = call(
        &client,
        "set_settings",
        json!({"section": "climate", "value": {}}),
    )
    .await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(text_of(&result), "finish MQTT and database setup first");
}

#[tokio::test]
async fn mcp_settings_without_a_setup_context_is_an_error() {
    let server = Server::start().await;
    let client = server.client().await;
    let result = call(&client, "get_settings", json!({"section": "climate"})).await;
    assert_eq!(result.is_error, Some(true));
    assert!(text_of(&result).contains("settings are not available"));
}

#[tokio::test]
async fn mcp_set_settings_error_never_echoes_a_secret() {
    let server = Server::start_with_setup(settings_ctx(nidoo::setup::Mode::Run)).await;
    let client = server.client().await;
    // Solcast on with no site fails validation; the key rides along in the body.
    let result = call(
        &client,
        "set_settings",
        json!({"section": "solcast",
               "value": {"enabled": true, "sites": [], "api_key": "sk-test-123"}}),
    )
    .await;
    assert_eq!(result.is_error, Some(true), "{}", text_of(&result));
    assert!(
        !text_of(&result).contains("sk-test-123"),
        "{}",
        text_of(&result)
    );
    assert!(!format!("{result:?}").contains("sk-test-123"));
}
