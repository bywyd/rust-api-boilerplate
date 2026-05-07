use crate::infra::config::app_config::LocalCacheConfig;
use moka::future::Cache;
use std::time::Duration;

/// In-process L1 cache — key/value strings, backed by Moka.
/// Values are JSON-serialized domain objects.
pub type LocalCache = Cache<String, String>;

pub fn build_local_cache(cfg: &LocalCacheConfig) -> LocalCache {
    Cache::builder()
        .max_capacity(cfg.max_capacity)
        .time_to_live(Duration::from_secs(cfg.ttl_seconds))
        .build()
}
