use crate::app::ws::policy::AppWsPolicy;
use crate::infra::cache::redis::RedisPool;
use crate::infra::config::app_config::AppConfig;
use crate::infra::ws::cluster;
use crate::infra::ws::hub::WsHub;
use crate::infra::ws::policy::WsPolicy;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Depth of the queue feeding the Redis relay. Full means Redis cannot keep up;
/// events are then still delivered locally, with a warning.
const CLUSTER_RELAY_BUFFER: usize = 1024;

/// Build the WebSocket hub and, when configured, start the Redis bridge that
/// mirrors published events across instances.
///
/// The bridge needs the hub (to deliver inbound frames) and the hub needs the
/// bridge (to relay outbound ones), so the channel is created first and its two
/// halves are handed out separately.
///
/// Clustering silently stays off when `cache.redis.enabled = false` — the bridge
/// reuses the cache pool and has nowhere to publish otherwise.
pub fn init(config: &AppConfig, redis: Option<&RedisPool>) -> Arc<WsHub> {
    let ws_cfg = &config.websocket;

    let cluster_enabled = ws_cfg.enabled && ws_cfg.cluster.enabled;

    if cluster_enabled && redis.is_none() {
        tracing::warn!(
            "websocket.cluster.enabled = true but Redis is unavailable — \
             websocket events will not reach other instances"
        );
    }

    let (cluster_tx, cluster_rx) = match (cluster_enabled, redis) {
        (true, Some(_)) => {
            let (tx, rx) = mpsc::channel(CLUSTER_RELAY_BUFFER);
            (Some(tx), Some(rx))
        }
        _ => (None, None),
    };

    let hub = Arc::new(WsHub::new(ws_cfg.clone(), cluster_tx));

    if let (Some(rx), Some(pool)) = (cluster_rx, redis) {
        cluster::start(
            Arc::clone(&hub),
            rx,
            pool.clone(),
            config.cache.redis.url.clone(),
            ws_cfg.cluster.channel.clone(),
        );
    }

    if ws_cfg.enabled {
        tracing::info!(
            node_id = %hub.node_id(),
            require_auth = ws_cfg.require_auth,
            cluster = hub.stats().cluster_enabled,
            max_connections = ws_cfg.max_connections,
            "Websocket hub initialised"
        );
    } else {
        tracing::debug!("Websocket endpoint disabled (websocket.enabled = false)");
    }

    hub
}

/// Build the application's WebSocket authorisation policy.
///
/// Kept separate from the hub so the policy can depend on infrastructure (an ORM
/// connection, a cache) without the hub depending on the application. It must
/// never hold `AppState`, which owns the hub — that would be a reference cycle.
pub fn build_policy(_config: &AppConfig) -> Arc<dyn WsPolicy> {
    Arc::new(AppWsPolicy::new())
}
