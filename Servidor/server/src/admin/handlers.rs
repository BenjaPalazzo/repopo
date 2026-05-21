//! Admin endpoints.
//!
//! Todos los routes bajo `/admin` requieren un bearer token del tipo `admin`
//! en `config.auth.interface_tokens`.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AppError, ApiResult};
use crate::AppState;

// =============================================================================
// Admin token guard
// =============================================================================

/// Extractor que valida que el bearer token corresponda a la interfaz "admin".
/// Usa `AppState` directamente (igual que `InterfaceToken`) para satisfacer
/// el trait bound de Axum 0.8.
pub struct AdminToken(pub String);

impl axum::extract::FromRequestParts<AppState> for AdminToken {
    type Rejection = axum::response::Response;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        use axum::response::IntoResponse;

        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .ok_or_else(|| AppError::Unauthorized.into_response())?;

        let name = state
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
    pub total_jobs:   i32,
    pub issued_to:    Option<Uuid>,
    pub expires_at:   Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize)]
pub struct CreateCodeResponse {
    pub code: Uuid,
}

/// `POST /admin/codes`
pub async fn create_code(
    _token: AdminToken,
    State(state): State<AppState>,
    Json(body): Json<CreateCodeRequest>,
) -> ApiResult<(StatusCode, Json<CreateCodeResponse>)> {
    if !["demo", "free", "pro"].contains(&body.granted_tier.as_str()) {
        return Err(AppError::Validation(format!(
            "invalid granted_tier '{}': must be one of demo, free, pro",
            body.granted_tier
        )));
    }
    if body.total_jobs < 1 {
        return Err(AppError::Validation(
            "total_jobs must be at least 1".to_string(),
        ));
    }

    let code = sqlx::query_scalar!(
        r#"
        INSERT INTO prepaid_codes (granted_tier, total_jobs, issued_to, expires_at)
        VALUES ($1, $2, $3, $4)
        RETURNING code
        "#,
        body.granted_tier,
        body.total_jobs,
        body.issued_to,
        body.expires_at,
    )
    .fetch_one(&state.pool)
    .await?;

    tracing::info!(
        code = %code,
        tier = %body.granted_tier,
        total_jobs = body.total_jobs,
        "prepaid code created"
    );

    Ok((StatusCode::CREATED, Json(CreateCodeResponse { code })))
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

/// `GET /admin/users`
pub async fn list_users(
    _token: AdminToken,
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<UserSummary>>> {
    let rows = sqlx::query!(
        r#"
        SELECT id, display_name, tier, created_at
        FROM users
        ORDER BY created_at DESC
        "#,
    )
    .fetch_all(&state.pool)
    .await?;

    let users = rows
        .into_iter()
        .map(|r| UserSummary {
            id:           r.id,
            display_name: r.display_name,
            tier:         r.tier,
            created_at:   r.created_at,
        })
        .collect();

    Ok(Json(users))
}

#[derive(Deserialize)]
pub struct SetTierRequest {
    pub tier: String,
}

/// `PATCH /admin/users/:id/tier`
pub async fn set_user_tier(
    _token: AdminToken,
    Path(user_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(body): Json<SetTierRequest>,
) -> ApiResult<StatusCode> {
    if !["demo", "free", "pro"].contains(&body.tier.as_str()) {
        return Err(AppError::Validation(format!(
            "invalid tier '{}': must be one of demo, free, pro",
            body.tier
        )));
    }

    let result = sqlx::query!(
        r#"
        UPDATE users
        SET tier = $1
        WHERE id = $2
        "#,
        body.tier,
        user_id,
    )
    .execute(&state.pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::JobNotFound);
    }

    tracing::info!(user_id = %user_id, tier = %body.tier, "user tier updated");
    Ok(StatusCode::NO_CONTENT)
}
