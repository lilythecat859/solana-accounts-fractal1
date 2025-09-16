//! Fractal-RPC – Axum server with WebSocket & prometheus
use {
    axum::{
        routing::{get, post},
        Json, Router,
    },
    fractal_shard::ShardedIndex,
    serde::{Deserialize, Serialize},
    solana_sdk::pubkey::Pubkey,
    std::{net::SocketAddr, sync::Arc},
    tokio::sync::broadcast,
    tower_http::cors::CorsLayer,
    tracing_subscriber::layer::SubscriberExt,
};

#[derive(Clone)]
struct AppState {
    index: Arc<ShardedIndex>,
    tx: broadcast::Sender<()>,
}

#[derive(Deserialize)]
struct Req {
    program: String,
}

#[derive(Serialize)]
struct Resp {
    pubkey: String,
    lamports: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let index = Arc::new(ShardedIndex::default());
    let (tx, _rx) = broadcast::channel(1024);
    let state = AppState { index, tx };

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/getProgramAccounts", post(handler))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr: SocketAddr = "0.0.0.0:8899".parse()?;
    tracing::info!(%addr, "starting fractal-rpc");
    axum::Server::bind(&addr).serve(app.into_make_service()).await?;
    Ok(())
}

async fn handler(
    State(state): State<AppState>,
    Json(req): Json<Req>,
) -> Result<Json<Vec<Resp>>, StatusCode> {
    let program = Pubkey::try_from(req.program.as_str()).map_err(|_| StatusCode::BAD_REQUEST)?;
    let out: Vec<Resp> = state
        .index
        .get_program_accounts(&program)
        .into_iter()
        .map(|(k, acc)| Resp {
            pubkey: k.to_string(),
            lamports: acc.lamports,
        })
        .collect();
    Ok(Json(out))
}
