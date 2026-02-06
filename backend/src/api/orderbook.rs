//! Orderbook API endpoints
//!
//! Returns the current orderbook state built via MoveVM `iter_orders` execution.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::api::AppState;
use crate::sandbox::orderbook_builder::SandboxOrderbook;
use crate::sandbox::state_loader::{PoolId, PoolRegistry};

// --- Orderbook API response types (formerly in sandbox::deepbook) ---

/// Orderbook snapshot for API responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderbookSnapshot {
    pub pool_id: String,
    pub base_symbol: String,
    pub quote_symbol: String,
    pub mid_price: Option<f64>,
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
    pub spread_bps: Option<u64>,
    pub bids: Vec<OrderbookLevel>,
    pub asks: Vec<OrderbookLevel>,
    pub timestamp: u64,
}

/// Level for API response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderbookLevel {
    pub price: f64,
    pub quantity: f64,
    pub total: f64,
    pub orders: usize,
}

/// Binance-style orderbook response (for frontend compatibility)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinanceOrderbook {
    pub symbol: String,
    #[serde(rename = "lastUpdateId")]
    pub last_update_id: u64,
    pub bids: Vec<[String; 2]>,
    pub asks: Vec<[String; 2]>,
}

/// Extended Binance-style response with additional market data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinanceOrderbookExtended {
    #[serde(flatten)]
    pub orderbook: BinanceOrderbook,
    #[serde(rename = "midPrice")]
    pub mid_price: Option<String>,
    #[serde(rename = "bestBid")]
    pub best_bid: Option<String>,
    #[serde(rename = "bestAsk")]
    pub best_ask: Option<String>,
    #[serde(rename = "spreadBps")]
    pub spread_bps: Option<u64>,
    #[serde(rename = "totalBidDepth")]
    pub total_bid_depth: String,
    #[serde(rename = "totalAskDepth")]
    pub total_ask_depth: String,
    pub timestamp: u64,
}

/// Shared pool registry wrapped for async access
pub type SharedPoolRegistry = Arc<RwLock<PoolRegistry>>;

/// Query parameters for orderbook endpoint
#[derive(Debug, Deserialize)]
pub struct OrderbookQuery {
    /// Pool to query (sui_usdc, wal_usdc, deep_usdc). Defaults to sui_usdc
    #[serde(default = "default_pool")]
    pub pool: String,
    /// Optional session_id to get session-specific orderbook (reflects consumed liquidity)
    pub session_id: Option<String>,
}

fn default_pool() -> String {
    "sui_usdc".to_string()
}

/// GET /api/orderbook - Returns the current orderbook snapshot
pub async fn get_orderbook(
    State(state): State<AppState>,
    Query(query): Query<OrderbookQuery>,
) -> Json<OrderbookResponse> {
    let pool_id = match PoolId::from_str(&query.pool) {
        Some(id) => id,
        None => {
            return Json(OrderbookResponse {
                success: false,
                error: Some(format!(
                    "Invalid pool '{}'. Valid pools: sui_usdc, wal_usdc, deep_usdc",
                    query.pool
                )),
                orderbook: None,
                stats: None,
            });
        }
    };

    // Try to use session-specific orderbook if session_id provided
    let session_arc = if let Some(ref sid) = query.session_id {
        state.session_manager.get_session(sid).await
    } else {
        None
    };

    let snapshot = if let Some(ref session_arc) = session_arc {
        let session = session_arc.read().await;
        match session.orderbooks.get(&pool_id) {
            Some(ob) => sandbox_orderbook_to_snapshot(ob),
            None => {
                return Json(OrderbookResponse {
                    success: false,
                    error: Some(format!(
                        "Pool '{}' orderbook not built",
                        pool_id.display_name()
                    )),
                    orderbook: None,
                    stats: None,
                });
            }
        }
    } else {
        // Global orderbook (no session)
        let orderbooks = state.orderbooks.read().await;
        match orderbooks.get(&pool_id) {
            Some(ob) => sandbox_orderbook_to_snapshot(ob),
            None => {
                return Json(OrderbookResponse {
                    success: false,
                    error: Some(format!(
                        "Pool '{}' orderbook not built",
                        pool_id.display_name()
                    )),
                    orderbook: None,
                    stats: None,
                });
            }
        }
    };

    // Get stats from registry for object counts
    let registry = state.pool_registry.read().await;
    let stats = registry.get(pool_id).map(|loader| {
        let s = loader.stats();
        StatsResponse {
            total_objects: s.total_objects,
            asks_slices: s.asks_slices,
            bids_slices: s.bids_slices,
            max_checkpoint: s.max_checkpoint,
            max_version: s.max_version,
        }
    });

    Json(OrderbookResponse {
        success: true,
        error: None,
        orderbook: Some(snapshot),
        stats,
    })
}

