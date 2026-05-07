use crate::app::auth::generate_token;
use crate::app::error::AppError;
use crate::infra::cache::local::LocalCache;
use crate::infra::config::app_config::AuthConfig;
use crate::infra::db::entities::user;
use crate::infra::db::pool::{DbConnection, DbPool};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

// DTOs

#[derive(Debug, Deserialize, Validate)]
pub struct CreateUserDto {
    #[validate(email(message = "must be a valid email address"))]
    pub email: String,
    #[validate(length(min = 2, max = 100, message = "must be between 2 and 100 characters"))]
    pub name: String,
    #[validate(length(min = 8, message = "must be at least 8 characters"))]
    pub password: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct UpdateUserDto {
    #[validate(length(min = 2, max = 100, message = "must be between 2 and 100 characters"))]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginDto {
    pub email: String,
    pub password: String,
}

// Response types

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserResponse {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: UserResponse,
}

impl From<user::Model> for UserResponse {
    fn from(m: user::Model) -> Self {
        Self {
            id: m.id,
            email: m.email,
            name: m.name,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

// Service

pub struct UserService<'a> {
    #[allow(dead_code)]
    db: &'a DbPool,
    orm: &'a DbConnection,
    cache: &'a LocalCache,
}

impl<'a> UserService<'a> {
    pub fn new(db: &'a DbPool, orm: &'a DbConnection, cache: &'a LocalCache) -> Self {
        Self { db, orm, cache }
    }

    fn cache_key(id: &Uuid) -> String {
        format!("user:{}", id)
    }

    // Queries

    pub async fn find_all(&self) -> Result<Vec<UserResponse>, AppError> {
        let users = user::Entity::find().all(self.orm).await?;
        Ok(users.into_iter().map(UserResponse::from).collect())
    }

    pub async fn find_by_id(&self, id: Uuid) -> Result<UserResponse, AppError> {
        let key = Self::cache_key(&id);

        // L1 cache hit
        if let Some(cached) = self.cache.get(&key).await {
            if let Ok(user) = serde_json::from_str::<UserResponse>(&cached) {
                tracing::debug!(user_id = %id, "Cache hit");
                return Ok(user);
            }
        }

        let model = user::Entity::find_by_id(id)
            .one(self.orm)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("User {} not found", id)))?;

        let response = UserResponse::from(model);

        // Populate cache
        if let Ok(json) = serde_json::to_string(&response) {
            self.cache.insert(key, json).await;
        }

        Ok(response)
    }

    // Commands

    pub async fn create(&self, dto: CreateUserDto) -> Result<UserResponse, AppError> {
        dto.validate()
            .map_err(|e| AppError::Validation(e.to_string()))?;

        let password_hash = hash_password(&dto.password)
            .map_err(|e| AppError::Internal(e.to_string()))?;

        let now = Utc::now();
        let model = user::ActiveModel {
            id: Set(Uuid::new_v4()),
            email: Set(dto.email),
            name: Set(dto.name),
            password_hash: Set(password_hash),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(self.orm)
        .await?;

        Ok(UserResponse::from(model))
    }

    pub async fn update(&self, id: Uuid, dto: UpdateUserDto) -> Result<UserResponse, AppError> {
        dto.validate()
            .map_err(|e| AppError::Validation(e.to_string()))?;

        let model = user::Entity::find_by_id(id)
            .one(self.orm)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("User {} not found", id)))?;

        let mut active: user::ActiveModel = model.into();

        if let Some(name) = dto.name {
            active.name = Set(name);
        }
        active.updated_at = Set(Utc::now());

        let updated = active.update(self.orm).await?;

        // Invalidate cache entry
        self.cache.invalidate(&Self::cache_key(&id)).await;

        Ok(UserResponse::from(updated))
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        let result = user::Entity::delete_by_id(id)
            .exec(self.orm)
            .await?;

        if result.rows_affected == 0 {
            return Err(AppError::NotFound(format!("User {} not found", id)));
        }

        self.cache.invalidate(&Self::cache_key(&id)).await;

        Ok(())
    }

    pub async fn login(
        &self,
        dto: LoginDto,
        auth_cfg: &AuthConfig,
    ) -> Result<LoginResponse, AppError> {
        let model = user::Entity::find()
            .filter(user::Column::Email.eq(&dto.email))
            .one(self.orm)
            .await?
            .ok_or_else(|| AppError::Unauthorized("Invalid credentials".to_string()))?;

        verify_password(&dto.password, &model.password_hash)
            .map_err(|_| AppError::Unauthorized("Invalid credentials".to_string()))?;

        let token = generate_token(model.id, &model.email, auth_cfg)?;

        Ok(LoginResponse {
            token,
            user: UserResponse::from(model),
        })
    }
}

// Helpers

fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!("Password hashing failed: {}", e))
}

fn verify_password(password: &str, hash: &str) -> Result<(), argon2::password_hash::Error> {
    let parsed = PasswordHash::new(hash)?;
    Argon2::default().verify_password(password.as_bytes(), &parsed)
}
