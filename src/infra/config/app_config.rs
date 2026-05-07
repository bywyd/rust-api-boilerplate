use anyhow::Result;
use config::{Config, Environment, File};
use serde::Deserialize;
use std::env;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub cache: CacheConfig,
    pub logging: LoggingConfig,
    pub http_client: HttpClientConfig,
    pub auth: AuthConfig,
    pub cors: CorsConfig,
    #[serde(default)]
    pub worker: WorkerConfig,
    #[serde(default)]
    pub queue: QueueConfig,
    #[serde(default)]
    pub updater: UpdaterConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    /// Number of actix worker threads. 0 = auto-detect via num_cpus.
    pub workers: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub connect_timeout_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CacheConfig {
    pub local: LocalCacheConfig,
    pub redis: RedisCacheConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LocalCacheConfig {
    pub max_capacity: u64,
    pub ttl_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RedisCacheConfig {
    pub enabled: bool,
    pub url: String,
    pub pool_size: usize,
}

fn default_log_rotation() -> String {
    "daily".to_string()
}
fn default_log_retention_days() -> u32 {
    7
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    /// Tracing filter string, e.g. "debug" or "myapp=debug,tower_http=info"
    pub level: String,
    /// "pretty" for human-readable, "json" for structured output
    pub format: String,
    /// Write logs to a file in addition to stdout.
    pub file_enabled: bool,
    /// Directory where log files are written.
    pub file_path: String,
    /// Log file rotation interval: `"daily"` | `"hourly"` | `"never"`.
    #[serde(default = "default_log_rotation")]
    pub file_rotation: String,
    /// Delete log files older than this many days. `0` disables cleanup.
    #[serde(default = "default_log_retention_days")]
    pub file_retention_days: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HttpClientConfig {
    pub timeout_seconds: u64,
    pub connect_timeout_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    pub jwt_secret: String,
    pub jwt_expiry_hours: u64,
}

/// CORS policy configuration.
///
/// - `allowed_origins`: list of exact origins (e.g. `["https://app.example.com"]`).
///   An empty list means **no origin is allowed** — set to `["*"]` only for fully public APIs.
/// - `allowed_methods`: HTTP verbs to permit (e.g. `["GET", "POST", "PUT", "DELETE", "OPTIONS"]`).
/// - `allowed_headers`: request headers the client may send (e.g. `["Content-Type", "Authorization"]`).
/// - `expose_headers`: response headers the browser may read (e.g. `["X-Request-Id"]`).
/// - `max_age_seconds`: how long (in seconds) the browser should cache a preflight response.
/// - `allow_credentials`: whether cookies / Authorization headers are allowed with cross-origin requests.
#[derive(Debug, Clone, Deserialize)]
pub struct CorsConfig {
    pub allowed_origins: Vec<String>,
    pub allowed_methods: Vec<String>,
    pub allowed_headers: Vec<String>,
    pub expose_headers: Vec<String>,
    pub max_age_seconds: usize,
    pub allow_credentials: bool,
}

fn default_worker_concurrency() -> usize {
    4
}
fn default_poll_interval_ms() -> u64 {
    1000
}
fn default_queue_backend() -> String {
    "channel".to_string()
}
fn default_max_retries() -> u32 {
    3
}
fn default_retry_delay_seconds() -> u64 {
    60
}

/// Controls whether and how the background worker runs inside the main process.
/// Set `enabled = true` to run the worker inline with the API server.
/// Set `enabled = false` (default) to run the worker as a separate binary (`cargo run --bin worker`).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct WorkerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_worker_concurrency")]
    pub concurrency: usize,
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

/// Configures the job queue backend and retry behaviour.
#[derive(Debug, Clone, Deserialize)]
pub struct QueueConfig {
    /// `"channel"` — in-memory (dev/test, lost on restart).
    /// `"database"` — MySQL-persisted (production, survives restarts).
    #[serde(default = "default_queue_backend")]
    pub backend: String,
    /// Maximum number of attempts before a job is marked permanently failed.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Base delay in seconds for exponential backoff between retries.
    #[serde(default = "default_retry_delay_seconds")]
    pub retry_delay_seconds: u64,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            backend: default_queue_backend(),
            max_retries: default_max_retries(),
            retry_delay_seconds: default_retry_delay_seconds(),
        }
    }
}

fn default_updater_check_interval() -> u64 {
    3600
}
fn default_restart_mode() -> RestartMode {
    RestartMode::Supervisor
}
fn default_current_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Whether the updater should spawn the new process itself or let an external
/// supervisor (systemd, Docker, etc.) restart the service after the binary is replaced.
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RestartMode {
    Spawn,
    #[default]
    Supervisor,
}

/// Self-update configuration.
///
/// When `enabled = false` (default) the updater is completely inert — no background
/// tasks are spawned, the HTTP endpoints return a 501, and the updater binary is
/// never invoked.
///
/// `auto_update = true` means the service will download and apply a new version
/// automatically when the background checker detects one. Set this to `false` to
/// only receive notifications via the `/api/updates/status` endpoint and require an
/// authorized operator to POST `/api/updates/apply`.
#[derive(Debug, Clone, Deserialize)]
pub struct UpdaterConfig {
    /// Master switch — set to `true` to activate the updater subsystem.
    #[serde(default)]
    pub enabled: bool,
    /// URL of the remote JSON manifest (see `VersionManifest` for schema).
    #[serde(default)]
    pub check_url: String,
    /// Automatically apply updates when a newer version is detected.
    /// When `false`, only logs and exposes the new version via the status API.
    #[serde(default)]
    pub auto_update: bool,
    /// How often (in seconds) to poll `check_url` in the background.
    /// Set to `0` to disable the background checker (manual check only).
    #[serde(default = "default_updater_check_interval")]
    pub check_interval_seconds: u64,
    /// What to do after the updater binary replaces the executable.
    /// `spawn`      — updater execs the new binary directly.
    /// `supervisor` — updater exits; systemd/Docker/supervisor restarts the service.
    #[serde(default = "default_restart_mode")]
    pub restart_mode: RestartMode,
    /// The running application version. Defaults to the value from `Cargo.toml` at
    /// compile time. Override in config if you manage versioning externally.
    #[serde(default = "default_current_version")]
    pub current_version: String,
}

impl Default for UpdaterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            check_url: String::new(),
            auto_update: false,
            check_interval_seconds: default_updater_check_interval(),
            restart_mode: default_restart_mode(),
            current_version: default_current_version(),
        }
    }
}

impl AppConfig {
    /// Load configuration in priority order (lowest → highest):
    /// 1. `{config_dir}/default.yaml`  (`config_dir` = `APP_CONFIG_DIR` env var, default `"config"`)
    /// 2. `{config_dir}/{APP_ENV}.yaml` (optional)
    /// 3. Environment variables (separator `__`, e.g. `DATABASE__URL`)
    ///
    /// `APP_CONFIG_DIR` lets installed services point at `/opt/myapp/config` regardless
    /// of the process working directory.
    pub fn load() -> Result<Self> {
        let app_env = env::var("APP_ENV").unwrap_or_else(|_| "development".to_string());
        let config_dir =
            env::var("APP_CONFIG_DIR").unwrap_or_else(|_| "config".to_string());

        let config = Config::builder()
            .add_source(File::with_name(&format!("{}/default", config_dir)))
            .add_source(
                File::with_name(&format!("{}/{}", config_dir, app_env)).required(false),
            )
            .add_source(Environment::default().separator("__"))
            .build()?;

        Ok(config.try_deserialize()?)
    }
}
