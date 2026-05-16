//! HTTP handlers for job CRUD and status endpoints.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use shared::models::{Job, JobState};
use shared::queries::{get_job, list_user_jobs};
use shared::types::JobRequest;

use crate::auth::InterfaceToken;
use crate::error::{AppError, ApiResult};
use crate::jobs::submit::{submit_job, SubmitResponse};
use crate::AppState;

// =============================================================================
// Request / response types
// =============================================================================

/// The body of `POST /jobs`.  Wraps the core `JobRequest` with the caller-
/// supplied user UUID (set by the interface using the UUID returned from
/// `/auth/telegram`).
#[derive(Deserialize)]
pub struct CreateJobRequest {
    pub user_id: Uuid,
    #[serde(flatten)]
    pub job:     JobRequest,
}

/// Public job representation returned by the API.
#[derive(Serialize)]
pub struct JobResponse {
    pub id:            Uuid,
    pub user_id:       Uuid,
    pub workflow:      String,
    pub state:         String,
    pub stage:         Option<String>,
    pub error:         Option<String>,
    pub effective_tier: String,
    pub work_dir:      String,
    pub created_at:    DateTime<Utc>,
    pub updated_at:    DateTime<Utc>,
}

impl From<Job> for JobResponse {
    fn from(j: Job) -> Self {
        let (state_str, stage_str, error_str) = match &j.state {
            JobState::IsceProcessing { stage } => (
                "isce_processing".to_string(),
                Some(stage.as_str().to_string()),
                None,
            ),
            JobState::MintpyProcessing { stage } => (
                "mintpy_processing".to_string(),
                Some(stage.as_str().to_string()),
                None,
            ),
            JobState::MiaplpyProcessing { stage } => (
                "miaplpy_processing".to_string(),
                Some(stage.as_str().to_string()),
                None,
            ),
            JobState::Failed { error } => (
                "failed".to_string(),
                None,
                Some(error.to_string()),
            ),
            other => (other.as_str().to_string(), None, None),
        };

        JobResponse {
            id:            j.id,
            user_id:       j.user_id,
            workflow:      j.workflow,
            state:         state_str,
            stage:         stage_str,
            error:         error_str,
            effective_tier: j.effective_tier.as_str().to_string(),
            work_dir:      j.work_dir,
            created_at:    j.created_at,
            updated_at:    j.updated_at,
        }
    }
}

// =============================================================================
// Handlers
// =============================================================================

/// `POST /jobs`
///
/// Validates the request, queries ASF, writes job files, and inserts the
/// job into the database in `queued` state.
pub async fn create_job(
    _token: InterfaceToken,
    State(state): State<AppState>,
    Json(body): Json<CreateJobRequest>,
) -> ApiResult<(StatusCode, Json<SubmitResponse>)> {
    let result = submit_job(body.job, body.user_id, &state.pool, &state.config).await?;
    Ok((StatusCode::CREATED, Json(result)))
}

/// `GET /jobs/:id`
///
/// Returns the current state of a single job.
pub async fn get_job_handler(
    _token: InterfaceToken,
    Path(job_id): Path<Uuid>,
    State(state): State<AppState>,
) -> ApiResult<Json<JobResponse>> {
    let job = get_job(&state.pool, job_id)
        .await?
        .ok_or(AppError::JobNotFound)?;

    Ok(Json(JobResponse::from(job)))
}

/// `GET /jobs?user_id=<uuid>`
///
/// Lists all jobs for the given user, newest first.
pub async fn list_jobs(
    _token: InterfaceToken,
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<ListJobsQuery>,
) -> ApiResult<Json<Vec<JobResponse>>> {
    let jobs = list_user_jobs(&state.pool, params.user_id).await?;
    Ok(Json(jobs.into_iter().map(JobResponse::from).collect()))
}

#[derive(Deserialize)]
pub struct ListJobsQuery {
    pub user_id: Uuid,
}

/// `DELETE /jobs/:id`
///
/// Cancels a job by setting its state to `cancelled`.  Only allowed while the
/// job is not yet in a terminal state.
pub async fn cancel_job(
    _token: InterfaceToken,
    Path(job_id): Path<Uuid>,
    State(state): State<AppState>,
) -> ApiResult<StatusCode> {
    let job = get_job(&state.pool, job_id)
        .await?
        .ok_or(AppError::JobNotFound)?;

    if job.state.is_terminal() {
        return Err(AppError::Validation(
            "job is already in a terminal state and cannot be cancelled".to_string(),
        ));
    }

    sqlx::query!(
        r#"
        UPDATE jobs
        SET job_state = 'cancelled', updated_at = NOW()
        WHERE id = $1
        "#,
        job_id,
    )
    .execute(&state.pool)
    .await?;

    tracing::info!(job_id = %job_id, "job cancelled");

    Ok(StatusCode::NO_CONTENT)
}
