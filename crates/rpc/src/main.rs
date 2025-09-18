//! Axum‑based RPC server exposing a **full accounts‑domain API**,
//! a simulation proxy, token‑specific fast paths, a distributed cache,
//! robust WebSocket subscriptions, metrics, health‑checks and security.
//
//! Build with the optional `distributed` feature to enable Redis‑backed
//! shared state across many Fractal instances.

use {
    axum::{
        extract::{
            ws::{Message, WebSocket, WebSocketUpgrade},
            Extension, Json, Query,
        },
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Response},
        routing::{get, post},
        Router,
    },
    clap::Parser,
    fractal_shard::ShardedIndex,
    prometheus::{
        Encoder, TextEncoder, register_histogram, register_int_counter_vec,
        register_int_gauge, Histogram, IntCounterVec, IntGauge,
    },
    redis::AsyncCommands,
    serde::{Deserialize, Serialize},
    solana_sdk::{pubkey::Pubkey, signature::Signature},
    std::{
        net::SocketAddr,
        sync::Arc,
        time::Instant,
    },
    tokio::sync::broadcast,
    tower::limit::RateLimitLayer,
    tower_http::cors::{Any, CorsLayer},
    tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt},
};

/// CLI arguments – mainly for the optional Redis URL and downstream validator RPC.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Optional Redis URL (e.g. redis://127.0.0.1/). Enables the distributed cache.
    #[arg(long, env = "REDIS_URL")]
    redis_url: Option<String>,

    /// URL of the validator (or Agave) that will handle `simulateTransaction`.
    #[arg(long, env = "DOWNSTREAM_RPC", default_value = "http://127.0.0.1:8899")]
    downstream_rpc: String,

    /// Optional API key that must be sent in the `x-api-key` header.
    #[arg(long, env = "API_KEY")]
    api_key: Option<String>,
}

// ---------- Prometheus metrics ----------
lazy_static::lazy_static! {
    static ref REQUEST_DURATION: Histogram = register_histogram!(
        "rpc_request_seconds",
        "RPC latency (seconds)",
        &["handler"]
    )
    .unwrap();

    static ref REQUEST_COUNT: IntCounterVec = register_int_counter_vec!(
        "rpc_requests_total",
        "Total number of RPC requests",
        &["handler", "status"]
    )
    .unwrap();

    static ref CACHE_SIZE: IntGauge = register_int_gauge!(
        "rpc_cache_accounts_total",
        "Number of accounts currently cached"
    )
    .unwrap();
}

// ---------- Request / response structs ----------
#[derive(Deserialize)]
struct GetProgramAccountsReq {
    program: String,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    filters: Option<Vec<Filter>>, // memcmp / datasize filters
}

#[derive(Deserialize)]
#[serde(tag = "memcmp", rename_all = "camelCase")]
enum Filter {
    Memcmp {
        offset: usize,
        bytes: String, // base64
    },
    DataSize {
        size: usize,
    },
}

#[derive(Serialize)]
struct AccountResp {
    pubkey: String,
    lamports: u64,
    data: String, // base64
    owner: String,
    executable: bool,
    rent_epoch: u64,
}

#[derive(Deserialize)]
struct SimulateTxReq {
    // The full request is exactly the same as Solana's simulateTransaction.
    // We forward it verbatim to the downstream RPC.
    #[serde(flatten)]
    inner: serde_json::Value,
}

// WebSocket event that we push to clients.
#[derive(Serialize, Clone, Debug)]
enum WsEvent {
    AccountUpdated {
        pubkey: String,
        lamports: u64,
        slot: u64,
    },
    SlotUpdated {
        slot: u64,
    },
}

// Shared state injected into every handler.
#[derive(Clone)]
struct AppState {
    index: Arc<ShardedIndex>,
    txs: Arc<broadcast::Sender<WsEvent>>,
    api_key: Option<String>,
    downstream_rpc: String,
    #[cfg(feature = "distributed")]
    redis: Option<redis::Client>,
}

