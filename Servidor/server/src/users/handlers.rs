//! Endpoints del perfil del usuario autenticado.

use axum::{
    extract::State,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::InterfaceToken;
use crate::error::{AppError, ApiResult};
use crate::AppState;

// =============================================================================
// Request / Response
// =============================================================================

/// Query params de `GET /users/me`: la interfaz pasa el user_id del usuario
/// que está consultando (el bot ya lo tiene de /auth/telegram).
#[derive(Deserialize)]
pub struct MeQuery {
    pub user_id: Uuid,
}

#[derive(Serialize)]
pub struct UserMeResponse {
    pub user_id:      Uuid,
    pub display_name: Option<String>,
    pub tier:         String,
}

// =============================================================================
// Handler
// =============================================================================

/// `GET /users/me?user_id=<uuid>`
///
/// Devuelve el tier actual del usuario y su display_name.
/// La interfaz usa esto para saber qué operaciones puede ofrecer al usuario
/// antes de arrancar el flujo de /analizar.
pub async fn get_me(
    _token: InterfaceToken,
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<MeQuery>,
) -> ApiResult<Json<UserMeResponse>> {
    let row = sqlx::query!(
        r#"
        SELECT id, display_name, tier
        FROM users
        WHERE id = $1
        "#,
        params.user_id,
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::JobNotFound)?; // user_id viene de auth, no debería fallar

    Ok(Json(UserMeResponse {
        user_id:      row.id,
        display_name: row.display_name,
        tier:         row.tier,
    }))
}
