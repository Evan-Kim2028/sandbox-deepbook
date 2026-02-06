//! System-level diagnostic endpoints.

use axum::{extract::State, Json};

use crate::api::AppState;
use crate::sandbox::router::RouterStartupCheckReport;
use crate::types::{ApiError, ApiResult};

/// GET /api/startup-check - Return fail-fast startup self-check diagnostics.
pub async fn get_startup_check(
    State(state): State<AppState>,
) -> ApiResult<Json<RouterStartupCheckReport>> {
    let router = state
        .router
        .as_ref()
        .ok_or_else(|| ApiError::Internal("MoveVM router is not initialized".into()))?;

    let report = router
        .startup_check()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to query startup-check: {}", e)))?;

    Ok(Json(report))
}

