use crate::app::error::AppError;
use crate::app::http::middleware::auth::AuthUser;
use crate::app::state::AppState;
use actix_web::{web, HttpResponse};
use chrono::Utc;
use serde::Serialize;
use std::sync::Arc;

#[derive(Serialize)]
struct ErrorResponse {
    error: &'static str,
    message: String,
}

fn not_enabled() -> HttpResponse {
    HttpResponse::NotImplemented().json(ErrorResponse {
        error: "NOT_IMPLEMENTED",
        message: "The self-updater is not enabled. Set updater.enabled = true in config.".into(),
    })
}

// GET /api/updates/status

/// Return the cached update status without hitting the remote manifest URL.
pub async fn get_update_status(
    state: web::Data<AppState>,
    _auth: AuthUser,
) -> Result<HttpResponse, AppError> {
    if !state.config.updater.enabled {
        return Ok(not_enabled());
    }

    let status = state.update_status.read().await;
    Ok(HttpResponse::Ok().json(&*status))
}

// GET /api/updates/check

/// Force a fresh check against the remote manifest URL and update the cached status.
pub async fn check_for_update(
    state: web::Data<AppState>,
    _auth: AuthUser,
) -> Result<HttpResponse, AppError> {
    if !state.config.updater.enabled {
        return Ok(not_enabled());
    }

    let svc = state
        .updater_service
        .as_ref()
        .ok_or_else(|| AppError::Internal("Updater service unavailable".into()))?;

    match svc.check_for_update().await? {
        Some(manifest) => {
            tracing::info!(
                version = %manifest.version,
                "Update available (manual check)"
            );
            let mut status = state.update_status.write().await;
            status.latest_version = Some(manifest.version.clone());
            status.update_available = true;
            status.last_checked_at = Some(Utc::now());
            status.release_notes = manifest.release_notes.clone();
            status.pending_manifest = Some(manifest);
            Ok(HttpResponse::Ok().json(&*status))
        }
        None => {
            let mut status = state.update_status.write().await;
            status.last_checked_at = Some(Utc::now());
            Ok(HttpResponse::Ok().json(&*status))
        }
    }
}

// POST /api/updates/apply

#[derive(Serialize)]
struct ApplyResponse {
    message: &'static str,
    version: String,
}

/// Initiate a download and apply of the pending update.
///
/// Returns:
/// - **202 Accepted** — update download started in the background.
/// - **400 Bad Request** — no update is available.
/// - **409 Conflict** — an update is already in progress.
/// - **501 Not Implemented** — updater is disabled in config.
pub async fn apply_update(
    state: web::Data<AppState>,
    _auth: AuthUser,
) -> Result<HttpResponse, AppError> {
    if !state.config.updater.enabled {
        return Ok(not_enabled());
    }

    let svc = state
        .updater_service
        .as_ref()
        .ok_or_else(|| AppError::Internal("Updater service unavailable".into()))?;

    // Read-check before acquiring write lock.
    let manifest = {
        let status = state.update_status.read().await;

        if status.in_progress {
            return Ok(HttpResponse::Conflict().json(ErrorResponse {
                error: "CONFLICT",
                message: "An update is already in progress.".into(),
            }));
        }

        if !status.update_available {
            return Ok(HttpResponse::BadRequest().json(ErrorResponse {
                error: "NO_UPDATE",
                message: "No update is currently available. Run GET /api/updates/check first."
                    .into(),
            }));
        }

        status
            .pending_manifest
            .clone()
            .ok_or_else(|| AppError::Internal("Pending manifest missing from status".into()))?
    };

    let version = manifest.version.clone();

    // Mark in-progress.
    {
        let mut status = state.update_status.write().await;
        status.in_progress = true;
    }

    // Fire-and-forget: download → verify → spawn updater binary.
    Arc::clone(svc).trigger_apply(manifest, Arc::clone(&state.update_status));

    Ok(HttpResponse::Accepted().json(ApplyResponse {
        message: "Update initiated. The service will restart once the download is complete.",
        version,
    }))
}
