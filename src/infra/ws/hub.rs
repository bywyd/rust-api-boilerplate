use crate::infra::config::app_config::WebSocketConfig;
use crate::infra::ws::cluster::{ClusterFrame, ClusterScope};
use crate::infra::ws::error::WsError;
use crate::infra::ws::message::ServerMessage;
use bytestring::ByteString;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use uuid::Uuid;

/// Identifier of a single socket, unique within this process.
pub type ConnectionId = Uuid;

/// A registered socket, from the hub's point of view.
///
/// The hub never touches the socket itself — it only pushes encoded frames into
/// `tx`. The session task owns the socket and drains the matching receiver.
struct Connection {
    /// Outbound queue. Bounded: see [`WebSocketConfig::send_buffer`].
    tx: mpsc::Sender<ByteString>,
    user_id: Option<Uuid>,
    /// Topics this connection is subscribed to, mirrored in `WsHub::topics`.
    topics: HashSet<String>,
    connected_at: DateTime<Utc>,
}

/// A snapshot of hub activity, exposed by `GET /api/ws/stats`.
#[derive(Debug, Clone, Serialize)]
pub struct HubStats {
    /// Identifier of this instance, used to suppress echo in cluster mode.
    pub node_id: Uuid,
    /// Live connections on this instance.
    pub connections: usize,
    /// Connections that presented a valid token.
    pub authenticated_connections: usize,
    /// Topics with at least one subscriber.
    pub topics: usize,
    /// Total subscriptions across all connections.
    pub subscriptions: usize,
    /// Frames delivered since start-up.
    pub messages_sent: u64,
    /// Connections dropped for failing to drain their outbound queue.
    pub slow_clients_dropped: u64,
    /// Whether the Redis cross-instance bridge is active.
    pub cluster_enabled: bool,
}

/// In-process registry of WebSocket connections and their topic subscriptions.
///
/// # Why it is synchronous
///
/// Every method is a plain `fn`, never `async`. Delivery is a non-blocking
/// `try_send` into each connection's bounded queue, so publishing never waits on
/// a socket and can be called from anywhere — an HTTP handler, a background job,
/// a `Drop` impl. A client that cannot keep up fills its queue and is
/// disconnected; it can never stall the publisher or the other subscribers.
///
/// # Topics
///
/// Topics are opaque strings. Nothing is created or destroyed explicitly: a
/// topic exists while at least one connection is subscribed to it. Authorisation
/// lives in [`WsPolicy`](crate::infra::ws::policy::WsPolicy), not here.
///
/// # Scope
///
/// A hub only knows the sockets attached to *this process*. Actix worker threads
/// share one hub through `Arc`, so multiple workers are covered; multiple
/// *instances* are not. Enable `websocket.cluster` to bridge them over Redis.
///
/// # Publishing
///
/// ```rust,ignore
/// // To everyone subscribed to a topic:
/// state.ws.publish("users", &json!({ "event": "user.created", "id": id }))?;
///
/// // To every socket of one user, regardless of subscriptions:
/// state.ws.publish_to_user(user_id, "notifications", &payload)?;
///
/// // To every connected socket:
/// state.ws.broadcast("system", &json!({ "event": "maintenance" }))?;
/// ```
pub struct WsHub {
    node_id: Uuid,
    connections: DashMap<ConnectionId, Connection>,
    /// Reverse index: topic -> subscribers. Keeps fan-out proportional to the
    /// number of subscribers instead of the number of connections.
    topics: DashMap<String, HashSet<ConnectionId>>,
    /// Reverse index: user -> their (possibly several) connections.
    users: DashMap<Uuid, HashSet<ConnectionId>>,
    /// Outbound side of the Redis bridge. `None` unless clustering is enabled.
    cluster_tx: Option<mpsc::Sender<ClusterFrame>>,
    config: WebSocketConfig,
    messages_sent: AtomicU64,
    slow_clients_dropped: AtomicU64,
}

