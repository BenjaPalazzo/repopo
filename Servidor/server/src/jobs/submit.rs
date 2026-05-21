//! Job submission pipeline.
//!
//! Validates the request, queries ASF for burst data, writes job files to
//! disk, and inserts the job row into the database.

use std::path::PathBuf;

use chrono::Datelike;
use sqlx::PgPool;
use uuid::Uuid;

use shared::models::UserTier;
use shared::types::{BoundingBox, JobRequest, ProcessingWorkflow, Sensor, TimeRange};

use crate::asf;
use crate::config::ServerConfig;
use crate::error::AppError;
use crate::specification::{SpecificationParams, write_job_files};

// =============================================================================
// Submitted job response
// =============================================================================

#[derive(serde::Serialize)]
pub struct SubmitResponse {
    pub job_id: Uuid,
    pub work_dir: String,
    pub path: u32,
}

// =============================================================================
// Entry point
// =============================================================================

/// Runs the full submission pipeline and returns the new job ID on success.
pub async fn submit_job(
    req: JobRequest,
    user_id: Uuid,
    pool: &PgPool,
    config: &ServerConfig,
) -> Result<SubmitResponse, AppError> {
    // -----------------------------------------------------------------
    // 1. Validate spatial and temporal bounds.
    // -----------------------------------------------------------------
    let bbox = BoundingBox::from_bounds(req.north, req.south, req.east, req.west)?;
    let time = TimeRange::from_bounds(req.start, req.end)?;

    // -----------------------------------------------------------------
    // 2. Resolve user tier and apply defaults.
    // -----------------------------------------------------------------
    let user_tier = fetch_user_tier(pool, user_id).await?;
    let effective_tier = resolve_effective_tier(pool, user_id, &req, &user_tier, config).await?;
    let tier_limits = config
        .user_tiers
        .for_tier(effective_tier.as_str())
        .ok_or_else(|| AppError::Internal("unknown effective tier".to_string()))?;

    // -----------------------------------------------------------------
    // 3. Enforce tier limits.
    // -----------------------------------------------------------------
    let aoi_km2 = bbox.area_km2() as u64;
    if !crate::config::TierLimitsConfig::within_limit(tier_limits.max_aoi_km2, aoi_km2) {
        return Err(AppError::TierLimitExceeded(format!(
            "AOI area {aoi_km2} km² exceeds the {max} km² limit for your tier",
            max = tier_limits.max_aoi_km2
        )));
    }

    let duration_days = time.duration_days() as u64;
    if !crate::config::TierLimitsConfig::within_limit(
        tier_limits.max_time_range_days,
        duration_days,
    ) {
        return Err(AppError::TierLimitExceeded(format!(
            "time range {duration_days} days exceeds the {max}-day limit for your tier",
            max = tier_limits.max_time_range_days
        )));
    }

    // Monthly job count check.
    let jobs_this_month = count_jobs_this_month(pool, user_id).await?;
    if !crate::config::TierLimitsConfig::within_limit(
        tier_limits.max_jobs_per_month,
        jobs_this_month,
    ) {
        return Err(AppError::TierLimitExceeded(format!(
            "monthly job limit of {} reached for your tier",
            tier_limits.max_jobs_per_month
        )));
    }

    // -----------------------------------------------------------------
    // 4. Resolve processing parameters (apply config defaults).
    // -----------------------------------------------------------------
    let workflow = req.workflow.unwrap_or_default();
    let range_looks = req.range_looks.unwrap_or(config.defaults.range_looks);
    let azimuth_looks = req.azimuth_looks.unwrap_or(config.defaults.azimuth_looks);
    let connections = req.connections.unwrap_or(config.defaults.connections);

    // -----------------------------------------------------------------
    // 5. Query ASF and select optimal path.
    // -----------------------------------------------------------------
    let asf_result = asf::search(&req, &config.asf.endpoint)
        .await
        .map_err(|e| match e {
            shared::types::FlightPathError::InsufficientMaterial => AppError::NoImagesFound,
            other => AppError::FlightPath(other),
        })?;

    // -----------------------------------------------------------------
    // 6. Create job working directory and write specification files.
    //    The job is in 'initializing' state while we do this.
    // -----------------------------------------------------------------
    let job_id = Uuid::new_v4();
    let work_dir = config.paths.jobs_root.join(job_id.to_string());

    std::fs::create_dir_all(&work_dir)?;

    let spec_params = SpecificationParams {
        dem_bbox: asf_result.dem_bbox,
        aoi_north: req.north,
        aoi_south: req.south,
        aoi_east: req.east,
        aoi_west: req.west,
        range_looks,
        azimuth_looks,
        connections,
    };

    write_job_files(
        &work_dir,
        &spec_params,
        &asf_result.burst_list,
        &asf_result.burst_stitch,
    )?;

    // -----------------------------------------------------------------
    // 7. Insert the job row in 'queued' state.
    //    Prepaid code is redeemed atomically in the same transaction.
    // -----------------------------------------------------------------
    let work_dir_str = work_dir.to_string_lossy().into_owned();
    let workflow_str = workflow.as_db_str();
    let tier_str = effective_tier.as_str();

    insert_job(
        pool,
        job_id,
        user_id,
        workflow_str,
        tier_str,
        &work_dir_str,
        req.prepaid_code,
    )
    .await?;

    tracing::info!(
        job_id = %job_id,
        user_id = %user_id,
        workflow = workflow_str,
        path = asf_result.path,
        "job submitted"
    );

    Ok(SubmitResponse {
        job_id,
        work_dir: work_dir_str,
        path: asf_result.path,
    })
}

