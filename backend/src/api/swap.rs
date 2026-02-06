//! Swap execution endpoints using Move VM
//!
//! Provides swap quotes and execution using MoveVM quote PTBs.
//! Supports direct pool routes and cross-pool two-hop routes
//! via the router thread (e.g., SUI -> USDC -> WAL).

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::api::AppState;
use crate::sandbox::router::{DebugPoolInfo, RouterHandle};
use crate::sandbox::state_loader::PoolId;
use crate::sandbox::swap_executor::{CommandInfo, EventInfo, PtbExecution, UserBalances};
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
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub custom: HashMap<String, String>,
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
            custom: b
                .custom
                .iter()
                .map(|(symbol, amount)| (symbol.clone(), amount.to_string()))
                .collect(),
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

fn is_debug_token(token: &str, debug_symbol: &str) -> bool {
    let t = token.to_uppercase();
    let debug = debug_symbol.to_uppercase();
    t == "DBG" || t == "DEBUG" || t == debug
}

/// Determine which pool to use based on tokens (single-pool only)
fn determine_pool(from: &str, to: &str, debug_symbol: &str) -> Option<PoolId> {
    let tokens = [from.to_uppercase(), to.to_uppercase()];
    let has_usdc = tokens.iter().any(|t| t == "USDC");
    let has_sui = tokens.iter().any(|t| t == "SUI");
    let has_deep = tokens.iter().any(|t| t == "DEEP");
    let has_wal = tokens.iter().any(|t| t == "WAL");
    let has_dbg = tokens.iter().any(|t| is_debug_token(t, debug_symbol));

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
        if has_dbg {
            return Some(PoolId::DebugUsdc);
        }
    }
    None
}

/// Determine the route for a swap, including two-hop routes
fn determine_route(from: &str, to: &str, debug_symbol: &str) -> Option<Route> {
    let from_upper = from.to_uppercase();
    let to_upper = to.to_uppercase();

    // If one side is USDC, it's a single-pool swap
    if from_upper == "USDC" || to_upper == "USDC" {
        return determine_pool(from, to, debug_symbol).map(Route::SinglePool);
    }

    // Neither side is USDC -> two-hop via USDC
    let first_pool = pool_for_base(&from_upper, debug_symbol)?;
    let second_pool = pool_for_base(&to_upper, debug_symbol)?;

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
fn pool_for_base(token: &str, debug_symbol: &str) -> Option<PoolId> {
    if is_debug_token(token, debug_symbol) {
        return Some(PoolId::DebugUsdc);
    }
    match token {
        "SUI" => Some(PoolId::SuiUsdc),
        "WAL" => Some(PoolId::WalUsdc),
        "DEEP" => Some(PoolId::DeepUsdc),
        _ => None,
    }
}

fn get_decimals(token: &str, debug_symbol: &str) -> i32 {
    let upper = token.to_uppercase();
    if is_debug_token(&upper, debug_symbol) {
        return 9;
    }
    match upper.as_str() {
        "SUI" | "WAL" => 9,
        "USDC" | "DEEP" => 6,
        _ => 9,
    }
}

fn format_human(amount: u64, decimals: i32) -> f64 {
    amount as f64 / 10f64.powi(decimals)
}

fn normalize_token(token: &str, debug_symbol: &str) -> String {
    let upper = token.to_uppercase();
    if is_debug_token(&upper, debug_symbol) {
        debug_symbol.to_uppercase()
    } else {
        upper
    }
}

async fn sync_debug_pool_state(state: &AppState, info: &DebugPoolInfo) {
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

async fn ensure_debug_pool_and_sync(state: &AppState, router: &RouterHandle) -> ApiResult<()> {
    let info = router
        .ensure_debug_pool()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to ensure debug pool: {}", e)))?;
    sync_debug_pool_state(state, &info).await;
    Ok(())
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

    let debug_symbol = state.debug_pool.read().await.token_symbol.clone();
    let from = normalize_token(&req.from_token, &debug_symbol);
    let to = normalize_token(&req.to_token, &debug_symbol);

    if from == to {
        return Err(ApiError::BadRequest("Cannot swap same token".into()));
    }

    // Determine route (optional explicit pool override for direct swaps)
    let route = if let Some(ref p) = req.pool {
        let pool_id = PoolId::from_str(p)
            .ok_or_else(|| ApiError::BadRequest(format!("Invalid pool: {}", p)))?;
        Route::SinglePool(pool_id)
    } else {
        determine_route(&from, &to, &debug_symbol)
            .ok_or_else(|| ApiError::BadRequest(format!("No route found for {} -> {}", from, to)))?
    };

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
            execute_single_pool_swap(
                &state,
                session_arc,
                pool_id,
                &from,
                &to,
                &debug_symbol,
                amount,
                start,
            )
            .await
        }
        Route::TwoHop {
            first_pool,
            second_pool,
        } => {
            execute_two_hop_swap(
                &state,
                session_arc,
                first_pool,
                second_pool,
                &from,
                &to,
                &debug_symbol,
                amount,
                start,
            )
            .await
        }
    }
}

