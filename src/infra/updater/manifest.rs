use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

fn default_binary_name() -> String {
    if cfg!(windows) {
        "api.exe".to_string()
    } else {
        "api".to_string()
    }
}

/// Remote version manifest served at `updater.check_url`.
///
/// Example JSON:
/// ```json
/// {
///   "version": "1.2.0",
///   "release_notes": "Bug fixes and performance improvements.",
///   "published_at": "2026-05-07T00:00:00Z",
///   "download_url": "https://cdn.example.com/releases/v1.2.0/api.zip",
///   "checksum_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
///   "binary_name": "api"
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionManifest {
    /// Semver version string, e.g. `"1.2.0"`.
    pub version: String,
    /// Human-readable changelog for this release.
    pub release_notes: Option<String>,
    /// UTC timestamp when this version was published.
    pub published_at: Option<DateTime<Utc>>,
    /// HTTPS URL of the zip archive containing the new binary.
    pub download_url: String,
    /// Lowercase hex SHA-256 of the zip archive at `download_url`.
    pub checksum_sha256: String,
    /// Filename of the target binary inside the zip archive.
    /// Defaults to `"api"` on Unix, `"api.exe"` on Windows.
    #[serde(default = "default_binary_name")]
    pub binary_name: String,
}
