//! Result retrieval for completed jobs.
//!
//! - Velocity map: served directly from `{work_dir}/results/velocity.png`.
//! - Timeseries:   a Docker container is run with the job's work_dir mounted,
//!   given lat/lon arguments; it writes output to `{work_dir}/results/`.

use std::path::PathBuf;

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use bollard::{
    container::{
        Config, CreateContainerOptions, LogsOptions, RemoveContainerOptions, StartContainerOptions,
        WaitContainerOptions,
    },
    models::HostConfig,
};
use futures::StreamExt;
use serde::Deserialize;
use tokio::fs;
use uuid::Uuid;

use shared::models::JobState;
use shared::queries::get_job;

use crate::error::{AppError, ApiResult};
use crate::AppState;

// =============================================================================
// Velocity map
// =============================================================================

/// `GET /jobs/:id/velocity`
///
/// Streams `{work_dir}/results/velocity.png` for a completed job.
pub async fn get_velocity(
    Path(job_id): Path<Uuid>,
    State(state): State<AppState>,
) -> ApiResult<Response> {
    let job = get_job(&state.pool, job_id)
        .await?
        .ok_or(AppError::JobNotFound)?;

    if !matches!(job.state, JobState::Completed) {
        return Err(AppError::ResultNotFound(
            "job is not yet completed".to_string(),
        ));
    }

    let path = PathBuf::from(&job.work_dir)
        .join("results")
        .join("velocity.png");

    serve_file(&path, "image/png").await
}

// =============================================================================
// Timeseries
// =============================================================================

#[derive(Deserialize)]
pub struct TimeseriesQuery {
    pub lat: f64,
    pub lon: f64,
    /// Output format: "png" (default) or "csv".
    #[serde(default = "TimeseriesQuery::default_format")]
    pub format: String,
}

impl TimeseriesQuery {
    fn default_format() -> String {
        "png".to_string()
    }
}

/// `GET /jobs/:id/timeseries?lat=<lat>&lon=<lon>&format=<png|csv>`
///
/// Runs the results container, which reads the MintPy HDF5 output and writes
/// a PNG or CSV to `{work_dir}/results/timeseries.<ext>`.  The file is then
/// streamed back.
pub async fn get_timeseries(
    Path(job_id): Path<Uuid>,
    Query(query): Query<TimeseriesQuery>,
    State(state): State<AppState>,
) -> ApiResult<Response> {
    let job = get_job(&state.pool, job_id)
        .await?
        .ok_or(AppError::JobNotFound)?;

    if !matches!(job.state, JobState::Completed) {
        return Err(AppError::ResultNotFound(
            "job is not yet completed".to_string(),
        ));
    }

    let format = match query.format.as_str() {
        "png" | "csv" => query.format.clone(),
        other => {
            return Err(AppError::Validation(format!(
                "unsupported format '{other}'; use 'png' or 'csv'"
            )));
        }
    };

    let output_filename = format!("timeseries.{format}");
    let output_path = PathBuf::from(&job.work_dir)
        .join("results")
        .join(&output_filename);

    // Run the results container synchronously.
    run_results_container(
        &state.docker,
        job_id,
        &job.work_dir,
        &state.config.containers.results,
        query.lat,
        query.lon,
        &format,
    )
    .await?;

    let mime = match format.as_str() {
        "png" => "image/png",
        "csv" => "text/csv",
        _     => "application/octet-stream",
    };

    serve_file(&output_path, mime).await
}

// =============================================================================
// Docker helper
// =============================================================================

async fn run_results_container(
    docker: &bollard::Docker,
    job_id: Uuid,
    work_dir: &str,
    image: &str,
    lat: f64,
    lon: f64,
    format: &str,
) -> Result<(), AppError> {
    let container_name = format!("sisar-{job_id}-results");

    let config = Config {
        image: Some(image.to_string()),
        cmd: Some(vec![
            lat.to_string(),
            lon.to_string(),
            format.to_string(),
        ]),
        host_config: Some(HostConfig {
            binds: Some(vec![format!("{work_dir}:/job:rw")]),
            network_mode: Some("none".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };

    // Create and start container.
    docker
        .create_container(
            Some(CreateContainerOptions {
                name: container_name.clone(),
                platform: None,
            }),
            config,
        )
        .await
        .map_err(|e| AppError::Internal(format!("create results container: {e}")))?;

    docker
        .start_container(&container_name, None::<StartContainerOptions<String>>)
        .await
        .map_err(|e| AppError::Internal(format!("start results container: {e}")))?;

    // Wait for container to finish.
    let mut wait_stream = docker.wait_container(
        &container_name,
        Some(WaitContainerOptions { condition: "not-running" }),
    );

    let exit_code = if let Some(result) = wait_stream.next().await {
        result
            .map_err(|e| AppError::Internal(format!("wait results container: {e}")))?
            .status_code
    } else {
        -1
    };

    // Always remove the container.
    let _ = docker
        .remove_container(
            &container_name,
            Some(RemoveContainerOptions { force: true, ..Default::default() }),
        )
        .await;

    if exit_code != 0 {
        return Err(AppError::Internal(format!(
            "results container exited with code {exit_code}"
        )));
    }

    Ok(())
}

// =============================================================================
// File streaming helper
// =============================================================================

async fn serve_file(path: &PathBuf, mime: &'static str) -> ApiResult<Response> {
    if !path.exists() {
        return Err(AppError::ResultNotFound(format!(
            "result file not found at {}",
            path.display()
        )));
    }

    let bytes = fs::read(path)
        .await
        .map_err(|e| AppError::Internal(format!("reading result file: {e}")))?;

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, mime)],
        bytes,
    )
        .into_response())
}
