use rust_api_boilerplate::bootstrap;
use rust_api_boilerplate::infra::config::app_config::AppConfig;
use std::sync::Arc;

/// Standalone worker binary.
///
/// Run independently with:
///   `cargo run --bin worker`
///
/// The worker reads the same config as the API server and processes jobs from
/// the configured queue backend (channel or database).
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let config = Arc::new(AppConfig::load()?);

    let _log_guard = bootstrap::logger::init(&config.logging)?;

    tracing::info!(
        app_env = %std::env::var("APP_ENV").unwrap_or_else(|_| "development".into()),
        version = env!("CARGO_PKG_VERSION"),
        "Starting standalone worker"
    );

    let state = bootstrap::build_state(Arc::clone(&config)).await?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);

    // Graceful shutdown on ctrl-c.
    tokio::spawn(async move {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %e, "Failed to listen for ctrl-c");
            return;
        }
        tracing::info!("Shutdown signal received");
        let _ = shutdown_tx.send(());
    });

    bootstrap::worker::run(state, shutdown_rx).await?;

    Ok(())
}
