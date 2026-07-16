use crate::infra::queue::backend::QueueBackend;
use crate::infra::queue::error::QueueError;
use crate::infra::queue::job::JobEnvelope;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json;
use std::sync::Arc;

/// High-level interface for enqueueing jobs.
///
/// Clone-safe; cheap to pass around (backend is behind an `Arc`).
#[derive(Clone)]
pub struct Dispatcher {
    backend: Arc<dyn QueueBackend>,
    default_max_attempts: u32,
}

impl Dispatcher {
    pub fn new(backend: Arc<dyn QueueBackend>, default_max_attempts: u32) -> Self {
        Self {
            backend,
            default_max_attempts,
        }
    }

    /// Enqueue a job to run as soon as a worker is free.
    pub async fn dispatch<J: Serialize>(&self, payload: &J) -> Result<(), QueueError> {
        self.dispatch_with_retries(payload, self.default_max_attempts)
            .await
    }

    /// Enqueue a job to run at a specific UTC time.
    pub async fn dispatch_at<J: Serialize>(
        &self,
        payload: &J,
        run_at: DateTime<Utc>,
    ) -> Result<(), QueueError> {
        let (job_type, value) = Self::prepare(payload)?;
        let envelope = JobEnvelope::new(job_type, value, self.default_max_attempts).run_at(run_at);
        self.backend.enqueue(envelope).await
    }

    /// Enqueue a job with an explicit maximum retry count.
    pub async fn dispatch_with_retries<J: Serialize>(
        &self,
        payload: &J,
        max_attempts: u32,
    ) -> Result<(), QueueError> {
        let (job_type, value) = Self::prepare(payload)?;
        let envelope = JobEnvelope::new(job_type, value, max_attempts);
        self.backend.enqueue(envelope).await
    }

    /// Enqueue an already-serialised payload.
    ///
    /// Used by the cron scheduler, which stores type-erased payload factories.
    /// The value must be a JSON object containing a `"type"` field.
    pub async fn dispatch_value(&self, value: serde_json::Value) -> Result<(), QueueError> {
        let job_type = Self::extract_type(&value)?;
        let envelope = JobEnvelope::new(job_type, value, self.default_max_attempts);
        self.backend.enqueue(envelope).await
    }

    fn prepare<J: Serialize>(payload: &J) -> Result<(String, serde_json::Value), QueueError> {
        let value = serde_json::to_value(payload)?;
        let job_type = Self::extract_type(&value)?;
        Ok((job_type, value))
    }

    /// Pull the `"type"` discriminator out of a serialised job payload.
    fn extract_type(value: &serde_json::Value) -> Result<String, QueueError> {
        value
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                QueueError::Execution(
                    "Job payload must contain a string field named \"type\"".to_string(),
                )
            })
            .map(|s| s.to_string())
    }
}
