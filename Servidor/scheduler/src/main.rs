mod config;
mod runner;
mod scheduler;
mod workflow;

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;

use crate::config::{load_system_config, load_workflow_config};
use crate::runner::Runner;
use crate::scheduler::Scheduler;

#[tokio::main]
async fn main() -> Result<()> {
    // -------------------------------------------------------------------------
    // Logging
    // -------------------------------------------------------------------------
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "scheduler=info,shared=info".into()),
        )
        .init();

    // -------------------------------------------------------------------------
    // Configuration
    // -------------------------------------------------------------------------
    let config_path = std::env::var("SCHEDULER_CONFIG")
        .unwrap_or_else(|_| "config.toml".to_string());

    let workflow_config_path = std::env::var("SCHEDULER_WORKFLOW_CONFIG")
        .unwrap_or_else(|_| "workflows.toml".to_string());

    let system_cfg = load_system_config(Path::new(&config_path))
        .with_context(|| format!("loading system config from {config_path}"))?;

    let workflow_cfg = load_workflow_config(Path::new(&workflow_config_path))
        .with_context(|| format!("loading workflow config from {workflow_config_path}"))?;

    tracing::info!(config = %config_path, workflow_config = %workflow_config_path, "configuration loaded");

    // -------------------------------------------------------------------------
    // Database
    // -------------------------------------------------------------------------
    let pool = PgPoolOptions::new()
        .max_connections(system_cfg.database.pool_size)
        .connect(&system_cfg.database.url)
        .await
        .context("connecting to PostgreSQL")?;

    tracing::info!("database connection pool established");

    // Run migrations at startup. Idempotent — already-applied migrations are
    // skipped. The migrations directory is embedded at compile time.
    sqlx::migrate!("../shared/migrations")
        .run(&pool)
        .await
        .context("running database migrations")?;

    tracing::info!("database migrations applied");

    // -------------------------------------------------------------------------
    // Docker runner
    // -------------------------------------------------------------------------
    let runner = Runner::connect(system_cfg.paths.logs_root.clone())
        .context("connecting to Docker daemon")?;

    tracing::info!("Docker runner connected");

    // -------------------------------------------------------------------------
    // Scheduler
    // -------------------------------------------------------------------------
    let scheduler = Arc::new(Scheduler::new(pool, runner, system_cfg, workflow_cfg));

    scheduler.run().await;

    // `run` loops forever. If it returns, something has gone wrong.
    anyhow::bail!("scheduler poll loop exited unexpectedly")
}
