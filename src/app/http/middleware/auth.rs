use crate::app::auth::verify_token;
use crate::app::error::AppError;
use crate::app::state::AppState;
use actix_web::{dev::Payload, web, FromRequest, HttpRequest};
use futures::future::{err, ok, Ready};
use uuid::Uuid;

/// The authenticated user extracted from a valid `Authorization: Bearer <token>` header.
///
/// Add `_auth: AuthUser` (or `auth: AuthUser`) as a handler parameter to require authentication.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AuthUser {
    pub user_id: Uuid,
    pub email: String,
}

impl FromRequest for AuthUser {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        let state = match req.app_data::<web::Data<AppState>>() {
            Some(s) => s,
            None => {
                return err(
                    AppError::Internal("App state unavailable in extractor".to_string()).into(),
                );
            }
        };

        let auth_header = match req.headers().get("Authorization") {
            Some(h) => h,
            None => {
                return err(
                    AppError::Unauthorized("Missing Authorization header".to_string()).into(),
                );
            }
        };

        let auth_str = match auth_header.to_str() {
            Ok(s) => s,
            Err(_) => {
                return err(
                    AppError::Unauthorized("Malformed Authorization header".to_string()).into(),
                );
            }
        };

        let token = match auth_str.strip_prefix("Bearer ") {
            Some(t) => t,
            None => {
                return err(
                    AppError::Unauthorized(
                        "Authorization header must start with 'Bearer '".to_string(),
                    )
                    .into(),
                );
            }
        };

        match verify_token(token, &state.config.auth) {
            Ok(claims) => match Uuid::parse_str(&claims.sub) {
                Ok(user_id) => ok(AuthUser {
                    user_id,
                    email: claims.email,
                }),
                Err(_) => {
                    err(AppError::Unauthorized("Invalid token subject".to_string()).into())
                }
            },
            Err(e) => err(e.into()),
        }
    }
}
