use crate::app::http::handlers;
use actix_web::web;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api")
            .route("/health", web::get().to(handlers::health::health_check))
            .service(
                web::scope("/auth")
                    .route("/login", web::post().to(handlers::user::login)),
            )
            .service(
                web::scope("/users")
                    .route("", web::get().to(handlers::user::list_users))
                    .route("", web::post().to(handlers::user::create_user))
                    .route("/{id}", web::get().to(handlers::user::get_user))
                    .route("/{id}", web::put().to(handlers::user::update_user))
                    .route("/{id}", web::delete().to(handlers::user::delete_user)),
            ),
    );
}
