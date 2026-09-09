//! Fixed-cardinality health for the relay's cross-pod Redis subscription paths.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::Notify;
use tokio::time::Instant;
use uuid::Uuid;

/// Number of required cross-pod Redis subscription paths in every relay pod.
pub const REQUIRED_PATH_COUNT: usize = 3;

/// Every required cross-pod Redis subscription path.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SubscriptionPath {
    /// Nostr event delivery from Redis to local WebSocket subscribers.
    Event,
    /// Membership and visibility cache invalidation from other relay pods.
    Cache,
    /// Live connection-control commands from other relay pods.
    ConnectionControl,
}

impl SubscriptionPath {
    /// Complete fixed path vocabulary.
    pub const ALL: [Self; REQUIRED_PATH_COUNT] =
        [Self::Event, Self::Cache, Self::ConnectionControl];

    /// Stable metric and lifecycle value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Event => "event",
            Self::Cache => "cache",
            Self::ConnectionControl => "connection_control",
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::Event => 0,
            Self::Cache => 1,
            Self::ConnectionControl => 2,
        }
    }
}

/// Current end-to-end state of one Redis subscription path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubscriptionPathState {
    /// The downstream consumer is attached but the first Redis handshake is incomplete.
    Connecting,
    /// The downstream consumer is alive and the Redis subscription handshake completed.
    Ready,
    /// The Redis connection or subscription failed and its retry backoff is active.
    Reconnecting,
    /// An owned network or consumer task exited unexpectedly.
    Failed,
    /// Explicit relay shutdown completed for the path.
    Stopped,
}

impl SubscriptionPathState {
    /// Complete fixed state vocabulary.
    pub const ALL: [Self; 5] = [
        Self::Connecting,
        Self::Ready,
        Self::Reconnecting,
        Self::Failed,
        Self::Stopped,
    ];

    /// Stable metric and lifecycle value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Ready => "ready",
            Self::Reconnecting => "reconnecting",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// Bounded transition reason; never contains Redis endpoints, topics, or raw errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubscriptionTransitionReason {
    /// The Redis connection could not be established.
    Dial,
    /// A `SUBSCRIBE`, `PSUBSCRIBE`, or dynamic subscription command failed.
    Subscribe,
    /// An established Redis message stream ended.
    StreamClosed,
    /// The relay-side broadcast consumer closed unexpectedly.
    ConsumerClosed,
    /// An owned task panicked.
    Panic,
    /// Explicit relay shutdown cancelled the task.
    Cancelled,
    /// The runtime owner disappeared without completing shutdown.
    OwnerDropped,
    /// Bounded shutdown did not finish before its deadline.
    ShutdownTimeout,
}

impl SubscriptionTransitionReason {
    /// Stable metric and lifecycle value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dial => "dial",
            Self::Subscribe => "subscribe",
            Self::StreamClosed => "stream_closed",
            Self::ConsumerClosed => "consumer_closed",
            Self::Panic => "panic",
            Self::Cancelled => "cancelled",
            Self::OwnerDropped => "owner_dropped",
            Self::ShutdownTimeout => "shutdown_timeout",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SubscriptionTransition {
    Connected,
    ConnectFailed,
    Disconnected,
    Reconnected,
    Terminal,
}

impl SubscriptionTransition {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::ConnectFailed => "connect_failed",
            Self::Disconnected => "disconnected",
            Self::Reconnected => "reconnected",
            Self::Terminal => "terminal",
        }
    }
}

/// Read-only state for one subscription path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubscriptionPathSnapshot {
    /// Current end-to-end path state.
    pub state: SubscriptionPathState,
    /// Whether the relay-side consumer was attached before Redis could deliver.
    pub consumer_attached: bool,
    /// Unix timestamp of the latest completed end-to-end ready transition.
    pub last_ready_timestamp_seconds: Option<u64>,
    /// Time spent reaching the latest completed end-to-end ready transition.
    pub last_readiness_duration: Option<Duration>,
    /// Time spent continuously outside `ready`; zero while ready.
    pub not_ready_duration: Duration,
    /// Time spent in the current state.
    pub state_duration: Duration,
    /// Consecutive failed connection/subscription attempts since the latest ready state.
    pub consecutive_failures: u64,
}

