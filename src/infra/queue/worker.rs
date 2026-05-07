use crate::infra::queue::backend::QueueBackend;
use crate::infra::queue::error::QueueError;
use crate::infra::queue::job::JobContext;
use crate::infra::queue::registry::JobRegistry;
use chrono::Utc;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::sync::Semaphore;

/// Background worker that polls a [`QueueBackend`] and dispatches jobs to
/// the registered [`JobRegistry`] handlers.
pub struct Worker {
    backend: Arc<dyn QueueBackend>,
    registry: Arc<JobRegistry>,
    ctx: Arc<JobContext>,
    concurrency: usize,
    poll_interval: Duration,
    retry_delay_seconds: u64,
}

impl Worker {
    pub fn new(
        backend: Arc<dyn QueueBackend>,
        registry: JobRegistry,
        ctx: JobContext,
        concurrency: usize,
        poll_interval_ms: u64,
        retry_delay_seconds: u64,
    ) -> Self {
        Self {
            backend,
            registry: Arc::new(registry),
            ctx: Arc::new(ctx),
            concurrency,
            poll_interval: Duration::from_millis(poll_interval_ms),
            retry_delay_seconds,
        }
    }

    /// Run the worker loop until the shutdown signal is received.
    pub async fn run(self, mut shutdown: broadcast::Receiver<()>) {
        let semaphore = Arc::new(Semaphore::new(self.concurrency));

        tracing::info!(
            concurrency = self.concurrency,
            poll_interval_ms = self.poll_interval.as_millis(),
            "Worker started"
        );

        loop {
            // Honour shutdown signal.
            if shutdown.try_recv().is_ok() {
                tracing::info!("Worker received shutdown signal, draining in-flight jobs…");
                // Wait until all permits are back (all in-flight tasks finished).
                let _ = semaphore.acquire_many(self.concurrency as u32).await;
                tracing::info!("Worker shutdown complete");
                return;
            }

            // Acquire a concurrency slot (non-blocking check first).
            let permit = match semaphore.clone().try_acquire_owned() {
                Ok(p) => p,
                Err(_) => {
                    // All slots busy — wait briefly before retrying.
                    tokio::time::sleep(self.poll_interval).await;
                    continue;
                }
            };

            match self.backend.dequeue().await {
                Ok(Some(envelope)) => {
                    let backend = Arc::clone(&self.backend);
                    let registry = Arc::clone(&self.registry);
                    let ctx = Arc::clone(&self.ctx);
                    let retry_delay = self.retry_delay_seconds;

                    tokio::spawn(async move {
                        let _permit = permit; // dropped when the task finishes
                        let id = envelope.id;
                        let job_type = envelope.job_type.clone();
                        let attempts = envelope.attempts;
                        let max_attempts = envelope.max_attempts;

                        tracing::info!(
                            job_id = %id,
                            job_type,
                            attempts,
                            max_attempts,
                            "Executing job"
                        );

                        let result = match registry.get(&job_type) {
                            Some(handler) => {
                                handler(envelope.payload.clone(), ctx).await
                            }
                            None => Err(QueueError::UnknownJobType(job_type.clone())),
                        };

                        match result {
                            Ok(()) => {
                                tracing::info!(job_id = %id, job_type, "Job completed");
                                if let Err(e) = backend.acknowledge(id).await {
                                    tracing::error!(
                                        job_id = %id,
                                        error = %e,
                                        "Failed to acknowledge job"
                                    );
                                }
                            }
                            Err(e) => {
                                let should_retry = attempts < max_attempts;
                                let retry_at = if should_retry {
                                    // Exponential back-off: delay * 2^(attempt-1), capped at 1 hour.
                                    let delay = retry_delay
                                        .saturating_mul(2u64.saturating_pow(attempts.saturating_sub(1)));
                                    let delay = delay.min(3600);
                                    tracing::warn!(
                                        job_id = %id,
                                        job_type,
                                        attempt = attempts,
                                        retry_delay_seconds = delay,
                                        error = %e,
                                        "Job failed, will retry"
                                    );
                                    Some(Utc::now() + chrono::Duration::seconds(delay as i64))
                                } else {
                                    tracing::error!(
                                        job_id = %id,
                                        job_type,
                                        attempts,
                                        error = %e,
                                        "Job permanently failed"
                                    );
                                    None
                                };

                                if let Err(ack_err) =
                                    backend.fail(id, &e.to_string(), retry_at).await
                                {
                                    tracing::error!(
                                        job_id = %id,
                                        error = %ack_err,
                                        "Failed to update job failure state"
                                    );
                                }
                            }
                        }
                    });
                }
                Ok(None) => {
                    // No jobs available — release permit and wait.
                    drop(permit);
                    tokio::time::sleep(self.poll_interval).await;
                }
                Err(e) => {
                    drop(permit);
                    tracing::error!(error = %e, "Error polling queue backend");
                    tokio::time::sleep(self.poll_interval).await;
                }
            }
        }
    }
}