/// Execute a single-pool swap with a real MoveVM pool::swap_exact_* PTB.
async fn execute_single_pool_swap(
    state: &AppState,
    session_arc: std::sync::Arc<tokio::sync::RwLock<crate::sandbox::swap_executor::TradingSession>>,
    pool_id: PoolId,
    from: &str,
    to: &str,
    debug_symbol: &str,
    amount: u64,
    start: std::time::Instant,
) -> ApiResult<Json<SwapResponse>> {
    let is_sell = from != "USDC";
    let router = state.router.as_ref().ok_or_else(|| {
        ApiError::Internal("MoveVM router is not initialized for single-hop quoting".into())
    })?;

    if pool_id == PoolId::DebugUsdc {
        ensure_debug_pool_and_sync(state, router).await?;
    }

    // Read mid price and DEEP balance without holding lock across await.
    let (mid_price, deep_budget) = {
        let session = session_arc.read().await;
        let mid = session
            .orderbooks
            .get(&pool_id)
            .and_then(|ob| ob.mid_price())
            .unwrap_or(0.0);
        (mid, session.balances.deep)
    };

    let vm_swap = router
        .execute_single_hop_swap(pool_id, amount, deep_budget, is_sell)
        .await
        .map_err(|e| {
            ApiError::Internal(format!(
                "MoveVM single-hop swap failed for {}: {}",
                pool_id.display_name(),
                e
            ))
        })?;
    if vm_swap.output_amount == 0 {
        return Err(ApiError::BadRequest(format!(
            "No output returned by MoveVM swap for {}",
            pool_id.display_name()
        )));
    }

    let consumed_input = amount.saturating_sub(vm_swap.input_refund);
    let input_human = format_human(consumed_input, get_decimals(from, debug_symbol));
    let output_human = format_human(vm_swap.output_amount, get_decimals(to, debug_symbol));
    let effective_price = if is_sell {
        if input_human > 0.0 {
            output_human / input_human
        } else {
            0.0
        }
    } else if output_human > 0.0 {
        input_human / output_human
    } else {
        0.0
    };

    let price_impact_bps = if mid_price > 0.0 {
        ((effective_price - mid_price).abs() / mid_price * 10_000.0) as u32
    } else {
        0
    };

    let commands = vec![
        CommandInfo {
            index: 0,
            command_type: "MoveCall".to_string(),
            package: "0x2".to_string(),
            module: "coin".to_string(),
            function: "split".to_string(),
            type_args: vec![],
        },
        CommandInfo {
            index: 1,
            command_type: "MoveCall".to_string(),
            package: "0x2".to_string(),
            module: "coin".to_string(),
            function: "split".to_string(),
            type_args: vec![],
        },
        CommandInfo {
            index: 2,
            command_type: "MoveCall".to_string(),
            package: "0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809"
                .to_string(),
            module: "pool".to_string(),
            function: if is_sell {
                "swap_exact_base_for_quote".to_string()
            } else {
                "swap_exact_quote_for_base".to_string()
            },
            type_args: vec![],
        },
        CommandInfo {
            index: 3,
            command_type: "MoveCall".to_string(),
            package: "0x2".to_string(),
            module: "coin".to_string(),
            function: "value".to_string(),
            type_args: vec![],
        },
        CommandInfo {
            index: 4,
            command_type: "MoveCall".to_string(),
            package: "0x2".to_string(),
            module: "coin".to_string(),
            function: "value".to_string(),
            type_args: vec![],
        },
        CommandInfo {
            index: 5,
            command_type: "MoveCall".to_string(),
            package: "0x2".to_string(),
            module: "coin".to_string(),
            function: "value".to_string(),
            type_args: vec![],
        },
        CommandInfo {
            index: 6,
            command_type: "MoveCall".to_string(),
            package: "0x2".to_string(),
            module: "coin".to_string(),
            function: "join".to_string(),
            type_args: vec![],
        },
        CommandInfo {
            index: 7,
            command_type: "MoveCall".to_string(),
            package: "0x2".to_string(),
            module: "coin".to_string(),
            function: "join".to_string(),
            type_args: vec![],
        },
        CommandInfo {
            index: 8,
            command_type: "MoveCall".to_string(),
            package: "0x2".to_string(),
            module: "transfer".to_string(),
            function: "public_transfer".to_string(),
            type_args: vec![],
        },
    ];
    let events: Vec<EventInfo> = vm_swap
        .events
        .iter()
        .map(|e| EventInfo {
            event_type: e.event_type.clone(),
            data: serde_json::json!({ "bcs": e.data_hex }),
        })
        .collect();
    let ptb_execution = PtbExecution {
        commands,
        status: "Success".to_string(),
        effects_digest: None,
        events,
        created_objects: vec![],
        mutated_objects: vec![
            pool_id.display_name().to_string(),
            format!("VMReserveCoin<{}>", from),
            "VMReserveCoin<DEEP>".to_string(),
        ],
        deleted_objects: vec![],
    };

    let mut session = session_arc.write().await;
    let execution_time = start.elapsed().as_millis() as u64;
    let result = session.apply_vm_swap(
        from,
        to,
        amount,
        vm_swap.input_refund,
        deep_budget,
        vm_swap.deep_refund,
        vm_swap.output_amount,
        effective_price,
        vm_swap.gas_used,
        execution_time,
        ptb_execution,
    );

    match result {
        Ok(swap_result) => {
            let input_human = format_human(consumed_input, get_decimals(from, debug_symbol));
            let output_human = format_human(swap_result.output_amount, get_decimals(to, debug_symbol));
            let requested_input_human = format_human(amount, get_decimals(from, debug_symbol));

            let message = format!(
                "Successfully traded {:.4} {} (requested {:.4}) for {:.4} {} @ ${:.6}",
                input_human,
                from,
                requested_input_human,
                output_human,
                to,
                swap_result.effective_price
            );

            let commands: Vec<CommandDetail> = swap_result
                .ptb_execution
                .commands
                .iter()
                .map(|cmd| {
                    let description = match cmd.function.as_str() {
                        "split" => match cmd.index {
                            0 => format!("Split {} input coin from VM reserve", from),
                            1 => "Split DEEP fee coin from VM reserve".to_string(),
                            _ => "Split coin from VM reserve".to_string(),
                        },
                        "swap_exact_base_for_quote" => {
                            format!("Execute DeepBook market sell: {} -> USDC", from)
                        }
                        "swap_exact_quote_for_base" => {
                            format!("Execute DeepBook market buy: USDC -> {}", to)
                        }
                        "value" => match cmd.index {
                            3 => format!("Read {} output amount from VM return coin", to),
                            4 => format!("Read {} refund amount from VM return coin", from),
                            5 => "Read DEEP refund amount from VM return coin".to_string(),
                            _ => "Read coin amount from VM return object".to_string(),
                        },
                        "join" => match cmd.index {
                            6 => format!("Join {} refund back into VM reserve", from),
                            7 => "Join DEEP refund back into VM reserve".to_string(),
                            _ => "Join refund coin back into VM reserve".to_string(),
                        },
                        "public_transfer" => match cmd.index {
                            8 => format!("Transfer {} output coin to sender", to),
                            _ => "Transfer returned coin to sender".to_string(),
                        },
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
                "PTB executed {} commands via MoveVM: reserve coin splits -> pool::swap_exact_* on {} -> coin::value(...) -> refund joins -> output transfer.",
                commands.len(),
                pool_id.display_name()
            );

            Ok(Json(SwapResponse {
                success: true,
                error: None,
                input_token: from.to_string(),
                output_token: to.to_string(),
                input_amount: amount.to_string(),
                input_amount_human: format_human(amount, get_decimals(from, debug_symbol)),
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
                input_amount_human: format_human(amount, get_decimals(from, debug_symbol)),
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

/// Execute a two-hop swap: from_token -> USDC -> to_token.
/// Runs a real chained MoveVM PTB with two DeepBook pool::swap_exact_* calls.
async fn execute_two_hop_swap(
    state: &AppState,
    session_arc: std::sync::Arc<tokio::sync::RwLock<crate::sandbox::swap_executor::TradingSession>>,
    first_pool: PoolId,
    second_pool: PoolId,
    from: &str,
    to: &str,
    debug_symbol: &str,
    amount: u64,
    start: std::time::Instant,
) -> ApiResult<Json<SwapResponse>> {
    let router = state.router.as_ref().ok_or_else(|| {
        ApiError::Internal("MoveVM router is not initialized for two-hop quoting".into())
    })?;

    if first_pool == PoolId::DebugUsdc || second_pool == PoolId::DebugUsdc {
        ensure_debug_pool_and_sync(state, router).await?;
    }

    // Ensure both pools exist and compute mids without holding lock across await.
    let (first_mid, second_mid, deep_budget) = {
        let session = session_arc.read().await;
        (
            session
                .orderbooks
                .get(&first_pool)
                .and_then(|ob| ob.mid_price())
                .unwrap_or(0.0),
            session
                .orderbooks
                .get(&second_pool)
                .and_then(|ob| ob.mid_price())
                .unwrap_or(0.0),
            session.balances.deep,
        )
    };

    let vm_swap = router
        .execute_two_hop_swap(first_pool, second_pool, amount, deep_budget)
        .await
        .map_err(|e| {
            let err_text = e.to_string();
            if err_text.contains("pool::swap_exact_quantity")
                && err_text.contains("ABORTED")
                && err_text.contains("sub_status: Some(6)")
            {
                ApiError::BadRequest(format!(
                    "Two-hop swap amount is too small for DeepBook execution on at least one leg; increase input amount and retry ({} -> {}).",
                    first_pool.display_name(),
                    second_pool.display_name(),
                ))
            } else {
                ApiError::Internal(format!(
                    "MoveVM two-hop swap failed ({} -> {}): {}",
                    first_pool.display_name(),
                    second_pool.display_name(),
                    err_text
                ))
            }
        })?;
    if vm_swap.output_amount == 0 {
        return Err(ApiError::BadRequest(
            "No output returned by MoveVM two-hop swap".into(),
        ));
    }

    // Calculate effective price and impact
    let from_decimals = get_decimals(from, debug_symbol);
    let to_decimals = get_decimals(to, debug_symbol);
    let consumed_input = amount.saturating_sub(vm_swap.input_refund);
    let input_human = format_human(consumed_input, from_decimals);
    let output_human = format_human(vm_swap.output_amount, to_decimals);
    let usdc_intermediate_human = vm_swap.intermediate_amount as f64 / 1_000_000.0;

    let effective_price = if input_human > 0.0 {
        output_human / input_human
    } else {
        0.0
    };

    // Estimate price impact from both legs using session orderbooks
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

    let commands = vec![
        CommandInfo {
            index: 0,
            command_type: "MoveCall".to_string(),
            package: "0x2".to_string(),
            module: "coin".to_string(),
            function: "split".to_string(),
            type_args: vec![],
        },
        CommandInfo {
            index: 1,
            command_type: "MoveCall".to_string(),
            package: "0x2".to_string(),
            module: "coin".to_string(),
            function: "split".to_string(),
            type_args: vec![],
        },
        CommandInfo {
            index: 2,
            command_type: "MoveCall".to_string(),
            package: "0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809"
                .to_string(),
            module: "pool".to_string(),
            function: "swap_exact_base_for_quote".to_string(),
            type_args: vec![],
        },
        CommandInfo {
            index: 3,
            command_type: "MoveCall".to_string(),
            package: "0x2".to_string(),
            module: "coin".to_string(),
            function: "value".to_string(),
            type_args: vec![],
        },
        CommandInfo {
            index: 4,
            command_type: "MoveCall".to_string(),
            package: "0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809"
                .to_string(),
            module: "pool".to_string(),
            function: "swap_exact_quote_for_base".to_string(),
            type_args: vec![],
        },
        CommandInfo {
            index: 5,
            command_type: "MoveCall".to_string(),
            package: "0x2".to_string(),
            module: "coin".to_string(),
            function: "value".to_string(),
            type_args: vec![],
        },
        CommandInfo {
            index: 6,
            command_type: "MoveCall".to_string(),
            package: "0x2".to_string(),
            module: "coin".to_string(),
            function: "value".to_string(),
            type_args: vec![],
        },
        CommandInfo {
            index: 7,
            command_type: "MoveCall".to_string(),
            package: "0x2".to_string(),
            module: "coin".to_string(),
            function: "value".to_string(),
            type_args: vec![],
        },
        CommandInfo {
            index: 8,
            command_type: "MoveCall".to_string(),
            package: "0x2".to_string(),
            module: "coin".to_string(),
            function: "value".to_string(),
            type_args: vec![],
        },
        CommandInfo {
            index: 9,
            command_type: "MoveCall".to_string(),
            package: "0x2".to_string(),
            module: "coin".to_string(),
            function: "join".to_string(),
            type_args: vec![],
        },
        CommandInfo {
            index: 10,
            command_type: "MoveCall".to_string(),
            package: "0x2".to_string(),
            module: "coin".to_string(),
            function: "join".to_string(),
            type_args: vec![],
        },
        CommandInfo {
            index: 11,
            command_type: "MoveCall".to_string(),
            package: "0x2".to_string(),
            module: "coin".to_string(),
            function: "join".to_string(),
            type_args: vec![],
        },
        CommandInfo {
            index: 12,
            command_type: "MoveCall".to_string(),
            package: "0x2".to_string(),
            module: "transfer".to_string(),
            function: "public_transfer".to_string(),
            type_args: vec![],
        },
    ];
    let events: Vec<EventInfo> = vm_swap
        .events
        .iter()
        .map(|e| EventInfo {
            event_type: e.event_type.clone(),
            data: serde_json::json!({ "bcs": e.data_hex }),
        })
        .collect();
    let ptb_execution = PtbExecution {
        commands,
        status: "Success".to_string(),
        effects_digest: None,
        events,
        created_objects: vec![],
        mutated_objects: vec![
            first_pool.display_name().to_string(),
            second_pool.display_name().to_string(),
            format!("VMReserveCoin<{}>", from),
            "VMReserveCoin<USDC>".to_string(),
            "VMReserveCoin<DEEP>".to_string(),
        ],
        deleted_objects: vec![],
    };

    let mut session = session_arc.write().await;
    let execution_time = start.elapsed().as_millis() as u64;
    let result = session.apply_vm_swap(
        from,
        to,
        amount,
        vm_swap.input_refund,
        deep_budget,
        vm_swap.deep_refund,
        vm_swap.output_amount,
        effective_price,
        vm_swap.gas_used,
        execution_time,
        ptb_execution,
    );

    match result {
        Ok(swap_result) => {
            let requested_input_human = format_human(amount, get_decimals(from, debug_symbol));

            let message = format!(
                "Successfully traded {:.4} {} (requested {:.4}) -> {:.2} USDC -> {:.4} {} (two-hop)",
                input_human, from, requested_input_human, usdc_intermediate_human, output_human, to
            );

            let commands: Vec<CommandDetail> = swap_result
                .ptb_execution
                .commands
                .iter()
                .map(|cmd| {
                    let description = match cmd.function.as_str() {
                        "split" => match cmd.index {
                            0 => format!("Split {} input coin from VM reserve", from),
                            1 => "Split DEEP fee coin from VM reserve".to_string(),
                            _ => "Split coin from VM reserve".to_string(),
                        },
                        "swap_exact_base_for_quote" => {
                            format!("Execute first leg: {} -> USDC", from)
                        }
                        "swap_exact_quote_for_base" => {
                            format!("Execute second leg: USDC -> {}", to)
                        }
                        "value" => match cmd.index {
                            3 => "Read intermediate USDC output from leg 1".to_string(),
                            5 => format!("Read {} output amount from leg 2", to),
                            6 => format!("Read {} refund amount from leg 1", from),
                            7 => "Read USDC refund amount from leg 2".to_string(),
                            8 => "Read DEEP refund amount from leg 2".to_string(),
                            _ => "Read coin amount from VM return object".to_string(),
                        },
                        "join" => match cmd.index {
                            9 => format!("Join {} refund back into VM reserve", from),
                            10 => "Join USDC refund back into VM reserve".to_string(),
                            11 => "Join DEEP refund back into VM reserve".to_string(),
                            _ => "Join refund coin back into VM reserve".to_string(),
                        },
                        "public_transfer" => match cmd.index {
                            12 => format!("Transfer {} output coin to sender", to),
                            _ => "Transfer returned coin to sender".to_string(),
                        },
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
                "PTB executed {} commands via MoveVM: reserve coin splits -> pool::swap_exact_base_for_quote({} -> USDC) -> pool::swap_exact_quote_for_base(USDC -> {}) -> coin::value(...) -> refund joins -> output transfer.",
                commands.len(), from, to
            );

            Ok(Json(SwapResponse {
                success: true,
                error: None,
                input_token: from.to_string(),
                output_token: to.to_string(),
                input_amount: amount.to_string(),
                input_amount_human: format_human(amount, from_decimals),
                output_amount: swap_result.output_amount.to_string(),
                output_amount_human: output_human,
                effective_price: swap_result.effective_price,
                price_impact_bps,
                gas_used: swap_result.gas_used.to_string(),
                execution_time_ms: execution_time,
                execution_method: "Move VM Two-Hop Pool PTB Execution".to_string(),
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
                input_amount_human: format_human(amount, get_decimals(from, debug_symbol)),
                output_amount: "0".to_string(),
                output_amount_human: 0.0,
                effective_price: 0.0,
                price_impact_bps: 0,
                gas_used: "0".to_string(),
                execution_time_ms: execution_time,
                execution_method: "Move VM Two-Hop Pool PTB Execution".to_string(),
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
    let debug_symbol = state.debug_pool.read().await.token_symbol.clone();
    let from = normalize_token(&req.from_token, &debug_symbol);
    let to = normalize_token(&req.to_token, &debug_symbol);

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
        determine_route(&from, &to, &debug_symbol)
            .ok_or_else(|| ApiError::BadRequest(format!("No route found for {} -> {}", from, to)))?
    };

    match route {
        Route::SinglePool(pool_id) => {
            get_single_pool_quote(&state, pool_id, &from, &to, &debug_symbol, amount, &req).await
        }
        Route::TwoHop {
            first_pool,
            second_pool,
        } => {
            get_two_hop_quote(
                &state,
                first_pool,
                second_pool,
                &from,
                &to,
                &debug_symbol,
                amount,
                &req,
            )
            .await
        }
    }
}

/// Quote for a single-pool swap using MoveVM quote calls.
async fn get_single_pool_quote(
    state: &AppState,
    pool_id: PoolId,
    from: &str,
    to: &str,
    debug_symbol: &str,
    amount: u64,
    req: &QuoteRequest,
) -> ApiResult<Json<QuoteResponse>> {
    let is_sell = from != "USDC";
    let router = state.router.as_ref().ok_or_else(|| {
        ApiError::Internal("MoveVM router is not initialized for single-hop quoting".into())
    })?;

    if pool_id == PoolId::DebugUsdc {
        ensure_debug_pool_and_sync(state, router).await?;
    }

    let mid_price = if let Some(ref sid) = req.session_id {
        if let Some(session_arc) = state.session_manager.get_session(sid).await {
            let session = session_arc.read().await;
            session
                .orderbooks
                .get(&pool_id)
                .and_then(|ob| ob.mid_price())
                .unwrap_or(0.0)
        } else {
            let orderbooks = state.orderbooks.read().await;
            orderbooks
                .get(&pool_id)
                .and_then(|ob| ob.mid_price())
                .unwrap_or(0.0)
        }
    } else {
        let orderbooks = state.orderbooks.read().await;
        orderbooks
            .get(&pool_id)
            .and_then(|ob| ob.mid_price())
            .unwrap_or(0.0)
    };

    let vm_quote = router
        .quote_single_hop(pool_id, amount, is_sell)
        .await
        .map_err(|e| {
            ApiError::Internal(format!(
                "MoveVM single-hop quote failed for {}: {}",
                pool_id.display_name(),
                e
            ))
        })?;

    let input_human = format_human(amount, get_decimals(from, debug_symbol));
    let output_human = format_human(vm_quote.output_amount, get_decimals(to, debug_symbol));

    let effective_price = if is_sell {
        if input_human > 0.0 {
            output_human / input_human
        } else {
            0.0
        }
    } else if output_human > 0.0 {
        input_human / output_human
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
        pool: pool_id.display_name().to_string(),
        input_token: from.to_string(),
        output_token: to.to_string(),
        input_amount: amount.to_string(),
        input_amount_human: input_human,
        estimated_output: vm_quote.output_amount.to_string(),
        estimated_output_human: output_human,
        effective_price,
        mid_price,
        price_impact_bps,
        levels_consumed: 0,
        orders_matched: 0,
        fully_fillable: vm_quote.output_amount > 0,
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
    debug_symbol: &str,
    amount: u64,
    req: &QuoteRequest,
) -> ApiResult<Json<QuoteResponse>> {
    let router = state.router.as_ref().ok_or_else(|| {
        ApiError::Internal("MoveVM router is not initialized for two-hop quoting".into())
    })?;
    if first_pool == PoolId::DebugUsdc || second_pool == PoolId::DebugUsdc {
        ensure_debug_pool_and_sync(state, router).await?;
    }
    let router_quote = router
        .quote_two_hop(first_pool, second_pool, amount)
        .await
        .map_err(|e| {
            ApiError::Internal(format!(
                "MoveVM router two-hop quote failed ({} -> {}): {}",
                first_pool.display_name(),
                second_pool.display_name(),
                e
            ))
        })?;

    // Estimate mid price from orderbooks.
    let (first_mid, second_mid) = if let Some(ref sid) = req.session_id {
        if let Some(session_arc) = state.session_manager.get_session(sid).await {
            let session = session_arc.read().await;
            let first_mid = session
                .orderbooks
                .get(&first_pool)
                .and_then(|ob| ob.mid_price())
                .unwrap_or(0.0);
            let second_mid = session
                .orderbooks
                .get(&second_pool)
                .and_then(|ob| ob.mid_price())
                .unwrap_or(0.0);
            (first_mid, second_mid)
        } else {
            let orderbooks = state.orderbooks.read().await;
            let first_mid = orderbooks
                .get(&first_pool)
                .and_then(|ob| ob.mid_price())
                .unwrap_or(0.0);
            let second_mid = orderbooks
                .get(&second_pool)
                .and_then(|ob| ob.mid_price())
                .unwrap_or(0.0);
            (first_mid, second_mid)
        }
    } else {
        let orderbooks = state.orderbooks.read().await;
        let first_mid = orderbooks
            .get(&first_pool)
            .and_then(|ob| ob.mid_price())
            .unwrap_or(0.0);
        let second_mid = orderbooks
            .get(&second_pool)
            .and_then(|ob| ob.mid_price())
            .unwrap_or(0.0);
        (first_mid, second_mid)
    };

    let from_decimals = get_decimals(from, debug_symbol);
    let to_decimals = get_decimals(to, debug_symbol);
    let input_human = format_human(amount, from_decimals);
    let output_human = format_human(router_quote.final_output, to_decimals);
    let usdc_human = router_quote.intermediate_amount as f64 / 1_000_000.0;

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
        pool: format!(
            "{} + {}",
            first_pool.display_name(),
            second_pool.display_name()
        ),
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
            from,
            first_pool.display_name(),
            second_pool.display_name(),
            to
        ),
        route_type: "two_hop".to_string(),
        intermediate_amount: Some(usdc_human),
    }))
}
