use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::info;

#[derive(Clone)]
struct AppState {
    start_time: Instant,
    port: u16,
    shutdown_tx: Arc<broadcast::Sender<()>>,
}

#[derive(Serialize)]
struct HealthResponse {
    status: String,
    version: String,
    uptime: u64,
    pid: u32,
    port: u16,
}

#[derive(Deserialize)]
struct RpcRequest {
    method: String,
    #[serde(default)]
    id: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct RpcResponse {
    jsonrpc: String,
    result: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<serde_json::Value>,
}

async fn health_handler(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime: state.start_time.elapsed().as_secs(),
        pid: std::process::id(),
        port: state.port,
    })
}

async fn rpc_handler(
    State(state): State<AppState>,
    Json(req): Json<RpcRequest>,
) -> Json<RpcResponse> {
    match req.method.as_str() {
        "shutdown" => {
            let _ = state.shutdown_tx.send(());
            Json(RpcResponse {
                jsonrpc: "2.0".to_string(),
                result: serde_json::json!({"status": "shutting_down"}),
                id: req.id,
            })
        }
        _ => Json(RpcResponse {
            jsonrpc: "2.0".to_string(),
            result: serde_json::json!({"error": "method_not_found"}),
            id: req.id,
        }),
    }
}

/// Start the health check HTTP server.
/// Returns a broadcast receiver that signals when shutdown is requested via RPC.
pub async fn start_health_server(
    port: u16,
) -> anyhow::Result<broadcast::Receiver<()>> {
    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let state = AppState {
        start_time: Instant::now(),
        port,
        shutdown_tx: Arc::new(shutdown_tx),
    };

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/rpc", post(rpc_handler))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            anyhow::anyhow!("myagent is already running (port {} in use)", port)
        } else {
            anyhow::anyhow!("Failed to bind port {}: {}", port, e)
        }
    })?;

    info!("Health server listening on http://{}", addr);

    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    Ok(shutdown_rx)
}
