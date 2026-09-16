use crate::infra::config::app_config::WebSocketConfig;
use crate::infra::ws::error::WsError;
use crate::infra::ws::hub::{ConnectionId, WsHub};
use crate::infra::ws::message::{error_code, ClientMessage, ServerMessage};
use crate::infra::ws::policy::{WsContext, WsPolicy};
use actix_ws::{AggregatedMessage, CloseCode, CloseReason, Session};
use bytestring::ByteString;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};

/// Everything the session task needs to run one connection.
pub struct SessionParams {
    pub hub: Arc<WsHub>,
    pub policy: Arc<dyn WsPolicy>,
    pub config: WebSocketConfig,
    pub connection_id: ConnectionId,
    pub context: WsContext,
}

/// Drive a single WebSocket connection until it closes.
///
/// One task owns one socket and multiplexes three sources:
///
/// 1. **outbound** — frames the hub pushed for this connection;
/// 2. **inbound** — frames the client sent, dispatched through [`WsPolicy`];
/// 3. **heartbeat** — a periodic ping, plus the idle check that reaps sockets
///    whose peer vanished without a close frame (the common case behind a NAT or
///    a laptop lid).
///
/// Whichever way the loop ends, the connection is unregistered from the hub and
/// the socket is closed, so no cleanup path is missed.
pub async fn run(
    params: SessionParams,
    mut session: Session,
    stream: actix_ws::MessageStream,
    mut outbound: mpsc::Receiver<ByteString>,
) {
    let SessionParams {
        hub,
        policy,
        config,
        connection_id,
        context,
    } = params;

    let mut stream = stream
        .max_frame_size(config.max_frame_size_bytes)
        .aggregate_continuations()
        .max_continuation_size(config.max_message_size_bytes);

    let heartbeat_interval = Duration::from_secs(config.heartbeat_interval_seconds.max(1));
    let client_timeout = Duration::from_secs(config.client_timeout_seconds.max(1));

    let mut ticker = interval(heartbeat_interval);
    // A stalled task must not fire a burst of catch-up pings.
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // The first tick resolves immediately; consume it so the first ping waits a
    // full interval.
    ticker.tick().await;

    let mut last_seen = Instant::now();

    // Let the application veto or prepare the connection (presence records,
    // implicit subscriptions) before any client frame is processed.
    if let Err(reason) = policy.on_connect(&context).await {
        tracing::debug!(connection_id = %connection_id, reason = %reason, "Websocket connection rejected by policy");
        hub.unregister(&connection_id);
        let _ = session
            .close(Some(CloseReason {
                code: CloseCode::Policy,
                description: Some(reason),
            }))
            .await;
        return;
    }

    let welcome = ServerMessage::Welcome {
        connection_id,
        user_id: context.user_id,
        heartbeat_interval_seconds: config.heartbeat_interval_seconds,
    };
    if !send(&mut session, &welcome).await {
        hub.unregister(&connection_id);
        return;
    }

    let close_reason: Option<CloseReason> = loop {
        tokio::select! {
            // ── Hub -> client ────────────────────────────────────────────────
            frame = outbound.recv() => {
                match frame {
                    Some(frame) => {
                        if session.text(frame).await.is_err() {
                            break None;
                        }
                    }
                    // The hub dropped our sender: this connection was evicted
                    // (slow client, or an explicit disconnect).
                    None => {
                        break Some(CloseReason {
                            code: CloseCode::Away,
                            description: Some("Connection closed by server".to_string()),
                        });
                    }
                }
            }

            // ── Client -> server ─────────────────────────────────────────────
            message = stream.recv() => {
                let Some(message) = message else {
                    break None; // stream ended
                };

                let message = match message {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::debug!(connection_id = %connection_id, error = %e, "Websocket protocol error");
                        break Some(CloseReason::from(CloseCode::Protocol));
                    }
                };

                last_seen = Instant::now();

                match message {
                    AggregatedMessage::Text(text) => {
                        if !handle_text(&hub, &policy, &context, &mut session, &text).await {
                            break None;
                        }
                    }
                    AggregatedMessage::Binary(_) => {
                        let msg = ServerMessage::error(
                            error_code::BAD_REQUEST,
                            "Binary frames are not supported — send JSON text frames",
                        );
                        if !send(&mut session, &msg).await {
                            break None;
                        }
                    }
                    AggregatedMessage::Ping(bytes) => {
                        if session.pong(&bytes).await.is_err() {
                            break None;
                        }
                    }
                    AggregatedMessage::Pong(_) => {}
                    AggregatedMessage::Close(reason) => break reason,
                }
            }

            // ── Heartbeat ────────────────────────────────────────────────────
            _ = ticker.tick() => {
                if last_seen.elapsed() > client_timeout {
                    tracing::debug!(
                        connection_id = %connection_id,
                        idle_seconds = last_seen.elapsed().as_secs(),
                        "Websocket client timed out"
                    );
                    break Some(CloseReason {
                        code: CloseCode::Away,
                        description: Some("Heartbeat timeout".to_string()),
                    });
                }

                if session.ping(b"").await.is_err() {
                    break None;
                }
            }
        }
    };

    hub.unregister(&connection_id);
    policy.on_disconnect(&context).await;
    let _ = session.close(close_reason).await;

    tracing::debug!(connection_id = %connection_id, "Websocket session ended");
}

/// Handle one inbound text frame. Returns `false` when the socket is gone and
/// the loop should stop.
async fn handle_text(
    hub: &Arc<WsHub>,
    policy: &Arc<dyn WsPolicy>,
    ctx: &WsContext,
    session: &mut Session,
    text: &str,
) -> bool {
    let message: ClientMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            let msg = ServerMessage::error(error_code::BAD_REQUEST, e.to_string());
            return send(session, &msg).await;
        }
    };

    let response = match message {
        ClientMessage::Subscribe { topic } => {
            match policy.authorize_subscribe(ctx, &topic).await {
                Err(reason) => ServerMessage::error(error_code::FORBIDDEN, reason),
                Ok(()) => match hub.subscribe(&ctx.connection_id, &topic) {
                    Ok(()) => ServerMessage::Subscribed { topic },
                    Err(e @ WsError::TopicLimit { .. }) => {
                        ServerMessage::error(error_code::LIMIT_EXCEEDED, e.to_string())
                    }
                    Err(e) => ServerMessage::error(error_code::INTERNAL, e.to_string()),
                },
            }
        }

        ClientMessage::Unsubscribe { topic } => {
            hub.unsubscribe(&ctx.connection_id, &topic);
            ServerMessage::Unsubscribed { topic }
        }

        ClientMessage::Publish { topic, payload } => {
            match policy.authorize_publish(ctx, &topic, &payload).await {
                Err(reason) => ServerMessage::error(error_code::FORBIDDEN, reason),
                Ok(()) => match hub.publish(&topic, &payload) {
                    // The publisher is a subscriber like any other, so it sees
                    // its own message come back as an `event` frame; no ack is
                    // needed here.
                    Ok(_) => return true,
                    Err(e) => ServerMessage::error(error_code::INTERNAL, e.to_string()),
                },
            }
        }

        ClientMessage::Ping => ServerMessage::Pong,
    };

    send(session, &response).await
}

/// Encode and write one server frame. Returns `false` once the socket is closed.
async fn send(session: &mut Session, message: &ServerMessage) -> bool {
    let frame = match message.encode() {
        Ok(f) => f,
        Err(e) => {
            tracing::error!(error = %e, "Failed to encode websocket frame");
            return true;
        }
    };
    session.text(frame).await.is_ok()
}
