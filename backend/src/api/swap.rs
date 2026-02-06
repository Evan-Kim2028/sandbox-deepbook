//! Swap execution endpoints using Move VM
//!
//! Provides swap quotes and execution by walking the MoveVM-built
//! SandboxOrderbook price levels. Supports cross-pool routes via
//! the router thread for two-hop swaps (e.g., SUI -> USDC -> WAL).

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};

use crate::api::AppState;
use crate::sandbox::orderbook_builder::SandboxOrderbook;
use crate::sandbox::state_loader::PoolId;
use crate::sandbox::swap_executor::UserBalances;
use crate::types::{ApiError, ApiResult};

#[derive(Debug, Deserialize)]
pub struct SwapRequest {
    pub session_id: String,
    pub pool: Option<String>,
    pub from_token: String,
    pub to_token: String,
    /// Amount in smallest unit (MIST for SUI, 6 decimals for USDC)
    pub amount: String,
}

#[derive(Debug, Serialize)]
pub struct SwapResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub input_token: String,
    pub output_token: String,
    pub input_amount: String,
    pub input_amount_human: f64,
    pub output_amount: String,
    pub output_amount_human: f64,
    pub effective_price: f64,
    pub price_impact_bps: u32,
    pub gas_used: String,
    pub execution_time_ms: u64,
    pub execution_method: String,
    pub message: String,
    pub ptb_execution: PtbExecutionInfo,
    pub balances_after: BalancesAfter,
    /// "direct" for single-pool, "two_hop" for cross-pool
    pub route_type: String,
    /// USDC intermediate amount for two-hop routes (human-readable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intermediate_amount: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct PtbExecutionInfo {
    pub commands: Vec<CommandDetail>,
    pub status: String,
    pub effects_digest: Option<String>,
    pub events: Vec<EventDetail>,
    pub summary: String,
}

#[derive(Debug, Serialize)]
pub struct CommandDetail {
    pub index: usize,
    pub command_type: String,
    pub package: String,
    pub module: String,
    pub function: String,
    pub type_args: Vec<String>,
    pub description: String,
}

