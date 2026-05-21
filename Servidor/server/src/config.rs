use std::{collections::HashMap, path::PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

// =============================================================================
// Top-level config
// =============================================================================

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub server:      HttpConfig,
    pub database:    DatabaseConfig,
    pub paths:       PathsConfig,
    pub auth:        AuthConfig,
    pub asf:         AsfConfig,
    pub defaults:    DefaultsConfig,
    pub containers:  ContainersConfig,
    pub user_tiers:  UserTiersConfig,
}

// =============================================================================
// HTTP listener
// =============================================================================

#[derive(Debug, Deserialize, Clone)]
pub struct HttpConfig {
    #[serde(default = "HttpConfig::default_host")]
    pub host: String,
    #[serde(default = "HttpConfig::default_port")]
    pub port: u16,
}

impl HttpConfig {
    fn default_host() -> String { "127.0.0.1".to_string() }
    fn default_port() -> u16 { 8080 }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

// =============================================================================
// Database
// =============================================================================

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub url: String,
    #[serde(default = "DatabaseConfig::default_pool_size")]
    pub pool_size: u32,
}

impl DatabaseConfig {
    fn default_pool_size() -> u32 { 10 }
}

// =============================================================================
// Paths
// =============================================================================

#[derive(Debug, Deserialize, Clone)]
pub struct PathsConfig {
    /// Root directory under which each job's working directory is created.
    /// Must match the scheduler's `jobs_root`.
    pub jobs_root: PathBuf,

    /// Root of the local SAR image archive. Used when building
    /// `burst_list.json` DATA/METADATA paths.
    pub archive_root: PathBuf,

    /// Root directory containing pre-computed demo jobs.
    /// Each subdirectory is a demo with the same layout as a job's work_dir.
    /// Defaults to `/sisar/demos` if not specified in config.toml.
    #[serde(default = "PathsConfig::default_demos_root")]
    pub demos_root: PathBuf,
}

impl PathsConfig {
    fn default_demos_root() -> PathBuf {
        PathBuf::from("/sisar/demos")
    }
}

// =============================================================================
// Auth
// =============================================================================

#[derive(Debug, Deserialize, Clone)]
pub struct AuthConfig {
    /// Map of interface name → bearer token.
    /// e.g. `{ "telegram": "abc123..." }`
    pub interface_tokens: HashMap<String, String>,
}

impl AuthConfig {
    /// Returns the interface name for the given token, if it matches any
    /// registered interface token.
    pub fn identify_token(&self, token: &str) -> Option<&str> {
        self.interface_tokens
            .iter()
            .find_map(|(name, t)| if t == token { Some(name.as_str()) } else { None })
    }
}

// =============================================================================
// ASF search
// =============================================================================

#[derive(Debug, Deserialize, Clone)]
pub struct AsfConfig {
    #[serde(default = "AsfConfig::default_endpoint")]
    pub endpoint: String,
}

impl AsfConfig {
    fn default_endpoint() -> String {
        "https://api.daac.asf.alaska.edu/services/search/param".to_string()
    }
}

// =============================================================================
// Processing defaults
// =============================================================================

#[derive(Debug, Deserialize, Clone)]
pub struct DefaultsConfig {
    #[serde(default = "DefaultsConfig::default_range_looks")]
    pub range_looks: u32,
    #[serde(default = "DefaultsConfig::default_azimuth_looks")]
    pub azimuth_looks: u32,
    #[serde(default = "DefaultsConfig::default_connections")]
    pub connections: u8,
}

impl DefaultsConfig {
    fn default_range_looks()   -> u32 { 20 }
    fn default_azimuth_looks() -> u32 { 5 }
    fn default_connections()   -> u8  { 1 }
}

// =============================================================================
// Containers
// =============================================================================

#[derive(Debug, Deserialize, Clone)]
pub struct ContainersConfig {
    /// Docker image used to extract and visualise timeseries from HDF5 output.
    pub results: String,
}

// =============================================================================
// User tier limits
// =============================================================================

#[derive(Debug, Deserialize, Clone)]
pub struct UserTiersConfig {
    pub demo: TierLimitsConfig,
    pub free: TierLimitsConfig,
    pub pro:  TierLimitsConfig,
}

impl UserTiersConfig {
    pub fn for_tier(&self, tier: &str) -> Option<&TierLimitsConfig> {
        match tier {
            "demo" => Some(&self.demo),
            "free" => Some(&self.free),
            "pro"  => Some(&self.pro),
            _      => None,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct TierLimitsConfig {
    /// Maximum AOI area in km². `0` = unlimited.
    pub max_aoi_km2: u64,
    /// Maximum time range in days. `0` = unlimited.
    pub max_time_range_days: u64,
    /// Maximum simultaneously active jobs. `0` = unlimited.
    pub max_concurrent_jobs: u64,
    /// Maximum jobs submitted per calendar month. `0` = unlimited.
    pub max_jobs_per_month: u64,
}

impl TierLimitsConfig {
    /// Returns `true` if `value` is within `limit`, treating `0` as unlimited.
    pub fn within_limit(limit: u64, value: u64) -> bool {
        limit == 0 || value <= limit
    }
}

// =============================================================================
// Loader
// =============================================================================

/// Loads and performs basic validation on the server configuration.
///
/// The config path is read from the `SERVER_CONFIG` environment variable,
/// defaulting to `config.toml` in the current working directory.
pub fn load_config() -> Result<ServerConfig> {
    let path = std::env::var("SERVER_CONFIG").unwrap_or_else(|_| "config.toml".to_string());

    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("reading server config from {path}"))?;

    let config: ServerConfig = toml::from_str(&raw)
        .with_context(|| format!("parsing server config from {path}"))?;

    validate(&config)?;

    Ok(config)
}

fn validate(cfg: &ServerConfig) -> Result<()> {
    if cfg.database.url.is_empty() {
        anyhow::bail!("database.url must not be empty");
    }
    if cfg.auth.interface_tokens.is_empty() {
        anyhow::bail!("auth.interface_tokens must contain at least one entry");
    }
    for (name, token) in &cfg.auth.interface_tokens {
        if token.is_empty() || token.starts_with("CHANGE_ME") {
            anyhow::bail!("auth.interface_tokens.{name} has not been set to a real secret");
        }
    }
    if cfg.containers.results.is_empty() {
        anyhow::bail!("containers.results must not be empty");
    }
    Ok(())
}
