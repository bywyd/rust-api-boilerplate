pub mod cache;
pub mod database;
pub mod logger;
pub mod server;
pub mod worker;

use crate::app::state::AppState;
use crate::infra::config::app_config::AppConfig;
use crate::infra::http_client::client::HttpClient;
use anyhow::Result;
use std::sync::Arc;

/// Orchestrate all infrastructure initialisation and return a shared `AppState`.
pub async fn build_state(config: Arc<AppConfig>) -> Result<Arc<AppState>> {
    tracing::info!("Initialising database…");
    let (db, orm) = database::init(&config.database).await?;

    tracing::info!("Initialising cache…");
    let (cache, redis) = cache::init(&config.cache).await?;

    tracing::info!("Initialising HTTP client…");
    let http_client = HttpClient::new(&config.http_client)?;

    tracing::info!("Initialising job queue…");
    let queue_backend = worker::build_backend(&config.queue, db.clone());
    let dispatcher = worker::build_dispatcher(Arc::clone(&queue_backend), &config.queue);

    Ok(Arc::new(AppState {
        db,
        orm,
        cache,
        redis,
        http_client,
        config,
        dispatcher,
        queue_backend,
    }))
}
