//! WebSocket transport.
//!
//! A [`WsHub`] keeps the connection table and the topic index; a session task
//! per socket ([`session::run`]) does the framing, heartbeat and command
//! dispatch. Between them sits a bounded queue per connection, which is what
//! keeps one slow client from stalling a broadcast.
//!
//! As with `infra::queue` and `app::jobs`, the mechanism lives here and the
//! policy lives in the app layer: who may subscribe to what is decided by
//! [`WsPolicy`], implemented by
//! [`AppWsPolicy`](crate::app::ws::policy::AppWsPolicy).
//!
//! # Publishing from anywhere
//!
//! ```rust,ignore
//! state.ws.publish("users", &json!({ "event": "user.created", "id": id }))?;
//! ```
//!
//! [`WsHub`] methods are synchronous and never block, so they are safe to call
//! from HTTP handlers, jobs and scheduled tasks alike.
pub mod cluster;
pub mod error;
pub mod hub;
pub mod message;
pub mod policy;
pub mod session;

pub use error::WsError;
pub use hub::{ConnectionId, HubStats, WsHub};
pub use message::{ClientMessage, ServerMessage};
pub use policy::{WsContext, WsPolicy};
