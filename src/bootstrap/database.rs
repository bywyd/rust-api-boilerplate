use crate::infra::config::app_config::DatabaseConfig;
use crate::infra::db::pool::{build_pool, DbConnection, DbPool};
use anyhow::Result;
use sea_orm::SqlxMySqlConnector;

/// Initialise the database layer:
/// 1. Build a `MySqlPool` (SQLx).
/// 2. Run pending migrations.
/// 3. Create a SeaORM `DatabaseConnection` that shares the same pool.
pub async fn init(cfg: &DatabaseConfig) -> Result<(DbPool, DbConnection)> {
    let pool = build_pool(cfg).await?;

    tracing::info!("Running database migrations…");
    sqlx::migrate!("./migrations").run(&pool).await?;
    tracing::info!("Migrations applied successfully");

    // Share the SQLx pool with SeaORM — avoids a second connection pool.
    let orm = SqlxMySqlConnector::from_sqlx_mysql_pool(pool.clone());

    Ok((pool, orm))
}