#[derive(Debug, Serialize)]
pub struct EventDetail {
    pub event_type: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct BalancesAfter {
    pub sui: String,
    pub sui_human: f64,
    pub usdc: String,
    pub usdc_human: f64,
    pub deep: String,
    pub deep_human: f64,
    pub wal: String,
    pub wal_human: f64,
}

impl From<&UserBalances> for BalancesAfter {
    fn from(b: &UserBalances) -> Self {
        Self {
            sui: b.sui.to_string(),
            sui_human: b.sui as f64 / 1_000_000_000.0,
            usdc: b.usdc.to_string(),
            usdc_human: b.usdc as f64 / 1_000_000.0,
            deep: b.deep.to_string(),
            deep_human: b.deep as f64 / 1_000_000.0,
            wal: b.wal.to_string(),
            wal_human: b.wal as f64 / 1_000_000_000.0,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct QuoteRequest {
    pub pool: Option<String>,
    pub from_token: String,
    pub to_token: String,
    pub amount: String,
    /// Optional session_id to quote against session-specific orderbook (reflects consumed liquidity)
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct QuoteResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub pool: String,
    pub input_token: String,
    pub output_token: String,
    pub input_amount: String,
    pub input_amount_human: f64,
    pub estimated_output: String,
    pub estimated_output_human: f64,
    pub effective_price: f64,
    pub mid_price: f64,
    pub price_impact_bps: u32,
    pub levels_consumed: usize,
    pub orders_matched: usize,
    pub fully_fillable: bool,
    pub route: String,
    /// "direct" for single-pool, "two_hop" for cross-pool
    pub route_type: String,
    /// USDC intermediate amount for two-hop routes (human-readable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intermediate_amount: Option<f64>,
}

/// Route classification for a swap
enum Route {
    /// Direct single-pool swap (e.g., SUI <-> USDC)
    SinglePool(PoolId),
    /// Two-hop swap via USDC intermediate (e.g., SUI -> USDC -> WAL)
    TwoHop {
        first_pool: PoolId,
        second_pool: PoolId,
    },
}

/// Determine which pool to use based on tokens (single-pool only)
fn determine_pool(from: &str, to: &str) -> Option<PoolId> {
    let tokens = [from.to_uppercase(), to.to_uppercase()];
    let has_usdc = tokens.iter().any(|t| t == "USDC");
    let has_sui = tokens.iter().any(|t| t == "SUI");
    let has_deep = tokens.iter().any(|t| t == "DEEP");
    let has_wal = tokens.iter().any(|t| t == "WAL");

    if has_usdc {
        if has_sui {
            return Some(PoolId::SuiUsdc);
        }
        if has_deep {
            return Some(PoolId::DeepUsdc);
        }
        if has_wal {
            return Some(PoolId::WalUsdc);
        }
    }
    None
}

/// Determine the route for a swap, including two-hop routes
fn determine_route(from: &str, to: &str) -> Option<Route> {
    let from_upper = from.to_uppercase();
    let to_upper = to.to_uppercase();

    // If one side is USDC, it's a single-pool swap
    if from_upper == "USDC" || to_upper == "USDC" {
        return determine_pool(from, to).map(Route::SinglePool);
    }

    // Neither side is USDC -> two-hop via USDC
    let first_pool = pool_for_base(&from_upper)?;
    let second_pool = pool_for_base(&to_upper)?;

    // Don't allow same-token swaps
    if first_pool == second_pool {
        return None;
    }

    Some(Route::TwoHop {
        first_pool,
        second_pool,
    })
}

/// Get the USDC pool for a given base token
fn pool_for_base(token: &str) -> Option<PoolId> {
    match token {
        "SUI" => Some(PoolId::SuiUsdc),
        "WAL" => Some(PoolId::WalUsdc),
        "DEEP" => Some(PoolId::DeepUsdc),
        _ => None,
    }
}

/// Calculate swap quote by walking the MoveVM-built SandboxOrderbook
fn calculate_quote(
    ob: &SandboxOrderbook,
    input_amount: u64,
    is_sell: bool, // true = sell base for quote, false = buy base with quote
) -> QuoteCalculation {
    let price_divisor = ob.price_divisor_value();
    let base_scale = 10f64.powi(ob.base_decimals as i32);
    let quote_scale = 1_000_000.0; // USDC always 6 decimals

    let mut remaining_input = input_amount as f64;
    let mut total_output = 0.0f64;
    let mut levels_consumed = 0;
    let mut total_orders_matched = 0;

    let levels = if is_sell {
        // Selling base: take from bids (buyers), sorted highest first
        &ob.bids
    } else {
        // Buying base: take from asks (sellers), sorted lowest first
        &ob.asks
    };

    for level in levels {
        if remaining_input <= 0.0 {
            break;
        }

        let price_usd = level.price as f64 / price_divisor;
        let level_qty = level.total_quantity as f64 / base_scale;

        if is_sell {
            // Selling base tokens for quote (USDC)
            // remaining_input is in base units (smallest unit)
            let input_base = remaining_input / base_scale;
            let take_qty = level_qty.min(input_base);
            if take_qty > 0.0 {
                total_output += take_qty * price_usd * quote_scale;
                remaining_input -= take_qty * base_scale;
                levels_consumed += 1;
                total_orders_matched += level.order_count;
            }
        } else {
            // Buying base tokens with quote (USDC)
            // remaining_input is in quote units (USDC smallest unit)
            let input_quote = remaining_input / quote_scale;
            let cost_for_level = level_qty * price_usd;
            if cost_for_level <= input_quote {
                // Take entire level
                total_output += level_qty * base_scale;
                remaining_input -= cost_for_level * quote_scale;
                levels_consumed += 1;
                total_orders_matched += level.order_count;
            } else {
                // Partial fill
                let take_qty = input_quote / price_usd;
                total_output += take_qty * base_scale;
                remaining_input = 0.0;
                levels_consumed += 1;
                total_orders_matched += 1;
            }
        }
    }

    // Calculate effective price
    let input_human = if is_sell {
        input_amount as f64 / base_scale
    } else {
        input_amount as f64 / quote_scale
    };

    let output_human = if is_sell {
        total_output / quote_scale
    } else {
        total_output / base_scale
    };

    let effective_price = if is_sell && output_human > 0.0 {
        output_human / input_human
    } else if !is_sell && input_human > 0.0 {
        input_human / output_human
    } else {
        0.0
    };

    QuoteCalculation {
        output_amount: total_output as u64,
        output_human,
        effective_price,
        levels_consumed,
        orders_matched: total_orders_matched,
        fully_filled: remaining_input <= 0.0,
    }
}

struct QuoteCalculation {
    output_amount: u64,
    output_human: f64,
    effective_price: f64,
    levels_consumed: usize,
    orders_matched: usize,
    fully_filled: bool,
}

fn get_decimals(token: &str) -> i32 {
    match token.to_uppercase().as_str() {
        "SUI" | "WAL" => 9,
        "USDC" | "DEEP" => 6,
        _ => 9,
    }
}

fn format_human(amount: u64, decimals: i32) -> f64 {
    amount as f64 / 10f64.powi(decimals)
}

/// POST /api/swap - Execute a swap in a session
pub async fn execute_swap(
    State(state): State<AppState>,
    Json(req): Json<SwapRequest>,
) -> ApiResult<Json<SwapResponse>> {
    let start = std::time::Instant::now();

    // Validate request
    if req.session_id.is_empty() {
        return Err(ApiError::BadRequest("session_id required".into()));
    }

    let from = req.from_token.to_uppercase();
    let to = req.to_token.to_uppercase();

    if from == to {
        return Err(ApiError::BadRequest("Cannot swap same token".into()));
    }

    // Determine route
    let route = determine_route(&from, &to).ok_or_else(|| {
        ApiError::BadRequest(format!("No route found for {} -> {}", from, to))
    })?;

    // Get session
    let session_arc = state
        .session_manager
        .get_session(&req.session_id)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("Session not found: {}", req.session_id)))?;

    // Parse amount
    let amount: u64 = req
        .amount
        .parse()
        .map_err(|_| ApiError::BadRequest("Invalid amount".into()))?;

    match route {
        Route::SinglePool(pool_id) => {
            execute_single_pool_swap(&state, session_arc, pool_id, &from, &to, amount, start).await
        }
        Route::TwoHop { first_pool, second_pool } => {
            execute_two_hop_swap(&state, session_arc, first_pool, second_pool, &from, &to, amount, start).await
        }
    }
}

/// Execute a single-pool swap (existing logic)
async fn execute_single_pool_swap(
    _state: &AppState,
    session_arc: std::sync::Arc<tokio::sync::RwLock<crate::sandbox::swap_executor::TradingSession>>,
    pool_id: PoolId,
    from: &str,
    to: &str,
    amount: u64,
    start: std::time::Instant,
) -> ApiResult<Json<SwapResponse>> {
    let is_sell = from != "USDC";

    let mut session = session_arc.write().await;
    let ob = session.orderbooks.get(&pool_id).ok_or_else(|| {
        ApiError::BadRequest(format!("Pool {} orderbook not built", pool_id.display_name()))
    })?;

    let quote = calculate_quote(ob, amount, is_sell);
    let mid_price = ob.mid_price().unwrap_or(0.0);

    let price_impact_bps = if mid_price > 0.0 {
        ((quote.effective_price - mid_price).abs() / mid_price * 10_000.0) as u32
    } else {
        0
    };

    let result = session.execute_swap(
        pool_id, from, to, amount, quote.output_amount, quote.effective_price,
    );

    if result.as_ref().map(|r| r.success).unwrap_or(false) {
        if let Some(session_ob) = session.orderbooks.get_mut(&pool_id) {
            session_ob.consume_liquidity(amount, is_sell);
        }
    }

    match result {
        Ok(swap_result) => {
            let execution_time = start.elapsed().as_millis() as u64;
            let from_decimals = get_decimals(from);
            let to_decimals = get_decimals(to);
            let input_human = format_human(amount, from_decimals);
            let output_human = format_human(swap_result.output_amount, to_decimals);

            let message = format!(
                "Successfully traded {:.4} {} for {:.4} {} @ ${:.6}",
                input_human, from, output_human, to, swap_result.effective_price
            );

            let commands: Vec<CommandDetail> = swap_result
                .ptb_execution
                .commands
                .iter()
                .map(|cmd| {
                    let description = match cmd.function.as_str() {
                        "split" => format!("Split {} coin for exact input amount", from),
                        "swap_exact_base_for_quote" => {
                            format!("Execute DeepBook market sell: {} -> USDC", from)
                        }
                        "swap_exact_quote_for_base" => {
                            format!("Execute DeepBook market buy: USDC -> {}", to)
                        }
                        "public_transfer" => format!("Transfer output {} to sender", to),
                        _ => format!("{}::{}", cmd.module, cmd.function),
                    };
                    CommandDetail {
                        index: cmd.index,
                        command_type: cmd.command_type.clone(),
                        package: cmd.package.clone(),
                        module: cmd.module.clone(),
                        function: cmd.function.clone(),
                        type_args: cmd.type_args.clone(),
                        description,
                    }
                })
                .collect();

            let summary = format!(
                "PTB executed {} commands: split input coin -> swap via DeepBook {} pool -> transfer output. Matched {} orders across {} price levels.",
                commands.len(),
                pool_id.display_name(),
                quote.orders_matched,
                quote.levels_consumed
            );

            Ok(Json(SwapResponse {
                success: true,
                error: None,
                input_token: from.to_string(),
                output_token: to.to_string(),
                input_amount: amount.to_string(),
                input_amount_human: input_human,
                output_amount: swap_result.output_amount.to_string(),
                output_amount_human: output_human,
                effective_price: swap_result.effective_price,
                price_impact_bps,
                gas_used: swap_result.gas_used.to_string(),
                execution_time_ms: execution_time,
                execution_method: "Move VM DeepBook PTB Execution".to_string(),
                message,
                ptb_execution: PtbExecutionInfo {
                    commands,
                    status: swap_result.ptb_execution.status,
                    effects_digest: swap_result.ptb_execution.effects_digest,
                    events: swap_result
                        .ptb_execution
                        .events
                        .iter()
                        .map(|e| EventDetail {
                            event_type: e.event_type.clone(),
                            data: e.data.clone(),
                        })
                        .collect(),
                    summary,
                },
                balances_after: BalancesAfter::from(&swap_result.balances_after),
                route_type: "direct".to_string(),
                intermediate_amount: None,
            }))
        }
        Err(e) => {
            let execution_time = start.elapsed().as_millis() as u64;
            Ok(Json(SwapResponse {
                success: false,
                error: Some(e.to_string()),
                input_token: from.to_string(),
                output_token: to.to_string(),
                input_amount: amount.to_string(),
                input_amount_human: format_human(amount, get_decimals(from)),
                output_amount: "0".to_string(),
                output_amount_human: 0.0,
                effective_price: 0.0,
                price_impact_bps: 0,
                gas_used: "0".to_string(),
                execution_time_ms: execution_time,
                execution_method: "Move VM DeepBook PTB Execution".to_string(),
                message: format!("Swap failed: {}", e),
                ptb_execution: PtbExecutionInfo {
                    commands: vec![],
                    status: "Failed".to_string(),
                    effects_digest: None,
                    events: vec![],
                    summary: format!("Transaction aborted: {}", e),
                },
                balances_after: BalancesAfter::from(&session.balances),
                route_type: "direct".to_string(),
                intermediate_amount: None,
            }))
        }
    }
}

/// Execute a two-hop swap: from_token -> USDC -> to_token
/// Chains two orderbook walks atomically, consuming liquidity from both pools.
async fn execute_two_hop_swap(
    _state: &AppState,
    session_arc: std::sync::Arc<tokio::sync::RwLock<crate::sandbox::swap_executor::TradingSession>>,
    first_pool: PoolId,
    second_pool: PoolId,
    from: &str,
    to: &str,
    amount: u64,
    start: std::time::Instant,
) -> ApiResult<Json<SwapResponse>> {
    let mut session = session_arc.write().await;

    // Leg 1: Sell from_token for USDC (sell base on first pool)
    let first_ob = session.orderbooks.get(&first_pool).ok_or_else(|| {
        ApiError::BadRequest(format!("Pool {} orderbook not built", first_pool.display_name()))
    })?;
    let leg1_quote = calculate_quote(first_ob, amount, true); // sell base for quote

    if leg1_quote.output_amount == 0 {
        return Err(ApiError::BadRequest("No liquidity on first leg".into()));
    }

    // Leg 2: Buy to_token with USDC (buy base on second pool)
    let second_ob = session.orderbooks.get(&second_pool).ok_or_else(|| {
        ApiError::BadRequest(format!("Pool {} orderbook not built", second_pool.display_name()))
    })?;
    let leg2_quote = calculate_quote(second_ob, leg1_quote.output_amount, false); // buy base with quote

    if leg2_quote.output_amount == 0 {
        return Err(ApiError::BadRequest("No liquidity on second leg".into()));
    }

    // Calculate effective price and impact
    let from_decimals = get_decimals(from);
    let to_decimals = get_decimals(to);
    let input_human = format_human(amount, from_decimals);
    let output_human = format_human(leg2_quote.output_amount, to_decimals);
    let usdc_intermediate_human = leg1_quote.output_amount as f64 / 1_000_000.0;

    let effective_price = if input_human > 0.0 {
        output_human / input_human
    } else {
        0.0
    };

    // Estimate price impact from both legs
    let first_mid = session.orderbooks.get(&first_pool).and_then(|ob| ob.mid_price()).unwrap_or(0.0);
    let second_mid = session.orderbooks.get(&second_pool).and_then(|ob| ob.mid_price()).unwrap_or(0.0);
    let ideal_output = if first_mid > 0.0 && second_mid > 0.0 {
        let usdc_ideal = input_human * first_mid;
        usdc_ideal / second_mid
    } else {
        0.0
    };
    let price_impact_bps = if ideal_output > 0.0 {
        ((ideal_output - output_human).abs() / ideal_output * 10_000.0) as u32
    } else {
        0
    };

    // Execute the swap: debit from_token, credit to_token
    let result = session.execute_two_hop_swap(
        first_pool,
        second_pool,
        from,
        to,
        amount,
        leg1_quote.output_amount,
        leg2_quote.output_amount,
        effective_price,
    );

    // Consume liquidity from both session orderbooks on success
    if result.as_ref().map(|r| r.success).unwrap_or(false) {
        if let Some(ob1) = session.orderbooks.get_mut(&first_pool) {
            ob1.consume_liquidity(amount, true); // sell base
        }
        if let Some(ob2) = session.orderbooks.get_mut(&second_pool) {
            ob2.consume_liquidity(leg1_quote.output_amount, false); // buy base with quote
        }
    }

    match result {
        Ok(swap_result) => {
            let execution_time = start.elapsed().as_millis() as u64;

            let message = format!(
                "Successfully traded {:.4} {} -> {:.2} USDC -> {:.4} {} (two-hop)",
                input_human, from, usdc_intermediate_human, output_human, to
            );

            let commands: Vec<CommandDetail> = swap_result
                .ptb_execution
                .commands
                .iter()
                .map(|cmd| {
                    let description = match cmd.function.as_str() {
                        "split" => format!("Split {} coin for exact input amount", from),
                        "swap_two_hop" => format!(
                            "Execute two-hop swap: {} -> USDC -> {} via router",
                            from, to
                        ),
                        "public_transfer" => format!("Transfer output {} to sender", to),
                        _ => format!("{}::{}", cmd.module, cmd.function),
                    };
                    CommandDetail {
                        index: cmd.index,
                        command_type: cmd.command_type.clone(),
                        package: cmd.package.clone(),
                        module: cmd.module.clone(),
                        function: cmd.function.clone(),
                        type_args: cmd.type_args.clone(),
                        description,
                    }
                })
                .collect();

            let summary = format!(
                "PTB executed {} commands: split input -> router::swap_two_hop({} -> USDC -> {}) -> transfer output. Matched {} orders across {} price levels.",
                commands.len(), from, to,
                leg1_quote.orders_matched + leg2_quote.orders_matched,
                leg1_quote.levels_consumed + leg2_quote.levels_consumed
            );

            Ok(Json(SwapResponse {
                success: true,
                error: None,
                input_token: from.to_string(),
                output_token: to.to_string(),
                input_amount: amount.to_string(),
                input_amount_human: input_human,
                output_amount: swap_result.output_amount.to_string(),
                output_amount_human: output_human,
                effective_price: swap_result.effective_price,
                price_impact_bps,
                gas_used: swap_result.gas_used.to_string(),
                execution_time_ms: execution_time,
                execution_method: "Move VM Router Two-Hop PTB Execution".to_string(),
                message,
                ptb_execution: PtbExecutionInfo {
                    commands,
                    status: swap_result.ptb_execution.status,
                    effects_digest: swap_result.ptb_execution.effects_digest,
                    events: swap_result
                        .ptb_execution
                        .events
                        .iter()
                        .map(|e| EventDetail {
                            event_type: e.event_type.clone(),
                            data: e.data.clone(),
                        })
                        .collect(),
                    summary,
                },
                balances_after: BalancesAfter::from(&swap_result.balances_after),
                route_type: "two_hop".to_string(),
                intermediate_amount: Some(usdc_intermediate_human),
            }))
        }
        Err(e) => {
            let execution_time = start.elapsed().as_millis() as u64;
            Ok(Json(SwapResponse {
                success: false,
                error: Some(e.to_string()),
                input_token: from.to_string(),
                output_token: to.to_string(),
                input_amount: amount.to_string(),
                input_amount_human: format_human(amount, get_decimals(from)),
                output_amount: "0".to_string(),
                output_amount_human: 0.0,
                effective_price: 0.0,
                price_impact_bps: 0,
                gas_used: "0".to_string(),
                execution_time_ms: execution_time,
                execution_method: "Move VM Router Two-Hop PTB Execution".to_string(),
                message: format!("Two-hop swap failed: {}", e),
                ptb_execution: PtbExecutionInfo {
                    commands: vec![],
                    status: "Failed".to_string(),
                    effects_digest: None,
                    events: vec![],
                    summary: format!("Two-hop transaction aborted: {}", e),
                },
                balances_after: BalancesAfter::from(&session.balances),
                route_type: "two_hop".to_string(),
                intermediate_amount: None,
            }))
        }
    }
}

/// POST /api/swap/quote - Get a quote without executing
pub async fn get_quote(
    State(state): State<AppState>,
    Json(req): Json<QuoteRequest>,
) -> ApiResult<Json<QuoteResponse>> {
    let from = req.from_token.to_uppercase();
    let to = req.to_token.to_uppercase();

    if from == to {
        return Err(ApiError::BadRequest("Cannot swap same token".into()));
    }

    // Parse amount
    let amount: u64 = req
        .amount
        .parse()
        .map_err(|_| ApiError::BadRequest("Invalid amount".into()))?;

    // Determine route
    let route = if let Some(ref p) = req.pool {
        // Explicit pool overrides route detection
        let pool_id = PoolId::from_str(p)
            .ok_or_else(|| ApiError::BadRequest(format!("Invalid pool: {}", p)))?;
        Route::SinglePool(pool_id)
    } else {
        determine_route(&from, &to).ok_or_else(|| {
            ApiError::BadRequest(format!("No route found for {} -> {}", from, to))
        })?
    };

    match route {
        Route::SinglePool(pool_id) => {
            get_single_pool_quote(&state, pool_id, &from, &to, amount, &req).await
        }
        Route::TwoHop { first_pool, second_pool } => {
            get_two_hop_quote(&state, first_pool, second_pool, &from, &to, amount, &req).await
        }
    }
}

/// Quote for a single-pool swap (existing logic)
async fn get_single_pool_quote(
    state: &AppState,
    pool_id: PoolId,
    from: &str,
    to: &str,
    amount: u64,
    req: &QuoteRequest,
) -> ApiResult<Json<QuoteResponse>> {
    let is_sell = from != "USDC";

    let session_arc = if let Some(ref sid) = req.session_id {
        state.session_manager.get_session(sid).await
    } else {
        None
    };

    struct QuoteResult {
        quote: QuoteCalculation,
        mid_price: f64,
        base_scale: f64,
    }

    let qr = if let Some(ref session_arc) = session_arc {
        let session = session_arc.read().await;
        let ob = session.orderbooks.get(&pool_id).ok_or_else(|| {
            ApiError::BadRequest(format!("Pool {} orderbook not built", pool_id.display_name()))
        })?;
        QuoteResult {
            quote: calculate_quote(ob, amount, is_sell),
            mid_price: ob.mid_price().unwrap_or(0.0),
            base_scale: 10f64.powi(ob.base_decimals as i32),
        }
    } else {
        let orderbooks = state.orderbooks.read().await;
        let ob = orderbooks.get(&pool_id).ok_or_else(|| {
            ApiError::BadRequest(format!("Pool {} orderbook not built", pool_id.display_name()))
        })?;
        QuoteResult {
            quote: calculate_quote(ob, amount, is_sell),
            mid_price: ob.mid_price().unwrap_or(0.0),
            base_scale: 10f64.powi(ob.base_decimals as i32),
        }
    };

    let quote_scale = 1_000_000.0;
    let input_human = if is_sell {
        amount as f64 / qr.base_scale
    } else {
        amount as f64 / quote_scale
    };

    let price_impact_bps = if qr.mid_price > 0.0 {
        ((qr.quote.effective_price - qr.mid_price).abs() / qr.mid_price * 10_000.0) as u32
    } else {
        0
    };

    Ok(Json(QuoteResponse {
        success: true,
        error: None,
        pool: pool_id.display_name().to_string(),
        input_token: from.to_string(),
        output_token: to.to_string(),
        input_amount: amount.to_string(),
        input_amount_human: input_human,
        estimated_output: qr.quote.output_amount.to_string(),
        estimated_output_human: qr.quote.output_human,
        effective_price: qr.quote.effective_price,
        mid_price: qr.mid_price,
        price_impact_bps,
        levels_consumed: qr.quote.levels_consumed,
        orders_matched: qr.quote.orders_matched,
        fully_fillable: qr.quote.fully_filled,
        route: format!("{} -> DeepBook {} -> {}", from, pool_id.display_name(), to),
        route_type: "direct".to_string(),
        intermediate_amount: None,
    }))
}

/// Quote for a two-hop swap: from_token -> USDC -> to_token
async fn get_two_hop_quote(
    state: &AppState,
    first_pool: PoolId,
    second_pool: PoolId,
    from: &str,
    to: &str,
    amount: u64,
    req: &QuoteRequest,
) -> ApiResult<Json<QuoteResponse>> {
    // Try MoveVM router first, fall back to Rust simulation
    if let Some(ref router) = state.router {
        match router.quote_two_hop(first_pool, second_pool, amount).await {
            Ok(router_quote) => {
                let from_decimals = get_decimals(from);
                let to_decimals = get_decimals(to);
                let input_human = format_human(amount, from_decimals);
                let output_human = format_human(router_quote.final_output, to_decimals);
                let usdc_human = router_quote.intermediate_amount as f64 / 1_000_000.0;

                let effective_price = if input_human > 0.0 {
                    output_human / input_human
                } else {
                    0.0
                };

                // Estimate mid price from both pools
                let orderbooks = state.orderbooks.read().await;
                let first_mid = orderbooks.get(&first_pool).and_then(|ob| ob.mid_price()).unwrap_or(0.0);
                let second_mid = orderbooks.get(&second_pool).and_then(|ob| ob.mid_price()).unwrap_or(0.0);
                let mid_price = if first_mid > 0.0 && second_mid > 0.0 {
                    first_mid / second_mid
                } else {
                    0.0
                };
                let price_impact_bps = if mid_price > 0.0 {
                    ((effective_price - mid_price).abs() / mid_price * 10_000.0) as u32
                } else {
                    0
                };

                return Ok(Json(QuoteResponse {
                    success: true,
                    error: None,
                    pool: format!("{} + {}", first_pool.display_name(), second_pool.display_name()),
                    input_token: from.to_string(),
                    output_token: to.to_string(),
                    input_amount: amount.to_string(),
                    input_amount_human: input_human,
                    estimated_output: router_quote.final_output.to_string(),
                    estimated_output_human: output_human,
                    effective_price,
                    mid_price,
                    price_impact_bps,
                    levels_consumed: 0,
                    orders_matched: 0,
                    fully_fillable: router_quote.final_output > 0,
                    route: format!(
                        "{} -> DeepBook {} -> USDC -> DeepBook {} -> {}",
                        from, first_pool.display_name(), second_pool.display_name(), to
                    ),
                    route_type: "two_hop".to_string(),
                    intermediate_amount: Some(usdc_human),
                }));
            }
            Err(e) => {
                tracing::warn!("MoveVM router quote failed, falling back to Rust simulation: {}", e);
            }
        }
    }

    // Fallback: Rust simulation (walk both orderbooks)
    let session_arc = if let Some(ref sid) = req.session_id {
        state.session_manager.get_session(sid).await
    } else {
        None
    };

    // Helper to get orderbooks for quoting
    let (leg1_quote, leg2_quote, first_mid, second_mid) = if let Some(ref session_arc) = session_arc {
        let session = session_arc.read().await;
        let first_ob = session.orderbooks.get(&first_pool).ok_or_else(|| {
            ApiError::BadRequest(format!("Pool {} not built", first_pool.display_name()))
        })?;
        let leg1 = calculate_quote(first_ob, amount, true);
        let first_mid = first_ob.mid_price().unwrap_or(0.0);

        let second_ob = session.orderbooks.get(&second_pool).ok_or_else(|| {
            ApiError::BadRequest(format!("Pool {} not built", second_pool.display_name()))
        })?;
        let leg2 = calculate_quote(second_ob, leg1.output_amount, false);
        let second_mid = second_ob.mid_price().unwrap_or(0.0);

        (leg1, leg2, first_mid, second_mid)
    } else {
        let orderbooks = state.orderbooks.read().await;
        let first_ob = orderbooks.get(&first_pool).ok_or_else(|| {
            ApiError::BadRequest(format!("Pool {} not built", first_pool.display_name()))
        })?;
        let leg1 = calculate_quote(first_ob, amount, true);
        let first_mid = first_ob.mid_price().unwrap_or(0.0);

        let second_ob = orderbooks.get(&second_pool).ok_or_else(|| {
            ApiError::BadRequest(format!("Pool {} not built", second_pool.display_name()))
        })?;
        let leg2 = calculate_quote(second_ob, leg1.output_amount, false);
        let second_mid = second_ob.mid_price().unwrap_or(0.0);

        (leg1, leg2, first_mid, second_mid)
    };

    let from_decimals = get_decimals(from);
    let to_decimals = get_decimals(to);
    let input_human = format_human(amount, from_decimals);
    let output_human = format_human(leg2_quote.output_amount, to_decimals);
    let usdc_human = leg1_quote.output_amount as f64 / 1_000_000.0;

    let effective_price = if input_human > 0.0 {
        output_human / input_human
    } else {
        0.0
    };

    let mid_price = if first_mid > 0.0 && second_mid > 0.0 {
        first_mid / second_mid
    } else {
        0.0
    };

    let price_impact_bps = if mid_price > 0.0 {
        ((effective_price - mid_price).abs() / mid_price * 10_000.0) as u32
    } else {
        0
    };

    Ok(Json(QuoteResponse {
        success: true,
        error: None,
        pool: format!("{} + {}", first_pool.display_name(), second_pool.display_name()),
        input_token: from.to_string(),
        output_token: to.to_string(),
        input_amount: amount.to_string(),
        input_amount_human: input_human,
        estimated_output: leg2_quote.output_amount.to_string(),
        estimated_output_human: output_human,
        effective_price,
        mid_price,
        price_impact_bps,
        levels_consumed: leg1_quote.levels_consumed + leg2_quote.levels_consumed,
        orders_matched: leg1_quote.orders_matched + leg2_quote.orders_matched,
        fully_fillable: leg1_quote.fully_filled && leg2_quote.fully_filled,
        route: format!(
            "{} -> DeepBook {} -> USDC -> DeepBook {} -> {}",
            from, first_pool.display_name(), second_pool.display_name(), to
        ),
        route_type: "two_hop".to_string(),
        intermediate_amount: Some(usdc_human),
    }))
}
