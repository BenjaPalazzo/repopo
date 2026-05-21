use serde::Deserialize;
use std::collections::HashMap;

/// Top-level workflow configuration, loaded from `workflows.toml`.
#[derive(Debug, Deserialize, Clone)]
pub struct WorkflowConfig {
    /// Default resource requirements grouped by tier. These apply to any step
    /// not listed in a per-step override.
    pub tiers: ResourceTiersConfig,

    /// Per-step resource overrides for ISCE2 stages. Key is the snake_case
    /// stage name matching the `IsceStage` enum variants
    /// (e.g. `"phase_unwrap"`).
    #[serde(default)]
    pub isce2_overrides: HashMap<String, StepResourceConfig>,

    /// Per-step resource overrides for MintPy stages.
    #[serde(default)]
    pub mintpy_overrides: HashMap<String, StepResourceConfig>,

    /// Per-step resource overrides for MiaplPy stages.
    #[serde(default)]
    pub miaplpy_overrides: HashMap<String, StepResourceConfig>,

    /// Resource requirements for the download/fetch step.
    #[serde(default = "WorkflowConfig::default_download_resources")]
    pub download: StepResourceConfig,
}

impl WorkflowConfig {
    fn default_download_resources() -> StepResourceConfig {
        StepResourceConfig {
            cpu_cores: 2,
            ram_gb: 4.0,
        }
    }
}

/// The three resource tiers used as defaults when no per-step override exists.
#[derive(Debug, Deserialize, Clone)]
pub struct ResourceTiersConfig {
    /// I/O-bound steps: unpacking, extracting, merging intermediate products.
    #[serde(default = "ResourceTiersConfig::default_light")]
    pub light: StepResourceConfig,

    /// Compute and I/O steps: resampling, geocoding, interferogram generation.
    #[serde(default = "ResourceTiersConfig::default_medium")]
    pub medium: StepResourceConfig,

    /// CPU/RAM-intensive steps: phase unwrapping.
    #[serde(default = "ResourceTiersConfig::default_heavy")]
    pub heavy: StepResourceConfig,
}

impl ResourceTiersConfig {
    fn default_light() -> StepResourceConfig {
        StepResourceConfig {
            cpu_cores: 2,
            ram_gb: 4.0,
        }
    }

    fn default_medium() -> StepResourceConfig {
        StepResourceConfig {
            cpu_cores: 4,
            ram_gb: 16.0,
        }
    }

    fn default_heavy() -> StepResourceConfig {
        StepResourceConfig {
            cpu_cores: 8,
            ram_gb: 32.0,
        }
    }
}

/// CPU and RAM requirement for a single processing step.
#[derive(Debug, Deserialize, Clone)]
pub struct StepResourceConfig {
    /// Number of logical CPU cores to allocate to the container.
    pub cpu_cores: u32,

    /// Amount of RAM to allocate to the container, in gigabytes.
    pub ram_gb: f64,
}
