use std::path::Path;

use crate::config::workflow::{StepResourceConfig, WorkflowConfig};
use shared::models::{IsceStage, JobState, MiaplpyStage, MintpyStage, ResourceTier};

// =============================================================================
// Resource requirement
// =============================================================================

/// Resolved CPU and RAM requirement for a single container invocation.
///
/// Produced by resolving a step's tier against the workflow config, with any
/// per-step override applied on top. Once resolved, this value is passed
/// directly to the resource pool and then to the Docker runner as container
/// resource limits.
#[derive(Debug, Clone)]
pub struct ResourceRequirement {
    pub cpu_cores: u32,
    pub ram_gb: f64,
}

impl From<&StepResourceConfig> for ResourceRequirement {
    fn from(c: &StepResourceConfig) -> Self {
        Self {
            cpu_cores: c.cpu_cores,
            ram_gb: c.ram_gb,
        }
    }
}

// =============================================================================
// Container invocation
// =============================================================================

/// Everything the Docker runner needs to launch a single processing step.
#[derive(Debug, Clone)]
pub struct ContainerSpec {
    /// The Docker image to run (e.g. `"sisar/isce2:latest"`).
    pub image: String,

    /// Command-line arguments passed to the container's entrypoint after the
    /// image name. For ISCE2 this is the snake_case stage name; for the
    /// download container this is empty (it reads the spec file directly).
    pub args: Vec<String>,

    pub host_archive: String,

    /// Absolute path on the host that will be bind-mounted into the container
    /// at the path defined by `container_workdir`. This is always the job's
    /// working directory.
    pub host_workdir: String,

    /// Mount point inside the container where the job directory appears.
    pub container_workdir: String,

    /// Resolved resource limits to apply to this container.
    pub resources: ResourceRequirement,
}

// =============================================================================
// Workflow step
// =============================================================================

/// A single dispatchable unit of work: a fully described container invocation
/// derived from a job's current state.
///
/// The scheduler calls `step_for_state()` once per dispatch cycle per job,
/// and passes the resulting `WorkflowStep` to the runner.
#[derive(Debug, Clone)]
pub struct WorkflowStep {
    /// Human-readable label used in log file names and tracing spans
    /// (e.g. `"isce2.phase_unwrap"`, `"download"`).
    pub label: String,

    /// The next `JobState` to write to the DB if this step succeeds.
    /// Computed eagerly so the scheduler never has to re-derive it after the
    /// container exits.
    pub next_state: JobState,

    /// Container invocation for this step.
    pub spec: ContainerSpec,
}

// =============================================================================
// Workflow resolver
// =============================================================================

/// Resolves the next `WorkflowStep` for a job given its current state.
///
/// Returns `None` when the state is terminal or not advanceable (e.g.
/// `Completed`, `Failed`, `Cancelled`, `Initializing`). The scheduler treats
/// `None` as "nothing to do for this job."
///
/// The `workflow` field on `Job` determines which processing suite follows
/// ISCE2 (`"sbas"` → MintPy, `"ps_insar"` | `"timeseries"` → MintPy,
/// with MiaplPy as an optional subsequent stage). This routing will be
/// fleshed out as MintPy and MiaplPy stages are defined.
pub fn step_for_state(
    job: &shared::models::Job,
    workflow_cfg: &WorkflowConfig,
    images: &ContainerImages,
    archive_dir: String,
) -> Option<WorkflowStep> {
    let job_workdir = job.work_dir.as_str();

    match &job.state {
        JobState::Queued => Some(download_step(
            archive_dir.as_str(),
            job_workdir,
            workflow_cfg,
            images,
        )),

        JobState::Downloading => {
            let first = IsceStage::first();
            Some(isce_step(&first, job_workdir, workflow_cfg, images))
        }

        JobState::IsceProcessing { stage } => {
            match stage.next() {
                Some(next_stage) => Some(isce_step(&next_stage, job_workdir, workflow_cfg, images)),
                // ISCE2 complete — route to the appropriate next suite based
                // on the job's workflow. Currently resolves to completion
                // until MintPy/MiaplPy stages are defined.
                None => post_isce_step(&job.workflow, job_workdir, workflow_cfg, images),
            }
        }

        JobState::MintpyProcessing { stage } => match stage.next() {
            Some(next_stage) => Some(mintpy_step(&next_stage, job_workdir, workflow_cfg, images)),
            None => post_mintpy_step(&job.workflow, job_workdir, workflow_cfg, images),
        },

        JobState::MiaplpyProcessing { stage } => {
            match stage.next() {
                Some(next_stage) => {
                    Some(miaplpy_step(&next_stage, job_workdir, workflow_cfg, images))
                }
                // MiaplPy is the final suite — no further steps.
                None => None,
            }
        }

        // Terminal or non-scheduler states.
        JobState::Initializing
        | JobState::Completed
        | JobState::Failed { .. }
        | JobState::Cancelled => None,
    }
}

