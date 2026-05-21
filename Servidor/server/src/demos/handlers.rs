//! Demo result endpoints.
//!
//! Demos are pre-computed InSAR jobs stored under `/sisar/demos/<name>/`
//! with the same directory layout as a normal job's `work_dir`.  They are
//! accessible to every authenticated user regardless of tier, and are never
//! recorded in the `jobs` table.
//!
//! ## Endpoints
//!
//! ```text
//! GET  /demos                              → list valid demo names
//! POST /demos/:name/results               → enqueue a result request
//! GET  /demos/:name/results/:request_id   → poll state or download file
//! ```
//!
//! ## Velocity fast-path
//!
//! Identical to the regular jobs fast-path: if `results/velocity.png` already
//! exists inside the demo directory it is served immediately without enqueuing
//! anything.
//!
//! ## work_dir column
//!
//! When a result request is enqueued for a demo the absolute demo path is
//! stored in `result_requests.work_dir`.  The results scheduler uses that
//! column directly and skips the JOIN with the `jobs` table.

use std::path::PathBuf;

use axum::{
    Json,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use shared::models::{ResultState, ResultType};
use shared::queries::{enqueue_result_request, get_result_request};

use crate::auth::InterfaceToken;
use crate::error::{AppError, ApiResult};
use crate::AppState;

// =============================================================================
// Helpers
// =============================================================================

/// Returns the absolute path to a demo directory and verifies it exists.
///
/// A directory is considered a valid demo if it exists under `demos_root` and
/// contains at least one of `results/velocity.png` or `results/timeseries*.png`.
fn demo_path(demos_root: &PathBuf, name: &str) -> Result<PathBuf, AppError> {
    // Reject path traversal attempts.
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(AppError::Validation("invalid demo name".to_string()));
    }

    let path = demos_root.join(name);

    if !path.is_dir() {
        return Err(AppError::JobNotFound); // reuse 404 variant
    }

    Ok(path)
}

/// Returns `true` if `dir` looks like a complete demo (has at least velocity).
fn is_valid_demo(dir: &PathBuf) -> bool {
    dir.join("results").join("velocity.png").exists()
}

// =============================================================================
// GET /demos
// =============================================================================

#[derive(Serialize)]
pub struct DemoEntry {
    pub name: String,
}

#[derive(Serialize)]
pub struct ListDemosResponse {
    pub demos: Vec<DemoEntry>,
}

/// Lists all valid demos found under `config.paths.demos_root`.
///
/// A demo is valid when its `results/velocity.png` file exists, indicating
/// the job completed successfully.  New folders dropped into the directory
/// are picked up automatically on the next call — no restart required.
pub async fn list_demos(
    _token: InterfaceToken,
    State(state): State<AppState>,
) -> ApiResult<Json<ListDemosResponse>> {
    let root = &state.config.paths.demos_root;

    let mut demos: Vec<DemoEntry> = Vec::new();

    let mut entries = tokio::fs::read_dir(root)
    .await
    .map_err(|e| AppError::Internal(format!("reading demos directory: {e}")))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| AppError::Internal(format!("iterating demos directory: {e}")))?
        {
            let path = entry.path();
            if path.is_dir() && is_valid_demo(&path) {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    demos.push(DemoEntry {
                        name: name.to_string(),
                    });
                }
            }
        }

        // Sort alphabetically for stable ordering.
        demos.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(Json(ListDemosResponse { demos }))
}

// =============================================================================
// POST /demos/:name/results
// =============================================================================

