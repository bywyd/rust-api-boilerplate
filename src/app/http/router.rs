use crate::app::http::handlers;
use crate::infra::config::app_config::WebSocketConfig;
use crate::infra::rate_limit::registry::RateLimitRegistry;
use actix_web::web;

/// Configure all application routes.
///
/// Rate limits are applied per scope using named rules from the [`RateLimitRegistry`].
/// Add new rules in `config/default.yaml` under `rate_limit.rules`, then reference
/// them by name with `.wrap(rate_limit.condition("rule-name"))`.
pub fn configure(
    cfg: &mut web::ServiceConfig,
    rate_limit: &RateLimitRegistry,
    websocket: &WebSocketConfig,
) {
    let mut api = web::scope("/api")
            .route("/health", web::get().to(handlers::health::health_check))
            .service(
                web::scope("/auth")
                    // Stricter budget: limits login brute-forcing.
                    .wrap(rate_limit.condition("auth"))
                    .route("/login", web::post().to(handlers::user::login)),
            )
            .service(
                web::scope("/users")
                    // General API budget.
                    .wrap(rate_limit.condition("default"))
                    .route("", web::get().to(handlers::user::list_users))
                    .route("", web::post().to(handlers::user::create_user))
                    .route("/{id}", web::get().to(handlers::user::get_user))
                    .route("/{id}", web::put().to(handlers::user::update_user))
                    .route("/{id}", web::delete().to(handlers::user::delete_user)),
            )
            .service(
                web::scope("/observability")
                    .route("/logs", web::get().to(handlers::observability::list_logs))
                    .route("/logs", web::delete().to(handlers::observability::truncate_logs))
                    .route("/logs/{id}", web::get().to(handlers::observability::get_log)),

            )
            .service(
                web::scope("/updates")
                    .route("/status", web::get().to(handlers::updater::get_update_status))
                    .route("/check", web::get().to(handlers::updater::check_for_update))
                    .route("/apply", web::post().to(handlers::updater::apply_update)),
            );

    // Nested inside `/api` rather than registered as its own `/api/ws` scope:
    // the first scope whose prefix matches wins, so a sibling scope under the
    // same prefix would never be reached.
    //
    // Registered only when `websocket.enabled = true`; the endpoint 404s otherwise.
    // The handshake uses the "default" budget — frames sent *on* an open socket
    // are not rate limited, so cap those with `websocket.send_buffer` and
    // `websocket.max_topics_per_connection` instead.
    if websocket.enabled {
        api = api.service(
            web::scope("/ws")
                .wrap(rate_limit.condition("default"))
                .route("", web::get().to(handlers::ws::ws_connect))
                .route("/stats", web::get().to(handlers::ws::ws_stats)),
        );
    }

    cfg.service(api);
}
