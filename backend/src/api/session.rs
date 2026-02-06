//! Session management endpoints

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::api::AppState;
use crate::sandbox::swap_executor::{SwapResult, UserBalances};
use crate::types::{ApiError, ApiResult};

#[derive(Debug, Serialize)]
pub struct SessionResponse {
    pub session_id: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub checkpoint: u64,
    pub balances: BalanceInfo,
}

#[derive(Debug, Serialize)]
pub struct BalanceInfo {
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

impl From<&UserBalances> for BalanceInfo {
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
pub struct CreateSessionRequest {}

#[derive(Debug, Serialize)]
pub struct SwapHistoryResponse {
    pub session_id: String,
    pub swap_count: usize,
    pub history: Vec<SwapResult>,
}

#[derive(Debug, Serialize)]
pub struct ResetResponse {
    pub success: bool,
    pub session_id: String,
    pub message: String,
    pub balances: BalanceInfo,
}

/// POST /api/session - Create a new sandbox session
pub async fn create_session(
    State(state): State<AppState>,
    Json(_req): Json<Option<CreateSessionRequest>>,
) -> ApiResult<Json<SessionResponse>> {
    let session_id = state
        .session_manager
        .create_session()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to create session: {}", e)))?;

    let session_arc = state
        .session_manager
        .get_session(&session_id)
        .await
        .ok_or_else(|| ApiError::Internal("Session creation failed".into()))?;

    let session = session_arc.read().await;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    Ok(Json(SessionResponse {
        session_id,
        created_at: now,
        expires_at: now + 3600, // 1 hour TTL
        checkpoint: session.checkpoint,
        balances: BalanceInfo::from(&session.balances),
    }))
}

/// GET /api/session/:id - Get session info
pub async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<SessionResponse>> {
    let session_arc = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("Session not found: {}", id)))?;

    let session = session_arc.read().await;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Calculate elapsed time since session creation
    let elapsed_secs = session.created_at.elapsed().as_secs();
    let created_at = now.saturating_sub(elapsed_secs);
    let expires_at = created_at + 3600; // 1 hour from creation

    Ok(Json(SessionResponse {
        session_id: id,
        created_at,
        expires_at,
        checkpoint: session.checkpoint,
        balances: BalanceInfo::from(&session.balances),
    }))
}

/// GET /api/session/:id/history - Get swap history for a session
pub async fn get_swap_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<SwapHistoryResponse>> {
    let session_arc = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("Session not found: {}", id)))?;

    let session = session_arc.read().await;

    Ok(Json(SwapHistoryResponse {
        session_id: id,
        swap_count: session.swap_history.len(),
        history: session.swap_history.clone(),
    }))
}

/// POST /api/session/:id/reset - Reset session to initial state
pub async fn reset_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<ResetResponse>> {
    let session_arc = state
        .session_manager
        .get_session(&id)
        .await
        .ok_or_else(|| ApiError::NotFound(format!("Session not found: {}", id)))?;

    // Clone fresh orderbooks from global state for the reset
    let fresh_orderbooks = state.orderbooks.read().await.clone();
    let mut session = session_arc.write().await;
    session.reset(fresh_orderbooks);

    Ok(Json(ResetResponse {
        success: true,
        session_id: id,
        message: "Session reset to initial state".to_string(),
        balances: BalanceInfo::from(&session.balances),
    }))
}
