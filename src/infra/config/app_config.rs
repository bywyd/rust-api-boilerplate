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

impl AppConfig {
    /// Load configuration in priority order (lowest → highest):
    /// 1. `config/default.yaml`
    /// 2. `config/{APP_ENV}.yaml` (optional)
    /// 3. Environment variables (separator `__`, e.g. `DATABASE__URL`)
    pub fn load() -> Result<Self> {
        let env = env::var("APP_ENV").unwrap_or_else(|_| "development".to_string());

        let config = Config::builder()
            .add_source(File::with_name("config/default"))
            .add_source(File::with_name(&format!("config/{}", env)).required(false))
            .add_source(Environment::default().separator("__"))
            .build()?;

        Ok(config.try_deserialize()?)
    }
}
