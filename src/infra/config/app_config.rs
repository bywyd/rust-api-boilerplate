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

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    /// Tracing filter string, e.g. "debug" or "myapp=debug,tower_http=info"
    pub level: String,
    /// "pretty" for human-readable, "json" for structured output
    pub format: String,
    pub file_enabled: bool,
    pub file_path: String,
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
