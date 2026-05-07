use crate::app::state::AppState;
use chrono::Utc;
use std::sync::Arc;
use tokio::time::{Duration, interval};

/// Start the background update-checker task.
///
/// The task is a no-op (returns immediately) when any of the following are true:
/// - `updater.enabled = false`
/// - `updater.check_interval_seconds = 0`
/// - the updater service was not initialised
pub fn start_update_checker(state: Arc<AppState>) {
    let interval_secs = state.config.updater.check_interval_seconds;

    if !state.config.updater.enabled || interval_secs == 0 {
        return;
    }

    let Some(svc) = state.updater_service.clone() else {
        return;
    };

    let auto_update = state.config.updater.auto_update;

    tokio::spawn(async move {
        // Skip the first (immediate) tick so we don't check on startup before
        // the server has fully initialised.
        let mut timer = interval(Duration::from_secs(interval_secs));
        timer.tick().await;

        loop {
            timer.tick().await;

            tracing::debug!("Running scheduled update check");

            match svc.check_for_update().await {
                Ok(Some(manifest)) => {
                    tracing::info!(
                        current = %state.config.updater.current_version,
                        latest  = %manifest.version,
                        "New version available"
                    );

                    // Store the result in the shared status.
                    {
                        let mut status = state.update_status.write().await;
                        status.latest_version = Some(manifest.version.clone());
                        status.update_available = true;
                        status.last_checked_at = Some(Utc::now());
                        status.release_notes = manifest.release_notes.clone();
                        status.pending_manifest = Some(manifest.clone());
                    }

                    // Auto-update: only trigger if not already in progress.
                    if auto_update {
                        let already_in_progress = {
                            let s = state.update_status.read().await;
                            s.in_progress
                        };

                        if !already_in_progress {
                            tracing::info!(
                                version = %manifest.version,
                                "Auto-update enabled — triggering apply"
                            );
                            {
                                let mut s = state.update_status.write().await;
                                s.in_progress = true;
                            }
                            Arc::clone(&svc)
                                .trigger_apply(manifest, Arc::clone(&state.update_status));
                        } else {
                            tracing::warn!("Auto-update skipped — update already in progress");
                        }
                    }
                }
                Ok(None) => {
                    tracing::debug!(
                        version = %state.config.updater.current_version,
                        "No update available"
                    );
                    let mut status = state.update_status.write().await;
                    status.last_checked_at = Some(Utc::now());
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Scheduled update check failed");
                }
            }
        }
    });
}
