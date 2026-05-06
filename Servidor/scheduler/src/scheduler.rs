use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use uuid::Uuid;

use shared::models::{Job, JobError, UserTier};
use shared::queries::{
    advance_job_state, claim_ready_jobs, complete_job, count_user_active_jobs, fail_job,
};

use crate::config::system::{SystemConfig, TierLimitsConfig};
use crate::config::workflow::WorkflowConfig;
use crate::runner::{Runner, StepOutcome};
use crate::workflow::definition::{ContainerImages, WorkflowStep, step_for_state};

// =============================================================================
// Resource pool
// =============================================================================

/// Tracks CPU cores and RAM currently allocated to running containers.
///
/// All mutations go through `try_acquire`, which atomically checks headroom
/// and records the allocation, and `release`, which is called when a container
/// exits. The pool is wrapped in `Arc<Mutex<...>>` so it can be shared across
/// the async tasks that manage individual containers.
#[derive(Debug)]
struct ResourcePool {
    total_cpu: u32,
    total_ram_gb: f64,
    used_cpu: u32,
    used_ram_gb: f64,
}

impl ResourcePool {
    fn new(total_cpu: u32, total_ram_gb: f64) -> Self {
        Self {
            total_cpu,
            total_ram_gb,
            used_cpu: 0,
            used_ram_gb: 0.0,
        }
    }

    /// Returns `true` and records the allocation if there is enough headroom
    /// for `cpu` cores and `ram_gb` gigabytes. Returns `false` without
    /// modifying state if there is not.
    fn try_acquire(&mut self, cpu: u32, ram_gb: f64) -> bool {
        if self.used_cpu + cpu <= self.total_cpu && self.used_ram_gb + ram_gb <= self.total_ram_gb {
            self.used_cpu += cpu;
            self.used_ram_gb += ram_gb;
            true
        } else {
            false
        }
    }

    /// Releases a previously acquired allocation. Must be called with the same
    /// values passed to the corresponding `try_acquire`.
    fn release(&mut self, cpu: u32, ram_gb: f64) {
        self.used_cpu = self.used_cpu.saturating_sub(cpu);
        self.used_ram_gb = (self.used_ram_gb - ram_gb).max(0.0);
    }

    fn available_cpu(&self) -> u32 {
        self.total_cpu.saturating_sub(self.used_cpu)
    }

    fn available_ram_gb(&self) -> f64 {
        (self.total_ram_gb - self.used_ram_gb).max(0.0)
    }
}

// =============================================================================
// Scheduler
// =============================================================================

/// The top-level scheduler. Holds all shared state and owns the poll loop.
pub struct Scheduler {
    pool: PgPool,
    runner: Runner,
    system_cfg: SystemConfig,
    workflow_cfg: WorkflowConfig,
    images: ContainerImages,
    resources: Arc<Mutex<ResourcePool>>,

    /// Tracks the number of containers currently running per job, used to
    /// enforce `max_concurrent_containers` and detect in-flight jobs.
    /// Key: job_id. Value: number of active container tasks for that job
    /// (always 0 or 1 in the current sequential-per-job model).
    active: Arc<Mutex<HashMap<Uuid, u32>>>,
}

