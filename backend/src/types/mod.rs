//! Shared types and error handling

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

/// Result type for API handlers
pub type ApiResult<T> = Result<T, ApiError>;

/// API error types
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
    code: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BAD_REQUEST"),
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        };

        let body = Json(ErrorResponse {
            error: self.to_string(),
            code: code.to_string(),
        });

        (status, body).into_response()
    }
}
