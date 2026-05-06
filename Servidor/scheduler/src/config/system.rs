use serde::Deserialize;
use std::path::PathBuf;

/// Top-level system configuration, loaded from `config.toml`.
#[derive(Debug, Deserialize, Clone)]
pub struct SystemConfig {
    pub paths: PathsConfig,
    pub database: DatabaseConfig,
    pub scheduler: SchedulerConfig,
    pub resources: ResourcesConfig,
    pub containers: ContainerImagesConfig,
    pub user_tiers: UserTiersConfig,
}

// -----------------------------------------------------------------------------
// Paths
// -----------------------------------------------------------------------------

#[derive(Debug, Deserialize, Clone)]
pub struct PathsConfig {
    /// Root directory under which each job's working directory is created.
    pub jobs_root: PathBuf,

    /// Root of the local SAR image archive.
    pub archive_root: PathBuf,

    /// Root directory for per-job, per-step log files.
    pub logs_root: PathBuf,
}

// -----------------------------------------------------------------------------
// Database
// -----------------------------------------------------------------------------

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub url: String,

    #[serde(default = "DatabaseConfig::default_pool_size")]
    pub pool_size: u32,
}

impl DatabaseConfig {
    fn default_pool_size() -> u32 { 10 }
}

// -----------------------------------------------------------------------------
// Scheduler
// -----------------------------------------------------------------------------

#[derive(Debug, Deserialize, Clone)]
pub struct SchedulerConfig {
    #[serde(default = "SchedulerConfig::default_poll_interval_secs")]
    pub poll_interval_secs: u64,

    #[serde(default = "SchedulerConfig::default_max_concurrent_containers")]
    pub max_concurrent_containers: usize,
}

impl SchedulerConfig {
    fn default_poll_interval_secs() -> u64 { 10 }
    fn default_max_concurrent_containers() -> usize { 16 }
}

// -----------------------------------------------------------------------------
// Resources
// -----------------------------------------------------------------------------

#[derive(Debug, Deserialize, Clone)]
pub struct ResourcesConfig {
    pub total_cpu_cores: u32,
    pub total_ram_gb: f64,
}

// -----------------------------------------------------------------------------
// Container images
// -----------------------------------------------------------------------------

#[derive(Debug, Deserialize, Clone)]
pub struct ContainerImagesConfig {
    pub download: String,
    pub isce2: String,
    pub mintpy: String,
    pub miaplpy: String,
}

// -----------------------------------------------------------------------------
// User tier limits
// -----------------------------------------------------------------------------

/// Submission limits for all account tiers. Enforced by the API server at
/// job submission time. The scheduler reads these only to enforce the
/// `max_concurrent_jobs` limit at dispatch time.
#[derive(Debug, Deserialize, Clone)]
pub struct UserTiersConfig {
    pub demo: TierLimitsConfig,
    pub free: TierLimitsConfig,
    pub pro: TierLimitsConfig,
}

/// Resource and usage limits for a single user tier.
#[derive(Debug, Deserialize, Clone)]
pub struct TierLimitsConfig {
    /// Maximum allowed AOI area in km². `0` means unlimited.
    pub max_aoi_km2: u64,

    /// Maximum allowed time range in days. `0` means unlimited.
    pub max_time_range_days: u64,

    /// Maximum number of jobs this user may have in an active (non-terminal)
    /// state simultaneously. `0` means unlimited.
    pub max_concurrent_jobs: u64,

    /// Maximum number of jobs this user may submit per calendar month.
    /// `0` means unlimited.
    pub max_jobs_per_month: u64,
}

impl TierLimitsConfig {
    /// Returns `true` if `value` is within the limit, respecting the
    /// convention that `0` means unlimited.
    pub fn within_limit(limit: u64, value: u64) -> bool {
        limit == 0 || value <= limit
    }
}
