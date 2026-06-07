use actix_web::{web, HttpResponse};
use serde::{Deserialize, Serialize};
use crate::app::state::AppState;
use crate::app::error::AppError;
use chrono::{DateTime, Utc};

#[derive(Deserialize)]
pub struct LogQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub method: Option<String>,
    pub status: Option<i32>,
    pub path: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ObservabilityLogSummary {
    pub id: i64,
    pub request_id: String,
    pub method: String,
    pub path: String,
    pub query_string: Option<String>,
    pub ip_address: Option<String>,
    pub response_status: i32,
    pub duration_ms: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ObservabilityLogDetail {
    pub id: i64,
    pub request_id: String,
    pub method: String,
    pub path: String,
    pub query_string: Option<String>,
    pub ip_address: Option<String>,
    pub request_headers: Option<String>,
    pub request_body: Option<String>,
    pub response_status: i32,
    pub response_headers: Option<String>,
    pub response_body: Option<String>,
    pub duration_ms: i32,
    pub created_at: DateTime<Utc>,
}

pub async fn list_logs(
    state: web::Data<AppState>,
    query: web::Query<LogQuery>,
) -> Result<HttpResponse, AppError> {
    let limit = query.limit.unwrap_or(50).min(500);
    let offset = query.offset.unwrap_or(0);

    let mut sql = "SELECT id, request_id, method, path, query_string, ip_address, response_status, duration_ms, created_at FROM http_observability_logs WHERE 1=1".to_string();
    
    if query.method.is_some() {
        sql.push_str(" AND method = ?");
    }
    if query.status.is_some() {
        sql.push_str(" AND response_status = ?");
    }
    if query.path.is_some() {
        sql.push_str(" AND path LIKE ?");
    }

    sql.push_str(" ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?");

    let mut q = sqlx::query_as::<_, ObservabilityLogSummary>(&sql);

    if let Some(ref m) = query.method {
        q = q.bind(m);
    }
    if let Some(s) = query.status {
        q = q.bind(s);
    }
    if let Some(ref p) = query.path {
        q = q.bind(format!("%{}%", p));
    }
    q = q.bind(limit).bind(offset);

    let logs = q.fetch_all(&state.db)
        .await
        .map_err(AppError::Database)?;

    Ok(HttpResponse::Ok().json(logs))
}

pub async fn get_log(
    state: web::Data<AppState>,
    path: web::Path<i64>,
) -> Result<HttpResponse, AppError> {
    let log_id = *path;
    let log = sqlx::query_as::<_, ObservabilityLogDetail>(
        "SELECT id, request_id, method, path, query_string, ip_address, request_headers, request_body, response_status, response_headers, response_body, duration_ms, created_at \
         FROM http_observability_logs WHERE id = ?"
    )
    .bind(log_id)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::Database)?;

    match log {
        Some(l) => Ok(HttpResponse::Ok().json(l)),
        None => Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": format!("Log with ID {} not found", log_id)
        }))),
    }
}

// pub async fn delete_old_logs(
//     state: web::Data<AppState>,
// ) -> Result<HttpResponse, AppError> {
//     let retention_days = state.config.observability.log_retention_days;
//     let cutoff_date = Utc::now() - chrono::Duration::days(retention_days as i64);

//     let result = sqlx::query("DELETE FROM http_observability_logs WHERE created_at < ?")
//         .bind(cutoff_date)
//         .execute(&state.db)
//         .await
//         .map_err(AppError::Database)?;

//     Ok(HttpResponse::Ok().json(serde_json::json!({
//         "deleted_count": result.rows_affected()
//     })))
// }

pub async fn truncate_logs(
    state: web::Data<AppState>,
) -> Result<HttpResponse, AppError> {
    sqlx::query("TRUNCATE TABLE http_observability_logs")
        .execute(&state.db)
        .await
        .map_err(AppError::Database)?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": "All logs truncated"
    })))
}