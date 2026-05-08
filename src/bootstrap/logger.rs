use crate::infra::config::app_config::LoggingConfig;
use anyhow::Result;
use std::path::Path;
use tracing_appender::rolling::{Rotation, RollingFileAppender};
use tracing_subscriber::{filter::EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Initialise the global tracing subscriber.
///
/// `process_name` is used as the log file prefix so each binary writes its own
/// files: `api.2026-05-07`, `worker.2026-05-07`, etc.
///
/// Returns an optional `WorkerGuard` that **must** be held for the process
/// lifetime so the non-blocking writer flushes all entries on shutdown.
pub fn init(
    cfg: &LoggingConfig,
    process_name: &str,
) -> Result<Option<tracing_appender::non_blocking::WorkerGuard>> {
    let env_filter = EnvFilter::try_new(&cfg.level)
        .unwrap_or_else(|_| EnvFilter::new("info"));

    if cfg.file_enabled {
        std::fs::create_dir_all(&cfg.file_path)?;

        let file_appender = build_appender(&cfg.file_rotation, &cfg.file_path, process_name);
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

        // Spawn the cleanup loop if retention is configured.
        if cfg.file_retention_days > 0 {
            let log_path = cfg.file_path.clone();
            let prefix = process_name.to_string();
            let retention = cfg.file_retention_days;
            tokio::spawn(async move {
                run_cleanup_loop(log_path, prefix, retention).await;
            });
        }

        if cfg.format == "json" {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer().json())                                             // stdout
                .with(fmt::layer().json().with_ansi(false).with_writer(non_blocking)) // file (no ANSI)
                .try_init()
                .map_err(|e| anyhow::anyhow!("Logger init failed: {}", e))?;
        } else {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt::layer().pretty())                                           // stdout
                .with(fmt::layer().with_ansi(false).with_writer(non_blocking))        // file (no ANSI)
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

// Rotation

fn build_appender(rotation: &str, dir: &str, prefix: &str) -> RollingFileAppender {
    let rot = match rotation {
        "hourly" => Rotation::HOURLY,
        "never"  => Rotation::NEVER,
        _        => Rotation::DAILY,
    };
    RollingFileAppender::builder()
        .rotation(rot)
        .filename_prefix(prefix)
        .filename_suffix("log")
        .build(dir)
        .expect("Failed to build log file appender")
}

// Retention / cleanup

/// Run an initial cleanup on startup, then once every 24 hours.
async fn run_cleanup_loop(log_path: String, prefix: String, retention_days: u32) {
    // Clean up immediately on start so stale files don't survive a restart.
    cleanup_old_logs(&log_path, &prefix, retention_days);

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(24 * 3600));
    interval.tick().await; // consume the immediate first tick
    loop {
        interval.tick().await;
        cleanup_old_logs(&log_path, &prefix, retention_days);
    }
}

/// Delete log files in `log_path` whose name starts with `prefix` and whose
/// last-modified time is older than `retention_days` days.
fn cleanup_old_logs(log_path: &str, prefix: &str, retention_days: u32) {
    let dir = Path::new(log_path);
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(u64::from(retention_days) * 24 * 3600);

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) => {
            tracing::warn!(
                error = %err,
                path = log_path,
                "Cannot read log directory for cleanup"
            );
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        // Only touch files belonging to this process (matched by prefix).
        if !name.starts_with(prefix) {
            continue;
        }

        let modified = match entry.metadata().and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => continue,
        };

        if modified < cutoff {
            match std::fs::remove_file(&path) {
                Ok(()) => tracing::info!(
                    file = %path.display(),
                    "Deleted old log file"
                ),
                Err(e) => tracing::warn!(
                    file = %path.display(),
                    error = %e,
                    "Failed to delete old log file"
                ),
            }
        }
    }
}