#[derive(Clone, Copy, Debug)]
struct PathStatus {
    state: SubscriptionPathState,
    consumer_attached: bool,
    network_ready: bool,
    ever_ready: bool,
    terminal: bool,
    last_ready_timestamp_seconds: Option<u64>,
    last_readiness_duration: Option<Duration>,
    not_ready_since: Instant,
    state_since: Instant,
    consecutive_failures: u64,
}

impl PathStatus {
    fn initial(now: Instant) -> Self {
        Self {
            state: SubscriptionPathState::Connecting,
            consumer_attached: false,
            network_ready: false,
            ever_ready: false,
            terminal: false,
            last_ready_timestamp_seconds: None,
            last_readiness_duration: None,
            not_ready_since: now,
            state_since: now,
            consecutive_failures: 0,
        }
    }

    fn snapshot(self, now: Instant) -> SubscriptionPathSnapshot {
        SubscriptionPathSnapshot {
            state: self.state,
            consumer_attached: self.consumer_attached,
            last_ready_timestamp_seconds: self.last_ready_timestamp_seconds,
            last_readiness_duration: self.last_readiness_duration,
            not_ready_duration: if self.state == SubscriptionPathState::Ready {
                Duration::ZERO
            } else {
                now.saturating_duration_since(self.not_ready_since)
            },
            state_duration: now.saturating_duration_since(self.state_since),
            consecutive_failures: self.consecutive_failures,
        }
    }

    fn set_state(&mut self, state: SubscriptionPathState, now: Instant) {
        if self.state != state {
            self.state = state;
            self.state_since = now;
        }
    }

    fn mark_ready(&mut self, now: Instant) {
        self.last_readiness_duration = Some(now.saturating_duration_since(self.not_ready_since));
        self.set_state(SubscriptionPathState::Ready, now);
        self.ever_ready = true;
        self.last_ready_timestamp_seconds = Some(unix_timestamp_seconds());
        self.consecutive_failures = 0;
    }

    fn mark_not_ready(&mut self, state: SubscriptionPathState, now: Instant) {
        if self.state == SubscriptionPathState::Ready {
            self.not_ready_since = now;
        }
        self.set_state(state, now);
    }
}

/// Shared health snapshot and metric publisher for all Redis subscription paths.
pub struct SubscriptionHealth {
    group_id: Uuid,
    sequence: AtomicU64,
    statuses: Mutex<[PathStatus; REQUIRED_PATH_COUNT]>,
    publish_lock: Mutex<()>,
    changed: Notify,
}

impl Default for SubscriptionHealth {
    fn default() -> Self {
        Self::new()
    }
}

