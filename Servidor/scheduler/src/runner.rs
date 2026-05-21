use std::path::PathBuf;

use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, LogOutput, LogsOptions, RemoveContainerOptions,
    StartContainerOptions, WaitContainerOptions,
};
use bollard::models::{HostConfig, ResourcesUlimits};
use futures::StreamExt;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use shared::models::JobError;

use crate::workflow::definition::{ContainerSpec, WorkflowStep};

// =============================================================================
// Errors
// =============================================================================

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("failed to connect to Docker daemon: {0}")]
    DaemonConnection(#[source] bollard::errors::Error),

    #[error("job {job_id}, step {step}: {source}")]
    Step {
        job_id: Uuid,
        step: String,
        #[source]
        source: StepError,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum StepError {
    #[error("failed to create log file at {path}: {source}")]
    LogFileCreate {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Docker API error: {0}")]
    Docker(#[from] bollard::errors::Error),

    #[error("container exited with non-zero status {exit_code}")]
    NonZeroExit { exit_code: i64 },

    #[error("log write error: {0}")]
    LogWrite(#[from] std::io::Error),
}

impl StepError {
    /// Converts a `StepError` into the typed `JobError` that will be
    /// persisted to the database.
    pub fn into_job_error(self) -> JobError {
        match self {
            StepError::NonZeroExit { exit_code } => JobError::ContainerFailed { exit_code },
            StepError::Docker(e) => JobError::ContainerStartFailed {
                reason: e.to_string(),
            },
            other => JobError::Internal {
                message: other.to_string(),
            },
        }
    }
}

// =============================================================================
// Outcome
// =============================================================================

/// The result of running a single workflow step.
pub enum StepOutcome {
    /// Container exited 0. The scheduler should write `step.next_state` to DB.
    Success,
    /// Container exited non-zero or failed to start. The scheduler should
    /// write `Failed { error }` to DB.
    Failed(JobError),
}

// =============================================================================
// Runner
// =============================================================================

/// Manages the Docker connection and executes workflow steps.
///
/// A single `Runner` instance is held by the scheduler for its lifetime.
/// It is cheaply cloneable — `bollard::Docker` is internally `Arc`-backed.
#[derive(Clone)]
pub struct Runner {
    docker: Docker,
    logs_root: PathBuf,
}

impl Runner {
    /// Connects to the local Docker daemon via the Unix socket and returns a
    /// ready `Runner`. Should be called once at scheduler startup.
    pub fn connect(logs_root: PathBuf) -> Result<Self, RunnerError> {
        let docker =
            Docker::connect_with_local_defaults().map_err(RunnerError::DaemonConnection)?;
        Ok(Self { docker, logs_root })
    }

    /// Executes a single workflow step for the given job.
    ///
    /// Handles the completion sentinel (empty image) as a no-op and returns
    /// `Success` immediately without touching Docker.
    ///
    /// For real steps: creates the container, starts it, streams stdout and
    /// stderr to separate log files, waits for exit, removes the container,
    /// and returns the outcome.
    pub async fn run_step(&self, job_id: Uuid, step: &WorkflowStep) -> StepOutcome {
        // Completion sentinel — no container to run.
        if step.spec.image.is_empty() {
            tracing::debug!(%job_id, step = %step.label, "completion sentinel — skipping container launch");
            return StepOutcome::Success;
        }

        match self.run_container(job_id, step).await {
            Ok(()) => StepOutcome::Success,
            Err(e) => {
                let job_error = e.into_job_error();
                tracing::error!(
                    %job_id,
                    step = %step.label,
                    error = %job_error,
                    "step failed"
                );
                StepOutcome::Failed(job_error)
            }
        }
    }

    // -------------------------------------------------------------------------
    // Internal
    // -------------------------------------------------------------------------

    async fn run_container(&self, job_id: Uuid, step: &WorkflowStep) -> Result<(), StepError> {
        let spec = &step.spec;
        let container_name = container_name(job_id, &step.label);

        tracing::info!(
            %job_id,
            step = %step.label,
            image = %spec.image,
            "launching container"
        );

        // Create -----------------------------------------------------------------
        let container_id = self.create_container(&container_name, spec).await?;

        // Start ------------------------------------------------------------------
        self.docker
            .start_container(&container_id, None::<StartContainerOptions<String>>)
            .await?;

        // Stream logs ------------------------------------------------------------
        let log_dir = self.logs_root.join(job_id.to_string());
        tokio::fs::create_dir_all(&log_dir)
            .await
            .map_err(|e| StepError::LogFileCreate {
                path: log_dir.clone(),
                source: e,
            })?;
        self.stream_logs(&container_id, &log_dir, &step.label)
            .await?;

        // Wait for exit ----------------------------------------------------------
        let exit_code = self.wait_for_exit(&container_id).await?;

        // Remove container -------------------------------------------------------
        self.docker
            .remove_container(
                &container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await?;

        if exit_code != 0 {
            return Err(StepError::NonZeroExit { exit_code });
        }

        tracing::info!(%job_id, step = %step.label, "step completed successfully");
        Ok(())
    }

    async fn create_container(
        &self,
        name: &str,
        spec: &ContainerSpec,
    ) -> Result<String, StepError> {
        // Convert cpu_cores to Docker's CPU quota units.
        // Docker uses cpu_period (default 100_000 µs) and cpu_quota.
        // cpu_quota = cpu_cores * cpu_period gives a hard core-count limit.
        let cpu_quota = spec.resources.cpu_cores as i64 * 100_000;

        // RAM in bytes.
        let memory_bytes = (spec.resources.ram_gb * 1024.0 * 1024.0 * 1024.0) as i64;

        let host_config = HostConfig {
            binds: Some(vec![
                format!("{}:{}", spec.host_workdir, spec.container_workdir),
                format!("{}:/archive", spec.host_archive),
            ]),
            cpu_quota: Some(cpu_quota),
            cpu_period: Some(100_000),
            memory: Some(memory_bytes),
            memory_swap: Some(memory_bytes),        // disable swap
            //network_mode: Some("none".to_string()), // no network after download
            ulimits: Some(vec![ResourcesUlimits {
                // Prevent runaway file descriptor usage inside containers.
                name: Some("nofile".to_string()),
                soft: Some(4096),
                hard: Some(4096),
            }]),
            ..Default::default()
        };

        let args: Vec<&str> = spec.args.iter().map(String::as_str).collect();

        let config = Config {
            image: Some(spec.image.as_str()),
            cmd: if args.is_empty() { None } else { Some(args) },
            working_dir: Some(spec.container_workdir.as_str()),
            host_config: Some(host_config),
            ..Default::default()
        };

        let response = self
            .docker
            .create_container(
                Some(CreateContainerOptions {
                    name,
                    platform: None,
                }),
                config,
            )
            .await?;

        Ok(response.id)
    }

    /// Streams container stdout and stderr to separate `.stdout` / `.stderr`
    /// files in the job's log directory. Both streams are consumed concurrently
    /// so that a container that blocks on stderr doesn't stall stdout draining.
    async fn stream_logs(
        &self,
        container_id: &str,
        log_dir: &PathBuf,
        step_label: &str,
    ) -> Result<(), StepError> {
        let stdout_path = log_dir.join(format!("{step_label}.stdout"));
        let stderr_path = log_dir.join(format!("{step_label}.stderr"));

        let mut stdout_file =
            File::create(&stdout_path)
                .await
                .map_err(|e| StepError::LogFileCreate {
                    path: stdout_path.clone(),
                    source: e,
                })?;
        let mut stderr_file =
            File::create(&stderr_path)
                .await
                .map_err(|e| StepError::LogFileCreate {
                    path: stderr_path.clone(),
                    source: e,
                })?;

        let mut log_stream = self.docker.logs(
            container_id,
            Some(LogsOptions::<String> {
                follow: true,
                stdout: true,
                stderr: true,
                ..Default::default()
            }),
        );

        while let Some(msg) = log_stream.next().await {
            match msg? {
                LogOutput::StdOut { message } => stdout_file.write_all(&message).await?,
                LogOutput::StdErr { message } => stderr_file.write_all(&message).await?,
                // Console/TTY output — route to stdout log.
                LogOutput::Console { message } => stdout_file.write_all(&message).await?,
                LogOutput::StdIn { .. } => {}
            }
        }

        stdout_file.flush().await?;
        stderr_file.flush().await?;

        Ok(())
    }

    async fn wait_for_exit(&self, container_id: &str) -> Result<i64, StepError> {
        let mut wait_stream = self.docker.wait_container(
            container_id,
            Some(WaitContainerOptions {
                condition: "not-running",
            }),
        );

        // `wait_container` yields exactly one item containing the exit code.
        match wait_stream.next().await {
            Some(Ok(response)) => Ok(response.status_code),
            Some(Err(e)) => Err(StepError::Docker(e)),
            None => Err(StepError::Docker(
                bollard::errors::Error::DockerResponseServerError {
                    status_code: 500,
                    message: "wait stream ended without a response".to_string(),
                },
            )),
        }
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Produces a deterministic, Docker-safe container name from the job ID and
/// step label. Colons and dots in step labels (e.g. `"isce2.phase_unwrap"`)
/// are replaced with hyphens since Docker does not allow them in names.
fn container_name(job_id: Uuid, step_label: &str) -> String {
    let safe_label = step_label.replace(['.', ':'], "-");
    format!("sisar-{job_id}-{safe_label}")
}
