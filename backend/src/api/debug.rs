//! Debug pool management endpoints.

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};

use crate::api::AppState;
use crate::sandbox::router::DebugPoolCreateConfig;
use crate::types::{ApiError, ApiResult};

#[derive(Debug, Serialize)]
pub struct EnsureDebugPoolResponse {
    pub success: bool,
    pub created: bool,
    pub pool_object_id: String,
    pub token_symbol: String,
    pub token_name: String,
    pub token_description: String,
    pub token_icon_url: String,
    pub token_decimals: u8,
    pub token_type: String,
    pub config: DebugPoolConfigResponse,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct DebugPoolStatusResponse {
    pub success: bool,
    pub created: bool,
    pub pool_object_id: Option<String>,
    pub token_symbol: String,
    pub token_name: String,
    pub token_description: String,
    pub token_icon_url: String,
    pub token_decimals: u8,
    pub token_type: String,
    pub config: DebugPoolConfigResponse,
}

#[derive(Debug, Serialize)]
pub struct DebugPoolListResponse {
    pub success: bool,
    pub pools: Vec<DebugPoolStatusResponse>,
}

#[derive(Debug, Serialize)]
pub struct DebugPoolConfigResponse {
    pub tick_size: u64,
    pub lot_size: u64,
    pub min_size: u64,
    pub whitelisted_pool: bool,
    pub pay_with_deep: bool,
    pub bid_price: u64,
    pub ask_price: u64,
    pub bid_quantity: u64,
    pub ask_quantity: u64,
    pub base_liquidity: u64,
    pub quote_liquidity: u64,
    pub deep_fee_budget: u64,
}

#[derive(Debug, Deserialize)]
pub struct EnsureDebugPoolRequest {
    pub token_symbol: Option<String>,
    pub token_name: Option<String>,
    pub token_description: Option<String>,
    pub token_icon_url: Option<String>,
    pub tick_size: Option<u64>,
    pub lot_size: Option<u64>,
    pub min_size: Option<u64>,
    pub whitelisted_pool: Option<bool>,
    pub pay_with_deep: Option<bool>,
    pub bid_price: Option<u64>,
    pub ask_price: Option<u64>,
    pub bid_quantity: Option<u64>,
    pub ask_quantity: Option<u64>,
    pub base_liquidity: Option<u64>,
    pub quote_liquidity: Option<u64>,
    pub deep_fee_budget: Option<u64>,
}

impl EnsureDebugPoolRequest {
    fn has_overrides(&self) -> bool {
        self.token_symbol.is_some()
            || self.token_name.is_some()
            || self.token_description.is_some()
            || self.token_icon_url.is_some()
            || self.tick_size.is_some()
            || self.lot_size.is_some()
            || self.min_size.is_some()
            || self.whitelisted_pool.is_some()
            || self.pay_with_deep.is_some()
            || self.bid_price.is_some()
            || self.ask_price.is_some()
            || self.bid_quantity.is_some()
            || self.ask_quantity.is_some()
            || self.base_liquidity.is_some()
            || self.quote_liquidity.is_some()
            || self.deep_fee_budget.is_some()
    }
}

fn normalize_symbol(raw: &str) -> Result<String, ApiError> {
    let symbol = raw.trim().to_uppercase();
    if symbol.len() < 2 || symbol.len() > 12 {
        return Err(ApiError::BadRequest(
            "token_symbol must be 2-12 characters".into(),
        ));
    }
    if !symbol
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(ApiError::BadRequest(
            "token_symbol may only contain A-Z, 0-9, and _".into(),
        ));
    }
    Ok(symbol)
}

fn cfg_to_response(cfg: &DebugPoolCreateConfig) -> DebugPoolConfigResponse {
    DebugPoolConfigResponse {
        tick_size: cfg.tick_size,
        lot_size: cfg.lot_size,
        min_size: cfg.min_size,
        whitelisted_pool: cfg.whitelisted_pool,
        pay_with_deep: cfg.pay_with_deep,
        bid_price: cfg.bid_price,
        ask_price: cfg.ask_price,
        bid_quantity: cfg.bid_quantity,
        ask_quantity: cfg.ask_quantity,
        base_liquidity: cfg.base_liquidity,
        quote_liquidity: cfg.quote_liquidity,
        deep_fee_budget: cfg.deep_fee_budget,
    }
}

