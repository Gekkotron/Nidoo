//! The `mcp-stdio` proxy: an MCP server over stdio for clients that only speak stdio
//! (Claude Desktop), relaying every request to the hub's streamable HTTP server at
//! `<server>/mcp`. It is a thin pass-through: nothing is cached or interpreted, the upstream's
//! answers and errors are returned as they are. Stdout carries the protocol frames, so any
//! logging goes to stderr.

use anyhow::Context;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ErrorData, Implementation, ListResourcesResult,
    ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse,
    ServerConfig, ServerPeerInfo,
};
use rmcp::service::{RequestContext, RunningService, ServiceError};
use rmcp::transport::{StreamableHttpClientTransport, stdio};
use rmcp::{RoleClient, RoleServer, ServerHandler, ServiceExt};

/// Connects to `<server_url>/mcp`, then serves MCP over stdio until the client closes stdin.
///
/// Fails before serving anything when the hub cannot be reached, so the launcher sees a clear
/// error on stderr and a non-zero exit.
pub async fn run_stdio_proxy(server_url: String) -> anyhow::Result<()> {
    let mcp_url = format!("{}/mcp", server_url.trim_end_matches('/'));
    let transport = StreamableHttpClientTransport::from_uri(mcp_url.clone());
    let upstream = ()
        .serve(transport)
        .await
        .with_context(|| format!("cannot reach the Nidoo MCP server at {mcp_url}"))?;
    let info = upstream
        .peer_info()
        .map(|peer| server_config(&peer))
        .with_context(|| format!("{mcp_url} answered initialize without describing itself"))?;
    tracing::info!(url = %mcp_url, "mcp-stdio proxy connected");

    let session = StdioProxy { upstream, info }
        .serve(stdio())
        .await
        .context("serving MCP over stdio")?;
    session.waiting().await.context("the stdio session")?;
    tracing::info!("mcp-stdio proxy stopped");
    Ok(())
}

/// What the stdio client is told about the server: the upstream's capabilities, identity and
/// instructions as received at connect time. The protocol version is left to this side's own
/// negotiation with the stdio client.
fn server_config(peer: &ServerPeerInfo) -> ServerConfig {
    let mut config = ServerConfig::new(peer.capabilities.clone());
    config.server_info = peer
        .server_info
        .clone()
        .unwrap_or_else(|| Implementation::new("oikos-os", env!("CARGO_PKG_VERSION")));
    config.instructions = peer.instructions.clone();
    config
}

/// The stdio server: one upstream client session, relayed request by request.
struct StdioProxy {
    upstream: RunningService<RoleClient, ()>,
    info: ServerConfig,
}

impl ServerHandler for StdioProxy {
    fn get_info(&self) -> ServerConfig {
        self.info.clone()
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        self.upstream
            .peer()
            .list_tools(request)
            .await
            .map_err(relay_error)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.upstream
            .peer()
            .call_tool_once(request)
            .await
            .map_err(relay_error)
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        self.upstream
            .peer()
            .list_resources(request)
            .await
            .map_err(relay_error)
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        self.upstream
            .peer()
            .read_resource_once(request)
            .await
            .map_err(relay_error)
    }
}

/// An upstream JSON-RPC error is handed over untouched; a transport failure becomes an
/// internal error carrying its text.
fn relay_error(err: ServiceError) -> ErrorData {
    match err {
        ServiceError::McpError(data) => data,
        other => ErrorData::internal_error(other.to_string(), None),
    }
}