impl Scheduler {
    pub fn new(
        pool: PgPool,
        runner: Runner,
        system_cfg: SystemConfig,
        workflow_cfg: WorkflowConfig,
    ) -> Self {
        let resources = Arc::new(Mutex::new(ResourcePool::new(
            system_cfg.resources.total_cpu_cores,
            system_cfg.resources.total_ram_gb,
        )));

        let images = ContainerImages {
            download: system_cfg.containers.download.clone(),
            isce2: system_cfg.containers.isce2.clone(),
            mintpy: system_cfg.containers.mintpy.clone(),
            miaplpy: system_cfg.containers.miaplpy.clone(),
        };

        Self {
            pool,
            runner,
            system_cfg,
            workflow_cfg,
            images,
            resources,
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Runs the scheduler poll loop indefinitely. This should be the only
    /// long-running task spawned in `main`.
    pub async fn run(self: Arc<Self>) {
        let interval = Duration::from_secs(self.system_cfg.scheduler.poll_interval_secs);
        tracing::info!(
            poll_interval_secs = interval.as_secs(),
            total_cpu = self.system_cfg.resources.total_cpu_cores,
            total_ram_gb = self.system_cfg.resources.total_ram_gb,
            "scheduler started"
        );

        loop {
            self.tick().await;
            tokio::time::sleep(interval).await;
        }
    }

    // -------------------------------------------------------------------------
    // Poll tick
    // -------------------------------------------------------------------------

    /// One scheduler tick: claim advanceable jobs from the DB and dispatch
    /// those that fit within available resources and per-user limits.
    async fn tick(&self) {
        let max = self.system_cfg.scheduler.max_concurrent_containers as i64;

        // How many slots are free? Claim at most that many jobs, minus however
        // many are already running, to avoid over-fetching.
        let running = self.active.lock().await.values().sum::<u32>() as i64;
        let claim_limit = (max - running).max(0);

        if claim_limit == 0 {
            tracing::debug!("all container slots occupied — skipping poll");
            return;
        }

        let jobs = match claim_ready_jobs(&self.pool, claim_limit).await {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(error = %e, "failed to claim ready jobs");
                return;
            }
        };

        if jobs.is_empty() {
            tracing::debug!("no advanceable jobs found");
            return;
        }

        tracing::debug!(count = jobs.len(), "claimed jobs for evaluation");

        for job in jobs {
            self.try_dispatch(job).await;
        }
    }

    // -------------------------------------------------------------------------
    // Dispatch
    // -------------------------------------------------------------------------

    /// Evaluates a single job and, if all conditions are met, spawns an async
    /// task to run its next step.
    async fn try_dispatch(&self, job: Job) {
        let job_id = job.id;
        let archive_path = self
            .system_cfg
            .paths
            .archive_root
            .to_string_lossy()
            .to_string();

        // Skip jobs already running a step in this scheduler instance.
        {
            let active = self.active.lock().await;
            if active.get(&job_id).copied().unwrap_or(0) > 0 {
                tracing::debug!(%job_id, "job already has an active step — skipping");
                return;
            }
        }

        // Resolve the next workflow step.
        let step = match step_for_state(&job, &self.workflow_cfg, &self.images, archive_path) {
            Some(s) => s,
            None => {
                // State is terminal or unrecognised — should not be in the
                // claimed set, but handle defensively.
                tracing::warn!(%job_id, state = ?job.state, "no step resolved for job state");
                return;
            }
        };

        // Per-user concurrency check.
        if !self.user_has_capacity(&job).await {
            tracing::debug!(
                %job_id,
                user_id = %job.user_id,
                "user concurrent job limit reached — deferring"
            );
            return;
        }

        // Resource headroom check.
        {
            let mut resources = self.resources.lock().await;
            let cpu = step.spec.resources.cpu_cores;
            let ram = step.spec.resources.ram_gb;

            if !resources.try_acquire(cpu, ram) {
                tracing::debug!(
                    %job_id,
                    step = %step.label,
                    required_cpu = cpu,
                    required_ram_gb = ram,
                    available_cpu = resources.available_cpu(),
                    available_ram_gb = resources.available_ram_gb(),
                    "insufficient resources — deferring"
                );
                return;
            }
        }

        // All checks passed — spawn the step task.
        self.spawn_step(job, step).await;
    }

    /// Spawns an async task that runs the step, updates the DB on completion,
    /// and releases resources. Updates the `active` map for the duration.
    async fn spawn_step(&self, job: Job, step: WorkflowStep) {
        let job_id = job.id;
        let cpu = step.spec.resources.cpu_cores;
        let ram = step.spec.resources.ram_gb;

        // Mark job as active.
        {
            let mut active = self.active.lock().await;
            *active.entry(job_id).or_insert(0) += 1;
        }

        let pool = self.pool.clone();
        let runner = self.runner.clone();
        let resources = self.resources.clone();
        let active = self.active.clone();

        let _handle: JoinHandle<()> = tokio::spawn(async move {
            let outcome = runner.run_step(job_id, &step).await;

            // Update DB based on outcome.
            match outcome {
                StepOutcome::Success => {
                    let next = &step.next_state;

                    // Completion sentinel produces a Completed next_state.
                    let result = if matches!(next, shared::models::JobState::Completed) {
                        complete_job(&pool, job_id).await
                    } else {
                        advance_job_state(&pool, job_id, next).await
                    };

                    if let Err(e) = result {
                        tracing::error!(
                            %job_id,
                            step = %step.label,
                            error = %e,
                            "failed to advance job state after successful step"
                        );
                    } else {
                        tracing::info!(
                            %job_id,
                            step = %step.label,
                            next_state = next.as_str(),
                            "job advanced"
                        );
                    }
                }

                StepOutcome::Failed(error) => {
                    if let Err(e) = fail_job(&pool, job_id, &error).await {
                        tracing::error!(
                            %job_id,
                            step = %step.label,
                            db_error = %e,
                            job_error = %error,
                            "failed to write failure state to DB"
                        );
                    } else {
                        tracing::warn!(
                            %job_id,
                            step = %step.label,
                            error = %error,
                            "job marked as failed"
                        );
                    }
                }
            }

            // Release resources and remove from active map.
            resources.lock().await.release(cpu, ram);
            let mut active = active.lock().await;
            let count = active.entry(job_id).or_insert(0);
            *count = count.saturating_sub(1);
            if *count == 0 {
                active.remove(&job_id);
            }
        });
    }

    // -------------------------------------------------------------------------
    // Per-user concurrency
    // -------------------------------------------------------------------------

    /// Returns `true` if the job's user is below their concurrent job limit.
    ///
    /// The limit is read from the config for the job's `effective_tier`, so a
    /// demo user with a prepaid pro code is checked against pro limits.
    async fn user_has_capacity(&self, job: &Job) -> bool {
        let limits = self.tier_limits(&job.effective_tier);

        // 0 means unlimited.
        if limits.max_concurrent_jobs == 0 {
            return true;
        }

        match count_user_active_jobs(&self.pool, job.user_id).await {
            Ok(count) => count < limits.max_concurrent_jobs as i64,
            Err(e) => {
                tracing::error!(
                    user_id = %job.user_id,
                    error = %e,
                    "failed to count user active jobs; allowing dispatch"
                );
                // Fail open: if we can't query, don't block the job.
                true
            }
        }
    }

    fn tier_limits(&self, tier: &UserTier) -> &TierLimitsConfig {
        match tier {
            UserTier::Demo => &self.system_cfg.user_tiers.demo,
            UserTier::Free => &self.system_cfg.user_tiers.free,
            UserTier::Pro => &self.system_cfg.user_tiers.pro,
        }
    }
}
