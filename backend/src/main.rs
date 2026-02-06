//! DeepBook Sandbox Backend
//!
//! HTTP API server wrapping sui-sandbox for forked mainnet PTB execution.
//! Builds MoveVM orderbooks at startup from Snowflake checkpoint 240M data.

use axum::{routing::get, Router};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use deepbook_sandbox_backend::api;
use deepbook_sandbox_backend::sandbox::orderbook_builder::{OrderbookBuilder, SandboxOrderbook};
use deepbook_sandbox_backend::sandbox::router;
use deepbook_sandbox_backend::sandbox::state_loader::{DeepBookConfig, PoolId, PoolRegistry, StateLoader};
use deepbook_sandbox_backend::sandbox::swap_executor::SessionManager;

#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Load environment variables
    dotenvy::dotenv().ok();

    // Create shared pool registry
    let pool_registry = Arc::new(RwLock::new(PoolRegistry::new()));
    tracing::info!("PoolRegistry initialized");

    // Session manager is created after orderbooks are built (needs global orderbooks)
    // See below after MoveVM orderbook construction

    // Define pool state files (relative to working directory)
    // Using validated checkpoint 240M state files
    let pool_files = [
        (PoolId::SuiUsdc, "./data/sui_usdc_state_cp240M.jsonl"),
        (PoolId::WalUsdc, "./data/wal_usdc_state_cp240M.jsonl"),
        (PoolId::DeepUsdc, "./data/deep_usdc_state_cp240M.jsonl"),
    ];

    // Load all pool states
    {
        let mut registry = pool_registry.write().await;
        for (pool_id, file_path) in &pool_files {
            let path = std::path::Path::new(file_path);
            if path.exists() {
                match registry.load_pool_from_file(*pool_id, path) {
                    Ok(count) => {
                        tracing::info!(
                            "Loaded {} pool: {} objects from {}",
                            pool_id.display_name(),
                            count,
                            path.display()
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to load {} pool from {}: {}",
                            pool_id.display_name(),
                            path.display(),
                            e
                        );
                    }
                }
            } else {
                tracing::warn!(
                    "{} state file not found: {}",
                    pool_id.display_name(),
                    path.display()
                );
            }
        }

        // Log summary
        let summary = registry.summary();
        tracing::info!(
            "Pool registry ready: {}/{} pools loaded",
            summary.total_pools,
            pool_files.len()
        );
        for pool in &summary.pools {
            tracing::info!(
                "  {} - {} objects, {} asks, {} bids (checkpoint {})",
                pool.pool_name,
                pool.total_objects,
                pool.asks_slices,
                pool.bids_slices,
                pool.checkpoint
            );
        }
    }

    // Build MoveVM orderbooks from loaded state (one-time startup cost)
    tracing::info!("Building MoveVM orderbooks from checkpoint 240M state...");
    let orderbooks = {
        // Collect the data we need from the registry while holding the lock
        let registry = pool_registry.read().await;
        let loaded_pools: Vec<PoolId> = registry.loaded_pools();

        // Collect pool state data (StateLoader references) for the blocking task
        // We need to clone/serialize the data since StateLoader is behind RwLock
        let pool_data: Vec<(PoolId, String)> = loaded_pools
            .iter()
            .filter_map(|pool_id| {
                let file_path = match pool_id {
                    PoolId::SuiUsdc => Some("./data/sui_usdc_state_cp240M.jsonl"),
                    PoolId::WalUsdc => Some("./data/wal_usdc_state_cp240M.jsonl"),
                    PoolId::DeepUsdc => Some("./data/deep_usdc_state_cp240M.jsonl"),
                };
                file_path.map(|p| (*pool_id, p.to_string()))
            })
            .collect();
        drop(registry);

        // Build orderbooks in a blocking task since OrderbookBuilder is not Send
        let result = tokio::task::spawn_blocking(move || {
            build_movevm_orderbooks(&pool_data)
        })
        .await
        .expect("spawn_blocking panicked");

        match result {
            Ok(map) => {
                tracing::info!(
                    "MoveVM orderbooks built: {} pools ready",
                    map.len()
                );
                for (pool_id, ob) in &map {
                    tracing::info!(
                        "  {} - {} bids, {} asks, mid=${:.6}",
                        pool_id.display_name(),
                        ob.bids.len(),
                        ob.asks.len(),
                        ob.mid_price().unwrap_or(0.0)
                    );
                }
                Arc::new(RwLock::new(map))
            }
            Err(e) => {
                tracing::error!("Failed to build MoveVM orderbooks: {}", e);
                tracing::warn!("Server will start but orderbook/swap endpoints won't work");
                Arc::new(RwLock::new(HashMap::new()))
            }
        }
    };

    // Create session manager with a snapshot of global orderbooks
    let session_manager = {
        let ob_snapshot = orderbooks.read().await.clone();
        Arc::new(SessionManager::new(ob_snapshot))
    };
    tracing::info!("SessionManager initialized with {} pool orderbooks", orderbooks.read().await.len());

    // Spawn router thread for cross-pool MoveVM quotes
    let router_handle = {
        let pool_files_for_router: Vec<(PoolId, String)> = pool_files
            .iter()
            .map(|(id, path)| (*id, path.to_string()))
            .collect();

        tracing::info!("Spawning router thread for cross-pool quotes...");
        let (handle, ready_rx) = router::spawn_router_thread(pool_files_for_router);

        match ready_rx.await {
            Ok(Ok(())) => {
                tracing::info!("Router thread ready - cross-pool quotes enabled");
                Some(handle)
            }
            Ok(Err(e)) => {
                tracing::warn!("Router thread setup failed: {} - cross-pool quotes disabled", e);
                None
            }
            Err(_) => {
                tracing::warn!("Router thread dropped ready channel - cross-pool quotes disabled");
                None
            }
        }
    };

    // Build router
    let app = Router::new()
        .route("/health", get(health_check))
        .nest(
            "/api",
            api::router(pool_registry, session_manager, orderbooks, router_handle),
        )
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );

    // Start server
    let addr = SocketAddr::from(([0, 0, 0, 0], 3001));
    tracing::info!("Starting server on {}", addr);
    tracing::info!("API endpoints:");
    tracing::info!("  GET  /health                  - Health check");
    tracing::info!("  POST /api/session             - Create new trading session");
    tracing::info!("  GET  /api/session/:id         - Get session info & balances");
    tracing::info!("  GET  /api/session/:id/history - Get swap history");
    tracing::info!("  POST /api/session/:id/reset   - Reset session to initial state");
    tracing::info!("  GET  /api/balance/:session_id - Get token balances");
    tracing::info!("  POST /api/faucet              - Mint tokens into session");
    tracing::info!("  POST /api/swap                - Execute swap (requires session_id)");
    tracing::info!("  POST /api/swap/quote          - Get swap quote (supports cross-pool routes)");
    tracing::info!("  GET  /api/pools               - List available pools");
    tracing::info!("  GET  /api/orderbook           - Get orderbook snapshot");
    tracing::info!("  GET  /api/orderbook/depth     - Get Binance-style depth");
    tracing::info!("  GET  /api/orderbook/stats     - Get pool statistics");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn health_check() -> &'static str {
    "ok"
}

