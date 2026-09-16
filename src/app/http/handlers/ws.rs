use crate::app::auth::verify_token;
use crate::app::error::AppError;
use crate::app::http::middleware::auth::AuthUser;
use crate::app::state::AppState;
use crate::infra::ws::policy::WsContext;
use crate::infra::ws::session::{self, SessionParams};
use actix_web::{web, HttpRequest, HttpResponse};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

/// Query string accepted on the handshake.
#[derive(Debug, Deserialize)]
pub struct WsQuery {
    /// JWT for clients that cannot set an `Authorization` header — every
    /// browser, in practice. Honoured only when
    /// `websocket.allow_query_token = true`.
    pub token: Option<String>,
}

/// `GET /api/ws` — upgrade the connection and hand it to a session task.
///
/// # Authentication
///
/// The token is read from `Authorization: Bearer <jwt>` first, then from
/// `?token=<jwt>` when `websocket.allow_query_token` is set. An invalid token is
/// always rejected; a *missing* one is rejected only when
/// `websocket.require_auth = true`, otherwise the connection proceeds anonymous
/// and [`AppWsPolicy`](crate::app::ws::policy::AppWsPolicy) restricts it to
/// public topics.
///
/// # Client example
///
/// ```js
/// const ws = new WebSocket(`ws://localhost:8080/api/ws?token=${jwt}`);
///
/// ws.onopen = () =>
///   ws.send(JSON.stringify({ type: "subscribe", topic: "public.prices" }));
///
/// ws.onmessage = (e) => {
///   const msg = JSON.parse(e.data);
///   if (msg.type === "event") console.log(msg.topic, msg.payload);
/// };
/// ```
pub async fn ws_connect(
    req: HttpRequest,
    body: web::Payload,
    query: web::Query<WsQuery>,
    state: web::Data<AppState>,
) -> Result<HttpResponse, actix_web::Error> {
    let cfg = &state.config.websocket;

    let identity = resolve_identity(&req, &query, &state)?;

    if cfg.require_auth && identity.is_none() {
        return Err(AppError::Unauthorized(
            "Authentication required for websocket connections".to_string(),
        )
        .into());
    }

    let (user_id, email) = match &identity {
        Some(user) => (Some(user.user_id), Some(user.email.clone())),
        None => (None, None),
    };

    // Complete the handshake before registering, so a failed upgrade never
    // leaves an orphan entry in the hub.
    let (response, session, stream) = actix_ws::handle(&req, body)?;

    // A refused registration means this instance is at capacity, which is a 503
    // (retry elsewhere / later) rather than a fault in the request.
    let (connection_id, outbound) = state.ws.register(user_id).map_err(|e| {
        tracing::warn!(error = %e, "Refused websocket connection");
        actix_web::error::ErrorServiceUnavailable(e.to_string())
    })?;

    let context = WsContext {
        connection_id,
        user_id,
        email,
        hub: Arc::clone(&state.ws),
    };

    let params = SessionParams {
        hub: Arc::clone(&state.ws),
        policy: Arc::clone(&state.ws_policy),
        config: cfg.clone(),
        connection_id,
        context,
    };

    tracing::info!(
        connection_id = %connection_id,
        user_id = ?user_id,
        peer = ?req.peer_addr(),
        "Websocket connection established"
    );

    // `spawn_local`: the session future stays on the actix worker thread that
    // owns this socket.
    actix_web::rt::spawn(session::run(params, session, stream, outbound));

    Ok(response)
}

/// `GET /api/ws/stats` — connection and topic counters for this instance.
///
/// Requires a valid JWT. Counts are per-process: behind a load balancer, poll
/// every instance (or scrape them into your metrics system) for a cluster-wide
/// view.
pub async fn ws_stats(
    state: web::Data<AppState>,
    _auth: AuthUser,
) -> Result<HttpResponse, AppError> {
    Ok(HttpResponse::Ok().json(state.ws.stats()))
}

/// Resolve the caller from the `Authorization` header, falling back to the
/// `token` query parameter when configuration allows it.
///
/// Returns `Ok(None)` when no token was supplied at all; a token that is present
/// but invalid is an error regardless of `require_auth`.
fn resolve_identity(
    req: &HttpRequest,
    query: &WsQuery,
    state: &AppState,
) -> Result<Option<AuthUser>, AppError> {
    let header_token = req
        .headers()
        .get("Authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_string);

    let token = match header_token {
        Some(token) => Some(token),
        None if state.config.websocket.allow_query_token => query.token.clone(),
        None => None,
    };

    let Some(token) = token else {
        return Ok(None);
    };

    let claims = verify_token(&token, &state.config.auth)?;
    let user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized("Invalid token subject".to_string()))?;

    Ok(Some(AuthUser {
        user_id,
        email: claims.email,
    }))
}