/// GET /api/pools - List all available pools and their status
pub async fn list_pools(State(state): State<AppState>) -> Json<PoolsListResponse> {
    let registry = state.pool_registry.read().await;
    let orderbooks = state.orderbooks.read().await;
    let summary = registry.summary();

    // Include all possible pools with their loaded status
    let pools: Vec<PoolInfo> = PoolId::all()
        .iter()
        .map(|pool_id| {
            let loaded = registry.is_loaded(*pool_id);
            let pool_summary = summary.pools.iter().find(|p| p.pool_id == *pool_id);
            let ob = orderbooks.get(pool_id);

            PoolInfo {
                pool_id: pool_id.as_str().to_string(),
                display_name: pool_id.display_name().to_string(),
                loaded,
                orderbook_ready: ob.is_some(),
                mid_price: ob.and_then(|o| o.mid_price()),
                bid_levels: ob.map(|o| o.bids.len()),
                ask_levels: ob.map(|o| o.asks.len()),
                total_objects: pool_summary.map(|p| p.total_objects),
                asks_slices: pool_summary.map(|p| p.asks_slices),
                bids_slices: pool_summary.map(|p| p.bids_slices),
                checkpoint: pool_summary.map(|p| p.checkpoint),
            }
        })
        .collect();

    Json(PoolsListResponse {
        total_loaded: summary.total_pools,
        pools,
    })
}

/// GET /api/orderbook/depth - Returns Binance-style orderbook depth
pub async fn get_depth(
    State(state): State<AppState>,
    Query(query): Query<OrderbookQuery>,
) -> Json<BinanceDepthResponse> {
    let pool_id = match PoolId::from_str(&query.pool) {
        Some(id) => id,
        None => {
            return Json(BinanceDepthResponse {
                success: false,
                error: Some(format!(
                    "Invalid pool '{}'. Valid pools: sui_usdc, wal_usdc, deep_usdc",
                    query.pool
                )),
                data: None,
            });
        }
    };

    let orderbooks = state.orderbooks.read().await;
    let ob = match orderbooks.get(&pool_id) {
        Some(ob) => ob,
        None => {
            return Json(BinanceDepthResponse {
                success: false,
                error: Some(format!(
                    "Pool '{}' orderbook not built",
                    pool_id.display_name()
                )),
                data: None,
            });
        }
    };

    let depth = sandbox_orderbook_to_binance(ob);
    Json(BinanceDepthResponse {
        success: true,
        error: None,
        data: Some(depth),
    })
}

/// GET /api/orderbook/stats - Get loaded state statistics
pub async fn get_stats(
    State(state): State<AppState>,
    Query(query): Query<OrderbookQuery>,
) -> Json<StatsOnlyResponse> {
    let pool_id = match PoolId::from_str(&query.pool) {
        Some(id) => id,
        None => {
            return Json(StatsOnlyResponse {
                loaded: false,
                pool: query.pool,
                stats: None,
            });
        }
    };

    let registry = state.pool_registry.read().await;

    let loader = match registry.get(pool_id) {
        Some(l) => l,
        None => {
            return Json(StatsOnlyResponse {
                loaded: false,
                pool: pool_id.as_str().to_string(),
                stats: None,
            });
        }
    };

    if !loader.is_loaded() {
        return Json(StatsOnlyResponse {
            loaded: false,
            pool: pool_id.as_str().to_string(),
            stats: None,
        });
    }

    let stats = loader.stats();
    Json(StatsOnlyResponse {
        loaded: true,
        pool: pool_id.as_str().to_string(),
        stats: Some(StatsResponse {
            total_objects: stats.total_objects,
            asks_slices: stats.asks_slices,
            bids_slices: stats.bids_slices,
            max_checkpoint: stats.max_checkpoint,
            max_version: stats.max_version,
        }),
    })
}

// --- Conversion helpers: SandboxOrderbook -> API response types ---