impl WsHub {
    /// Build an empty hub.
    ///
    /// `cluster_tx` is the outbound half of the Redis bridge; pass `None` to run
    /// single-instance. See [`crate::bootstrap::ws::init`].
    pub fn new(config: WebSocketConfig, cluster_tx: Option<mpsc::Sender<ClusterFrame>>) -> Self {
        Self {
            node_id: Uuid::new_v4(),
            connections: DashMap::new(),
            topics: DashMap::new(),
            users: DashMap::new(),
            cluster_tx,
            config,
            messages_sent: AtomicU64::new(0),
            slow_clients_dropped: AtomicU64::new(0),
        }
    }

    /// This instance's identifier.
    pub fn node_id(&self) -> Uuid {
        self.node_id
    }

    /// The configuration this hub was built with.
    pub fn config(&self) -> &WebSocketConfig {
        &self.config
    }

    // ── Connection lifecycle ─────────────────────────────────────────────────

    /// Register a new socket and return its id plus the receiver the session
    /// task must drain.
    ///
    /// Fails with [`WsError::ConnectionLimit`] once `websocket.max_connections`
    /// live connections are registered.
    pub fn register(
        &self,
        user_id: Option<Uuid>,
    ) -> Result<(ConnectionId, mpsc::Receiver<ByteString>), WsError> {
        let limit = self.config.max_connections;
        if limit > 0 && self.connections.len() >= limit {
            return Err(WsError::ConnectionLimit { limit });
        }

        let id = Uuid::new_v4();
        let (tx, rx) = mpsc::channel(self.config.send_buffer.max(1));

        self.connections.insert(
            id,
            Connection {
                tx,
                user_id,
                topics: HashSet::new(),
                connected_at: Utc::now(),
            },
        );

        if let Some(user_id) = user_id {
            self.users.entry(user_id).or_default().insert(id);
        }

        tracing::debug!(
            connection_id = %id,
            user_id = ?user_id,
            connections = self.connections.len(),
            "Websocket connection registered"
        );

        Ok((id, rx))
    }

    /// Remove a socket and all of its subscriptions. Idempotent.
    pub fn unregister(&self, id: &ConnectionId) {
        // Take the entry out first so its guard is released before the topic and
        // user indexes are touched — the maps are always locked in this order.
        let Some((_, conn)) = self.connections.remove(id) else {
            return;
        };

        for topic in &conn.topics {
            self.detach_from_topic(topic, id);
        }

        if let Some(user_id) = conn.user_id {
            if let Some(mut set) = self.users.get_mut(&user_id) {
                set.remove(id);
            }
            self.users.remove_if(&user_id, |_, set| set.is_empty());
        }

        tracing::debug!(
            connection_id = %id,
            connections = self.connections.len(),
            "Websocket connection unregistered"
        );
    }

    /// Close a connection from outside its session task.
    ///
    /// Dropping the hub's sender ends the session loop, which closes the socket
    /// and completes the teardown.
    pub fn disconnect(&self, id: &ConnectionId) {
        self.unregister(id);
    }

    // ── Subscriptions ────────────────────────────────────────────────────────

    /// Subscribe a connection to `topic`.
    ///
    /// Authorisation is the caller's responsibility — the session task consults
    /// [`WsPolicy`](crate::infra::ws::policy::WsPolicy) first.
    pub fn subscribe(&self, id: &ConnectionId, topic: &str) -> Result<(), WsError> {
        let limit = self.config.max_topics_per_connection;

        {
            let mut conn = self
                .connections
                .get_mut(id)
                .ok_or(WsError::UnknownConnection)?;

            if limit > 0 && !conn.topics.contains(topic) && conn.topics.len() >= limit {
                return Err(WsError::TopicLimit { limit });
            }
            conn.topics.insert(topic.to_string());
        }

        self.topics.entry(topic.to_string()).or_default().insert(*id);
        Ok(())
    }

