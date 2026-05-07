use crate::infra::cache::local::LocalCache;
use crate::infra::cache::redis::RedisPool;
use crate::infra::config::app_config::AppConfig;
use crate::infra::db::pool::{DbConnection, DbPool};
use crate::infra::http_client::client::HttpClient;
use crate::infra::queue::error::QueueError;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::sync::Arc;
use uuid::Uuid;

// ── JobStatus ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Retrying,
}

impl fmt::Display for JobStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JobStatus::Pending => write!(f, "pending"),
            JobStatus::Running => write!(f, "running"),
            JobStatus::Completed => write!(f, "completed"),
            JobStatus::Failed => write!(f, "failed"),
            JobStatus::Retrying => write!(f, "retrying"),
        }
    }
}

// ── JobEnvelope ───────────────────────────────────────────────────────────────

/// The container that wraps any job payload in transit through the queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobEnvelope {
    pub id: Uuid,
    pub job_type: String,
    pub payload: Value,
    pub status: JobStatus,
    pub attempts: u32,
    pub max_attempts: u32,
    pub run_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub error: Option<String>,
}

impl JobEnvelope {
    pub fn new(job_type: impl Into<String>, payload: Value, max_attempts: u32) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            job_type: job_type.into(),
            payload,
            status: JobStatus::Pending,
            attempts: 0,
            max_attempts,
            run_at: now,
            created_at: now,
            error: None,
        }
    }

    /// Schedule the job to run at a specific time instead of immediately.
    pub fn run_at(mut self, at: DateTime<Utc>) -> Self {
        self.run_at = at;
        self
    }
}

// ── JobContext ─────────────────────────────────────────────────────────────────

/// Context injected into every job handler. Mirrors `AppState` but lives in
/// `infra` to avoid circular dependencies.
#[derive(Clone)]
pub struct JobContext {
    pub db: DbPool,
    pub orm: DbConnection,
    pub cache: LocalCache,
    pub redis: Option<RedisPool>,
    pub http_client: HttpClient,
    pub config: Arc<AppConfig>,
}

// ── Job trait ─────────────────────────────────────────────────────────────────

/// Implement this trait for every job type in `app/jobs/`.
#[async_trait]
pub trait Job: Send + Sync {
    /// Unique string identifier used to route the job to this handler.
    /// Must be stable — changing it will orphan enqueued jobs.
    fn job_type() -> &'static str
    where
        Self: Sized;

    async fn execute(&self, ctx: &JobContext) -> Result<(), QueueError>;
}
