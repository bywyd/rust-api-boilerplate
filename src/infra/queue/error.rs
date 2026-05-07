use thiserror::Error;

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("Channel error: {0}")]
    Channel(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Job execution error: {0}")]
    Execution(String),

    #[error("Unknown job type: {0}")]
    UnknownJobType(String),
}
