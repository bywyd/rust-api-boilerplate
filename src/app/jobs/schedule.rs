use crate::app::jobs::heartbeat_job::HeartbeatJob;
use crate::infra::scheduler::registry::CronRegistry;

/// Build a [`CronRegistry`] with all cron-scheduled jobs registered.
///
/// Add periodic jobs here. Each entry pairs a cron expression with a factory
/// that builds a fresh job instance every time the schedule fires; the payload
/// is then dispatched onto the normal job queue and executed by a worker.
///
/// The cron format uses 7 fields with seconds (see [`CronRegistry`] docs):
/// `sec min hour day-of-month month day-of-week [year]`, evaluated in UTC.
///
/// Every scheduled job must also be registered in
/// [`build_registry`](crate::app::jobs::registry::build_registry) so a worker
/// knows how to execute it.
pub fn build_schedule() -> anyhow::Result<CronRegistry> {
    let mut cron = CronRegistry::new();

    // Example: run the heartbeat job every 5 minutes.
    cron.register::<HeartbeatJob>("0 */5 * * * *", HeartbeatJob::new)?;

    Ok(cron)
}
