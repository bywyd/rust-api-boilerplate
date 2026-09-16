use crate::infra::ws::policy::{WsContext, WsPolicy};
use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

/// Prefix for topics anyone may subscribe to, signed in or not.
const PUBLIC_PREFIX: &str = "public.";

/// Prefix for per-user topics: `user.<uuid>`. Only that user may subscribe.
const USER_PREFIX: &str = "user.";

/// The application's WebSocket authorisation rules.
///
/// This is the file to edit when adding real-time features — the hub itself
/// stays generic. The shipped defaults are deliberately conservative:
///
/// | Topic              | Who may subscribe                         |
/// |--------------------|-------------------------------------------|
/// | `public.*`         | anyone, including anonymous connections    |
/// | `user.<uuid>`      | only the user whose id matches            |
/// | anything else      | any authenticated user                    |
///
/// Client-initiated publishing is refused outright. Servers usually want events
/// to originate from a handler or a job (`state.ws.publish(...)`) so they pass
/// through validation and persistence first; relay-style features such as chat
/// are the exception, and want an explicit rule here.
///
/// # Adding a rule
///
/// ```rust,ignore
/// // Let members of a room receive its events:
/// if let Some(room_id) = topic.strip_prefix("room.") {
///     let Some(user_id) = ctx.user_id else {
///         return Err("Authentication required".to_string());
///     };
///     return match self.is_room_member(user_id, room_id).await {
///         true => Ok(()),
///         false => Err("Not a member of this room".to_string()),
///     };
/// }
/// ```
///
/// Rules may hit the database — every method is `async`. Give the struct the
/// dependencies it needs (an ORM connection, a cache handle) and build it in
/// [`bootstrap::ws::init`](crate::bootstrap::ws::init). Note that it must not
/// hold `AppState`, which owns the hub: that would be a reference cycle.
pub struct AppWsPolicy;

impl AppWsPolicy {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AppWsPolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl WsPolicy for AppWsPolicy {
    /// Subscribe every authenticated connection to its own `user.<uuid>` topic,
    /// so `state.ws.publish_to_user(...)` reaches it without the client having
    /// to ask.
    async fn on_connect(&self, ctx: &WsContext) -> Result<(), String> {
        if let Some(user_id) = ctx.user_id {
            let topic = format!("{USER_PREFIX}{user_id}");
            if let Err(e) = ctx.hub.subscribe(&ctx.connection_id, &topic) {
                tracing::warn!(
                    connection_id = %ctx.connection_id,
                    error = %e,
                    "Failed to auto-subscribe connection to its user topic"
                );
            }
        }
        Ok(())
    }

    async fn authorize_subscribe(&self, ctx: &WsContext, topic: &str) -> Result<(), String> {
        if topic.is_empty() {
            return Err("Topic must not be empty".to_string());
        }

        // Open topics — broadcast data that is public anyway.
        if topic.starts_with(PUBLIC_PREFIX) {
            return Ok(());
        }

        // Private per-user topics: the id in the topic must be the caller's.
        if let Some(raw_id) = topic.strip_prefix(USER_PREFIX) {
            let Some(user_id) = ctx.user_id else {
                return Err("Authentication required for user topics".to_string());
            };
            return match Uuid::parse_str(raw_id) {
                Ok(id) if id == user_id => Ok(()),
                Ok(_) => Err("Cannot subscribe to another user's topic".to_string()),
                Err(_) => Err("Malformed user topic".to_string()),
            };
        }

        // Everything else is for signed-in clients. Tighten this per topic as
        // the application grows.
        if ctx.is_authenticated() {
            Ok(())
        } else {
            Err(format!("Authentication required to subscribe to '{topic}'"))
        }
    }

    async fn authorize_publish(
        &self,
        _ctx: &WsContext,
        _topic: &str,
        _payload: &Value,
    ) -> Result<(), String> {
        Err("Clients may not publish on this server".to_string())
    }

    async fn on_disconnect(&self, ctx: &WsContext) {
        tracing::debug!(
            connection_id = %ctx.connection_id,
            user_id = ?ctx.user_id,
            "Websocket client disconnected"
        );
    }
}