    /// Unsubscribe a connection from `topic`. Silent when not subscribed.
    pub fn unsubscribe(&self, id: &ConnectionId, topic: &str) {
        if let Some(mut conn) = self.connections.get_mut(id) {
            conn.topics.remove(topic);
        }
        self.detach_from_topic(topic, id);
    }

    /// Remove `id` from a topic's subscriber set, dropping the topic once it is
    /// empty so idle topics do not accumulate.
    fn detach_from_topic(&self, topic: &str, id: &ConnectionId) {
        if let Some(mut set) = self.topics.get_mut(topic) {
            set.remove(id);
        }
        self.topics.remove_if(topic, |_, set| set.is_empty());
    }

    // ── Publishing ───────────────────────────────────────────────────────────

    /// Deliver `payload` to every subscriber of `topic`, and to the other
    /// instances when clustering is enabled.
    ///
    /// Returns how many sockets on *this* instance received the frame.
    pub fn publish<T: Serialize>(&self, topic: &str, payload: &T) -> Result<usize, WsError> {
        let value = serde_json::to_value(payload)?;
        self.forward_to_cluster(ClusterScope::Topic, topic, &value);
        self.publish_local(topic, &value)
    }

    /// Deliver `payload` to every socket belonging to `user_id`, regardless of
    /// what it is subscribed to.
    pub fn publish_to_user<T: Serialize>(
        &self,
        user_id: Uuid,
        topic: &str,
        payload: &T,
    ) -> Result<usize, WsError> {
        let value = serde_json::to_value(payload)?;
        self.forward_to_cluster(ClusterScope::User { user_id }, topic, &value);
        self.publish_to_user_local(user_id, topic, &value)
    }

    /// Deliver `payload` to every connected socket, subscribed or not.
    ///
    /// Reserve this for genuinely global announcements; prefer topics otherwise.
    pub fn broadcast<T: Serialize>(&self, topic: &str, payload: &T) -> Result<usize, WsError> {
        let value = serde_json::to_value(payload)?;
        self.forward_to_cluster(ClusterScope::All, topic, &value);
        self.broadcast_local(topic, &value)
    }

    /// Send one event to a single connection, bypassing topics entirely.
    ///
    /// Local to this instance — cluster peers are not consulted, because a
    /// connection id only means something on the node that issued it.
    pub fn send_to<T: Serialize>(
        &self,
        id: &ConnectionId,
        topic: &str,
        payload: &T,
    ) -> Result<bool, WsError> {
        let frame = Self::event_frame(topic, &serde_json::to_value(payload)?)?;
        Ok(self.deliver(std::iter::once(*id), &frame) == 1)
    }

    // ── Local delivery (also the landing point for cluster frames) ───────────

    pub(crate) fn publish_local(&self, topic: &str, payload: &Value) -> Result<usize, WsError> {
        let Some(subscribers) = self.topics.get(topic) else {
            return Ok(0);
        };
        // Copy the ids so the shard lock is released before delivery, which may
        // re-enter the map to evict slow clients.
        let ids: Vec<ConnectionId> = subscribers.iter().copied().collect();
        drop(subscribers);

        let frame = Self::event_frame(topic, payload)?;
        Ok(self.deliver(ids, &frame))
    }

    pub(crate) fn publish_to_user_local(
        &self,
        user_id: Uuid,
        topic: &str,
        payload: &Value,
    ) -> Result<usize, WsError> {
        let Some(conns) = self.users.get(&user_id) else {
            return Ok(0);
        };
        let ids: Vec<ConnectionId> = conns.iter().copied().collect();
        drop(conns);

        let frame = Self::event_frame(topic, payload)?;
        Ok(self.deliver(ids, &frame))
    }

    pub(crate) fn broadcast_local(&self, topic: &str, payload: &Value) -> Result<usize, WsError> {
        let ids: Vec<ConnectionId> = self.connections.iter().map(|e| *e.key()).collect();
        let frame = Self::event_frame(topic, payload)?;
        Ok(self.deliver(ids, &frame))
    }

