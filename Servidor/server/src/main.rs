mod admin;
mod asf;
mod auth;
mod config;
mod error;
mod jobs;
mod specification;

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    Router,
    routing::{delete, get, post},
};
use bollard::Docker;
use sqlx::postgres::PgPoolOptions;
use tower_http::trace::TraceLayer;

use crate::config::ServerConfig;

// =============================================================================
// Shared application state
// =============================================================================

/// State injected into every axum handler via `State<AppState>`.
#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub config: Arc<ServerConfig>,
    pub docker: Docker,
}

// =============================================================================
// Entry point
// =============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    // -------------------------------------------------------------------------
    // Logging
    // -------------------------------------------------------------------------
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "server=info,shared=info,tower_http=info".into()),
        )
        .init();

    // -------------------------------------------------------------------------
    // Configuration
    // -------------------------------------------------------------------------
    let config = config::load_config().context("loading server config")?;

    tracing::info!(
        host = %config.server.host,
        port = config.server.port,
        "configuration loaded"
    );

    // -------------------------------------------------------------------------
    // Database
    // -------------------------------------------------------------------------
    let pool = PgPoolOptions::new()
        .max_connections(config.database.pool_size)
        .connect(&config.database.url)
        .await
        .context("connecting to PostgreSQL")?;

    tracing::info!("database connection pool established");

    sqlx::migrate!("../shared/migrations")
        .run(&pool)
        .await
        .context("running database migrations")?;

    tracing::info!("database migrations applied");

    // -------------------------------------------------------------------------
    // Docker client (for results container)
    // -------------------------------------------------------------------------
    let docker = Docker::connect_with_local_defaults().context("connecting to Docker daemon")?;

    tracing::info!("Docker client connected");

    // -------------------------------------------------------------------------
    // Router
    // -------------------------------------------------------------------------
    let bind_addr = config.server.bind_addr();

    let state = AppState {
        pool,
        config: Arc::new(config),
        docker,
    };

    let app = Router::new()
        // Auth
        .route("/auth/telegram", post(auth::telegram_identify))
        // Jobs
        .route("/jobs", post(jobs::handlers::create_job))
        .route("/jobs", get(jobs::handlers::list_jobs))
        .route("/jobs/{id}", get(jobs::handlers::get_job_handler))
        .route("/jobs/{id}", delete(jobs::handlers::cancel_job))
        // Results
        .route("/jobs/{id}/velocity", get(jobs::results::get_velocity))
        .route("/jobs/{id}/timeseries", get(jobs::results::get_timeseries))
        // Admin (stubs)
        //.route("/admin/codes",          post(admin::handlers::create_code))
        //.route("/admin/users",          get(admin::handlers::list_users))
        //.route("/admin/users/{id}/tier", post(admin::handlers::set_user_tier))
        // Middleware
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // -------------------------------------------------------------------------
    // Bind and serve
    // -------------------------------------------------------------------------
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("binding to {bind_addr}"))?;

    tracing::info!(addr = %bind_addr, "SISAR API server listening");

    axum::serve(listener, app)
        .await
        .context("axum serve returned")?;

    Ok(())
}
