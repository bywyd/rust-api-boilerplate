use crate::infra::cache::local::LocalCache;
use crate::infra::cache::redis::RedisPool;
use crate::infra::config::app_config::AppConfig;
use crate::infra::db::pool::{DbConnection, DbPool};
use crate::infra::email::client::EmailClient;
use crate::infra::http_client::client::HttpClient;
use crate::infra::queue::backend::QueueBackend;
use crate::infra::queue::dispatcher::Dispatcher;
use crate::infra::rate_limit::registry::RateLimitRegistry;
use crate::infra::updater::{UpdateStatus, UpdaterService};
use crate::infra::ws::policy::WsPolicy;
use crate::infra::ws::WsHub;
use std::sync::Arc;
use tokio::sync::RwLock;

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
    /// Cached update state. Always present; reflects whether updater is enabled.
    pub update_status: Arc<RwLock<UpdateStatus>>,
    /// The update service. `None` when `updater.enabled = false`.
    pub updater_service: Option<Arc<UpdaterService>>,
    /// SMTP email client. `None` when `email.enabled = false`.
    pub email: Option<Arc<EmailClient>>,
    /// Named rate-limit rules. Shared across all actix workers.
    pub rate_limit: Arc<RateLimitRegistry>,
    /// WebSocket connection hub. Publish to connected clients from anywhere:
    /// `state.ws.publish("topic", &payload)?`.
    pub ws: Arc<WsHub>,
    /// Authorisation rules applied to websocket subscriptions and publishes.
    pub ws_policy: Arc<dyn WsPolicy>,
}
