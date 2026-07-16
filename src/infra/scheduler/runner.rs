use crate::infra::queue::dispatcher::Dispatcher;
use crate::infra::scheduler::registry::CronEntry;
use chrono::{DateTime, Utc};
use tokio::sync::broadcast;

/// Runtime wrapper around a [`CronEntry`] that tracks its next fire time.
struct ScheduledJob {
    entry: CronEntry,
    next_run: DateTime<Utc>,
}

/// Drives a set of cron-scheduled jobs, dispatching each onto the job queue
/// when its schedule is due.
///
/// The loop sleeps until the soonest upcoming fire time rather than polling on
/// a fixed tick, so it stays idle between jobs.
pub struct Scheduler {
    jobs: Vec<ScheduledJob>,
    dispatcher: Dispatcher,
}

impl Scheduler {
    /// Build a scheduler from registered cron entries. Entries whose schedule
    /// has no upcoming occurrence are skipped with a warning.
    pub fn new(entries: Vec<CronEntry>, dispatcher: Dispatcher) -> Self {
        let now = Utc::now();
        let jobs = entries
            .into_iter()
            .filter_map(|entry| match entry.schedule.after(&now).next() {
                Some(next_run) => {
                    tracing::info!(
                        job = %entry.name,
                        cron = %entry.expr,
                        next_run = %next_run,
                        "Registered scheduled job"
                    );
                    Some(ScheduledJob { entry, next_run })
                }
                None => {
                    tracing::warn!(
                        job = %entry.name,
                        cron = %entry.expr,
                        "Cron schedule has no upcoming occurrences — skipping"
                    );
                    None
                }
            })
            .collect();

        Self { jobs, dispatcher }
    }

    /// Run the scheduler loop until the shutdown signal is received.
    pub async fn run(mut self, mut shutdown: broadcast::Receiver<()>) {
        if self.jobs.is_empty() {
            tracing::info!("Scheduler has no jobs registered — idle until shutdown");
            let _ = shutdown.recv().await;
            return;
        }

        tracing::info!(jobs = self.jobs.len(), "Scheduler started");

        loop {
            // Sleep until the soonest scheduled job (or a bounded max so we
            // periodically re-evaluate even if all fire times are far away).
            let now = Utc::now();
            let next = self
                .jobs
                .iter()
                .map(|j| j.next_run)
                .min()
                .unwrap_or_else(|| now + chrono::Duration::hours(1));

            let wait = (next - now)
                .to_std()
                .unwrap_or(std::time::Duration::ZERO)
                .min(std::time::Duration::from_secs(3600));

            tokio::select! {
                _ = shutdown.recv() => {
                    tracing::info!("Scheduler received shutdown signal");
                    return;
                }
                _ = tokio::time::sleep(wait) => {}
            }

            let now = Utc::now();
            for job in &mut self.jobs {
                if job.next_run > now {
                    continue;
                }

                self.dispatcher
                    .dispatch_and_log(&job.entry)
                    .await;

                // Advance to the next occurrence strictly after now so we don't
                // re-fire the same slot on a subsequent tight loop.
                match job.entry.schedule.after(&now).next() {
                    Some(next_run) => job.next_run = next_run,
                    None => {
                        tracing::warn!(
                            job = %job.entry.name,
                            "No further occurrences — this job will not fire again"
                        );
                        // Push far into the future so it's never selected again.
                        job.next_run = now + chrono::Duration::weeks(52 * 100);
                    }
                }
            }
        }
    }
}

impl Dispatcher {
    /// Build the entry's payload and enqueue it, logging the outcome.
    /// A failure to build or enqueue one job never aborts the scheduler loop.
    async fn dispatch_and_log(&self, entry: &CronEntry) {
        let payload = match (entry.build_payload)() {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(
                    job = %entry.name,
                    error = %e,
                    "Failed to build scheduled job payload — skipping this run"
                );
                return;
            }
        };

        match self.dispatch_value(payload).await {
            Ok(()) => tracing::info!(
                job = %entry.name,
                cron = %entry.expr,
                "Scheduled job dispatched to queue"
            ),
            Err(e) => tracing::error!(
                job = %entry.name,
                error = %e,
                "Failed to dispatch scheduled job"
            ),
        }
    }
}
