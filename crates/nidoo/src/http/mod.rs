//! The HTTP surface over the hub: `/health`, the JSON API under `/api/v1`, the WebSocket
//! stream at `/ws`, the MCP server at `/mcp` and the embedded web UI for every other path.

mod api;
mod static_ui;
mod tts;
mod ws;

use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use axum::routing::{any, get};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tower_http::cors::CorsLayer;

use crate::config::HttpConfig;
use crate::hub::Hub;
use crate::mcp;
use crate::setup::{SetupCtx, routes as setup_routes};
use crate::weather::open_meteo::Endpoints;

/// The API, the WebSocket, the MCP server and the web UI, with CORS open to any
/// origin (no auth in v1). Unknown paths under `/api` answer a JSON 404; every other unknown
/// path gets the web UI. The MCP service is nested, so it takes precedence over the fallback.
pub fn router(hub: Arc<Hub>, ctx: Arc<SetupCtx>) -> Router {
    router_with(hub, ctx, Endpoints::default())
}

/// [`router`] with the geocoding endpoint the setup lookups use (tests point it at a stub).
pub fn router_with(hub: Arc<Hub>, ctx: Arc<SetupCtx>, geocoding: Endpoints) -> Router {
    router_with_solcast(hub, ctx, geocoding, nidoo_solcast::BASE_URL)
}

/// [`router_with`] with the Solcast API address the setup's key test calls too.
pub fn router_with_solcast(
    hub: Arc<Hub>,
    ctx: Arc<SetupCtx>,
    geocoding: Endpoints,
    solcast_base: &str,
) -> Router {
    Router::new()
        .merge(setup_routes::router_with_solcast(
            ctx,
            geocoding,
            solcast_base,
        ))
        .route("/health", get(api::health))
        .nest("/api/v1", api::routes())
        .route("/api/{*rest}", any(api::not_found))
        .route("/ws", get(ws::upgrade))
        .nest_service("/mcp", mcp::mcp_service(Arc::clone(&hub)))
        .fallback(static_ui::index)
        .layer(CorsLayer::permissive())
        .with_state(hub)
}

/// Setup mode: the setup routes, `/health` and the web UI for every other path.
pub fn setup_router(ctx: Arc<SetupCtx>) -> Router {
    setup_router_with(ctx, Endpoints::default())
}

/// [`setup_router`] with its own geocoding endpoint.
pub fn setup_router_with(ctx: Arc<SetupCtx>, geocoding: Endpoints) -> Router {
    setup_router_with_solcast(ctx, geocoding, nidoo_solcast::BASE_URL)
}

/// [`setup_router_with`] with the Solcast API address the key test calls.
pub fn setup_router_with_solcast(
    ctx: Arc<SetupCtx>,
    geocoding: Endpoints,
    solcast_base: &str,
) -> Router {
    Router::new()
        .route(
            "/health",
            get(|| async { axum::Json(serde_json::json!({"status": "setup"})) }),
        )
        .merge(setup_routes::router_with_solcast(
            ctx,
            geocoding,
            solcast_base,
        ))
        .route("/api/{*rest}", any(api::not_found))
        .fallback(static_ui::index)
        .layer(CorsLayer::permissive())
}

/// Binds `cfg.bind` and serves `router` on a spawned task; the task only ends when aborted
/// or when the listener fails.
pub async fn serve(cfg: &HttpConfig, router: Router) -> anyhow::Result<JoinHandle<()>> {
    let listener = TcpListener::bind(&cfg.bind)
        .await
        .with_context(|| format!("binding {}", cfg.bind))?;
    let addr = listener.local_addr().context("local address")?;
    tracing::info!(%addr, "http listening");
    Ok(tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, router).await {
            tracing::error!(error = %err, "http server stopped");
        }
    }))
}