// =============================================================================
// Helpers
// =============================================================================

async fn fetch_user_tier(pool: &PgPool, user_id: Uuid) -> Result<UserTier, AppError> {
    let row = sqlx::query!("SELECT tier FROM users WHERE id = $1", user_id,)
        .fetch_optional(pool)
        .await?
        .ok_or(AppError::JobNotFound)?; // user_id came from auth, so this shouldn't happen

    UserTier::from_str(&row.tier)
        .ok_or_else(|| AppError::Internal(format!("unknown tier in DB: {}", row.tier)))
}

/// Resolves the effective tier, accounting for a prepaid code if provided.
/// Validates the code (existence, expiry, remaining uses) but does NOT
/// consume it yet — consumption is atomic with the DB insert.
async fn resolve_effective_tier(
    pool: &PgPool,
    _user_id: Uuid,
    req: &JobRequest,
    account_tier: &UserTier,
    _config: &ServerConfig,
) -> Result<UserTier, AppError> {
    let Some(code) = req.prepaid_code else {
        return Ok(*account_tier);
    };

    let row = sqlx::query!(
        r#"
        SELECT granted_tier, expires_at, total_jobs, used_jobs
        FROM prepaid_codes
        WHERE code = $1
        "#,
        code,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::InvalidPrepaidCode)?;

    // Expirado
    if let Some(exp) = row.expires_at {
        if exp < chrono::Utc::now() {
            return Err(AppError::InvalidPrepaidCode);
        }
    }

    // Sin usos restantes
    if row.used_jobs >= row.total_jobs {
        return Err(AppError::CodeExhausted);
    }

    UserTier::from_str(&row.granted_tier).ok_or_else(|| {
        AppError::Internal(format!(
            "unknown granted_tier in prepaid_codes: {}",
            row.granted_tier
        ))
    })
}

async fn count_jobs_this_month(pool: &PgPool, user_id: Uuid) -> Result<u64, AppError> {
    let now = chrono::Utc::now();
    let month_start = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
        chrono::NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap(),
        chrono::Utc,
    );

    let row = sqlx::query!(
        r#"SELECT COUNT(*) AS "count!" FROM jobs WHERE user_id = $1 AND created_at >= $2"#,
        user_id,
        month_start,
    )
    .fetch_one(pool)
    .await?;

    Ok(row.count as u64)
}

async fn insert_job(
    pool: &PgPool,
    job_id: Uuid,
    user_id: Uuid,
    workflow: &str,
    effective_tier: &str,
    work_dir: &str,
    prepaid_code: Option<Uuid>,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;

    sqlx::query!(
        r#"
        INSERT INTO jobs (id, user_id, workflow, job_state, effective_tier, work_dir)
        VALUES ($1, $2, $3, 'queued', $4, $5)
        "#,
        job_id,
        user_id,
        workflow,
        effective_tier,
        work_dir,
    )
    .execute(&mut *tx)
    .await?;

    if let Some(code) = prepaid_code {
        // Consume one job credit atomically, re-checking availability to
        // prevent race conditions between concurrent submissions.
        let updated = sqlx::query!(
            r#"
            UPDATE prepaid_codes
            SET used_jobs = used_jobs + 1
            WHERE code = $1
              AND used_jobs < total_jobs
              AND (expires_at IS NULL OR expires_at > NOW())
            "#,
            code,
        )
        .execute(&mut *tx)
        .await?;

        if updated.rows_affected() == 0 {
            // Race condition: code was exhausted or expired between validation
            // and insert. Roll back and surface a clear error.
            tx.rollback().await?;
            return Err(AppError::CodeExhausted);
        }

        // Audit log: one row per code use.
        sqlx::query!(
            r#"
            INSERT INTO code_uses (code, used_by, job_id)
            VALUES ($1, $2, $3)
            "#,
            code,
            user_id,
            job_id,
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}
