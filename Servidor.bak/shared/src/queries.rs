use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{Job, JobError, JobRow, JobRowError, JobState};

// =============================================================================
// Errors
// =============================================================================

#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("failed to decode job row {job_id}: {source}")]
    RowDecode {
        job_id: Uuid,
        #[source]
        source: JobRowError,
    },
}

// =============================================================================
// Scheduler queries
// =============================================================================

/// Atomically claims up to `limit` jobs ready to be advanced, ordered by
/// effective tier priority (Pro first) then age (oldest first).
///
/// Uses `FOR UPDATE SKIP LOCKED` so that concurrent scheduler instances on a
/// cluster never double-claim the same job.
pub async fn claim_ready_jobs(pool: &PgPool, limit: i64) -> Result<Vec<Job>, QueryError> {
    let rows = sqlx::query_as!(
        JobRow,
        r#"
        SELECT
            id,
            user_id,
            workflow,
            job_state,
            job_stage,
            job_error_kind,
            job_error_message,
            effective_tier,
            work_dir,
            created_at,
            updated_at
        FROM jobs
        WHERE job_state = ANY($1)
        ORDER BY
            CASE effective_tier
                WHEN 'pro'  THEN 2
                WHEN 'free' THEN 1
                ELSE             0
            END DESC,
            created_at ASC
        LIMIT $2
        FOR UPDATE SKIP LOCKED
        "#,
        &advanceable_states(),
        limit,
    )
    .fetch_all(pool)
    .await?;

    decode_rows(rows)
}

/// Returns the number of jobs owned by `user_id` that are currently in an
/// active (non-terminal, non-initializing) state.
///
/// Used by the scheduler to enforce per-user concurrent job limits before
/// dispatching a new step.
pub async fn count_user_active_jobs(pool: &PgPool, user_id: Uuid) -> Result<i64, QueryError> {
    let row = sqlx::query!(
        r#"
        SELECT COUNT(*) AS "count!"
        FROM jobs
        WHERE user_id  = $1
          AND job_state = ANY($2)
        "#,
        user_id,
        &advanceable_states(),
    )
    .fetch_one(pool)
    .await?;

    Ok(row.count)
}

/// Advances a job to the next state after a step completes successfully.
/// Clears error columns and refreshes `updated_at`.
pub async fn advance_job_state(
    pool: &PgPool,
    job_id: Uuid,
    next_state: &JobState,
) -> Result<(), QueryError> {
    sqlx::query!(
        r#"
        UPDATE jobs
        SET
            job_state         = $2,
            job_stage         = $3,
            job_error_kind    = NULL,
            job_error_message = NULL,
            updated_at        = NOW()
        WHERE id = $1
        "#,
        job_id,
        next_state.as_str(),
        next_state.as_stage_str(),
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Transitions a job to `Failed`, persisting the typed error.
pub async fn fail_job(pool: &PgPool, job_id: Uuid, error: &JobError) -> Result<(), QueryError> {
    sqlx::query!(
        r#"
        UPDATE jobs
        SET
            job_state         = 'failed',
            job_stage         = NULL,
            job_error_kind    = $2,
            job_error_message = $3,
            updated_at        = NOW()
        WHERE id = $1
        "#,
        job_id,
        error.kind(),
        error.to_db_string(),
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Transitions a job to `Completed`. Terminal — not claimed again.
pub async fn complete_job(pool: &PgPool, job_id: Uuid) -> Result<(), QueryError> {
    sqlx::query!(
        r#"
        UPDATE jobs
        SET
            job_state         = 'completed',
            job_stage         = NULL,
            job_error_kind    = NULL,
            job_error_message = NULL,
            updated_at        = NOW()
        WHERE id = $1
        "#,
        job_id,
    )
    .execute(pool)
    .await?;

    Ok(())
}

// =============================================================================
// Shared reads (API server + scheduler)
// =============================================================================

/// Fetches a single job by ID. Returns `None` if not found.
pub async fn get_job(pool: &PgPool, job_id: Uuid) -> Result<Option<Job>, QueryError> {
    let row = sqlx::query_as!(
        JobRow,
        r#"
        SELECT
            id,
            user_id,
            workflow,
            job_state,
            job_stage,
            job_error_kind,
            job_error_message,
            effective_tier,
            work_dir,
            created_at,
            updated_at
        FROM jobs
        WHERE id = $1
        "#,
        job_id,
    )
    .fetch_optional(pool)
    .await?;

    row.map(|r| {
        let id = r.id;
        Job::try_from(r).map_err(|source| QueryError::RowDecode { job_id: id, source })
    })
    .transpose()
}

/// Returns all jobs belonging to a user, ordered newest first.
/// Intended for the API server's job listing endpoint.
pub async fn list_user_jobs(pool: &PgPool, user_id: Uuid) -> Result<Vec<Job>, QueryError> {
    let rows = sqlx::query_as!(
        JobRow,
        r#"
        SELECT
            id,
            user_id,
            workflow,
            job_state,
            job_stage,
            job_error_kind,
            job_error_message,
            effective_tier,
            work_dir,
            created_at,
            updated_at
        FROM jobs
        WHERE user_id = $1
        ORDER BY created_at DESC
        "#,
        user_id,
    )
    .fetch_all(pool)
    .await?;

    decode_rows(rows)
}

// =============================================================================
// Helpers
// =============================================================================

fn advanceable_states() -> Vec<String> {
    vec![
        "queued".to_string(),
        "downloading".to_string(),
        "isce_processing".to_string(),
        "mintpy_processing".to_string(),
        "miaplpy_processing".to_string(),
        "results_generating".to_string(),
    ]
}

fn decode_rows(rows: Vec<JobRow>) -> Result<Vec<Job>, QueryError> {
    rows.into_iter()
        .map(|row| {
            let id = row.id;
            Job::try_from(row).map_err(|source| QueryError::RowDecode { job_id: id, source })
        })
        .collect()
}
