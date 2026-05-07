use crate::infra::cache::local::{build_local_cache, LocalCache};
use crate::infra::cache::redis::{build_redis_pool, RedisPool};
use crate::infra::config::app_config::CacheConfig;
use anyhow::Result;

/// Initialise the cache layer.
///
/// - L1 (Moka) is always created.
/// - L2 (Redis) is only created when `cfg.redis.enabled == true`.
///   If Redis is enabled but unreachable the app still starts, logging a warning.
pub async fn init(cfg: &CacheConfig) -> Result<(LocalCache, Option<RedisPool>)> {
    let local = build_local_cache(&cfg.local);
    tracing::info!(
        max_capacity = cfg.local.max_capacity,
        ttl_seconds = cfg.local.ttl_seconds,
        "L1 cache (Moka) initialised"
    );

    let redis = if cfg.redis.enabled {
        match build_redis_pool(&cfg.redis) {
            Ok(pool) => {
                tracing::info!(url = %cfg.redis.url, "L2 cache (Redis) initialised");
                Some(pool)
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "Redis unavailable — degrading to L1 cache only"
                );
                None
            }
        }
    } else {
        tracing::info!("Redis disabled, using L1 cache only");
        None
    };

    Ok((local, redis))
}
