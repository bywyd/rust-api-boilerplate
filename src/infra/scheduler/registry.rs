use crate::infra::queue::error::QueueError;
use crate::infra::queue::job::Job;
use anyhow::Context;
use cron::Schedule;
use serde::Serialize;
use serde_json::Value;
use std::str::FromStr;
use std::sync::Arc;

/// A type-erased factory that builds a fresh job payload each time the
/// schedule fires. Returns the serialised JSON that gets handed to the
/// [`Dispatcher`](crate::infra::queue::dispatcher::Dispatcher).
pub type PayloadFactory = Arc<dyn Fn() -> Result<Value, QueueError> + Send + Sync>;

/// A single cron entry: a parsed schedule plus the payload it dispatches.
pub struct CronEntry {
    /// The job type identifier (used for logging).
    pub name: String,
    /// The original cron expression (used for logging).
    pub expr: String,
    /// Parsed cron schedule used to compute the next fire time.
    pub schedule: Schedule,
    /// Builds the payload to enqueue when the schedule fires.
    pub build_payload: PayloadFactory,
}

/// Registry of cron-scheduled jobs.
///
/// Register every scheduled job once at startup via [`CronRegistry::register`].
/// When a schedule fires, the produced payload is dispatched onto the normal
/// job queue, so it inherits the same retry, backend and worker behaviour as
/// jobs enqueued directly.
///
/// # Cron expression format
///
/// Expressions use **7 fields** with seconds (and optional year):
///
/// ```text
/// sec  min  hour  day-of-month  month  day-of-week  [year]
/// ```
///
/// | Expression        | Meaning                        |
/// |-------------------|--------------------------------|
/// | `0 * * * * *`     | every minute (at second 0)     |
/// | `0 */5 * * * *`   | every 5 minutes                |
/// | `0 0 * * * *`     | every hour, on the hour        |
/// | `0 0 0 * * *`     | every day at midnight (UTC)    |
/// | `0 0 3 * * Mon`   | every Monday at 03:00 (UTC)    |
///
/// All times are evaluated in **UTC**.
#[derive(Default)]
pub struct CronRegistry {
    entries: Vec<CronEntry>,
}

impl CronRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a [`Job`] to run on a cron `expr`.
    ///
    /// `factory` is invoked each time the schedule fires to build a fresh job
    /// instance, so per-run state (timestamps, ids) is always current.
    ///
    /// Returns an error if `expr` is not a valid cron expression, letting the
    /// caller fail fast at startup with `?`.
    ///
    /// ```rust,ignore
    /// cron.register::<HeartbeatJob>("0 */5 * * * *", HeartbeatJob::new)?;
    /// ```
    pub fn register<J>(
        &mut self,
        expr: &str,
        factory: impl Fn() -> J + Send + Sync + 'static,
    ) -> anyhow::Result<&mut Self>
    where
        J: Job + Serialize + 'static,
    {
        let name = J::job_type().to_string();
        let schedule = Schedule::from_str(expr).with_context(|| {
            format!("invalid cron expression '{expr}' for scheduled job '{name}'")
        })?;

        let build_payload: PayloadFactory = Arc::new(move || {
            serde_json::to_value(factory()).map_err(QueueError::from)
        });

        self.entries.push(CronEntry {
            name,
            expr: expr.to_string(),
            schedule,
            build_payload,
        });
        Ok(self)
    }

    /// Consume the registry, yielding its entries for the runner.
    pub fn into_entries(self) -> Vec<CronEntry> {
        self.entries
    }

    /// Number of registered schedules.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::jobs::heartbeat_job::HeartbeatJob;
    use chrono::Utc;

    #[test]
    fn registers_valid_cron_and_builds_payload() {
        let mut cron = CronRegistry::new();
        cron.register::<HeartbeatJob>("0 */5 * * * *", HeartbeatJob::new)
            .expect("valid cron should register");

        assert_eq!(cron.len(), 1);

        let entries = cron.into_entries();
        let entry = &entries[0];
        assert_eq!(entry.name, "heartbeat");

        // The factory must yield a payload carrying the routing `type` field.
        let payload = (entry.build_payload)().expect("payload builds");
        assert_eq!(payload.get("type").and_then(|v| v.as_str()), Some("heartbeat"));

        // A "*/5 minutes" schedule always has an upcoming occurrence.
        assert!(entry.schedule.after(&Utc::now()).next().is_some());
    }

    #[test]
    fn rejects_invalid_cron() {
        let mut cron = CronRegistry::new();
        let err = cron
            .register::<HeartbeatJob>("not a cron expr", HeartbeatJob::new)
            .err()
            .expect("invalid cron should error");
        assert!(err.to_string().contains("invalid cron expression"));
    }
}
