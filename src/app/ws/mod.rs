//! Application-level WebSocket policy.
//!
//! The transport lives in [`infra::ws`](crate::infra::ws); this module decides
//! what connected clients are allowed to do. It is the WebSocket counterpart of
//! `app::jobs` sitting on top of `infra::queue`.
pub mod policy;

pub use policy::AppWsPolicy;
