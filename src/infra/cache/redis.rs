use crate::infra::config::app_config::RedisCacheConfig;
use anyhow::Result;
use deadpool_redis::{Config, Runtime};

/// L2 distributed cache pool — deadpool-backed Redis connection pool.
pub type RedisPool = deadpool_redis::Pool;

pub fn build_redis_pool(cfg: &RedisCacheConfig) -> Result<RedisPool> {
    let mut deadpool_cfg = Config::from_url(&cfg.url);
    deadpool_cfg.pool = Some(deadpool_redis::PoolConfig {
        max_size: cfg.pool_size,
        ..Default::default()
    });
    let pool = deadpool_cfg.create_pool(Some(Runtime::Tokio1))?;
    Ok(pool)
}