// ---------- Main ----------
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ---------- CLI ----------
    let args = Args::parse();

    // ---------- tracing ----------
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::new(
                std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
            ),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // ---------- shared state ----------
    let index = Arc::new(ShardedIndex::default());

    #[cfg(feature = "distributed")]
    if let Some(ref url) = args.redis_url {
        // Enable Redis backing.
        let client = redis::Client::open(url.clone())?;
        // Tell the index to use Redis for get/insert.
        let mut_index = Arc::get_mut(&mut Arc::clone(&index)).unwrap();
        mut_index.enable_redis(url)?;
        tracing::info!("Redis distributed cache enabled: {}", url);
    }

    let (tx, _rx) = broadcast::channel::<WsEvent>(8192);
    let state = AppState {
        index: index.clone(),
        txs: Arc::new(tx),
        api_key: args.api_key.clone(),
        downstream_rpc: args.downstream_rpc.clone(),
        #[cfg(feature = "distributed")]
        redis: args
            .redis_url
            .as_ref()
            .map(|url| redis::Client::open(url.clone()).unwrap()),
    };

    // ---------- router ----------
    let rate_limiter = RateLimitLayer::new(3000, std::time::Duration::from_secs(1));
    let cors = CorsLayer::new()
        .allow_origin(Any) // replace with a whitelist in production
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        .route("/getProgramAccounts", post(get_program_accounts))
        .route("/getMultipleAccounts", post(get_multiple_accounts))
        .route("/getAccountInfo", post(get_account_info))
        .route("/getTokenAccountsByOwner", post(get_token_accounts_by_owner))
        .route("/getLargestTokenAccounts", post(get_largest_token_accounts))
        .route("/simulateTransaction", post(simulate_transaction))
        .route("/ws", get(websocket_handler))
        .layer(rate_limiter)
        .layer(cors)
        .layer(Extension(state));

    // ---------- serve ----------
    let addr: SocketAddr = "0.0.0.0:8899".parse()?;
    let server = axum::Server::bind(&addr).serve(app.into_make_service());

    // Graceful shutdown on SIGTERM / Ctrl‑C
    let shutdown_signal = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    tracing::info!("Fractal RPC listening on {}", addr);
    server
        .with_graceful_shutdown(shutdown_signal)
        .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Helper: API‑key validation (optional)
