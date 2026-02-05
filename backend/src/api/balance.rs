//! Balance and faucet endpoints

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::api::AppState;
use crate::types::{ApiError, ApiResult};

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

    let token = req.token.to_uppercase();
    if !["SUI", "USDC", "WAL", "DEEP"].contains(&token.as_str()) {
        return Err(ApiError::BadRequest(format!("Unknown token: {}", token)));
    }

    let amount: u64 = req
        .amount
        .parse()
        .map_err(|_| ApiError::BadRequest("Invalid amount".into()))?;

    let mut session = session_arc.write().await;
    session.balances.add(&token, amount);

    let new_balance = session.balances.get(&token);
    let decimals = match token.as_str() {
        "SUI" | "WAL" => 9,
        "USDC" | "DEEP" => 6,
        _ => 9,
    };

    Ok(Json(FaucetResponse {
        success: true,
        new_balance: new_balance.to_string(),
        new_balance_human: new_balance as f64 / 10f64.powi(decimals),
        token,
    }))
}
