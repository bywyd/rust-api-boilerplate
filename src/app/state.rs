use crate::infra::cache::local::LocalCache;
use crate::infra::cache::redis::RedisPool;
use crate::infra::config::app_config::AppConfig;
use crate::infra::db::pool::{DbConnection, DbPool};
use crate::infra::http_client::client::HttpClient;
use crate::infra::queue::backend::QueueBackend;
use crate::infra::queue::dispatcher::Dispatcher;
use std::sync::Arc;

/// Shared application state injected into every actix-web handler via `web::Data<AppState>`.
pub struct AppState {
    /// Raw SQLx pool — for migrations and low-level queries.
    pub db: DbPool,
    /// SeaORM connection (shares the same underlying pool as `db`).
    pub orm: DbConnection,
    /// L1 in-process cache (Moka).
    pub cache: LocalCache,
    /// L2 distributed cache (Redis). `None` when Redis is disabled or unreachable.
    #[allow(dead_code)]
    pub redis: Option<RedisPool>,
    /// Shared HTTP client for outbound requests.
    #[allow(dead_code)]
    pub http_client: HttpClient,
    /// Loaded application configuration.
    pub config: Arc<AppConfig>,
    /// High-level interface for dispatching background jobs.
    pub dispatcher: Dispatcher,
    /// Queue storage backend (channel or database).
    pub queue_backend: Arc<dyn QueueBackend>,
}