    fn event_frame(topic: &str, payload: &Value) -> Result<ByteString, WsError> {
        ServerMessage::Event {
            topic: topic.to_string(),
            payload: payload.clone(),
        }
        .encode()
    }

    /// Push one encoded frame to each of `ids`.
    ///
    /// `frame` is a [`ByteString`], so every clone is a refcount bump rather
    /// than a copy of the JSON. Connections whose queue is full are collected
    /// and dropped *after* the loop, so no map guard is held while mutating.
    fn deliver(&self, ids: impl IntoIterator<Item = ConnectionId>, frame: &ByteString) -> usize {
        let mut delivered = 0usize;
        let mut slow: Vec<ConnectionId> = Vec::new();

        for id in ids {
            let Some(conn) = self.connections.get(&id) else {
                continue;
            };
            match conn.tx.try_send(frame.clone()) {
                Ok(()) => delivered += 1,
                Err(TrySendError::Full(_)) => slow.push(id),
                // The session task is already tearing down; it will unregister.
                Err(TrySendError::Closed(_)) => {}
            }
        }

        for id in &slow {
            tracing::warn!(
                connection_id = %id,
                buffer = self.config.send_buffer,
                "Websocket outbound buffer full — dropping slow client"
            );
            self.slow_clients_dropped.fetch_add(1, Ordering::Relaxed);
            self.disconnect(id);
        }

        self.messages_sent
            .fetch_add(delivered as u64, Ordering::Relaxed);
        delivered
    }

    /// Hand a published event to the Redis bridge. A no-op when clustering is
    /// off; never blocks.
    fn forward_to_cluster(&self, scope: ClusterScope, topic: &str, payload: &Value) {
        let Some(tx) = &self.cluster_tx else {
            return;
        };

        let frame = ClusterFrame {
            node: self.node_id,
            scope,
            topic: topic.to_string(),
            payload: payload.clone(),
        };

        if tx.try_send(frame).is_err() {
            tracing::warn!(
                topic = %topic,
                "Websocket cluster relay queue is full — event delivered locally only"
            );
        }
    }

    /// Deliver a frame that arrived from another instance. Never re-published to
    /// Redis, which is what keeps the bridge loop-free.
    pub(crate) fn dispatch_cluster_frame(&self, frame: &ClusterFrame) -> Result<usize, WsError> {
        match frame.scope {
            ClusterScope::Topic => self.publish_local(&frame.topic, &frame.payload),
            ClusterScope::User { user_id } => {
                self.publish_to_user_local(user_id, &frame.topic, &frame.payload)
            }
            ClusterScope::All => self.broadcast_local(&frame.topic, &frame.payload),
        }
    }

    // ── Introspection ────────────────────────────────────────────────────────

    /// Live connections on this instance.
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Number of subscribers to `topic` on this instance.
    pub fn subscriber_count(&self, topic: &str) -> usize {
        self.topics.get(topic).map(|s| s.len()).unwrap_or(0)
    }