/// Convert a MoveVM-built SandboxOrderbook to an OrderbookSnapshot for the API
fn sandbox_orderbook_to_snapshot(ob: &SandboxOrderbook) -> OrderbookSnapshot {
    let price_div = ob.price_divisor_value();
    let base_scale = 10f64.powi(ob.base_decimals as i32);

    let bids: Vec<OrderbookLevel> = ob
        .bids
        .iter()
        .map(|l| {
            let price = l.price as f64 / price_div;
            let quantity = l.total_quantity as f64 / base_scale;
            OrderbookLevel {
                price,
                quantity,
                total: price * quantity,
                orders: l.order_count,
            }
        })
        .collect();

    let asks: Vec<OrderbookLevel> = ob
        .asks
        .iter()
        .map(|l| {
            let price = l.price as f64 / price_div;
            let quantity = l.total_quantity as f64 / base_scale;
            OrderbookLevel {
                price,
                quantity,
                total: price * quantity,
                orders: l.order_count,
            }
        })
        .collect();

    let best_bid = bids.first().map(|l| l.price);
    let best_ask = asks.first().map(|l| l.price);
    let mid_price = match (best_bid, best_ask) {
        (Some(bid), Some(ask)) => Some((bid + ask) / 2.0),
        (Some(bid), None) => Some(bid),
        (None, Some(ask)) => Some(ask),
        _ => None,
    };
    let spread_bps = match (best_bid, best_ask) {
        (Some(bid), Some(ask)) if bid > 0.0 => Some(((ask - bid).abs() / bid * 10_000.0) as u64),
        _ => None,
    };

    let base_symbol = match ob.pool_id {
        PoolId::SuiUsdc => "SUI",
        PoolId::DeepUsdc => "DEEP",
        PoolId::WalUsdc => "WAL",
        PoolId::DebugUsdc => "DBG",
    };

    OrderbookSnapshot {
        pool_id: ob.pool_id.as_str().to_string(),
        base_symbol: base_symbol.to_string(),
        quote_symbol: "USDC".to_string(),
        mid_price,
        best_bid,
        best_ask,
        spread_bps,
        bids,
        asks,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    }
}

/// Convert a MoveVM-built SandboxOrderbook to Binance-style format
fn sandbox_orderbook_to_binance(ob: &SandboxOrderbook) -> BinanceOrderbookExtended {
    let price_div = ob.price_divisor_value();
    let base_scale = 10f64.powi(ob.base_decimals as i32);

    let base_symbol = match ob.pool_id {
        PoolId::SuiUsdc => "SUI",
        PoolId::DeepUsdc => "DEEP",
        PoolId::WalUsdc => "WAL",
        PoolId::DebugUsdc => "DBG",
    };
    let symbol = format!("{}USDC", base_symbol);

    let bids: Vec<[String; 2]> = ob
        .bids
        .iter()
        .map(|l| {
            let price = l.price as f64 / price_div;
            let quantity = l.total_quantity as f64 / base_scale;
            [format!("{:.6}", price), format!("{:.4}", quantity)]
        })
        .collect();

    let asks: Vec<[String; 2]> = ob
        .asks
        .iter()
        .map(|l| {
            let price = l.price as f64 / price_div;
            let quantity = l.total_quantity as f64 / base_scale;
            [format!("{:.6}", price), format!("{:.4}", quantity)]
        })
        .collect();

    let bid_depth: f64 = ob
        .bids
        .iter()
        .map(|l| l.total_quantity as f64 / base_scale)
        .sum();
    let ask_depth: f64 = ob
        .asks
        .iter()
        .map(|l| l.total_quantity as f64 / base_scale)
        .sum();

    let best_bid = ob.best_bid();
    let best_ask = ob.best_ask();
    let mid_price = ob.mid_price();
    let spread_bps = ob.spread_bps();

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    BinanceOrderbookExtended {
        orderbook: BinanceOrderbook {
            symbol,
            last_update_id: ob.checkpoint,
            bids,
            asks,
        },
        mid_price: mid_price.map(|p| format!("{:.6}", p)),
        best_bid: best_bid.map(|p| format!("{:.6}", p)),
        best_ask: best_ask.map(|p| format!("{:.6}", p)),
        spread_bps,
        total_bid_depth: format!("{:.4}", bid_depth),
        total_ask_depth: format!("{:.4}", ask_depth),
        timestamp,
    }
}

// Request/Response types

#[derive(Debug, Serialize)]
pub struct OrderbookResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orderbook: Option<OrderbookSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<StatsResponse>,
}

#[derive(Debug, Serialize)]
pub struct PoolsListResponse {
    pub total_loaded: usize,
    pub pools: Vec<PoolInfo>,
}

#[derive(Debug, Serialize)]
pub struct PoolInfo {
    pub pool_id: String,
    pub display_name: String,
    pub loaded: bool,
    pub orderbook_ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mid_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bid_levels: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ask_levels: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_objects: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asks_slices: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bids_slices: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct StatsResponse {
    pub total_objects: usize,
    pub asks_slices: usize,
    pub bids_slices: usize,
    pub max_checkpoint: u64,
    pub max_version: u64,
}

#[derive(Debug, Serialize)]
pub struct StatsOnlyResponse {
    pub loaded: bool,
    pub pool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<StatsResponse>,
}

#[derive(Debug, Serialize)]
pub struct BinanceDepthResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<BinanceOrderbookExtended>,
}
