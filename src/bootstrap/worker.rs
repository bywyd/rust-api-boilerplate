use crate::app::jobs::registry::build_registry;
use crate::app::state::AppState;
use crate::infra::config::app_config::QueueConfig;
use crate::infra::db::pool::DbPool;
use crate::infra::queue::backend::QueueBackend;
use crate::infra::queue::backends::channel::ChannelBackend;
use crate::infra::queue::backends::database::DatabaseBackend;
use crate::infra::queue::dispatcher::Dispatcher;
use crate::infra::queue::job::JobContext;
use crate::infra::queue::worker::Worker;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::broadcast;

const CHANNEL_CAPACITY: usize = 1_024;

/// Build the queue backend specified in `QueueConfig`.
pub fn build_backend(cfg: &QueueConfig, pool: DbPool) -> Arc<dyn QueueBackend> {
    match cfg.backend.as_str() {
        "database" => {
            tracing::info!("Queue backend: database (MySQL)");
            Arc::new(DatabaseBackend::new(pool))
        }
        _ => {
            if cfg.backend != "channel" {
                tracing::warn!(
                    backend = %cfg.backend,
                    "Unknown queue backend — falling back to in-memory channel"
                );
            } else {
                tracing::info!("Queue backend: in-memory channel");
            }
            Arc::new(ChannelBackend::new(CHANNEL_CAPACITY))
        }
    }
}

/// Build a [`Dispatcher`] from an already-constructed backend.
pub fn build_dispatcher(backend: Arc<dyn QueueBackend>, cfg: &QueueConfig) -> Dispatcher {
    Dispatcher::new(backend, cfg.max_retries)
}

/// Run the background worker loop.
///
/// Intended to be called either from `main.rs` (inline mode) or from
/// `bin/worker.rs` (standalone process mode).
pub async fn run(state: Arc<AppState>, shutdown: broadcast::Receiver<()>) -> Result<()> {
    let ctx = JobContext {
        db: state.db.clone(),
        orm: state.orm.clone(),
        cache: state.cache.clone(),
        redis: state.redis.clone(),
        http_client: state.http_client.clone(),
        config: Arc::clone(&state.config),
        email: state.email.clone(),
    };

    let registry = build_registry();
    let worker_cfg = &state.config.worker;
    let queue_cfg = &state.config.queue;

    let worker = Worker::new(
        Arc::clone(&state.queue_backend),
        registry,
        ctx,
        worker_cfg.concurrency,
        worker_cfg.poll_interval_ms,
        queue_cfg.retry_delay_seconds,
    );

    worker.run(shutdown).await;
    Ok(())
}
