pub mod manifest;

use crate::app::error::AppError;
use crate::infra::config::app_config::{RestartMode, UpdaterConfig};
use chrono::{DateTime, Utc};
use manifest::VersionManifest;
use semver::Version;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

// Status

/// Cached update state stored in `AppState` and returned by the status endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateStatus {
    /// The version currently running (from config).
    pub current_version: String,
    /// The latest version seen on the remote manifest, if any check has succeeded.
    pub latest_version: Option<String>,
    /// Whether a newer version is available.
    pub update_available: bool,
    /// When the manifest was last successfully fetched.
    pub last_checked_at: Option<DateTime<Utc>>,
    /// Release notes from the latest manifest, if available.
    pub release_notes: Option<String>,
    /// `true` while a download / update is in progress.
    pub in_progress: bool,
    /// The full manifest for the pending update. Not serialised — used internally
    /// by the apply endpoint and the background auto-updater.
    #[serde(skip)]
    pub pending_manifest: Option<VersionManifest>,
}

impl UpdateStatus {
    pub fn new(current_version: String) -> Self {
        Self {
            current_version,
            latest_version: None,
            update_available: false,
            last_checked_at: None,
            release_notes: None,
            in_progress: false,
            pending_manifest: None,
        }
    }
}

// Service─

pub struct UpdaterService {
    config: Arc<UpdaterConfig>,
    http_client: reqwest::Client,
}

impl UpdaterService {
    pub fn new(config: Arc<UpdaterConfig>) -> anyhow::Result<Self> {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build updater HTTP client: {}", e))?;
        Ok(Self { config, http_client })
    }

    // Check─

    /// Fetch the remote manifest and return it if a newer version is available.
    /// Returns `None` when the running version is already up to date.
    pub async fn check_for_update(&self) -> Result<Option<VersionManifest>, AppError> {
        if self.config.check_url.is_empty() {
            return Err(AppError::Internal(
                "updater.check_url is not configured".into(),
            ));
        }

        let manifest: VersionManifest = self
            .http_client
            .get(&self.config.check_url)
            .send()
            .await
            .map_err(|e| {
                AppError::Internal(format!("Failed to fetch update manifest: {}", e))
            })?
            .json()
            .await
            .map_err(|e| {
                AppError::Internal(format!("Failed to parse update manifest: {}", e))
            })?;

        let current = Version::parse(&self.config.current_version).map_err(|e| {
            AppError::Internal(format!("Invalid current_version in config: {}", e))
        })?;
        let remote = Version::parse(&manifest.version).map_err(|e| {
            AppError::Internal(format!("Invalid version in remote manifest: {}", e))
        })?;

        if remote > current {
            Ok(Some(manifest))
        } else {
            Ok(None)
        }
    }

    // Download & verify─

    /// Download the zip archive from `manifest.download_url`, verify its SHA-256
    /// checksum and write it to a temporary file. Returns the path to that file.
    pub async fn download_and_verify(
        &self,
        manifest: &VersionManifest,
    ) -> Result<PathBuf, AppError> {
        tracing::info!(
            version = %manifest.version,
            url = %manifest.download_url,
            "Downloading update"
        );

        let bytes = self
            .http_client
            .get(&manifest.download_url)
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("Update download failed: {}", e)))?
            .bytes()
            .await
            .map_err(|e| {
                AppError::Internal(format!("Failed to read update download body: {}", e))
            })?;

        // Verify checksum before writing to disk.
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let computed: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect();

        if computed != manifest.checksum_sha256.to_lowercase() {
            return Err(AppError::Internal(format!(
                "Checksum mismatch for update zip: expected {}, got {}",
                manifest.checksum_sha256, computed
            )));
        }

        let tmp_path = std::env::temp_dir()
            .join(format!("api-update-{}.zip", manifest.version));

        tokio::fs::write(&tmp_path, &bytes).await.map_err(|e| {
            AppError::Internal(format!("Failed to write update archive to disk: {}", e))
        })?;

        tracing::info!(path = %tmp_path.display(), "Update archive verified and saved");
        Ok(tmp_path)
    }

    // Spawn updater─

    /// Spawn the standalone `updater` binary.
    ///
    /// The updater is expected to live next to the running executable. It runs as
    /// an independent process: it waits for this process to exit, replaces the
    /// binary, then optionally re-spawns the service.
    pub async fn spawn_updater(
        &self,
        zip_path: &PathBuf,
        manifest: &VersionManifest,
    ) -> Result<(), AppError> {
        let current_exe = std::env::current_exe().map_err(|e| {
            AppError::Internal(format!("Cannot resolve current executable path: {}", e))
        })?;

        let exe_dir = current_exe.parent().ok_or_else(|| {
            AppError::Internal("Cannot resolve executable directory".into())
        })?;

        let updater_name = if cfg!(windows) { "updater.exe" } else { "updater" };
        let updater_path = exe_dir.join(updater_name);

        if !updater_path.exists() {
            return Err(AppError::Internal(format!(
                "Updater binary not found at: {}",
                updater_path.display()
            )));
        }

        let pid = std::process::id().to_string();
        let restart_mode = match self.config.restart_mode {
            RestartMode::Spawn => "spawn",
            RestartMode::Supervisor => "supervisor",
        };

        std::process::Command::new(&updater_path)
            .arg("--zip-path")
            .arg(zip_path)
            .arg("--target-path")
            .arg(&current_exe)
            .arg("--binary-name")
            .arg(&manifest.binary_name)
            .arg("--pid")
            .arg(&pid)
            .arg("--restart-mode")
            .arg(restart_mode)
            .spawn()
            .map_err(|e| AppError::Internal(format!("Failed to spawn updater process: {}", e)))?;

        tracing::info!(pid = %pid, restart_mode, "Updater process spawned");
        Ok(())
    }

    // Trigger apply (fire-and-forget)─

    /// Download, verify and spawn the updater in a background tokio task.
    ///
    /// `status_lock` is used to reset `in_progress` if the operation fails.
    /// On success the external updater process takes over — `in_progress` is left
    /// `true` because the service is expected to be replaced.
    pub fn trigger_apply(
        self: Arc<Self>,
        manifest: VersionManifest,
        status_lock: Arc<RwLock<UpdateStatus>>,
    ) {
        tokio::spawn(async move {
            match self.download_and_verify(&manifest).await {
                Ok(zip_path) => match self.spawn_updater(&zip_path, &manifest).await {
                    Ok(()) => {
                        tracing::info!(
                            version = %manifest.version,
                            "Updater spawned — service will restart momentarily"
                        );
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to spawn updater");
                        let mut s = status_lock.write().await;
                        s.in_progress = false;
                    }
                },
                Err(e) => {
                    tracing::error!(error = %e, "Failed to download/verify update");
                    let mut s = status_lock.write().await;
                    s.in_progress = false;
                }
            }
        });
    }
}
