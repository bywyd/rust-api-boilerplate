use crate::app::error::AppError;
use crate::app::http::middleware::auth::AuthUser;
use crate::app::services::user_service::{CreateUserDto, LoginDto, UpdateUserDto, UserService};
use crate::app::state::AppState;
use actix_web::{web, HttpResponse};
use uuid::Uuid;

pub async fn list_users(
    state: web::Data<AppState>,
    query: web::Query<crate::app::http::pagination::PaginationParams>,
) -> Result<HttpResponse, AppError> {
    let service = UserService::new(&state.db, &state.orm, &state.cache);
    let result = service.find_all(&query.into_inner()).await?;
    Ok(HttpResponse::Ok().json(result))
}

pub async fn get_user(
    state: web::Data<AppState>,
    path: web::Path<Uuid>,
) -> Result<HttpResponse, AppError> {
    let service = UserService::new(&state.db, &state.orm, &state.cache);
    let user = service.find_by_id(*path).await?;
    Ok(HttpResponse::Ok().json(user))
}

pub async fn create_user(
    state: web::Data<AppState>,
    body: web::Json<CreateUserDto>,
) -> Result<HttpResponse, AppError> {
    let service = UserService::new(&state.db, &state.orm, &state.cache);
    let user = service.create(body.into_inner()).await?;
    Ok(HttpResponse::Created().json(user))
}

pub async fn update_user(
    state: web::Data<AppState>,
    path: web::Path<Uuid>,
    body: web::Json<UpdateUserDto>,
) -> Result<HttpResponse, AppError> {
    let service = UserService::new(&state.db, &state.orm, &state.cache);
    let user = service.update(*path, body.into_inner()).await?;
    Ok(HttpResponse::Ok().json(user))
}

/// Requires a valid JWT — add `_auth: AuthUser` to enforce authentication.
pub async fn delete_user(
    state: web::Data<AppState>,
    path: web::Path<Uuid>,
    _auth: AuthUser,
) -> Result<HttpResponse, AppError> {
    let service = UserService::new(&state.db, &state.orm, &state.cache);
    service.delete(*path).await?;
    Ok(HttpResponse::NoContent().finish())
}

pub async fn login(
    state: web::Data<AppState>,
    body: web::Json<LoginDto>,
) -> Result<HttpResponse, AppError> {
    let service = UserService::new(&state.db, &state.orm, &state.cache);
    let response = service.login(body.into_inner(), &state.config.auth).await?;
    Ok(HttpResponse::Ok().json(response))
}