/// Build MoveVM orderbooks for all pools (runs in blocking thread)
///
/// Creates an OrderbookBuilder per pool, loads packages via gRPC,
/// loads pool state from JSONL files, and calls iter_orders to build
/// the orderbook. Returns the SandboxOrderbook results (Send+Sync).
fn build_movevm_orderbooks(
    pool_data: &[(PoolId, String)],
) -> anyhow::Result<HashMap<PoolId, SandboxOrderbook>> {
    let mut results = HashMap::new();

    // We need a tokio runtime handle for the async gRPC calls inside
    // load_packages_from_grpc. Since we're in spawn_blocking, we use
    // a new runtime for the async portions.
    for (pool_id, file_path) in pool_data {
        let path = std::path::Path::new(file_path);
        if !path.exists() {
            tracing::warn!(
                "Skipping {} - state file not found: {}",
                pool_id.display_name(),
                file_path
            );
            continue;
        }

        tracing::info!("Building {} orderbook via MoveVM...", pool_id.display_name());

        // Each pool gets its own builder + runtime (OrderbookBuilder is not Send)
        let rt = tokio::runtime::Runtime::new()?;

        let mut builder = OrderbookBuilder::new()?;
        rt.block_on(builder.load_packages_from_grpc())?;

        let config = DeepBookConfig::for_pool(*pool_id);
        let pool_wrapper = config.pool_wrapper.clone();

        let mut loader = StateLoader::with_config(config);
        loader
            .load_from_file(path)
            .map_err(|e| anyhow::anyhow!("Failed to load {}: {}", file_path, e))?;

        let stats = loader.stats();

        // Load pool state into the simulation environment
        builder.load_pool_state(&loader, *pool_id)?;

        // Build the orderbook via iter_orders PTB execution
        match builder.build_orderbook(*pool_id, &pool_wrapper, stats.max_checkpoint) {
            Ok(orderbook) => {
                tracing::info!(
                    "  {} built: {} bids, {} asks, mid=${:.6}",
                    pool_id.display_name(),
                    orderbook.bids.len(),
                    orderbook.asks.len(),
                    orderbook.mid_price().unwrap_or(0.0)
                );
                results.insert(*pool_id, orderbook);
            }
            Err(e) => {
                tracing::error!(
                    "  Failed to build {} orderbook: {}",
                    pool_id.display_name(),
                    e
                );
            }
        }

        // Drop builder and runtime before next pool
        drop(builder);
        drop(rt);
    }

    Ok(results)
}
