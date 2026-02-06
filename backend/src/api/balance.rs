//! Balance and faucet endpoints

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::api::AppState;
use crate::types::{ApiError, ApiResult};

const SUI_TYPE: &str = "0x2::sui::SUI";
const USDC_TYPE: &str =
    "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC";
const WAL_TYPE: &str =
    "0x356a26eb9e012a68958082340d4c4116e7f55615cf27affcff209cf0ae544f59::wal::WAL";
const DEEP_TYPE: &str =
    "0xdeeb7a4662eec9f2f3def03fb937a663dddaa2e215b8078a284d026b7946c270::deep::DEEP";
const DEBUG_TYPE: &str =
    "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa::debug_token::DEBUG_TOKEN";

#[derive(Debug, Serialize)]
pub struct BalanceResponse {
    pub session_id: String,
    pub balances: TokenBalances,
}

#[derive(Debug, Serialize)]
pub struct TokenBalances {
    /// SUI balance in MIST (1 SUI = 1_000_000_000 MIST)
    pub sui: String,
    pub sui_human: f64,
    /// USDC balance (6 decimals)
    pub usdc: String,
    pub usdc_human: f64,
    /// DEEP balance (6 decimals)
    pub deep: String,
    pub deep_human: f64,
    /// WAL balance (9 decimals)
    pub wal: String,
    pub wal_human: f64,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub custom: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct FaucetRequest {
    pub session_id: String,
    pub token: String, // "sui" | "usdc" | "wal" | "deep"
    pub amount: String,
}

#[derive(Debug, Serialize)]
pub struct FaucetResponse {
    pub success: bool,
    pub new_balance: String,
    pub new_balance_human: f64,
    pub token: String,
}

/// GET /api/balance/:session_id - Get token balances for a session
pub async fn get_balance(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<Json<BalanceResponse>> {
    let session_arc = state
        .session_manager
        .get_session(&session_id)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("Session not found: {}", session_id)))?;

    let session = session_arc.read().await;
    let b = &session.balances;

    Ok(Json(BalanceResponse {
        session_id,
        balances: TokenBalances {
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
        },
    }))
}

/// POST /api/faucet - Mint tokens into a session
pub async fn faucet(
    State(state): State<AppState>,
    Json(req): Json<FaucetRequest>,
) -> ApiResult<Json<FaucetResponse>> {
    let session_arc = state
        .session_manager
        .get_session(&req.session_id)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("Session not found: {}", req.session_id)))?;

    let debug_symbol = state.debug_pool.read().await.token_symbol.to_uppercase();
    let token_upper = req.token.to_uppercase();
    let token = if token_upper == "DEBUG" || token_upper == "DBG" || token_upper == debug_symbol {
        debug_symbol.clone()
    } else {
        token_upper
    };
    if !["SUI", "USDC", "WAL", "DEEP"].contains(&token.as_str()) && token != debug_symbol {
        return Err(ApiError::BadRequest(format!("Unknown token: {}", token)));
    }

    let amount: u64 = req
        .amount
        .parse()
        .map_err(|_| ApiError::BadRequest("Invalid amount".into()))?;

    let coin_type = match token.as_str() {
        "SUI" => SUI_TYPE,
        "USDC" => USDC_TYPE,
        "WAL" => WAL_TYPE,
        "DEEP" => DEEP_TYPE,
        _ if token == debug_symbol => DEBUG_TYPE,
        _ => return Err(ApiError::BadRequest(format!("Unknown token: {}", token))),
    };

    let router = state
        .router
        .as_ref()
        .ok_or_else(|| ApiError::Internal("MoveVM router is not initialized".into()))?;
    let vm_result = router
        .vm_faucet(coin_type.to_string(), amount)
        .await
        .map_err(|e| {
            ApiError::Internal(format!(
                "VM faucet execution failed for {} (type {}): {}",
                token, coin_type, e
            ))
        })?;
    if vm_result.amount != amount {
        return Err(ApiError::Internal(format!(
            "VM faucet amount mismatch: requested {}, minted {}",
            amount, vm_result.amount
        )));
    }

    let mut session = session_arc.write().await;
    session.balances.add(&token, vm_result.amount);

    let new_balance = session.balances.get(&token);
    let decimals = match token.as_str() {
        "SUI" | "WAL" => 9,
        "USDC" | "DEEP" => 6,
        _ if token == debug_symbol => 9,
        _ => 9,
    };

    Ok(Json(FaucetResponse {
        success: true,
        new_balance: new_balance.to_string(),
        new_balance_human: new_balance as f64 / 10f64.powi(decimals),
        token,
    }))
}
