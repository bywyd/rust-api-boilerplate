use crate::infra::config::app_config::DatabaseConfig;
use anyhow::Result;
use sqlx::mysql::MySqlPoolOptions;
use std::time::Duration;

/// Type alias for the raw SQLx connection pool.
/// Used for migrations and raw queries.
pub type DbPool = sqlx::MySqlPool;

/// Type alias for the SeaORM connection (wraps the same underlying pool).
pub type DbConnection = sea_orm::DatabaseConnection;

/// Build a MySQL connection pool from config.
pub async fn build_pool(cfg: &DatabaseConfig) -> Result<DbPool> {
    let pool = MySqlPoolOptions::new()
        .max_connections(cfg.max_connections)
        .min_connections(cfg.min_connections)
        .acquire_timeout(Duration::from_secs(cfg.connect_timeout_seconds))
        .connect(&cfg.url)
        .await?;

    Ok(pool)
}
