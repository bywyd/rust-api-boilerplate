use crate::infra::ws::error::WsError;
use bytestring::ByteString;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// A frame sent by the client to the server.
///
/// The wire format is JSON, discriminated by a `type` field:
///
/// ```json
/// { "type": "subscribe",   "topic": "public.prices" }
/// { "type": "unsubscribe", "topic": "public.prices" }
/// { "type": "publish",     "topic": "chat.42", "payload": { "text": "hi" } }
/// { "type": "ping" }
/// ```
///
/// Unknown `type` values are rejected with an [`ServerMessage::Error`] frame
/// rather than closing the socket, so a client can be upgraded independently of
/// the server.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Start receiving events published to `topic`.
    Subscribe { topic: String },
    /// Stop receiving events published to `topic`.
    Unsubscribe { topic: String },
    /// Publish `payload` to every subscriber of `topic`.
    ///
    /// Denied by default — see
    /// [`AppWsPolicy::authorize_publish`](crate::app::ws::policy::AppWsPolicy).
    Publish { topic: String, payload: Value },
    /// Application-level liveness check, answered with [`ServerMessage::Pong`].
    ///
    /// This is independent of the protocol-level ping/pong the server uses for
    /// its heartbeat; it exists for clients that cannot observe control frames
    /// (the browser WebSocket API, for one).
    Ping,
}

/// A frame sent by the server to the client.
///
/// ```json
/// { "type": "welcome", "connection_id": "…", "user_id": null, "heartbeat_interval_seconds": 20 }
/// { "type": "subscribed",   "topic": "public.prices" }
/// { "type": "event",        "topic": "public.prices", "payload": { … } }
/// { "type": "error",        "code": "FORBIDDEN", "message": "…" }
/// ```
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Sent once, immediately after the handshake succeeds.
    Welcome {
        connection_id: Uuid,
        user_id: Option<Uuid>,
        heartbeat_interval_seconds: u64,
    },
    /// Acknowledges a `subscribe`.
    Subscribed { topic: String },
    /// Acknowledges an `unsubscribe`.
    Unsubscribed { topic: String },
    /// An application event delivered to a subscriber of `topic`.
    Event { topic: String, payload: Value },
    /// Answers a `ping`.
    Pong,
    /// A rejected command. The socket stays open.
    Error {
        code: &'static str,
        message: String,
    },
}

/// Error codes carried by [`ServerMessage::Error`]. Stable strings — clients may
/// match on them.
pub mod error_code {
    /// The frame could not be parsed as a [`ClientMessage`](super::ClientMessage).
    pub const BAD_REQUEST: &str = "BAD_REQUEST";
    /// The policy refused the subscription or publish.
    pub const FORBIDDEN: &str = "FORBIDDEN";
    /// A per-connection budget (topics) was exhausted.
    pub const LIMIT_EXCEEDED: &str = "LIMIT_EXCEEDED";
    /// The server failed to process an otherwise valid command.
    pub const INTERNAL: &str = "INTERNAL";
}

impl ServerMessage {
    /// Encode to the JSON text frame put on the wire.
    ///
    /// The result is a [`ByteString`] so a single encoded frame can be cloned
    /// cheaply (refcount bump, no copy) for every recipient of a broadcast.
    pub fn encode(&self) -> Result<ByteString, WsError> {
        Ok(ByteString::from(serde_json::to_string(self)?))
    }

    /// Convenience constructor for an error frame.
    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        Self::Error {
            code,
            message: message.into(),
        }
    }
}
