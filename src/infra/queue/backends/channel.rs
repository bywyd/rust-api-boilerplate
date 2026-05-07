use crate::infra::queue::backend::QueueBackend;
use crate::infra::queue::error::QueueError;
use crate::infra::queue::job::{JobEnvelope, JobStatus};
use async_channel::{bounded, Receiver, Sender};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use std::sync::Arc;
use uuid::Uuid;

/// In-memory queue backend backed by a bounded async channel.
///
/// Jobs are lost when the process restarts. Intended for development and
/// testing. Does not honour `run_at` scheduling on retries — retried jobs
/// are re-enqueued immediately.
#[derive(Clone)]
pub struct ChannelBackend {
    sender: Sender<JobEnvelope>,
    receiver: Receiver<JobEnvelope>,
    /// Tracks envelopes that have been dequeued but not yet ack'd or failed.
    in_flight: Arc<DashMap<Uuid, JobEnvelope>>,
}

impl ChannelBackend {
    pub fn new(capacity: usize) -> Self {
        let (sender, receiver) = bounded(capacity);
        Self {
            sender,
            receiver,
            in_flight: Arc::new(DashMap::new()),
        }
    }
}

#[async_trait]
impl QueueBackend for ChannelBackend {
    async fn enqueue(&self, envelope: JobEnvelope) -> Result<(), QueueError> {
        self.sender
            .send(envelope)
            .await
            .map_err(|e| QueueError::Channel(e.to_string()))
    }

    async fn dequeue(&self) -> Result<Option<JobEnvelope>, QueueError> {
        match self.receiver.try_recv() {
            Ok(mut envelope) => {
                envelope.attempts += 1;
                envelope.status = JobStatus::Running;
                self.in_flight.insert(envelope.id, envelope.clone());
                Ok(Some(envelope))
            }
            Err(async_channel::TryRecvError::Empty) => Ok(None),
            Err(e) => Err(QueueError::Channel(e.to_string())),
        }
    }

    async fn acknowledge(&self, id: Uuid) -> Result<(), QueueError> {
        self.in_flight.remove(&id);
        Ok(())
    }

    async fn fail(
        &self,
        id: Uuid,
        error: &str,
        retry_at: Option<DateTime<Utc>>,
    ) -> Result<(), QueueError> {
        if let Some((_, mut envelope)) = self.in_flight.remove(&id) {
            if retry_at.is_some() && envelope.attempts < envelope.max_attempts {
                envelope.status = JobStatus::Retrying;
                envelope.error = Some(error.to_string());
                // Note: run_at scheduling is not honoured — retried immediately.
                self.sender
                    .send(envelope)
                    .await
                    .map_err(|e| QueueError::Channel(e.to_string()))?;
            } else {
                tracing::error!(
                    job_id = %id,
                    job_type = %envelope.job_type,
                    attempts = envelope.attempts,
                    error,
                    "Job permanently failed"
                );
            }
        }
        Ok(())
    }

    async fn pending_count(&self) -> Result<u64, QueueError> {
        Ok(self.receiver.len() as u64)
    }
}
