use axum::{
    extract::{FromRequestParts, State},
    http::{request::Parts, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{config::ServerConfig, error::AppError, AppState};

// =============================================================================
// Interface identity extractor
// =============================================================================

/// Identifies which interface sent the request by matching the `Authorization:
/// Bearer <token>` header against `config.auth.interface_tokens`.
///
/// Used as an axum extractor on every protected route.
pub struct InterfaceToken(pub String);

impl FromRequestParts<AppState> for InterfaceToken {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = extract_bearer(parts.headers.clone())
            .ok_or_else(|| AppError::Unauthorized.into_response())?;

        let name = state
            .config
            .auth
            .identify_token(&token)
            .ok_or_else(|| AppError::Unauthorized.into_response())?;

        Ok(InterfaceToken(name.to_string()))
    }
}

fn extract_bearer(headers: HeaderMap) -> Option<String> {
    let value = headers.get(axum::http::header::AUTHORIZATION)?;
    let s = value.to_str().ok()?;
    s.strip_prefix("Bearer ").map(|t| t.to_string())
}

// =============================================================================
// Telegram identification endpoint
// =============================================================================

#[derive(Deserialize)]
pub struct TelegramIdentifyRequest {
    pub telegram_id:  String,
    pub display_name: Option<String>,
}

#[derive(Serialize)]
pub struct TelegramIdentifyResponse {
    pub user_id: Uuid,
    pub tier:    String,
}

/// `POST /auth/telegram`
///
/// Upserts a user identity for the given Telegram user ID and returns the
/// SISAR user UUID and account tier. The bot stores the UUID and attaches it
/// to subsequent job requests.
///
/// Requires a valid interface bearer token (the Telegram bot's pre-shared
/// secret).
pub async fn telegram_identify(
    _token: InterfaceToken,
    State(state): State<AppState>,
    Json(body): Json<TelegramIdentifyRequest>,
) -> Result<Json<TelegramIdentifyResponse>, AppError> {
    let pool = &state.pool;
    let display_name = body.display_name.as_deref();

    // Try to find an existing identity for this Telegram user.
    let existing = sqlx::query!(
        r#"
        SELECT u.id AS user_id, u.tier
        FROM identities i
        JOIN users u ON u.id = i.user_id
        WHERE i.provider = 'telegram' AND i.provider_id = $1
        "#,
        body.telegram_id,
    )
    .fetch_optional(pool)
    .await?;

    if let Some(row) = existing {
        // Update display name if it changed.
        if display_name.is_some() {
            sqlx::query!(
                r#"
                UPDATE identities
                SET display_name = $1
                WHERE provider = 'telegram' AND provider_id = $2
                "#,
                display_name,
                body.telegram_id,
            )
            .execute(pool)
            .await?;
        }

        return Ok(Json(TelegramIdentifyResponse {
            user_id: row.user_id,
            tier: row.tier,
        }));
    }

    // New user: create the users row, then the identity row.
    let user_id: Uuid = sqlx::query_scalar!(
        r#"
        INSERT INTO users (display_name)
        VALUES ($1)
        RETURNING id
        "#,
        display_name,
    )
    .fetch_one(pool)
    .await?;

    sqlx::query!(
        r#"
        INSERT INTO identities (user_id, provider, provider_id, display_name)
        VALUES ($1, 'telegram', $2, $3)
        "#,
        user_id,
        body.telegram_id,
        display_name,
    )
    .execute(pool)
    .await?;

    tracing::info!(
        user_id = %user_id,
        telegram_id = %body.telegram_id,
        "new user registered via Telegram"
    );

    Ok(Json(TelegramIdentifyResponse {
        user_id,
        tier: "free".to_string(),
    }))
}
