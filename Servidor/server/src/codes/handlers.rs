//! Endpoints públicos para consultar prepaid codes.

use axum::{
    extract::{Path, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::auth::InterfaceToken;
use crate::error::{AppError, ApiResult};
use crate::AppState;

// =============================================================================
// Response
// =============================================================================

/// Información pública de un prepaid code.
/// No expone datos sensibles como `issued_to`.
#[derive(Serialize)]
pub struct CodeInfoResponse {
    /// El UUID del código (lo mismo que el path param).
    pub code: Uuid,
    /// Tier de capacidad de los trabajos pagados con este código.
    /// Determina los límites de área, rango temporal y formatos disponibles.
    pub capacity_tier: String,
    /// Cantidad total de trabajos que otorga el código.
    pub total_jobs: i32,
    /// Cantidad de trabajos ya consumidos.
    pub used_jobs: i32,
    /// Trabajos restantes disponibles.
    pub remaining_jobs: i32,
    /// Fecha de expiración, si aplica.
    pub expires_at: Option<DateTime<Utc>>,
    /// Si el código está activo (no expirado y con usos restantes).
    pub is_valid: bool,
}

// =============================================================================
// Handler
// =============================================================================

/// `GET /codes/:code`
///
/// Consulta la información de un prepaid code: vigencia, cantidad de trabajos
/// totales, consumidos y restantes, y capacidad (tier).
///
/// No consume el código ni lo asocia a ningún usuario.
/// Cualquier interfaz autenticada puede consultarlo (el código es compartible).
pub async fn get_code_info(
    _token: InterfaceToken,
    Path(code): Path<Uuid>,
    State(state): State<AppState>,
) -> ApiResult<Json<CodeInfoResponse>> {
    let row = sqlx::query!(
        r#"
        SELECT granted_tier, total_jobs, used_jobs, expires_at
        FROM prepaid_codes
        WHERE code = $1
        "#,
        code,
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::CodeNotFound)?;

    let now = Utc::now();
    let expired = row.expires_at.map(|exp| exp < now).unwrap_or(false);
    let remaining = (row.total_jobs - row.used_jobs).max(0);
    let is_valid = !expired && remaining > 0;

    Ok(Json(CodeInfoResponse {
        code,
        capacity_tier: row.granted_tier,
        total_jobs: row.total_jobs,
        used_jobs: row.used_jobs,
        remaining_jobs: remaining,
        expires_at: row.expires_at,
        is_valid,
    }))
}