// ---------------------------------------------------------------------------
fn check_api_key(state: &AppState, headers: &HeaderMap) -> Result<(), (StatusCode, String)> {
    if let Some(ref required) = state.api_key {
        match headers.get("x-api-key") {
            Some(got) if got == required => Ok(()),
            _ => Err((StatusCode::UNAUTHORIZED, "missing or invalid API key".into())),
        }
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health_handler(
    Extension(state): Extension<AppState>,
) -> impl IntoResponse {
    // The cache is considered healthy when it contains at least one account.
    if state.index.len() > 0 {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "cache empty")
    }
}

async fn metrics_handler() -> impl IntoResponse {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buf = Vec::new();
    encoder.encode(&metric_families, &mut buf).unwrap();
    (
        [("Content-Type", encoder.format_type())],
        String::from_utf8(buf).unwrap(),
    )
}

// ---------------------------------------------------------------------------
// GET /getProgramAccounts
// ---------------------------------------------------------------------------
async fn get_program_accounts(
    Extension(state): Extension<AppState>,
    headers: HeaderMap,
    Json(req): Json<GetProgramAccountsReq>,
) -> Result<Json<Vec<AccountResp>>, (StatusCode, String)> {
    // ---------- API‑key ----------
    check_api_key(&state, &headers)?;

    let start = Instant::now();

    // ---------- parse program pubkey ----------
    let program = Pubkey::try_from(req.program.as_str())
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid program pubkey".into()))?;

    // ---------- fetch accounts ----------
    let mut accounts = state.index.get_program_accounts(&program);

    // ---------- apply optional filters ----------
    if let Some(filters) = req.filters {
        accounts = apply_filters(accounts, filters);
    }

    // ---------- pagination ----------
    if let Some(offset) = req.offset {
        if offset < accounts.len() {
            accounts.drain(0..offset);
        } else {
            accounts.clear();
        }
    }
    if let Some(limit) = req.limit {
        accounts.truncate(limit);
    }

    // ---------- transform to response ----------
    let out: Vec<AccountResp> = accounts
        .into_iter()
        .map(|(k, acc)| AccountResp {
            pubkey: k.to_string(),
            lamports: acc.lamports,
            data: base64::encode(&acc.data),
            owner: acc.owner.to_string(),
            executable: acc.executable,
            rent_epoch: acc.rent_epoch,
        })
        .collect();

    // ---------- metrics ----------
    let elapsed = start.elapsed().as_secs_f64();
    REQUEST_DURATION
        .with_label_values(&["getProgramAccounts"])
        .observe(elapsed);
    REQUEST_COUNT
        .with_label_values(&["getProgramAccounts", "200"])
        .inc();
    CACHE_SIZE.set(state.index.len() as i64);

    Ok(Json(out))
}

// ---------------------------------------------------------------------------
// Helper: apply memcmp / datasize filters (very similar to Solana's own logic)
// ---------------------------------------------------------------------------
fn apply_filters(
    accounts: Vec<(Pubkey, Arc<solana_sdk::account::Account>)>,
    filters: Vec<Filter>,
) -> Vec<(Pubkey, Arc<solana_sdk::account::Account>)> {
    accounts
        .into_iter()
        .filter(|(_, acc)| {
            filters.iter().all(|f| match f {
                Filter::Memcmp { offset, bytes } => {
                    let decoded = base64::decode(bytes).unwrap_or_default();
                    if *offset + decoded.len() > acc.data.len() {
                        false
                    } else {
                        acc.data[*offset..*offset + decoded.len()] == decoded[..]
                    }
                }
                Filter::DataSize { size } => acc.data.len() == *size,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// GET /getMultipleAccounts
// ---------------------------------------------------------------------------
#[derive(Deserialize)]
struct GetMultipleAccountsReq {
    pubkeys: Vec<String>,
    #[serde(default)]
    encoding: Option<String>, // base64, base58, jsonParsed (we only support base64)
}

async fn get_multiple_accounts(
    Extension(state): Extension<AppState>,
    headers: HeaderMap,
    Json(req): Json<GetMultipleAccountsReq>,
) -> Result<Json<Vec<Option<AccountResp>>>, (StatusCode, String)> {
    check_api_key(&state, &headers)?;
    let start = Instant::now();

    let mut out = Vec::with_capacity(req.pubkeys.len());
    for pk_str in &req.pubkeys {
        let pk = Pubkey::try_from(pk_str.as_str())
            .map_err(|_| (StatusCode::BAD_REQUEST, "invalid pubkey".into()))?;
        if let Some(acc) = state.index.get(&pk) {
            out.push(Some(AccountResp {
                pubkey: pk.to_string(),
                lamports: acc.lamports,
                data: base64::encode(&acc.data),
                owner: acc.owner.to_string(),
                executable: acc.executable,
                rent_epoch: acc.rent_epoch,
            }));
        } else {
            out.push(None);
        }
    }

    // metrics
    let elapsed = start.elapsed().as_secs_f64();
    REQUEST_DURATION
        .with_label_values(&["getMultipleAccounts"])
        .observe(elapsed);
    REQUEST_COUNT
        .with_label_values(&["getMultipleAccounts", "200"])
        .inc();

    Ok(Json(out))
}

// ---------------------------------------------------------------------------
// GET /getAccountInfo
// ---------------------------------------------------------------------------
#[derive(Deserialize)]
struct GetAccountInfoReq {
    pubkey: String,
    #[serde(default)]
    encoding: Option<String>,
}

async fn get_account_info(
    Extension(state): Extension<AppState>,
    headers: HeaderMap,
    Json(req): Json<GetAccountInfoReq>,
) -> Result<Json<Option<AccountResp>>, (StatusCode, String)> {
    check_api_key(&state, &headers)?;
    let start = Instant::now();

    let pk = Pubkey::try_from(req.pubkey.as_str())
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid pubkey".into()))?;
    let resp = state
        .index
        .get(&pk)
        .map(|acc| AccountResp {
            pubkey: pk.to_string(),
            lamports: acc.lamports,
            data: base64::encode(&acc.data),
            owner: acc.owner.to_string(),
            executable: acc.executable,
            rent_epoch: acc.rent_epoch,
        });

    // metrics
    let elapsed = start.elapsed().as_secs_f64();
    REQUEST_DURATION
        .with_label_values(&["getAccountInfo"])
        .observe(elapsed);
    REQUEST_COUNT
        .with_label_values(&["getAccountInfo", "200"])
        .inc();

    Ok(Json(resp))
}

// ---------------------------------------------------------------------------
// GET /getTokenAccountsByOwner (fast path)
// ---------------------------------------------------------------------------
#[derive(Deserialize)]
struct GetTokenAccountsByOwnerReq {
    owner: String,
    #[serde(default)]
    mint: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

async fn get_token_accounts_by_owner(
    Extension(state): Extension<AppState>,
    headers: HeaderMap,
    Json(req): Json<GetTokenAccountsByOwnerReq>,
) -> Result<Json<Vec<AccountResp>>, (StatusCode, String)> {
    check_api_key(&state, &headers)?;
    let start = Instant::now();

    let owner_pk = Pubkey::try_from(req.owner.as_str())
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid owner pubkey".into()))?;

    // Fast‑path: the owner index already gives us all accounts owned by this
    // address (including SPL‑Token accounts). We just filter by mint if requested.
    let mut accounts: Vec<(Pubkey, Arc<solana_sdk::account::Account>)> = {
        if let Some(owner_vec) = state.index.owner_index.get(&owner_pk) {
            let vec = owner_vec.read().unwrap();
            vec.iter()
                .filter_map(|pk| state.index.get(pk).map(|acc| (*pk, acc)))
                .collect()
        } else {
            Vec::new()
        }
    };

    // Optional mint filter – token accounts have the mint in the first 32 bytes.
    if let Some(mint_str) = req.mint {
        let mint_pk = Pubkey::try_from(mint_str.as_str())
            .map_err(|_| (StatusCode::BAD_REQUEST, "invalid mint".into()))?;
        accounts.retain(|(_, acc)| {
            acc.data.len() >= 32 && &acc.data[0..32] == mint_pk.as_ref()
        });
    }

    // Pagination
    if let Some(offset) = req.offset {
        if offset < accounts.len() {
            accounts.drain(0..offset);
        } else {
            accounts.clear();
        }
    }
    if let Some(limit) = req.limit {
        accounts.truncate(limit);
    }

    let out: Vec<AccountResp> = accounts
        .into_iter()
        .map(|(k, acc)| AccountResp {
            pubkey: k.to_string(),
            lamports: acc.lamports,
            data: base64::encode(&acc.data),
            owner: acc.owner.to_string(),
            executable: acc.executable,
            rent_epoch: acc.rent_epoch,
        })
        .collect();

    // metrics
    let elapsed = start.elapsed().as_secs_f64();
    REQUEST_DURATION
        .with_label_values(&["getTokenAccountsByOwner"])
        .observe(elapsed);
    REQUEST_COUNT
        .with_label_values(&["getTokenAccountsByOwner", "200"])
        .inc();

    Ok(Json(out))
}

// ---------------------------------------------------------------------------
// GET /getLargestTokenAccounts (fast path)
// ---------------------------------------------------------------------------
#[derive(Deserialize)]
struct GetLargestTokenAccountsReq {
    mint: String,
    #[serde(default)]
    limit: Option<usize>,
}

async fn get_largest_token_accounts(
    Extension(state): Extension<AppState>,
    headers: HeaderMap,
    Json(req): Json<GetLargestTokenAccountsReq>,
) -> Result<Json<Vec<AccountResp>>, (StatusCode, String)> {
    check_api_key(&state, &headers)?;
    let start = Instant::now();

    let mint_pk = Pubkey::try_from(req.mint.as_str())
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid mint".into()))?;
    let limit = req.limit.unwrap_or(10);

    let accounts = state
        .index
        .get_largest_token_accounts(&mint_pk, limit);

    let out: Vec<AccountResp> = accounts
        .into_iter()
        .map(|(k, acc)| AccountResp {
            pubkey: k.to_string(),
            lamports: acc.lamports,
            data: base64::encode(&acc.data),
            owner: acc.owner.to_string(),
            executable: acc.executable,
            rent_epoch: acc.rent_epoch,
        })
        .collect();

    // metrics
    let elapsed = start.elapsed().as_secs_f64();
    REQUEST_DURATION
        .with_label_values(&["getLargestTokenAccounts"])
        .observe(elapsed);
    REQUEST_COUNT
        .with_label_values(&["getLargestTokenAccounts", "200"])
        .inc();

    Ok(Json(out))
}

// ---------------------------------------------------------------------------
// POST /simulateTransaction (proxy mode)
// ---------------------------------------------------------------------------
async fn simulate_transaction(
    Extension(state): Extension<AppState>,
    headers: HeaderMap,
    Json(req): Json<SimulateTxReq>,
) -> Result<Response, (StatusCode, String)> {
    check_api_key(&state, &headers)?;

    // Forward the request body unchanged to the downstream RPC.
    let client = reqwest::Client::new();
    let downstream_url = format!("{}/simulateTransaction", state.downstream_rpc);
    let resp = client
        .post(&downstream_url)
        .json(&req.inner)
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("downstream error: {e}")))?;

    // Pass through status code and body.
    let status = resp.status();
    let body = resp
        .bytes()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("downstream read error: {e}")))?;

    Ok((status, body).into_response())
}

// ---------------------------------------------------------------------------
// WebSocket handler (with per‑client filters & hot‑account channel)
// ---------------------------------------------------------------------------
async fn websocket_handler(
    ws: WebSocketUpgrade,
    Extension(state): Extension<AppState>,
    Query(params): Query<WsQuery>,
) -> impl IntoResponse {
    // `params` can contain optional `program` or `owner` filters.
    ws.on_upgrade(move |socket| handle_socket(socket, state.txs.clone(), params))
}

#[derive(Deserialize, Default)]
struct WsQuery {
    program: Option<String>,
    owner: Option<String>,
}

async fn handle_socket(
    socket: WebSocket,
    txs: Arc<broadcast::Sender<WsEvent>>,
    filter: WsQuery,
) {
    let (mut sender, _receiver) = socket.split();
    let mut rx = txs.subscribe();

    // Pre‑parse filter pubkeys (if any) for fast comparison.
    let program_filter = filter
        .program
        .as_ref()
        .and_then(|s| Pubkey::try_from(s.as_str()).ok());
    let owner_filter = filter
        .owner
        .as_ref()
        .and_then(|s| Pubkey::try_from(s.as_str()).ok());

    while let Ok(evt) = rx.recv().await {
        // Apply per‑client filter.
        let pass = match &evt {
            WsEvent::AccountUpdated { pubkey, .. } => {
                if let Some(p) = program_filter {
                    // We need the account to check its owner – cheap because we have it cached.
                    let pk = Pubkey::try_from(pubkey.as_str()).unwrap();
                    if let Some(acc) = txs
                        .clone()
                        .into_inner()
                        .index
                        .get(&pk)
                    {
                        acc.owner == p
                    } else {
                        false
                    }
                } else if let Some(o) = owner_filter {
                    // For owner filter we just compare the pubkey directly.
                    pubkey == &o.to_string()
                } else {
                    true
                }
            }
            WsEvent::SlotUpdated { .. } => true,
        };
        if !pass {
            continue;
        }

        // Serialize and send.
        match serde_json::to_string(&evt) {
            Ok(txt) => {
                if sender.send(Message::Text(txt)).await.is_err() {
                    break;
                }
            }
            Err(e) => {
                tracing::error!("failed to serialize WsEvent: {e}");
                break;
            }
        }
    }
}
