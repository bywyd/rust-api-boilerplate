use crate::app::jobs::email_job::EmailJob;
use crate::app::jobs::heartbeat_job::HeartbeatJob;
use crate::infra::queue::registry::JobRegistry;

/// Build a [`JobRegistry`] with all application-level jobs registered.
///
/// Add new job types here as the application grows. Cron-scheduled jobs (see
/// [`build_schedule`](crate::app::jobs::schedule::build_schedule)) must also be
/// registered here so a worker knows how to execute them.
pub fn build_registry() -> JobRegistry {
    let mut registry = JobRegistry::new();
    registry.register::<EmailJob>();
    registry.register::<HeartbeatJob>();
    registry
}
