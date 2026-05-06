//! Admin endpoints (stubs).
//!
//! All routes under `/admin` require the caller to present a bearer token
//! that belongs to the `admin` interface key in `config.auth.interface_tokens`.
//! Regular bot tokens are rejected.
//!
//! These are placeholder implementations.  Full functionality will be added
//! in a later iteration.

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::InterfaceToken;
use crate::error::{AppError, ApiResult};
use crate::AppState;

// =============================================================================
// Admin token guard
// =============================================================================

/// Wraps `InterfaceToken` and additionally checks that the token belongs to
/// the `admin` interface.  Used as an extractor on admin routes.
pub struct AdminToken(pub String);

impl<S> axum::extract::FromRequestParts<S> for AdminToken
where
    S: Send + Sync + std::ops::Deref<Target = AppState>,
{
    type Rejection = axum::response::Response;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        // Re-use the generic interface token extractor.
        // We can't call InterfaceToken::from_request_parts directly here
        // without the same state type, so we inline the check.
        let app: &AppState = state.deref();
        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .ok_or_else(|| AppError::Unauthorized.into_response())?;

        let name = app
            .config
            .auth
            .identify_token(token)
            .ok_or_else(|| AppError::Unauthorized.into_response())?;

        if name != "admin" {
            return Err(AppError::Unauthorized.into_response());
        }

        Ok(AdminToken(name.to_string()))
    }
}

// =============================================================================
// Prepaid codes
// =============================================================================

#[derive(Deserialize)]
pub struct CreateCodeRequest {
    pub granted_tier: String,
    pub issued_to:    Option<Uuid>,
    pub expires_at:   Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize)]
pub struct CreateCodeResponse {
    pub id:   Uuid,
    pub code: Uuid,
}

/// `POST /admin/codes` — creates a new prepaid code.
pub async fn create_code(
    _token: AdminToken,
    State(_state): State<AppState>,
    Json(_body): Json<CreateCodeRequest>,
) -> ApiResult<(StatusCode, Json<CreateCodeResponse>)> {
    // TODO: insert into prepaid_codes and return the generated code UUID.
    Err(AppError::Internal("not yet implemented".to_string()))
}

// =============================================================================
// Users
// =============================================================================

#[derive(Serialize)]
pub struct UserSummary {
    pub id:           Uuid,
    pub display_name: Option<String>,
    pub tier:         String,
    pub created_at:   chrono::DateTime<chrono::Utc>,
}

/// `GET /admin/users` — lists all registered users.
pub async fn list_users(
    _token: AdminToken,
    State(_state): State<AppState>,
) -> ApiResult<Json<Vec<UserSummary>>> {
    // TODO: query users table and return summaries.
    Err(AppError::Internal("not yet implemented".to_string()))
}

/// `PATCH /admin/users/:id/tier` — updates a user's account tier.
pub async fn set_user_tier(
    _token: AdminToken,
    axum::extract::Path(_user_id): axum::extract::Path<Uuid>,
    State(_state): State<AppState>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<StatusCode> {
    // TODO: validate tier string and update users.tier.
    Err(AppError::Internal("not yet implemented".to_string()))
}