impl SubscriptionHealth {
    /// Create the per-process fixed path snapshot.
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            group_id: Uuid::new_v4(),
            sequence: AtomicU64::new(1),
            statuses: Mutex::new([PathStatus::initial(now); REQUIRED_PATH_COUNT]),
            publish_lock: Mutex::new(()),
            changed: Notify::new(),
        }
    }

    /// Record that the downstream relay consumer is attached and able to buffer messages.
    pub fn consumer_attached(&self, path: SubscriptionPath) {
        let _publish_guard = self.lock_publish();
        let mut transition = None;
        let snapshot = {
            let mut statuses = self.lock_statuses();
            let status = &mut statuses[path.index()];
            if status.terminal || status.consumer_attached {
                return;
            }
            status.consumer_attached = true;
            if status.network_ready {
                transition = Some(if status.ever_ready {
                    SubscriptionTransition::Reconnected
                } else {
                    SubscriptionTransition::Connected
                });
                status.mark_ready(Instant::now());
            }
            status.snapshot(Instant::now())
        };
        self.publish(
            path,
            snapshot,
            transition,
            transition.map(|_| SubscriptionTransitionReason::Subscribe),
        );
    }

    /// Record that a Redis connection/subscription attempt has started.
    pub fn connecting(&self, path: SubscriptionPath) {
        if self.update_nonterminal(path, |status| {
            status.network_ready = false;
            status.mark_not_ready(SubscriptionPathState::Connecting, Instant::now());
            (None, None)
        }) {
            metrics::counter!(
                "buzz_redis_subscription_attempts_total",
                "path" => path.as_str(),
            )
            .increment(1);
        }
    }

    /// Record a completed Redis subscription handshake.
    pub fn network_ready(&self, path: SubscriptionPath) {
        self.update_nonterminal(path, |status| {
            status.network_ready = true;
            if !status.consumer_attached {
                return (None, None);
            }
            let transition = if status.ever_ready {
                SubscriptionTransition::Reconnected
            } else {
                SubscriptionTransition::Connected
            };
            status.mark_ready(Instant::now());
            (
                Some(transition),
                Some(SubscriptionTransitionReason::Subscribe),
            )
        });
    }

    /// Record a failed connection/subscription or an established stream closure.
    pub fn reconnecting(&self, path: SubscriptionPath, reason: SubscriptionTransitionReason) {
        self.update_nonterminal(path, |status| {
            status.network_ready = false;
            status.mark_not_ready(SubscriptionPathState::Reconnecting, Instant::now());
            status.consecutive_failures = status.consecutive_failures.saturating_add(1);
            let transition = if status.ever_ready {
                SubscriptionTransition::Disconnected
            } else {
                SubscriptionTransition::ConnectFailed
            };
            (Some(transition), Some(reason))
        });
    }

    /// Record an unexpected terminal task outcome. Failure is latched until process exit.
    pub fn failed(&self, path: SubscriptionPath, reason: SubscriptionTransitionReason) {
        self.update(path, |status| {
            status.network_ready = false;
            status.mark_not_ready(SubscriptionPathState::Failed, Instant::now());
            status.terminal = true;
            (Some(SubscriptionTransition::Terminal), Some(reason))
        });
    }

    /// Record successful explicit shutdown unless the path already failed.
    pub fn stopped(&self, path: SubscriptionPath) {
        self.update_nonterminal(path, |status| {
            status.network_ready = false;
            status.consumer_attached = false;
            status.mark_not_ready(SubscriptionPathState::Stopped, Instant::now());
            status.terminal = true;
            (
                Some(SubscriptionTransition::Terminal),
                Some(SubscriptionTransitionReason::Cancelled),
            )
        });
    }

    /// Return the current state for one path.
    pub fn snapshot(&self, path: SubscriptionPath) -> SubscriptionPathSnapshot {
        self.lock_statuses()[path.index()].snapshot(Instant::now())
    }

    /// Return all path snapshots in [`SubscriptionPath::ALL`] order.
    pub fn snapshots(&self) -> [SubscriptionPathSnapshot; REQUIRED_PATH_COUNT] {
        let statuses = self.lock_statuses();
        let now = Instant::now();
        [
            statuses[0].snapshot(now),
            statuses[1].snapshot(now),
            statuses[2].snapshot(now),
        ]
    }

    /// Number of required paths whose end-to-end state is currently ready.
    pub fn ready_path_count(&self) -> usize {
        self.lock_statuses()
            .iter()
            .filter(|status| status.state == SubscriptionPathState::Ready)
            .count()
    }

    /// Whether every required path is currently ready.
    pub fn all_ready(&self) -> bool {
        self.ready_path_count() == REQUIRED_PATH_COUNT
    }

    /// Refresh current gauges so stable paths survive the recorder's idle timeout.
    pub fn refresh_metrics(&self) {
        let _publish_guard = self.lock_publish();
        let snapshots = self.snapshots();
        for (index, path) in SubscriptionPath::ALL.iter().copied().enumerate() {
            publish_path_gauges(path, snapshots[index]);
        }
        publish_aggregate_gauges(&snapshots);
    }

    /// Wait without polling or sleeps until one path reaches `expected`.
    pub async fn wait_for_state(
        &self,
        path: SubscriptionPath,
        expected: SubscriptionPathState,
        timeout: Duration,
    ) -> Result<SubscriptionPathSnapshot, tokio::time::error::Elapsed> {
        tokio::time::timeout(timeout, async {
            loop {
                let changed = self.changed.notified();
                let snapshot = self.snapshot(path);
                if snapshot.state == expected {
                    return snapshot;
                }
                changed.await;
            }
        })
        .await
    }

    fn update_nonterminal(
        &self,
        path: SubscriptionPath,
        mutate: impl FnOnce(
            &mut PathStatus,
        ) -> (
            Option<SubscriptionTransition>,
            Option<SubscriptionTransitionReason>,
        ),
    ) -> bool {
        self.update(path, mutate)
    }

    fn update(
        &self,
        path: SubscriptionPath,
        mutate: impl FnOnce(
            &mut PathStatus,
        ) -> (
            Option<SubscriptionTransition>,
            Option<SubscriptionTransitionReason>,
        ),
    ) -> bool {
        let _publish_guard = self.lock_publish();
        let (snapshot, transition, reason) = {
            let mut statuses = self.lock_statuses();
            let status = &mut statuses[path.index()];
            if status.terminal {
                return false;
            }
            let (transition, reason) = mutate(status);
            (status.snapshot(Instant::now()), transition, reason)
        };
        self.publish(path, snapshot, transition, reason);
        true
    }

    fn publish(
        &self,
        path: SubscriptionPath,
        snapshot: SubscriptionPathSnapshot,
        transition: Option<SubscriptionTransition>,
        reason: Option<SubscriptionTransitionReason>,
    ) {
        publish_path_gauges(path, snapshot);
        let snapshots = self.snapshots();
        publish_aggregate_gauges(&snapshots);
        if let (Some(transition), Some(reason)) = (transition, reason) {
            metrics::counter!(
                "buzz_redis_subscription_transitions_total",
                "path" => path.as_str(),
                "transition" => transition.as_str(),
                "reason" => reason.as_str(),
            )
            .increment(1);
        }
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        tracing::info!(
            event_name = "buzz_redis_subscription_lifecycle",
            schema_version = 1_u64,
            subscription_group_id = %self.group_id,
            sequence,
            path = path.as_str(),
            state = snapshot.state.as_str(),
            consumer_attached = snapshot.consumer_attached,
            transition = transition.map(SubscriptionTransition::as_str).unwrap_or("none"),
            reason = reason.map(SubscriptionTransitionReason::as_str).unwrap_or("none"),
            "Redis subscription path lifecycle"
        );
        self.changed.notify_waiters();
    }

    fn lock_statuses(&self) -> std::sync::MutexGuard<'_, [PathStatus; REQUIRED_PATH_COUNT]> {
        self.statuses
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock_publish(&self) -> std::sync::MutexGuard<'_, ()> {
        self.publish_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn publish_path_gauges(path: SubscriptionPath, snapshot: SubscriptionPathSnapshot) {
    for state in SubscriptionPathState::ALL {
        metrics::gauge!(
            "buzz_redis_subscription_path_state",
            "path" => path.as_str(),
            "state" => state.as_str(),
        )
        .set(if snapshot.state == state { 1.0 } else { 0.0 });
    }
    if let Some(timestamp) = snapshot.last_ready_timestamp_seconds {
        metrics::gauge!(
            "buzz_redis_subscription_last_ready_timestamp_seconds",
            "path" => path.as_str(),
        )
        .set(timestamp as f64);
    }
    if let Some(duration) = snapshot.last_readiness_duration {
        metrics::gauge!(
            "buzz_redis_subscription_last_readiness_duration_seconds",
            "path" => path.as_str(),
        )
        .set(duration.as_secs_f64());
    }
    metrics::gauge!(
        "buzz_redis_subscription_not_ready_duration_seconds",
        "path" => path.as_str(),
    )
    .set(snapshot.not_ready_duration.as_secs_f64());
    metrics::gauge!(
        "buzz_redis_subscription_state_duration_seconds",
        "path" => path.as_str(),
    )
    .set(snapshot.state_duration.as_secs_f64());
    metrics::gauge!(
        "buzz_redis_subscription_consecutive_failures",
        "path" => path.as_str(),
    )
    .set(snapshot.consecutive_failures as f64);
}

