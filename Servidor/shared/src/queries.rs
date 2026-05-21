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

// =============================================================================
// Result request queries
// =============================================================================

use crate::models::{
    ResultRequest, ResultRequestRow, ResultRequestRowError, ResultState, ResultType,
};

#[derive(Debug, thiserror::Error)]
pub enum ResultQueryError {
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("failed to decode result_request row {id}: {source}")]
    RowDecode {
        id: Uuid,
        #[source]
        source: ResultRequestRowError,
    },
}

/// Inserts a new result request in `queued` state.
/// Returns the full created record.
///
/// `work_dir` is `None` for regular job result requests (scheduler resolves it
/// via the jobs table) and `Some(path)` for demo result requests (scheduler
/// uses it directly without touching the jobs table).
pub async fn enqueue_result_request(
    pool: &PgPool,
    job_id: Option<Uuid>,
    result_type: ResultType,
    params: Option<serde_json::Value>,
    work_dir: Option<String>,
) -> Result<ResultRequest, ResultQueryError> {
    let row = sqlx::query_as!(
        ResultRequestRow,
        r#"
        INSERT INTO result_requests (job_id, result_type, params, state, work_dir)
        VALUES ($1, $2, $3, 'queued', $4)
        RETURNING
            id,
            job_id,
            result_type,
            params       AS "params: serde_json::Value",
            state,
            error,
            work_dir,
            created_at,
            updated_at
        "#,
        job_id,
        result_type.as_str(),
        params,
        work_dir,
    )
    .fetch_one(pool)
    .await?;

    let id = row.id;
    ResultRequest::try_from(row).map_err(|source| ResultQueryError::RowDecode { id, source })
}

/// Atomically claims up to `limit` queued result requests, oldest first.
/// Uses `FOR UPDATE SKIP LOCKED` so concurrent scheduler instances never
/// double-claim the same request.
pub async fn claim_ready_result_requests(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<ResultRequest>, ResultQueryError> {
    let rows = sqlx::query_as!(
        ResultRequestRow,
        r#"
        SELECT
            id,
            job_id,
            result_type,
            params       AS "params: serde_json::Value",
            state,
            error,
            work_dir,
            created_at,
            updated_at
        FROM result_requests
        WHERE state = 'queued'
        ORDER BY created_at ASC
        LIMIT $1
        FOR UPDATE SKIP LOCKED
        "#,
        limit,
    )
    .fetch_all(pool)
    .await?;

    decode_result_rows(rows)
}

/// Transitions a result request to `running`.
pub async fn start_result_request(
    pool: &PgPool,
    id: Uuid,
) -> Result<(), ResultQueryError> {
    sqlx::query!(
        r#"
        UPDATE result_requests
        SET state = 'running', updated_at = NOW()
        WHERE id = $1
        "#,
        id,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Transitions a result request to `completed`.
pub async fn complete_result_request(
    pool: &PgPool,
    id: Uuid,
) -> Result<(), ResultQueryError> {
    sqlx::query!(
        r#"
        UPDATE result_requests
        SET state = 'completed', error = NULL, updated_at = NOW()
        WHERE id = $1
        "#,
        id,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Transitions a result request to `failed`, persisting the error message.
pub async fn fail_result_request(
    pool: &PgPool,
    id: Uuid,
    error: &str,
) -> Result<(), ResultQueryError> {
    sqlx::query!(
        r#"
        UPDATE result_requests
        SET state = 'failed', error = $2, updated_at = NOW()
        WHERE id = $1
        "#,
        id,
        error,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetches a single result request by ID. Returns `None` if not found.
pub async fn get_result_request(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<ResultRequest>, ResultQueryError> {
    let row = sqlx::query_as!(
        ResultRequestRow,
        r#"
        SELECT
            id,
            job_id,
            result_type,
            params       AS "params: serde_json::Value",
            state,
            error,
            work_dir,
            created_at,
            updated_at
        FROM result_requests
        WHERE id = $1
        "#,
        id,
    )
    .fetch_optional(pool)
    .await?;

    row.map(|r| {
        let id = r.id;
        ResultRequest::try_from(r).map_err(|source| ResultQueryError::RowDecode { id, source })
    })
    .transpose()
}

/// Returns the most recent result request for a given job and type,
/// or `None` if none exists.
pub async fn get_latest_result_request_for_job(
    pool: &PgPool,
    job_id: Uuid,
    result_type: ResultType,
) -> Result<Option<ResultRequest>, ResultQueryError> {
    let row = sqlx::query_as!(
        ResultRequestRow,
        r#"
        SELECT
            id,
            job_id,
            result_type,
            params       AS "params: serde_json::Value",
            state,
            error,
            work_dir,
            created_at,
            updated_at
        FROM result_requests
        WHERE job_id = $1 AND result_type = $2
        ORDER BY created_at DESC
        LIMIT 1
        "#,
        job_id,
        result_type.as_str(),
    )
    .fetch_optional(pool)
    .await?;

    row.map(|r| {
        let id = r.id;
        ResultRequest::try_from(r).map_err(|source| ResultQueryError::RowDecode { id, source })
    })
    .transpose()
}

// =============================================================================
// Helpers
// =============================================================================

fn decode_result_rows(
    rows: Vec<ResultRequestRow>,
) -> Result<Vec<ResultRequest>, ResultQueryError> {
    rows.into_iter()
        .map(|row| {
            let id = row.id;
            ResultRequest::try_from(row)
                .map_err(|source| ResultQueryError::RowDecode { id, source })
        })
        .collect()
}