// =============================================================================
// Step constructors
// =============================================================================

/// Routes to the appropriate next suite after ISCE2 completes, based on the
/// job's workflow. Returns a completion bridge when the next suite has no
/// stages defined yet.
fn post_isce_step(
    workflow: &str,
    job_workdir: &str,
    workflow_cfg: &WorkflowConfig,
    images: &ContainerImages,
) -> Option<WorkflowStep> {
    match workflow {
        // All supported workflows currently transition through MintPy after
        // ISCE2. This match will expand as workflow routing diverges.
        "sbas" | "ps_insar" | "timeseries" => {
            // TODO: replace with MintPy first-stage step once stages defined.
            let first = MintpyStage::first();
            Some(mintpy_step(&first, job_workdir, workflow_cfg, images))
        }
        _ => {
            tracing::warn!(
                workflow,
                "unknown workflow for post-ISCE routing; completing job"
            );
            Some(completion_bridge_step(job_workdir, workflow_cfg, images))
        }
    }
}

/// Routes to the appropriate next suite after MintPy completes.
fn post_mintpy_step(
    workflow: &str,
    job_workdir: &str,
    workflow_cfg: &WorkflowConfig,
    images: &ContainerImages,
) -> Option<WorkflowStep> {
    match workflow {
        "ps_insar" => {
            // PS-InSAR continues into MiaplPy after MintPy.
            // TODO: replace with MiaplPy first-stage step once stages defined.
            Some(completion_bridge_step(job_workdir, workflow_cfg, images))
        }
        _ => {
            // SBAS and time-series workflows end after MintPy.
            None
        }
    }
}

fn download_step(
    archive_dir: &str,
    job_workdir: &str,
    workflow_cfg: &WorkflowConfig,
    images: &ContainerImages,
) -> WorkflowStep {
    let resources = ResourceRequirement::from(&workflow_cfg.download);

    WorkflowStep {
        label: "download".to_string(),
        next_state: JobState::Downloading,
        spec: ContainerSpec {
            image: images.download.clone(),
            args: vec![],
            host_archive: archive_dir.to_string(),
            host_workdir: job_workdir.to_string(),
            container_workdir: CONTAINER_WORKDIR.to_string(),
            resources,
        },
    }
}

fn isce_step(
    stage: &IsceStage,
    job_workdir: &str,
    workflow_cfg: &WorkflowConfig,
    images: &ContainerImages,
) -> WorkflowStep {
    let resources = resolve_resources(
        stage.as_str(),
        stage.resource_tier(),
        &workflow_cfg.isce2_overrides,
        workflow_cfg,
    );

    WorkflowStep {
        label: format!("isce2.{}", stage.as_str()),
        next_state: JobState::IsceProcessing {
            stage: stage.clone(),
        },
        spec: ContainerSpec {
            image: images.isce2.clone(),
            args: vec![stage.as_str().to_string()],
            host_archive: job_workdir.to_string(),
            host_workdir: job_workdir.to_string(),
            container_workdir: CONTAINER_WORKDIR.to_string(),
            resources,
        },
    }
}

