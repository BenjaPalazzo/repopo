use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

/// The unified error type returned by all API handlers.
///
/// Each variant maps to an appropriate HTTP status code and a JSON body of
/// the form `{ "error": "<message>" }`.
#[derive(Debug, Error)]
pub enum AppError {
    // --- 400 Bad Request ---
    #[error("invalid bounding box: {0}")]
    InvalidBoundingBox(#[from] shared::types::BoundingBoxError),

    #[error("invalid time range: {0}")]
    InvalidTimeRange(#[from] shared::types::TimeRangeError),

    #[error("validation error: {0}")]
    Validation(String),

    // --- 401 Unauthorized ---
    #[error("missing or invalid bearer token")]
    Unauthorized,

    // --- 403 Forbidden ---
    #[error("tier limit exceeded: {0}")]
    TierLimitExceeded(String),

    // --- 404 Not Found ---
    #[error("job not found")]
    JobNotFound,

    #[error("result not available: {0}")]
    ResultNotFound(String),

    // --- 409 Conflict ---
    #[error("prepaid code already redeemed or invalid")]
    InvalidPrepaidCode,

    // --- 422 Unprocessable ---
    #[error("no SAR images found for the given bounds and time range")]
    NoImagesFound,

    #[error("flight path error: {0}")]
    FlightPath(#[from] shared::types::FlightPathError),

    // --- 500 Internal Server Error ---
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("query error: {0}")]
    Query(#[from] shared::queries::QueryError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            // 400
            AppError::InvalidBoundingBox(_)  => (StatusCode::BAD_REQUEST, self.to_string()),
            AppError::InvalidTimeRange(_)    => (StatusCode::BAD_REQUEST, self.to_string()),
            AppError::Validation(m)          => (StatusCode::BAD_REQUEST, m.clone()),

            // 401
            AppError::Unauthorized           => (StatusCode::UNAUTHORIZED, self.to_string()),

            // 403
            AppError::TierLimitExceeded(m)   => (StatusCode::FORBIDDEN, m.clone()),

            // 404
            AppError::JobNotFound            => (StatusCode::NOT_FOUND, self.to_string()),
            AppError::ResultNotFound(m)      => (StatusCode::NOT_FOUND, m.clone()),

            // 409
            AppError::InvalidPrepaidCode     => (StatusCode::CONFLICT, self.to_string()),

            // 422
            AppError::NoImagesFound          => (StatusCode::UNPROCESSABLE_ENTITY, self.to_string()),
            AppError::FlightPath(_)          => (StatusCode::UNPROCESSABLE_ENTITY, self.to_string()),

            // 500
            AppError::Database(e) => {
                tracing::error!("database error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
            AppError::Query(e) => {
                tracing::error!("query error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
            AppError::Io(e) => {
                tracing::error!("io error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
            AppError::Internal(m) => {
                tracing::error!("internal error: {m}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
        };

        let body = Json(json!({ "error": message }));
        (status, body).into_response()
    }
}

/// Convenience alias.
pub type ApiResult<T> = Result<T, AppError>;
