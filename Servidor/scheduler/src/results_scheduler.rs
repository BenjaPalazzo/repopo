//! Scheduler loop for `result_requests`.
//!
//! Runs as a parallel `tokio` task alongside the main job scheduler.
//! Every `poll_interval_secs` it claims queued result requests, verifies the
//! parent job is `completed`, builds the Docker container spec, and launches
//! it via the existing `Runner`.
//!
//! Resource management for result containers is intentionally unconstrained:
//! they are lightweight (HDF5 read + plot) and are not tracked in the main
//! `ResourcePool`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use uuid::Uuid;

use shared::models::{JobState, ResultRequest, ResultState};
use shared::queries::{
    claim_ready_result_requests, complete_result_request, fail_result_request, get_job,
    start_result_request,
};

use crate::config::system::SystemConfig;
use crate::runner::{Runner, StepOutcome};
use crate::workflow::definition::{
    ContainerImages, ContainerSpec, ResourceRequirement, WorkflowStep, result_container_spec,
};

// =============================================================================
// ResultsScheduler
// =============================================================================

pub struct ResultsScheduler {
    pool:       PgPool,
    runner:     Runner,
    system_cfg: SystemConfig,
    images:     ContainerImages,

    /// Tracks result_request IDs currently being processed (in-flight).
    active: Arc<Mutex<HashMap<Uuid, ()>>>,
}

impl ResultsScheduler {
    pub fn new(pool: PgPool, runner: Runner, system_cfg: SystemConfig) -> Self {
        let images = ContainerImages {
            download: system_cfg.containers.download.clone(),
            isce2:    system_cfg.containers.isce2.clone(),
            mintpy:   system_cfg.containers.mintpy.clone(),
            miaplpy:  system_cfg.containers.miaplpy.clone(),
            results:  system_cfg.containers.results.clone(),
        };

        Self {
            pool,
            runner,
            system_cfg,
            images,
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Runs the results scheduler poll loop indefinitely.
    /// Should be spawned with `tokio::spawn` alongside the main scheduler.
    pub async fn run(self: Arc<Self>) {
        let interval =
            Duration::from_secs(self.system_cfg.scheduler.poll_interval_secs);

        tracing::info!(
            poll_interval_secs = interval.as_secs(),
            "results scheduler started"
        );

        loop {
            self.tick().await;
            tokio::time::sleep(interval).await;
        }
    }

    // -------------------------------------------------------------------------
    // Poll tick
    // -------------------------------------------------------------------------

    async fn tick(&self) {
        // Use the same max_concurrent_containers limit to cap result jobs too,
        // minus however many are already in-flight.
        let max = self.system_cfg.scheduler.max_concurrent_containers as i64;
        let running = self.active.lock().await.len() as i64;
        let claim_limit = (max - running).max(0);

        if claim_limit == 0 {
            tracing::debug!("all result container slots occupied — skipping poll");
            return;
        }

        let requests = match claim_ready_result_requests(&self.pool, claim_limit).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "failed to claim result requests");
                return;
            }
        };

        if requests.is_empty() {
            tracing::debug!("no queued result requests found");
            return;
        }

        tracing::debug!(count = requests.len(), "claimed result requests for evaluation");

