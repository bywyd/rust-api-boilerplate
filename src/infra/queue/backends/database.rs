use crate::infra::db::pool::DbPool;
use crate::infra::queue::backend::QueueBackend;
use crate::infra::queue::error::QueueError;
use crate::infra::queue::job::{JobEnvelope, JobStatus};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

/// MySQL-persisted queue backend.
///
/// Uses `SELECT ... FOR UPDATE SKIP LOCKED` to allow multiple worker processes
/// to safely dequeue jobs without conflicts. Jobs survive process restarts.
///
/// Uses the dynamic `sqlx::query` API (no compile-time DB connection required).
pub struct DatabaseBackend {
    pool: DbPool,
}

impl DatabaseBackend {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl QueueBackend for DatabaseBackend {
    async fn enqueue(&self, envelope: JobEnvelope) -> Result<(), QueueError> {
        let payload = serde_json::to_string(&envelope.payload)?;
        let status = envelope.status.to_string();
        let id = envelope.id.to_string();
        let run_at = envelope.run_at.naive_utc();
        let created_at = envelope.created_at.naive_utc();

        sqlx::query(
            r#"
            INSERT INTO jobs
                (id, job_type, payload, status, attempts, max_attempts, run_at, created_at, error)
            VALUES
                (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(id)
        .bind(&envelope.job_type)
        .bind(payload)
        .bind(status)
        .bind(envelope.attempts)
        .bind(envelope.max_attempts)
        .bind(run_at)
        .bind(created_at)
        .bind(&envelope.error)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn dequeue(&self) -> Result<Option<JobEnvelope>, QueueError> {
        let mut tx = self.pool.begin().await?;

        let row = sqlx::query(
            r#"
            SELECT id, job_type, payload, status, attempts, max_attempts,
                   run_at, created_at, error
            FROM jobs
            WHERE status IN ('pending', 'retrying')
              AND run_at <= NOW()
            ORDER BY run_at ASC
            LIMIT 1
            FOR UPDATE SKIP LOCKED
            "#,
        )
        .fetch_optional(&mut *tx)
        .await?;

        let row = match row {
            Some(r) => r,
            None => {
                tx.rollback().await?;
                return Ok(None);
            }
        };

        let id_str: String = row.try_get("id")?;
        let attempts: i32 = row.try_get("attempts")?;
        let max_attempts: i32 = row.try_get("max_attempts")?;
        let new_attempts = attempts + 1;

        sqlx::query("UPDATE jobs SET status = 'running', attempts = ? WHERE id = ?")
            .bind(new_attempts)
            .bind(&id_str)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        let job_type: String = row.try_get("job_type")?;
        let payload_str: String = row.try_get("payload")?;
        let error: Option<String> = row.try_get("error")?;
        let run_at_naive: NaiveDateTime = row.try_get("run_at")?;
        let created_at_naive: NaiveDateTime = row.try_get("created_at")?;

        let payload: Value = serde_json::from_str(&payload_str)?;
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| QueueError::Channel(format!("Invalid UUID in jobs table: {e}")))?;

        let envelope = JobEnvelope {
            id,
            job_type,
            payload,
            status: JobStatus::Running,
            attempts: new_attempts as u32,
            max_attempts: max_attempts as u32,
            run_at: DateTime::from_naive_utc_and_offset(run_at_naive, Utc),
            created_at: DateTime::from_naive_utc_and_offset(created_at_naive, Utc),
            error,
        };

        Ok(Some(envelope))
    }

    async fn acknowledge(&self, id: Uuid) -> Result<(), QueueError> {
        sqlx::query("UPDATE jobs SET status = 'completed' WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn fail(
        &self,
        id: Uuid,
        error: &str,
        retry_at: Option<DateTime<Utc>>,
    ) -> Result<(), QueueError> {
        let id_str = id.to_string();

        if let Some(at) = retry_at {
            sqlx::query(
                "UPDATE jobs SET status = 'retrying', error = ?, run_at = ? WHERE id = ?",
            )
            .bind(error)
            .bind(at.naive_utc())
            .bind(&id_str)
            .execute(&self.pool)
            .await?;
        } else {
            sqlx::query(
                "UPDATE jobs SET status = 'failed', error = ?, failed_at = NOW() WHERE id = ?",
            )
            .bind(error)
            .bind(&id_str)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    async fn pending_count(&self) -> Result<u64, QueueError> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) as count FROM jobs
            WHERE status IN ('pending', 'retrying')
              AND run_at <= NOW()
            "#,
        )
        .fetch_one(&self.pool)
        .await?;

        let count: i64 = row.try_get("count")?;
        Ok(count as u64)
    }
}
