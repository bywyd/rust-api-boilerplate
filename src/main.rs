use rust_api_boilerplate::bootstrap;
use rust_api_boilerplate::infra::config::app_config::AppConfig;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env (silently ignored if file is absent)
    dotenvy::dotenv().ok();

    let config = Arc::new(AppConfig::load()?);

    // Initialise logging/tracing — hold the guard for the process lifetime
    // so the file appender flushes on shutdown.
    let _log_guard = bootstrap::logger::init(&config.logging, "api")?;

    tracing::info!(
        app_env = %std::env::var("APP_ENV").unwrap_or_else(|_| "development".into()),
        version = env!("CARGO_PKG_VERSION"),
        "Starting {}", env!("CARGO_PKG_NAME")
    );

    let state = bootstrap::build_state(Arc::clone(&config)).await?;

    // Start the background update checker if the updater is enabled and
    // check_interval_seconds > 0. This is a no-op when updater.enabled = false.
    bootstrap::updater::start_update_checker(Arc::clone(&state));

    // Optionally run the background worker in the same process.
    // Set `worker.enabled = true` in config to activate.
    // For production, prefer running `cargo run --bin worker` as a separate process.
    if config.worker.enabled {
        tracing::info!("Inline worker enabled — starting alongside HTTP server");
        let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);
        let worker_state = Arc::clone(&state);

        tokio::spawn(async move {
            if let Err(e) = bootstrap::worker::run(worker_state, shutdown_rx).await {
                tracing::error!(error = %e, "Worker task exited with error");
            }
        });

        // Shut down worker when the process receives a ctrl-c.
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            let _ = shutdown_tx.send(());
        });
    }

    bootstrap::server::run(state, &config.server).await?;

    Ok(())
}