fn mintpy_step(
    stage: &MintpyStage,
    job_workdir: &str,
    workflow_cfg: &WorkflowConfig,
    images: &ContainerImages,
) -> WorkflowStep {
    let resources = resolve_resources(
        stage.as_str(),
        stage.resource_tier(),
        &workflow_cfg.mintpy_overrides,
        workflow_cfg,
    );

    WorkflowStep {
        label: format!("mintpy.{}", stage.as_str()),
        next_state: JobState::MintpyProcessing {
            stage: stage.clone(),
        },
        spec: ContainerSpec {
            image: images.mintpy.clone(),
            args: vec![stage.as_str().to_string()],
            host_archive: job_workdir.to_string(),
            host_workdir: job_workdir.to_string(),
            container_workdir: CONTAINER_WORKDIR.to_string(),
            resources,
        },
    }
}

fn miaplpy_step(
    stage: &MiaplpyStage,
    job_workdir: &str,
    workflow_cfg: &WorkflowConfig,
    images: &ContainerImages,
) -> WorkflowStep {
    let resources = resolve_resources(
        stage.as_str(),
        stage.resource_tier(),
        &workflow_cfg.miaplpy_overrides,
        workflow_cfg,
    );

    WorkflowStep {
        label: format!("miaplpy.{}", stage.as_str()),
        next_state: JobState::MiaplpyProcessing {
            stage: stage.clone(),
        },
        spec: ContainerSpec {
            image: images.miaplpy.clone(),
            args: vec![stage.as_str().to_string()],
            host_archive: job_workdir.to_string(),
            host_workdir: job_workdir.to_string(),
            container_workdir: CONTAINER_WORKDIR.to_string(),
            resources,
        },
    }
}

/// Temporary bridge step used when a processing suite completes but the next
/// suite's stages are not yet defined. Produces a `Completed` next-state so
/// the job terminates cleanly rather than stalling.
///
/// Remove and replace with the appropriate handoff once MintPy/MiaplPy stages
/// are defined.
fn completion_bridge_step(
    job_workdir: &str,
    _workflow_cfg: &WorkflowConfig,
    _images: &ContainerImages,
) -> WorkflowStep {
    // This step produces no container — it is a sentinel that tells the
    // scheduler to write `Completed` directly. The runner must handle a
    // `WorkflowStep` whose `spec.image` is empty as a no-op container launch,
    // writing the next_state directly instead.
    WorkflowStep {
        label: "complete".to_string(),
        next_state: JobState::Completed,
        spec: ContainerSpec {
            image: String::new(),
            args: vec![],
            host_archive: job_workdir.to_string(),
            host_workdir: job_workdir.to_string(),
            container_workdir: CONTAINER_WORKDIR.to_string(),
            resources: ResourceRequirement {
                cpu_cores: 0,
                ram_gb: 0.0,
            },
        },
    }
}

// =============================================================================
// Resource resolution
// =============================================================================

/// Resolves the resource requirement for a step by checking for a per-step
/// override first, then falling back to the tier default.
fn resolve_resources(
    stage_key: &str,
    tier: ResourceTier,
    overrides: &std::collections::HashMap<String, StepResourceConfig>,
    cfg: &WorkflowConfig,
) -> ResourceRequirement {
    if let Some(override_cfg) = overrides.get(stage_key) {
        return ResourceRequirement::from(override_cfg);
    }

    let tier_cfg = match tier {
        ResourceTier::Light => &cfg.tiers.light,
        ResourceTier::Medium => &cfg.tiers.medium,
        ResourceTier::Heavy => &cfg.tiers.heavy,
    };

    ResourceRequirement::from(tier_cfg)
}

// =============================================================================
// Container images
// =============================================================================

/// Resolved image tags for all processing containers. Built once at startup
/// from `SystemConfig::containers` and passed through to step constructors.
#[derive(Debug, Clone)]
pub struct ContainerImages {
    pub download: String,
    pub isce2: String,
    pub mintpy: String,
    pub miaplpy: String,
}

/// Mount point inside every processing container where the job directory
/// appears. Containers read their spec files and write outputs relative to
/// this path.
const CONTAINER_WORKDIR: &str = "/job";
