use thiserror::Error;

/// Failures raised by the WebSocket hub.
///
/// These are deliberately separate from `AppError`: most of them are recoverable
/// signals for the caller (a full connection table, a client exceeding its
/// subscription budget) rather than HTTP responses.
#[derive(Debug, Error)]
pub enum WsError {
    #[error("Failed to serialise websocket payload: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Connection limit reached ({limit} connections on this instance)")]
    ConnectionLimit { limit: usize },

    #[error("Subscription limit reached ({limit} topics per connection)")]
    TopicLimit { limit: usize },

    #[error("Connection is no longer registered")]
    UnknownConnection,
}