#[derive(Deserialize)]
pub struct DemoResultRequestBody {
    /// One of: `"velocity"`, `"timeseries"`.
    pub result_type: String,
    /// Type-specific parameters.
    /// - `"timeseries"`: `{ "lat": <f64>, "lon": <f64> }` — required.
    /// - `"velocity"`: omit or pass `null`.
    pub params: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct EnqueueDemoResultResponse {
    pub result_request_id: Uuid,
    pub state: String,
}

/// Enqueues a result generation request for the given demo.
///
/// Velocity fast-path: if `results/velocity.png` already exists in the demo
/// directory it is returned immediately (200) without writing to
/// `result_requests`.
///
/// For any other type (or when velocity.png is absent) a `result_requests` row
/// is inserted with `work_dir` set to the demo's absolute path, and the ID is
/// returned (202).  The results scheduler picks it up and runs the Docker
/// container without touching the `jobs` table.
///
/// A sentinel UUID derived deterministically from the demo name is stored in
/// `result_requests.job_id` so the column's NOT NULL constraint is satisfied.
/// The scheduler skips the jobs-table JOIN whenever `work_dir IS NOT NULL`.
pub async fn enqueue_demo_result(
    _token: InterfaceToken,
    Path(name): Path<String>,
                                 State(state): State<AppState>,
                                 Json(body): Json<DemoResultRequestBody>,
) -> ApiResult<Response> {
    let root = &state.config.paths.demos_root;
    let demo_dir = demo_path(root, &name)?;

    // Resolve and validate result type (only velocity and timeseries for demos).
    let result_type = match body.result_type.as_str() {
        "velocity" => ResultType::Velocity,
        "timeseries" => ResultType::Timeseries,
        other => {
            return Err(AppError::Validation(format!(
                "unknown result_type '{other}' for demos; valid: velocity, timeseries"
            )));
        }
    };

    // Validate params for timeseries.
    if result_type == ResultType::Timeseries {
        let p = body.params.as_ref().ok_or_else(|| {
            AppError::Validation(
                "timeseries requires params: { \"lat\": <f64>, \"lon\": <f64> }".to_string(),
            )
        })?;
        let lat = p.get("lat").and_then(|v| v.as_f64());
        let lon = p.get("lon").and_then(|v| v.as_f64());
        match (lat, lon) {
            (Some(lat), Some(lon)) => {
                if !(-90.0..=90.0).contains(&lat) {
                    return Err(AppError::Validation(format!(
                        "lat must be in [-90, 90]; got {lat}"
                    )));
                }
                if !(-180.0..=180.0).contains(&lon) {
                    return Err(AppError::Validation(format!(
                        "lon must be in [-180, 180]; got {lon}"
                    )));
                }
            }
            _ => {
                return Err(AppError::Validation(
                    "timeseries params must include numeric 'lat' and 'lon' fields".to_string(),
                ));
            }
        }
    }

    // Velocity fast-path.
    if result_type == ResultType::Velocity {
        let velocity_path = demo_dir.join("results").join("velocity.png");
        if velocity_path.exists() {
            let bytes = tokio::fs::read(&velocity_path)
            .await
            .map_err(|e| AppError::Internal(format!("reading velocity.png: {e}")))?;
            return Ok((
                StatusCode::OK,
                [(header::CONTENT_TYPE, "image/png")],
                       bytes,
            )
            .into_response());
        }
    }

    // Sentinel job_id: UUID v5 derived from the demo name so it is stable and
    // unique per demo without requiring a row in the jobs table.

    let work_dir_str = demo_dir
    .to_str()
    .ok_or_else(|| AppError::Internal("demo path is not valid UTF-8".to_string()))?
    .to_string();

    // job_id = None: demos have no row in the jobs table.
    // work_dir is stored directly so the scheduler never touches jobs.
    let req = enqueue_result_request(
        &state.pool,
        None,
        result_type,
        body.params,
        Some(work_dir_str),
    )
    .await
    .map_err(|e| AppError::Internal(format!("enqueue_result_request: {e}")))?;

    Ok((
        StatusCode::ACCEPTED,
        Json(EnqueueDemoResultResponse {
            result_request_id: req.id,
            state: req.state.as_str().to_string(),
        }),
    )
    .into_response())
}

// =============================================================================
// GET /demos/:name/results/:request_id
// =============================================================================

#[derive(Serialize)]
pub struct DemoResultStatusResponse {
    pub result_request_id: Uuid,
    pub demo_name: String,
    pub result_type: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Returns the state of a demo result request, or streams the output file
/// when completed.
///
/// - `queued` / `running` / `failed` → JSON status (200)
/// - `completed` → file bytes with correct MIME (200)
pub async fn get_demo_result(
    _token: InterfaceToken,
    Path((name, request_id)): Path<(String, Uuid)>,
                             State(state): State<AppState>,
) -> ApiResult<Response> {
    let root = &state.config.paths.demos_root;
    let demo_dir = demo_path(root, &name)?;

    let req = get_result_request(&state.pool, request_id)
    .await
    .map_err(|e| AppError::Internal(format!("get_result_request: {e}")))?
    .ok_or_else(|| AppError::ResultNotFound("result request not found".to_string()))?;

    // Verify this request belongs to the named demo by comparing work_dir.
    let expected_work_dir = demo_dir
    .to_str()
    .ok_or_else(|| AppError::Internal("demo path is not valid UTF-8".to_string()))?;

    if req.work_dir.as_deref() != Some(expected_work_dir) {
        return Err(AppError::ResultNotFound(
            "result request does not belong to this demo".to_string(),
        ));
    }

    match req.state {
        ResultState::Completed => {
            let results_dir = demo_dir.join("results");

            let file_path = req.resolve_output_file(&results_dir).ok_or_else(|| {
                AppError::ResultNotFound(format!(
                    "output file not found for result type '{}' in {}",
                    req.result_type.as_str(),
                                                 results_dir.display()
                ))
            })?;

            let mime = req.result_type.mime().unwrap_or("application/octet-stream");

            let bytes = tokio::fs::read(&file_path)
            .await
            .map_err(|e| AppError::Internal(format!("reading result file: {e}")))?;

            Ok((StatusCode::OK, [(header::CONTENT_TYPE, mime)], bytes).into_response())
        }

        ResultState::Queued | ResultState::Running | ResultState::Failed => Ok((
            StatusCode::OK,
            Json(DemoResultStatusResponse {
                result_request_id: req.id,
                demo_name: name,
                result_type: req.result_type.as_str().to_string(),
                 state: req.state.as_str().to_string(),
                 error: req.error,
                 created_at: req.created_at,
                 updated_at: req.updated_at,
            }),
        )
        .into_response()),
    }
}
