use crate::app::jobs::schedule::build_schedule;
use crate::app::state::AppState;
use crate::infra::scheduler::Scheduler;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Start the cron scheduler in the background.
///
/// No-op when `scheduler.enabled = false`. When enabled, the scheduler
/// dispatches due jobs onto the shared queue; a worker (inline or standalone)
/// then executes them. The `shutdown` receiver stops the scheduler loop on
/// process termination.
///
/// Returns an error only if the cron schedule fails to build (invalid cron
/// expression), so a misconfiguration fails fast at startup.
pub fn start_scheduler(state: Arc<AppState>, shutdown: broadcast::Receiver<()>) -> Result<()> {
    if !state.config.scheduler.enabled {
        tracing::debug!("Scheduler disabled (scheduler.enabled = false)");
        return Ok(());
    }

    let registry = build_schedule()?;
    tracing::info!(jobs = registry.len(), "Scheduler enabled");

    let scheduler = Scheduler::new(registry.into_entries(), state.dispatcher.clone());
    tokio::spawn(scheduler.run(shutdown));

    Ok(())
}