    /// `true` when `user_id` has at least one socket on this instance.
    ///
    /// With clustering enabled this answers only for the local node — a user
    /// connected to a different instance reads as offline here.
    pub fn is_user_online(&self, user_id: &Uuid) -> bool {
        self.users
            .get(user_id)
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    /// When the given connection was registered.
    pub fn connected_at(&self, id: &ConnectionId) -> Option<DateTime<Utc>> {
        self.connections.get(id).map(|c| c.connected_at)
    }

    /// A snapshot for the stats endpoint.
    pub fn stats(&self) -> HubStats {
        let mut authenticated = 0usize;
        let mut subscriptions = 0usize;
        for entry in self.connections.iter() {
            if entry.user_id.is_some() {
                authenticated += 1;
            }
            subscriptions += entry.topics.len();
        }

        HubStats {
            node_id: self.node_id,
            connections: self.connections.len(),
            authenticated_connections: authenticated,
            topics: self.topics.len(),
            subscriptions,
            messages_sent: self.messages_sent.load(Ordering::Relaxed),
            slow_clients_dropped: self.slow_clients_dropped.load(Ordering::Relaxed),
            cluster_enabled: self.cluster_tx.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config() -> WebSocketConfig {
        WebSocketConfig::default()
    }

    fn hub(config: WebSocketConfig) -> WsHub {
        WsHub::new(config, None)
    }

    /// Pull the next frame off a connection's queue and decode it.
    fn next_event(rx: &mut mpsc::Receiver<ByteString>) -> Value {
        let frame = rx.try_recv().expect("a frame should be queued");
        serde_json::from_str(&frame).expect("frames are valid JSON")
    }

    #[test]
    fn publishes_only_to_subscribers_of_the_topic() {
        let hub = hub(config());
        let (subscriber, mut sub_rx) = hub.register(None).unwrap();
        let (_bystander, mut bystander_rx) = hub.register(None).unwrap();

        hub.subscribe(&subscriber, "prices").unwrap();

        let delivered = hub.publish("prices", &json!({ "btc": 42 })).unwrap();

        assert_eq!(delivered, 1);
        let event = next_event(&mut sub_rx);
        assert_eq!(event["type"], "event");
        assert_eq!(event["topic"], "prices");
        assert_eq!(event["payload"]["btc"], 42);

        // A connection that never subscribed must not see the event.
        assert!(bystander_rx.try_recv().is_err());
    }

    #[test]
    fn unsubscribe_stops_delivery_and_releases_the_topic() {
        let hub = hub(config());
        let (id, mut rx) = hub.register(None).unwrap();

        hub.subscribe(&id, "prices").unwrap();
        hub.unsubscribe(&id, "prices");

        assert_eq!(hub.publish("prices", &json!({})).unwrap(), 0);
        assert!(rx.try_recv().is_err());
        // The topic is dropped once empty, so idle topics cannot accumulate.
        assert_eq!(hub.stats().topics, 0);
    }

    #[test]
    fn unregister_clears_every_index() {
        let hub = hub(config());
        let user_id = Uuid::new_v4();
        let (id, _rx) = hub.register(Some(user_id)).unwrap();
        hub.subscribe(&id, "a").unwrap();
        hub.subscribe(&id, "b").unwrap();

        hub.unregister(&id);

        let stats = hub.stats();
        assert_eq!(stats.connections, 0);
        assert_eq!(stats.topics, 0);
        assert_eq!(stats.subscriptions, 0);
        assert!(!hub.is_user_online(&user_id));

        // Unregistering twice is a no-op, not a panic.
        hub.unregister(&id);
    }

    #[test]
    fn publish_to_user_reaches_every_socket_of_that_user() {
        let hub = hub(config());
        let user_id = Uuid::new_v4();
        let other_user = Uuid::new_v4();

        let (_phone, mut phone_rx) = hub.register(Some(user_id)).unwrap();
        let (_laptop, mut laptop_rx) = hub.register(Some(user_id)).unwrap();
        let (_someone_else, mut other_rx) = hub.register(Some(other_user)).unwrap();

        // No subscription needed: user delivery bypasses topics.
        let delivered = hub
            .publish_to_user(user_id, "notifications", &json!({ "unread": 3 }))
            .unwrap();

        assert_eq!(delivered, 2);
        assert_eq!(next_event(&mut phone_rx)["payload"]["unread"], 3);
        assert_eq!(next_event(&mut laptop_rx)["topic"], "notifications");
        assert!(other_rx.try_recv().is_err());
        assert!(hub.is_user_online(&user_id));
    }

    #[test]
    fn broadcast_reaches_connections_with_no_subscriptions() {
        let hub = hub(config());
        let (_a, mut a_rx) = hub.register(None).unwrap();
        let (_b, mut b_rx) = hub.register(Some(Uuid::new_v4())).unwrap();

        assert_eq!(hub.broadcast("system", &json!({ "msg": "bye" })).unwrap(), 2);
        assert_eq!(next_event(&mut a_rx)["topic"], "system");
        assert_eq!(next_event(&mut b_rx)["payload"]["msg"], "bye");
    }

    #[test]
    fn enforces_the_per_connection_topic_budget() {
        let hub = hub(WebSocketConfig {
            max_topics_per_connection: 2,
            ..config()
        });
        let (id, _rx) = hub.register(None).unwrap();

        hub.subscribe(&id, "one").unwrap();
        hub.subscribe(&id, "two").unwrap();
        // Re-subscribing to a topic already held must not consume budget.
        hub.subscribe(&id, "one").unwrap();

        assert!(matches!(
            hub.subscribe(&id, "three"),
            Err(WsError::TopicLimit { limit: 2 })
        ));
    }

    #[test]
    fn enforces_the_connection_limit() {
        let hub = hub(WebSocketConfig {
            max_connections: 1,
            ..config()
        });

        let (id, _rx) = hub.register(None).unwrap();
        assert!(matches!(
            hub.register(None),
            Err(WsError::ConnectionLimit { limit: 1 })
        ));

        // Capacity is reclaimed when a connection goes away.
        hub.unregister(&id);
        assert!(hub.register(None).is_ok());
    }

    #[test]
    fn drops_a_client_that_will_not_drain_its_queue() {
        let hub = hub(WebSocketConfig {
            send_buffer: 1,
            ..config()
        });
        let (slow, _slow_rx) = hub.register(None).unwrap();
        let (_healthy, mut healthy_rx) = hub.register(None).unwrap();
        hub.subscribe(&slow, "firehose").unwrap();
        hub.subscribe(&_healthy, "firehose").unwrap();

        // First event fits in both buffers.
        assert_eq!(hub.publish("firehose", &json!({ "n": 1 })).unwrap(), 2);
        // The healthy client drains; the slow one does not.
        let _ = healthy_rx.try_recv().unwrap();

        // Second event overflows the slow client, which is evicted rather than
        // being allowed to stall the publisher.
        assert_eq!(hub.publish("firehose", &json!({ "n": 2 })).unwrap(), 1);

        let stats = hub.stats();
        assert_eq!(stats.connections, 1);
        assert_eq!(stats.slow_clients_dropped, 1);
        assert_eq!(hub.subscriber_count("firehose"), 1);
    }

    #[test]
    fn stats_reflect_connections_topics_and_traffic() {
        let hub = hub(config());
        let (anon, _anon_rx) = hub.register(None).unwrap();
        let (authed, _authed_rx) = hub.register(Some(Uuid::new_v4())).unwrap();
        hub.subscribe(&anon, "shared").unwrap();
        hub.subscribe(&authed, "shared").unwrap();
        hub.subscribe(&authed, "private").unwrap();

        hub.publish("shared", &json!({})).unwrap();

        let stats = hub.stats();
        assert_eq!(stats.connections, 2);
        assert_eq!(stats.authenticated_connections, 1);
        assert_eq!(stats.topics, 2);
        assert_eq!(stats.subscriptions, 3);
        assert_eq!(stats.messages_sent, 2);
        assert!(!stats.cluster_enabled);
    }

    #[test]
    fn cluster_frames_are_delivered_locally_without_being_relayed_again() {
        let hub = hub(config());
        let (id, mut rx) = hub.register(None).unwrap();
        hub.subscribe(&id, "prices").unwrap();

        let frame = ClusterFrame {
            node: Uuid::new_v4(), // a peer instance
            scope: ClusterScope::Topic,
            topic: "prices".to_string(),
            payload: json!({ "btc": 7 }),
        };

        assert_eq!(hub.dispatch_cluster_frame(&frame).unwrap(), 1);
        assert_eq!(next_event(&mut rx)["payload"]["btc"], 7);
    }
}
