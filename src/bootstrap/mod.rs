pub mod cache;
pub mod database;
pub mod logger;
pub mod scheduler;
pub mod server;
pub mod updater;
pub mod worker;
pub mod ws;

use crate::app::state::AppState;
use crate::infra::config::app_config::AppConfig;
use crate::infra::http_client::client::HttpClient;
use crate::infra::rate_limit::registry::RateLimitRegistry;
use crate::infra::updater::{UpdateStatus, UpdaterService};
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::RwLock;

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

    tracing::info!("Initialising updater…");
    let update_status = Arc::new(RwLock::new(UpdateStatus::new(
        config.updater.current_version.clone(),
    )));
    let updater_service = if config.updater.enabled {
        tracing::info!("Updater enabled (auto_update={})", config.updater.auto_update);
        let svc = UpdaterService::new(Arc::new(config.updater.clone()))?;
        Some(Arc::new(svc))
    } else {
        tracing::debug!("Updater disabled");
        None
    };

    tracing::info!("Initialising email client…");
    let email = if config.email.enabled {
        tracing::info!(
            smtp_host = %config.email.smtp_host,
            smtp_port = config.email.smtp_port,
            tls_mode = %config.email.tls_mode,
            "Email client enabled"
        );
        let client = crate::infra::email::client::EmailClient::new(&config.email)?;
        Some(Arc::new(client))
    } else {
        tracing::debug!("Email client disabled (email.enabled = false)");
        None
    };

    tracing::info!("Initialising rate limit registry…");
    let rate_limit = Arc::new(RateLimitRegistry::from_config(&config.rate_limit));

    tracing::info!("Initialising websocket hub…");
    let ws = ws::init(&config, redis.as_ref());
    let ws_policy = ws::build_policy(&config);

    Ok(Arc::new(AppState {
        db,
        orm,
        cache,
        redis,
        http_client,
        config,
        dispatcher,
        queue_backend,
        update_status,
        updater_service,
        email,
        rate_limit,
        ws,
        ws_policy,
    }))
}