fn publish_aggregate_gauges(snapshots: &[SubscriptionPathSnapshot; REQUIRED_PATH_COUNT]) {
    let ready = snapshots
        .iter()
        .filter(|snapshot| snapshot.state == SubscriptionPathState::Ready)
        .count();
    metrics::gauge!("buzz_redis_subscription_paths_ready").set(ready as f64);
    metrics::gauge!("buzz_redis_subscription_all_ready").set(if ready == REQUIRED_PATH_COUNT {
        1.0
    } else {
        0.0
    });
}

fn unix_timestamp_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn ready_requires_both_consumer_and_network() {
        let health = SubscriptionHealth::new();
        health.network_ready(SubscriptionPath::Event);
        assert_eq!(
            health.snapshot(SubscriptionPath::Event).state,
            SubscriptionPathState::Connecting
        );

        health.consumer_attached(SubscriptionPath::Event);
        let snapshot = health.snapshot(SubscriptionPath::Event);
        assert_eq!(snapshot.state, SubscriptionPathState::Ready);
        assert!(snapshot.consumer_attached);
        assert!(snapshot.last_ready_timestamp_seconds.is_some());
    }

    #[test]
    fn all_ready_requires_all_three_paths() {
        let health = SubscriptionHealth::new();
        for path in SubscriptionPath::ALL {
            health.consumer_attached(path);
            health.network_ready(path);
        }
        assert_eq!(health.ready_path_count(), REQUIRED_PATH_COUNT);
        assert!(health.all_ready());
    }

    #[test]
    fn failure_is_latched_against_later_network_updates() {
        let health = SubscriptionHealth::new();
        health.consumer_attached(SubscriptionPath::Cache);
        health.failed(
            SubscriptionPath::Cache,
            SubscriptionTransitionReason::ConsumerClosed,
        );
        health.network_ready(SubscriptionPath::Cache);
        assert_eq!(
            health.snapshot(SubscriptionPath::Cache).state,
            SubscriptionPathState::Failed
        );
    }

    #[tokio::test(start_paused = true)]
    async fn readiness_duration_includes_retries_and_resets_after_recovery() {
        let health = SubscriptionHealth::new();
        health.consumer_attached(SubscriptionPath::Event);
        health.connecting(SubscriptionPath::Event);
        tokio::time::advance(Duration::from_secs(2)).await;
        health.reconnecting(SubscriptionPath::Event, SubscriptionTransitionReason::Dial);
        tokio::time::advance(Duration::from_secs(3)).await;
        health.connecting(SubscriptionPath::Event);
        tokio::time::advance(Duration::from_secs(4)).await;
        health.network_ready(SubscriptionPath::Event);

        let initial = health.snapshot(SubscriptionPath::Event);
        assert_eq!(
            initial.last_readiness_duration,
            Some(Duration::from_secs(9))
        );
        assert_eq!(initial.not_ready_duration, Duration::ZERO);
        assert_eq!(initial.consecutive_failures, 0);

        health.reconnecting(
            SubscriptionPath::Event,
            SubscriptionTransitionReason::StreamClosed,
        );
        tokio::time::advance(Duration::from_secs(5)).await;
        let recovering = health.snapshot(SubscriptionPath::Event);
        assert_eq!(recovering.not_ready_duration, Duration::from_secs(5));
        assert_eq!(recovering.state_duration, Duration::from_secs(5));
        assert_eq!(recovering.consecutive_failures, 1);

        health.connecting(SubscriptionPath::Event);
        tokio::time::advance(Duration::from_secs(2)).await;
        health.network_ready(SubscriptionPath::Event);
        let recovered = health.snapshot(SubscriptionPath::Event);
        assert_eq!(
            recovered.last_readiness_duration,
            Some(Duration::from_secs(7))
        );
        assert_eq!(recovered.consecutive_failures, 0);
    }

    #[tokio::test]
    async fn wait_for_state_observes_transition_without_sleep() {
        let health = Arc::new(SubscriptionHealth::new());
        let waiter_health = Arc::clone(&health);
        let waiter = tokio::spawn(async move {
            waiter_health
                .wait_for_state(
                    SubscriptionPath::ConnectionControl,
                    SubscriptionPathState::Ready,
                    Duration::from_secs(1),
                )
                .await
        });
        health.consumer_attached(SubscriptionPath::ConnectionControl);
        health.network_ready(SubscriptionPath::ConnectionControl);
        let snapshot = waiter.await.expect("waiter task").expect("ready state");
        assert_eq!(snapshot.state, SubscriptionPathState::Ready);
    }
}
