use crate::infra::queue::error::QueueError;
use crate::infra::queue::job::{Job, JobContext};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Example cron-scheduled job.
///
/// Registered in [`build_schedule`](crate::app::jobs::schedule::build_schedule)
/// as a template for periodic work (cache warming, cleanup, digests, …). It
/// simply logs the time it was scheduled. Replace the body of [`execute`] with
/// your own periodic logic, or copy this file as a starting point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatJob {
    /// Identifies this struct to the [`JobRegistry`]. Must match `job_type()`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// When the scheduler produced this run.
    pub scheduled_at: DateTime<Utc>,
}

impl HeartbeatJob {
    pub fn new() -> Self {
        Self {
            r#type: Self::job_type().to_string(),
            scheduled_at: Utc::now(),
        }
    }
}

impl Default for HeartbeatJob {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Job for HeartbeatJob {
    fn job_type() -> &'static str {
        "heartbeat"
    }

    async fn execute(&self, _ctx: &JobContext) -> Result<(), QueueError> {
        tracing::info!(
            scheduled_at = %self.scheduled_at,
            "Heartbeat job ran"
        );
        Ok(())
    }
}
