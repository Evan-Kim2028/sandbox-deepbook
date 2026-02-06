//! API endpoints for the sandbox service

use axum::{
    routing::{get, post},
    Router,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

mod balance;
mod debug;
pub mod orderbook;
mod session;
mod swap;
mod system;

pub use orderbook::SharedPoolRegistry;

use crate::sandbox::orderbook_builder::SandboxOrderbook;
use crate::sandbox::router::{DebugPoolCreateConfig, RouterHandle};
use crate::sandbox::state_loader::{PoolId, PoolRegistry};
use crate::sandbox::swap_executor::SessionManager;

/// MoveVM-built orderbooks cached at startup, keyed by PoolId
pub type SharedOrderbooks = Arc<RwLock<HashMap<PoolId, SandboxOrderbook>>>;
pub type SharedDebugPoolState = Arc<RwLock<DebugPoolState>>;

/// Runtime metadata for the active debug pool/token exposed to API handlers.
#[derive(Debug, Clone)]
pub struct DebugPoolState {
    pub created: bool,
    pub pool_object_id: Option<String>,
    pub token_symbol: String,
    pub token_name: String,
    pub token_description: String,
    pub token_icon_url: String,
    pub token_decimals: u8,
    pub token_type: String,
    pub config: DebugPoolCreateConfig,
}

impl Default for DebugPoolState {
    fn default() -> Self {
        let cfg = DebugPoolCreateConfig::default();
        Self {
            created: false,
            pool_object_id: None,
            token_symbol: cfg.token_symbol.clone(),
            token_name: cfg.token_name.clone(),
            token_description: cfg.token_description.clone(),
            token_icon_url: cfg.token_icon_url.clone(),
            token_decimals: cfg.token_decimals,
            token_type: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa::debug_token::DEBUG_TOKEN"
                .to_string(),
            config: cfg,
        }
    }
}

/// Shared application state containing both pool registry and session manager
#[derive(Clone)]
pub struct AppState {
    pub pool_registry: SharedPoolRegistry,
    pub session_manager: Arc<SessionManager>,
    pub orderbooks: SharedOrderbooks,
    pub router: Option<RouterHandle>,
    pub debug_pool: SharedDebugPoolState,
}

impl AppState {
    pub fn new(
        pool_registry: Arc<RwLock<PoolRegistry>>,
        session_manager: Arc<SessionManager>,
        orderbooks: SharedOrderbooks,
        router: Option<RouterHandle>,
    ) -> Self {
        Self {
            pool_registry,
            session_manager,
            orderbooks,
            router,
            debug_pool: Arc::new(RwLock::new(DebugPoolState::default())),
        }
    }
}

/// Create the API router with all endpoints
pub fn router(
    pool_registry: SharedPoolRegistry,
    session_manager: Arc<SessionManager>,
    orderbooks: SharedOrderbooks,
    router_handle: Option<RouterHandle>,
) -> Router {
    let app_state = AppState::new(pool_registry, session_manager, orderbooks, router_handle);

    Router::new()
        // Session management
        .route("/session", post(session::create_session))
        .route("/session/:id", get(session::get_session))
        .route("/session/:id/history", get(session::get_swap_history))
        .route("/session/:id/reset", post(session::reset_session))
        // Wallet operations
        .route("/balance/:session_id", get(balance::get_balance))
        .route("/faucet", post(balance::faucet))
        // Swap operations
        .route("/swap", post(swap::execute_swap))
        .route("/swap/quote", post(swap::get_quote))
        .route("/startup-check", get(system::get_startup_check))
        .route(
            "/debug/pool",
            get(debug::get_debug_pool_status).post(debug::ensure_debug_pool),
        )
        .route("/debug/pools", get(debug::list_debug_pools))
        // Pool listing
        .route("/pools", get(orderbook::list_pools))
        // Orderbook (supports ?pool=sui_usdc|wal_usdc|deep_usdc)
        .route("/orderbook", get(orderbook::get_orderbook))
        .route("/orderbook/depth", get(orderbook::get_depth))
        .route("/orderbook/stats", get(orderbook::get_stats))
        .with_state(app_state)
}
