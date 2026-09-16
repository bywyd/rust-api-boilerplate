use crate::infra::ws::hub::{ConnectionId, WsHub};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

/// Everything a policy decision can depend on, for one connection.
///
/// `hub` is included so a policy can react to a decision — for example
/// auto-subscribing a freshly authenticated user to their personal topic in
/// [`WsPolicy::on_connect`].
#[derive(Clone)]
pub struct WsContext {
    /// Identifier of this socket, unique per process.
    pub connection_id: ConnectionId,
    /// The authenticated user, or `None` for an anonymous connection.
    pub user_id: Option<Uuid>,
    /// The email claim from the JWT, when authenticated.
    pub email: Option<String>,
    /// The hub this connection is registered with.
    pub hub: Arc<WsHub>,
}

impl WsContext {
    /// `true` when the connection presented a valid token.
    pub fn is_authenticated(&self) -> bool {
        self.user_id.is_some()
    }
}

/// Application-defined authorisation and lifecycle rules for WebSocket
/// connections.
///
/// The hub is pure mechanism: it fans messages out to whoever is subscribed. It
/// deliberately knows nothing about *who may subscribe to what* — that is
/// application policy, implemented by
/// [`AppWsPolicy`](crate::app::ws::policy::AppWsPolicy) in the `app` layer. This
/// mirrors the split between `infra::queue` (transport) and `app::jobs`
/// (business logic).
///
/// Every method returning `Result<(), String>` reports the rejection reason in
/// the `Err` variant; it is relayed to the client in a
/// [`ServerMessage::Error`](crate::infra::ws::message::ServerMessage::Error)
/// frame and never closes the socket.
#[async_trait]
pub trait WsPolicy: Send + Sync + 'static {
    /// Called once after the socket is registered, before any client frame is
    /// read. Returning `Err` closes the connection with the given reason.
    async fn on_connect(&self, _ctx: &WsContext) -> Result<(), String> {
        Ok(())
    }

    /// Decide whether this connection may subscribe to `topic`.
    async fn authorize_subscribe(&self, ctx: &WsContext, topic: &str) -> Result<(), String>;

    /// Decide whether this connection may publish `payload` to `topic`.
    ///
    /// Client-originated publishing is a broadcast primitive handed to
    /// untrusted input — keep this strict.
    async fn authorize_publish(
        &self,
        ctx: &WsContext,
        topic: &str,
        payload: &Value,
    ) -> Result<(), String>;

    /// Called once after the socket is removed from the hub. Use it to release
    /// presence records or emit a "user left" event.
    async fn on_disconnect(&self, _ctx: &WsContext) {}
}