        for req in requests {
            self.try_dispatch(req).await;
        }
    }

    // -------------------------------------------------------------------------
    // Dispatch
    // -------------------------------------------------------------------------

    async fn try_dispatch(&self, req: ResultRequest) {
        let req_id = req.id;
        let job_id = req.job_id;

        // Skip if already in flight.
        {
            let active = self.active.lock().await;
            if active.contains_key(&req_id) {
                tracing::debug!(%req_id, "result request already in flight — skipping");
                return;
            }
        }

        // Resolve work_dir.
        //
        // For demo result requests `work_dir` is populated by the API server
        // (e.g. `/sisar/demos/<name>`).  For regular job result requests it is
        // NULL and we resolve it by fetching the parent job from the DB.
        let work_dir: String = if let Some(dir) = req.work_dir.clone() {
            // Demo path: no DB lookup needed, no completed-state check.
            tracing::debug!(%req_id, work_dir = %dir, "using demo work_dir from result_request");
            dir
        } else {
            // Regular job path: verify parent job exists and is completed.
            let job_id = match req.job_id {
                Some(id) => id,
                None => {
                    tracing::warn!(%req_id, "result request has no job_id and no work_dir — failing");
                    let _ = fail_result_request(&self.pool, req_id, "no job_id and no work_dir").await;
                    return;
                }
            };
            let job = match get_job(&self.pool, job_id).await {
                Ok(Some(j)) => j,
                Ok(None) => {
                    tracing::warn!(%req_id, %job_id, "parent job not found — failing result request");
                    let _ = fail_result_request(&self.pool, req_id, "parent job not found").await;
                    return;
                }
                Err(e) => {
                    tracing::error!(%req_id, %job_id, error = %e, "failed to fetch parent job");
                    return;
                }
            };

            if !matches!(job.state, JobState::Completed) {
                tracing::debug!(
                    %req_id,
                    %job_id,
                    state = job.state.as_str(),
                    "parent job not yet completed — deferring result request"
                );
                return;
            }

            job.work_dir
        };

        // Derive the absolute host output path.
        let output_filename = match req.output_filename() {
            Some(f) => f,
            None => {
                let msg = format!(
                    "result type '{}' is not yet implemented",
                    req.result_type.as_str()
                );
                tracing::warn!(%req_id, "{}", msg);
                let _ = fail_result_request(&self.pool, req_id, &msg).await;
                return;
            }
        };

        let output_path = PathBuf::from(&work_dir)
            .join("results")
            .join(&output_filename);
        let output_path_str = output_path.to_string_lossy().to_string();

        // Build container spec.
        let spec = match result_container_spec(
            &req.result_type,
            &req.params,
            &work_dir,
            &output_path_str,
            &self.images.results,
        ) {
            Some(s) => s,
            None => {
                let msg = format!(
                    "could not build container spec for result type '{}'",
                    req.result_type.as_str()
                );
                tracing::warn!(%req_id, "{}", msg);
                let _ = fail_result_request(&self.pool, req_id, &msg).await;
                return;
            }
        };

        // Wrap spec in a WorkflowStep so the existing Runner can run it.
        // `next_state` is unused by the results scheduler but required by the type.
        let step = WorkflowStep {
            label:      format!("results.{}", req.result_type.as_str()),
            next_state: shared::models::JobState::Completed, // sentinel — unused
            spec,
        };

        self.spawn_result_step(req_id, step).await;
    }

    /// Spawns a task that runs the result container, then updates the DB.
    async fn spawn_result_step(&self, req_id: Uuid, step: WorkflowStep) {
        // Mark as in-flight.
        self.active.lock().await.insert(req_id, ());

        let pool   = self.pool.clone();
        let runner = self.runner.clone();
        let active = self.active.clone();

        let _handle: JoinHandle<()> = tokio::spawn(async move {
            // Transition to running.
            if let Err(e) = start_result_request(&pool, req_id).await {
                tracing::error!(%req_id, error = %e, "failed to mark result request as running");
            }

            let outcome = runner.run_step(req_id, &step).await;

            match outcome {
                StepOutcome::Success => {
                    if let Err(e) = complete_result_request(&pool, req_id).await {
                        tracing::error!(%req_id, error = %e, "failed to complete result request");
                    } else {
                        tracing::info!(%req_id, step = %step.label, "result request completed");
                    }
                }
                StepOutcome::Failed(err) => {
                    let msg = err.to_string();
                    if let Err(e) = fail_result_request(&pool, req_id, &msg).await {
                        tracing::error!(%req_id, error = %e, "failed to fail result request");
                    } else {
                        tracing::warn!(%req_id, step = %step.label, error = %msg, "result request failed");
                    }
                }
            }

            active.lock().await.remove(&req_id);
        });
    }
}