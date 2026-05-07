use crate::infra::config::app_config::LoggingConfig;
use anyhow::Result;
use tracing_subscriber::{filter::EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Initialise the global tracing subscriber.
///
/// Returns an optional `WorkerGuard` that must be held for the lifetime of the
/// process. Dropping it flushes any pending log entries to the file writer.
pub fn init(
    cfg: &LoggingConfig,
) -> Result<Option<tracing_appender::non_blocking::WorkerGuard>> {
    let env_filter = EnvFilter::try_new(&cfg.level)
        .unwrap_or_else(|_| EnvFilter::new("info"));

    if cfg.file_enabled {
        std::fs::create_dir_all(&cfg.file_path)?;
        let file_appender = tracing_appender::rolling::daily(&cfg.file_path, "app.log");
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

        if cfg.format == "json" {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer().json())
                .with(fmt::layer().json().with_writer(non_blocking))
                .try_init()
                .map_err(|e| anyhow::anyhow!("Logger init failed: {}", e))?;
        } else {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer().pretty())
                .with(fmt::layer().with_writer(non_blocking))
                .try_init()
                .map_err(|e| anyhow::anyhow!("Logger init failed: {}", e))?;
        }

        Ok(Some(guard))
    } else if cfg.format == "json" {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt::layer().json())
            .try_init()
            .map_err(|e| anyhow::anyhow!("Logger init failed: {}", e))?;
        Ok(None)
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt::layer().pretty())
            .try_init()
            .map_err(|e| anyhow::anyhow!("Logger init failed: {}", e))?;
        Ok(None)
    }
}
