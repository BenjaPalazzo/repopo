pub mod system;
pub mod workflow;

pub use system::SystemConfig;
pub use workflow::WorkflowConfig;

use anyhow::{Context, Result};
use std::path::Path;

/// Loads and validates the system configuration from `config.toml`.
pub fn load_system_config(path: &Path) -> Result<SystemConfig> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read system config at {}", path.display()))?;

    let config: SystemConfig = toml::from_str(&raw)
        .with_context(|| format!("Failed to parse system config at {}", path.display()))?;

    validate_system_config(&config)?;

    Ok(config)
}

/// Loads and validates the workflow configuration from `workflows.toml`.
pub fn load_workflow_config(path: &Path) -> Result<WorkflowConfig> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read workflow config at {}", path.display()))?;

    let config: WorkflowConfig = toml::from_str(&raw)
        .with_context(|| format!("Failed to parse workflow config at {}", path.display()))?;

    validate_workflow_config(&config)?;

    Ok(config)
}

fn validate_system_config(config: &SystemConfig) -> Result<()> {
    if config.resources.total_cpu_cores == 0 {
        anyhow::bail!("resources.total_cpu_cores must be greater than 0");
    }
    if config.resources.total_ram_gb <= 0.0 {
        anyhow::bail!("resources.total_ram_gb must be greater than 0");
    }
    if config.scheduler.max_concurrent_containers == 0 {
        anyhow::bail!("scheduler.max_concurrent_containers must be greater than 0");
    }
    if config.database.url.is_empty() {
        anyhow::bail!("database.url must not be empty");
    }
    Ok(())
}

fn validate_workflow_config(config: &WorkflowConfig) -> Result<()> {
    let tiers = &config.tiers;
    if tiers.light.cpu_cores == 0 || tiers.medium.cpu_cores == 0 || tiers.heavy.cpu_cores == 0 {
        anyhow::bail!("All resource tier cpu_cores values must be greater than 0");
    }
    if tiers.light.ram_gb <= 0.0 || tiers.medium.ram_gb <= 0.0 || tiers.heavy.ram_gb <= 0.0 {
        anyhow::bail!("All resource tier ram_gb values must be greater than 0");
    }
    Ok(())
}