fn build_requested_config(req: EnsureDebugPoolRequest) -> Result<DebugPoolCreateConfig, ApiError> {
    let mut cfg = DebugPoolCreateConfig::default();

    if let Some(symbol) = req.token_symbol {
        cfg.token_symbol = normalize_symbol(&symbol)?;
    }
    if let Some(name) = req.token_name {
        cfg.token_name = name.trim().to_string();
    }
    if let Some(description) = req.token_description {
        cfg.token_description = description.trim().to_string();
    }
    if let Some(icon_url) = req.token_icon_url {
        cfg.token_icon_url = icon_url.trim().to_string();
    }
    if let Some(v) = req.tick_size {
        cfg.tick_size = v;
    }
    if let Some(v) = req.lot_size {
        cfg.lot_size = v;
    }
    if let Some(v) = req.min_size {
        cfg.min_size = v;
    }
    if let Some(v) = req.whitelisted_pool {
        cfg.whitelisted_pool = v;
    }
    if let Some(v) = req.pay_with_deep {
        cfg.pay_with_deep = v;
    }
    if let Some(v) = req.bid_price {
        cfg.bid_price = v;
    }
    if let Some(v) = req.ask_price {
        cfg.ask_price = v;
    }
    if let Some(v) = req.bid_quantity {
        cfg.bid_quantity = v;
    }
    if let Some(v) = req.ask_quantity {
        cfg.ask_quantity = v;
    }
    if let Some(v) = req.base_liquidity {
        cfg.base_liquidity = v;
    }
    if let Some(v) = req.quote_liquidity {
        cfg.quote_liquidity = v;
    }
    if let Some(v) = req.deep_fee_budget {
        cfg.deep_fee_budget = v;
    }

    Ok(cfg)
}

async fn sync_debug_state(state: &AppState, info: &crate::sandbox::router::DebugPoolInfo) {
    let mut debug = state.debug_pool.write().await;
    debug.created = true;
    debug.pool_object_id = Some(info.pool_object_id.clone());
    debug.token_symbol = info.token_symbol.clone();
    debug.token_name = info.config.token_name.clone();
    debug.token_description = info.config.token_description.clone();
    debug.token_icon_url = info.config.token_icon_url.clone();
    debug.token_decimals = info.config.token_decimals;
    debug.token_type = info.token_type.clone();
    debug.config = info.config.clone();
}

fn status_from_state(debug: &crate::api::DebugPoolState) -> DebugPoolStatusResponse {
    DebugPoolStatusResponse {
        success: true,
        created: debug.created,
        pool_object_id: debug.pool_object_id.clone(),
        token_symbol: debug.token_symbol.clone(),
        token_name: debug.token_name.clone(),
        token_description: debug.token_description.clone(),
        token_icon_url: debug.token_icon_url.clone(),
        token_decimals: debug.token_decimals,
        token_type: debug.token_type.clone(),
        config: cfg_to_response(&debug.config),
    }
}

/// GET /api/debug/pool - Return active debug pool configuration/status.
pub async fn get_debug_pool_status(
    State(state): State<AppState>,
) -> ApiResult<Json<DebugPoolStatusResponse>> {
    let debug = state.debug_pool.read().await;

    Ok(Json(status_from_state(&debug)))
}

/// GET /api/debug/pools - List created custom pools/tokens (currently max 1).
pub async fn list_debug_pools(
    State(state): State<AppState>,
) -> ApiResult<Json<DebugPoolListResponse>> {
    let debug = state.debug_pool.read().await;
    let pools = if debug.created {
        vec![status_from_state(&debug)]
    } else {
        Vec::new()
    };

    Ok(Json(DebugPoolListResponse {
        success: true,
        pools,
    }))
}

/// POST /api/debug/pool - Create+seed debug pool in local VM (idempotent).
pub async fn ensure_debug_pool(
    State(state): State<AppState>,
    Json(req): Json<Option<EnsureDebugPoolRequest>>,
) -> ApiResult<Json<EnsureDebugPoolResponse>> {
    let router = state
        .router
        .as_ref()
        .ok_or_else(|| ApiError::Internal("MoveVM router is not initialized".into()))?;

    let info = match req {
        Some(body) if body.has_overrides() => {
            let cfg = build_requested_config(body)?;
            router
                .ensure_debug_pool_with_config(cfg)
                .await
                .map_err(|e| ApiError::Internal(format!("Failed to ensure debug pool: {}", e)))?
        }
        _ => router
            .ensure_debug_pool()
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to ensure debug pool: {}", e)))?,
    };

    sync_debug_state(&state, &info).await;

    Ok(Json(EnsureDebugPoolResponse {
        success: true,
        created: true,
        pool_object_id: info.pool_object_id,
        token_symbol: info.token_symbol,
        token_name: info.config.token_name.clone(),
        token_description: info.config.token_description.clone(),
        token_icon_url: info.config.token_icon_url.clone(),
        token_decimals: info.config.token_decimals,
        token_type: info.token_type,
        config: cfg_to_response(&info.config),
        message: "Debug token/USDC pool is ready in local VM".to_string(),
    }))
}
