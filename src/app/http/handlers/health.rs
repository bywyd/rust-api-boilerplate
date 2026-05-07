use actix_web::HttpResponse;
use chrono::Utc;
use serde::Serialize;

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    timestamp: chrono::DateTime<chrono::Utc>,
}

pub async fn health_check() -> HttpResponse {
    HttpResponse::Ok().json(HealthResponse {
        status: "ok",
        timestamp: Utc::now(),
    })
}
