use crate::app::http::middleware::security_headers::SecurityHeaders;
use crate::app::http::router;
use crate::app::state::AppState;
use crate::infra::config::app_config::{CorsConfig, ServerConfig};
use actix_cors::Cors;
use actix_web::http::{header, Method};
use actix_web::{middleware, web, App, HttpServer};

/// Start the actix-web HTTP server and block until it shuts down.
pub async fn run(state: std::sync::Arc<AppState>, cfg: &ServerConfig) -> std::io::Result<()> {
    let addr = format!("{}:{}", cfg.host, cfg.port);
    let workers = if cfg.workers == 0 {
        num_cpus::get()
    } else {
        cfg.workers
    };

    tracing::info!(address = %addr, workers, "Starting HTTP server");

    // Clone config values that must be moved into the closure.
    let cors_cfg = state.config.cors.clone();

    // The rate-limit registry is already wrapped in Arc; cloning just bumps the
    // ref-count so the same underlying GovernorConfig (and its RateLimiter) is
    // shared across all actix worker threads.
    let rate_registry = std::sync::Arc::clone(&state.rate_limit);

    // `web::Data::from(Arc<T>)` avoids a double-Arc wrap.
    let state = web::Data::from(state);

    HttpServer::new(move || {
        let cors = build_cors(&cors_cfg);

        App::new()
            .app_data(state.clone())
            .wrap(cors)
            // Inject security headers on every response.
            .wrap(SecurityHeaders)
            .wrap(middleware::Logger::default())
            .configure(|cfg| router::configure(cfg, &rate_registry))
    })
    .workers(workers)
    .bind(&addr)?
    .run()
    .await
}

/// Build a [`Cors`] middleware instance from the application CORS configuration.
///
/// Rules applied:
/// - Each `allowed_origins` entry is registered as an exact origin.
/// - Methods and headers are parsed from their string representations.
/// - Credentials support is only enabled when `allow_credentials: true` **and**
///   no wildcard origin (`*`) is present — the browser forbids that combination.
fn build_cors(cfg: &CorsConfig) -> Cors {
    let has_wildcard = cfg.allowed_origins.iter().any(|o| o == "*");

    let mut cors = Cors::default();

    if has_wildcard {
        cors = cors.allow_any_origin();
    } else {
        for origin in &cfg.allowed_origins {
            cors = cors.allowed_origin(origin);
        }
    }

    // Methods
    let methods: Vec<Method> = cfg
        .allowed_methods
        .iter()
        .filter_map(|m| m.parse::<Method>().ok())
        .collect();
    cors = cors.allowed_methods(methods);

    // Request headers
    let allowed_headers: Vec<header::HeaderName> = cfg
        .allowed_headers
        .iter()
        .filter_map(|h| h.parse::<header::HeaderName>().ok())
        .collect();
    cors = cors.allowed_headers(allowed_headers);

    // Exposed response headers
    let expose_headers: Vec<header::HeaderName> = cfg
        .expose_headers
        .iter()
        .filter_map(|h| h.parse::<header::HeaderName>().ok())
        .collect();
    if !expose_headers.is_empty() {
        cors = cors.expose_headers(expose_headers);
    }

    // Preflight cache
    cors = cors.max_age(cfg.max_age_seconds);

    // Credentials — disallowed with wildcard origins per the CORS spec
    if cfg.allow_credentials && !has_wildcard {
        cors = cors.supports_credentials();
    }

    cors
}
