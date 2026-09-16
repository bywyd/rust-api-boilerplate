use crate::infra::cache::redis::RedisPool;
use crate::infra::ws::hub::WsHub;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

/// How long to wait before re-subscribing after the pub/sub connection drops.
const RECONNECT_DELAY: Duration = Duration::from_secs(5);

/// Which sockets a relayed event is aimed at on the receiving instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClusterScope {
    /// Everyone subscribed to `ClusterFrame::topic`.
    Topic,
    /// Every socket belonging to one user.
    User { user_id: Uuid },
    /// Every connected socket.
    All,
}

/// One event relayed between instances over Redis pub/sub.
///
/// `node` is the id of the instance that published it. Receivers drop frames
/// carrying their own id, which is what prevents an event from echoing back into
/// the hub that produced it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterFrame {
    pub node: Uuid,
    pub scope: ClusterScope,
    pub topic: String,
    pub payload: Value,
}

/// Start the Redis bridge: one task publishing local events, one consuming
/// remote ones.
///
/// Both tasks run for the lifetime of the process. The subscriber reconnects on
/// its own if Redis goes away; while it is down, events are still delivered
/// locally, so a Redis outage degrades a cluster to independent instances rather
/// than breaking WebSocket delivery outright.
pub fn start(
    hub: Arc<WsHub>,
    outbound: mpsc::Receiver<ClusterFrame>,
    pool: RedisPool,
    redis_url: String,
    channel: String,
) {
    tokio::spawn(publish_loop(outbound, pool, channel.clone()));
    tokio::spawn(subscribe_loop(hub, redis_url, channel));
}

/// Drain locally published events onto the Redis channel.
async fn publish_loop(
    mut outbound: mpsc::Receiver<ClusterFrame>,
    pool: RedisPool,
    channel: String,
) {
    while let Some(frame) = outbound.recv().await {
        let payload = match serde_json::to_string(&frame) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(error = %e, "Failed to serialise websocket cluster frame");
                continue;
            }
        };

        let mut conn = match pool.get().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    topic = %frame.topic,
                    "Redis unavailable — websocket event not relayed to other instances"
                );
                continue;
            }
        };

        if let Err(e) = redis::cmd("PUBLISH")
            .arg(&channel)
            .arg(&payload)
            .exec_async(&mut conn)
            .await
        {
            tracing::warn!(
                error = %e,
                topic = %frame.topic,
                "Failed to relay websocket event to other instances"
            );
        }
    }

    tracing::debug!("Websocket cluster publisher stopped — hub dropped");
}

/// Subscribe to the Redis channel and feed remote events into the local hub.
async fn subscribe_loop(hub: Arc<WsHub>, redis_url: String, channel: String) {
    let node_id = hub.node_id();

    loop {
        match run_subscriber(&hub, node_id, &redis_url, &channel).await {
            Ok(()) => tracing::warn!("Websocket cluster subscription ended — reconnecting"),
            Err(e) => tracing::warn!(error = %e, "Websocket cluster subscription failed — retrying"),
        }
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

/// One subscription attempt. Returns when the stream ends or errors.
async fn run_subscriber(
    hub: &Arc<WsHub>,
    node_id: Uuid,
    redis_url: &str,
    channel: &str,
) -> redis::RedisResult<()> {
    // A dedicated connection: a subscriber cannot serve other commands, so it
    // must not come from the shared pool.
    let client = redis::Client::open(redis_url)?;
    let mut pubsub = client.get_async_pubsub().await?;
    pubsub.subscribe(channel).await?;

    tracing::info!(
        channel = %channel,
        node_id = %node_id,
        "Websocket cluster bridge subscribed"
    );

    let mut stream = pubsub.into_on_message();

    while let Some(msg) = stream.next().await {
        let raw: String = match msg.get_payload() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "Unreadable websocket cluster frame");
                continue;
            }
        };

        let frame: ClusterFrame = match serde_json::from_str(&raw) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(error = %e, "Malformed websocket cluster frame");
                continue;
            }
        };

        // Our own event coming back around.
        if frame.node == node_id {
            continue;
        }

        match hub.dispatch_cluster_frame(&frame) {
            Ok(delivered) => tracing::trace!(
                topic = %frame.topic,
                origin = %frame.node,
                delivered,
                "Delivered websocket event from peer instance"
            ),
            Err(e) => tracing::warn!(
                error = %e,
                topic = %frame.topic,
                "Failed to deliver websocket event from peer instance"
            ),
        }
    }

    Ok(())
}
