use crate::infra::queue::error::QueueError;
use crate::infra::queue::job::JobEnvelope;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Abstraction over the queue storage mechanism.
///
/// Implement this to add new backends (e.g. RabbitMQ, SQS).
/// Both [`super::backends::channel::ChannelBackend`] and
/// [`super::backends::database::DatabaseBackend`] implement this trait.
#[async_trait]
pub trait QueueBackend: Send + Sync {
    /// Push a job envelope into the queue.
    async fn enqueue(&self, envelope: JobEnvelope) -> Result<(), QueueError>;

    /// Atomically dequeue the next due job and mark it as `running`.
    /// Returns `None` when no jobs are available.
    async fn dequeue(&self) -> Result<Option<JobEnvelope>, QueueError>;

    /// Mark a job as successfully completed and remove it from the active set.
    async fn acknowledge(&self, id: Uuid) -> Result<(), QueueError>;

    /// Mark a job as failed.
    ///
    /// - If `retry_at` is `Some`, the job is re-scheduled (`Retrying` status).
    /// - If `retry_at` is `None`, the job is permanently failed (`Failed` status).
    async fn fail(
        &self,
        id: Uuid,
        error: &str,
        retry_at: Option<DateTime<Utc>>,
    ) -> Result<(), QueueError>;

    /// Number of jobs waiting to be processed (includes `retrying` jobs whose
    /// `run_at` is in the past).
    async fn pending_count(&self) -> Result<u64, QueueError>;
}
