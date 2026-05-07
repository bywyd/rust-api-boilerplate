mod app;
mod bootstrap;
mod infra;

use infra::config::app_config::AppConfig;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env (silently ignored if file is absent)
    dotenvy::dotenv().ok();

    let config = Arc::new(AppConfig::load()?);

    // Initialise logging/tracing — hold the guard for the process lifetime
    // so the file appender flushes on shutdown.
    let _log_guard = bootstrap::logger::init(&config.logging)?;

    tracing::info!(
        app_env = %std::env::var("APP_ENV").unwrap_or_else(|_| "development".into()),
        version = env!("CARGO_PKG_VERSION"),
        "Starting rust-api-boilerplate"
    );

    let state = bootstrap::build_state(Arc::clone(&config)).await?;

    bootstrap::server::run(state, &config.server).await?;

    Ok(())
}

