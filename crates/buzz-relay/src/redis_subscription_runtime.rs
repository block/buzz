//! Process-owned Redis subscription sockets and their relay-side consumers.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use buzz_pubsub::health::{SubscriptionHealth, SubscriptionPath, SubscriptionTransitionReason};
use futures_util::FutureExt;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::AppState;

const OWNED_TASK_COUNT: usize = 6;

#[derive(Clone, Copy)]
enum TaskKind {
    Network,
    Consumer,
}

struct OwnedTask {
    path: SubscriptionPath,
    handle: JoinHandle<()>,
}

/// Owns both halves of every required cross-pod subscription path.
///
/// Each path is ready only when its relay-side consumer is attached and its
/// dedicated Redis connection has completed `SUBSCRIBE` or `PSUBSCRIBE`.
#[must_use = "the Redis subscription runtime must be retained and shut down"]
pub struct RedisSubscriptionRuntime {
    cancel: CancellationToken,
    health: Arc<SubscriptionHealth>,
    tasks: Vec<OwnedTask>,
    shutdown_complete: bool,
}

impl RedisSubscriptionRuntime {
    /// Attach all consumers before starting any Redis socket, then retain all
    /// six task handles for supervision and bounded shutdown.
    pub fn start(state: Arc<AppState>) -> Self {
        let cancel = CancellationToken::new();
        let health = state.pubsub.subscription_health();
        let mut tasks = Vec::with_capacity(OWNED_TASK_COUNT);

        let event_rx = state.pubsub.subscribe_local();
        health.consumer_attached(SubscriptionPath::Event);
        tasks.push(spawn_owned(
            SubscriptionPath::Event,
            TaskKind::Consumer,
            Arc::clone(&health),
            cancel.clone(),
            run_event_consumer(Arc::clone(&state), event_rx, cancel.clone()),
        ));

        let cache_rx = state.pubsub.subscribe_cache_invalidations();
        health.consumer_attached(SubscriptionPath::Cache);
        tasks.push(spawn_owned(
            SubscriptionPath::Cache,
            TaskKind::Consumer,
            Arc::clone(&health),
            cancel.clone(),
            run_cache_consumer(Arc::clone(&state), cache_rx, cancel.clone()),
        ));

        let control_rx = state.pubsub.subscribe_conn_control();
        health.consumer_attached(SubscriptionPath::ConnectionControl);
        tasks.push(spawn_owned(
            SubscriptionPath::ConnectionControl,
            TaskKind::Consumer,
            Arc::clone(&health),
            cancel.clone(),
            run_control_consumer(Arc::clone(&state), control_rx, cancel.clone()),
        ));

        let pubsub = Arc::clone(&state.pubsub);
        tasks.push(spawn_owned(
            SubscriptionPath::Event,
            TaskKind::Network,
            Arc::clone(&health),
            cancel.clone(),
            pubsub.run_subscriber_until_cancelled(cancel.clone()),
        ));

        let pubsub = Arc::clone(&state.pubsub);
        tasks.push(spawn_owned(
            SubscriptionPath::Cache,
            TaskKind::Network,
            Arc::clone(&health),
            cancel.clone(),
            pubsub.run_cache_invalidation_subscriber_until_cancelled(cancel.clone()),
        ));

        let pubsub = Arc::clone(&state.pubsub);
        tasks.push(spawn_owned(
            SubscriptionPath::ConnectionControl,
            TaskKind::Network,
            Arc::clone(&health),
            cancel.clone(),
            pubsub.run_conn_control_subscriber_until_cancelled(cancel.clone()),
        ));

        debug_assert_eq!(tasks.len(), OWNED_TASK_COUNT);
        Self {
            cancel,
            health,
            tasks,
            shutdown_complete: false,
        }
    }

    /// Cancel and await every owned task within one shared graceful deadline.
    /// Tasks that miss it are aborted and joined before shutdown reports completion.
    pub async fn shutdown(mut self, timeout: Duration) {
        self.cancel.cancel();
        let deadline = tokio::time::Instant::now() + timeout;
        let mut timed_out = Vec::new();
        for mut task in self.tasks.drain(..) {
            if tokio::time::timeout_at(deadline, &mut task.handle)
                .await
                .is_err()
            {
                task.handle.abort();
                self.health
                    .failed(task.path, SubscriptionTransitionReason::ShutdownTimeout);
                timed_out.push(task);
            }
        }

        for task in timed_out {
            let _ = task.handle.await;
        }

        for path in SubscriptionPath::ALL {
            self.health.stopped(path);
        }
        self.shutdown_complete = true;
    }
}

impl Drop for RedisSubscriptionRuntime {
    fn drop(&mut self) {
        if self.shutdown_complete {
            return;
        }
        self.cancel.cancel();
        for task in &self.tasks {
            task.handle.abort();
        }
        for path in SubscriptionPath::ALL {
            self.health
                .failed(path, SubscriptionTransitionReason::OwnerDropped);
        }
    }
}

fn spawn_owned(
    path: SubscriptionPath,
    kind: TaskKind,
    health: Arc<SubscriptionHealth>,
    cancel: CancellationToken,
    future: impl Future<Output = ()> + Send + 'static,
) -> OwnedTask {
    let handle = tokio::spawn(async move {
        let result = AssertUnwindSafe(future).catch_unwind().await;
        if cancel.is_cancelled() {
            return;
        }
        let reason = match result {
            Err(_) => SubscriptionTransitionReason::Panic,
            Ok(()) => match kind {
                TaskKind::Network => SubscriptionTransitionReason::StreamClosed,
                TaskKind::Consumer => SubscriptionTransitionReason::ConsumerClosed,
            },
        };
        health.failed(path, reason);
    });
    OwnedTask { path, handle }
}

async fn run_event_consumer(
    state: Arc<AppState>,
    mut rx: tokio::sync::broadcast::Receiver<buzz_pubsub::ChannelEvent>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            result = rx.recv() => match result {
                Ok(channel_event) => {
                    crate::handlers::event::fan_out_pubsub_event(&state, channel_event).await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    metrics::counter!("buzz_multinode_fanout_lag_total").increment(n);
                    tracing::warn!("Multi-node fan-out lagged by {n} messages");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    }
}

async fn run_cache_consumer(
    state: Arc<AppState>,
    mut rx: tokio::sync::broadcast::Receiver<
        buzz_pubsub::cache_invalidation::ScopedCacheInvalidation,
    >,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            result = rx.recv() => match result {
                Ok(scoped) => {
                    state.apply_cache_invalidation(scoped.community_id, scoped.invalidation);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    metrics::counter!("buzz_cache_invalidation_lag_total").increment(n);
                    tracing::warn!("Cache-invalidation consumer lagged by {n} messages");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    }
}

async fn run_control_consumer(
    state: Arc<AppState>,
    mut rx: tokio::sync::broadcast::Receiver<buzz_pubsub::conn_control::ScopedConnControl>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            result = rx.recv() => match result {
                Ok(scoped) => match scoped.command {
                    buzz_pubsub::conn_control::ConnControl::DisconnectCommunity => {
                        state.community_connections.disconnect_community(scoped.community_id);
                    }
                    buzz_pubsub::conn_control::ConnControl::DisconnectPubkey {
                        pubkey,
                        event_id,
                        reason,
                    } => {
                        state.conn_manager.disconnect_pubkey(
                            scoped.community_id,
                            &pubkey,
                            &event_id,
                            &reason,
                        );
                    }
                },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    metrics::counter!("buzz_conn_control_lag_total").increment(n);
                    tracing::warn!("Connection-control consumer lagged by {n} messages");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_pubsub::health::SubscriptionPathState;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn production_runtime_owns_both_halves_and_attaches_consumers_first() {
        let state = crate::state::tests::test_state().await;
        let health = state.pubsub.subscription_health();
        let runtime = RedisSubscriptionRuntime::start(state);

        assert_eq!(runtime.tasks.len(), OWNED_TASK_COUNT);
        for path in SubscriptionPath::ALL {
            assert!(health.snapshot(path).consumer_attached);
        }

        for path in SubscriptionPath::ALL {
            health
                .wait_for_state(
                    path,
                    SubscriptionPathState::Reconnecting,
                    Duration::from_secs(2),
                )
                .await
                .expect("unreachable Redis must be observable as reconnecting");
        }

        runtime.shutdown(Duration::from_secs(1)).await;
        for path in SubscriptionPath::ALL {
            assert_eq!(health.snapshot(path).state, SubscriptionPathState::Stopped);
        }
    }

    #[tokio::test]
    async fn dropping_owner_latches_every_path_failed() {
        let state = crate::state::tests::test_state().await;
        let health = state.pubsub.subscription_health();
        let runtime = RedisSubscriptionRuntime::start(state);

        drop(runtime);

        for path in SubscriptionPath::ALL {
            assert_eq!(health.snapshot(path).state, SubscriptionPathState::Failed);
        }
    }

    #[tokio::test]
    async fn owned_task_panic_latches_path_failed() {
        let health = Arc::new(SubscriptionHealth::new());
        let task = spawn_owned(
            SubscriptionPath::Event,
            TaskKind::Consumer,
            Arc::clone(&health),
            CancellationToken::new(),
            async { panic!("test panic") },
        );

        task.handle.await.expect("panic is supervised inside task");
        assert_eq!(
            health.snapshot(SubscriptionPath::Event).state,
            SubscriptionPathState::Failed
        );
    }

    #[tokio::test]
    async fn unexpected_consumer_return_latches_path_failed() {
        let health = Arc::new(SubscriptionHealth::new());
        let task = spawn_owned(
            SubscriptionPath::Cache,
            TaskKind::Consumer,
            Arc::clone(&health),
            CancellationToken::new(),
            async {},
        );

        task.handle.await.expect("task join");
        assert_eq!(
            health.snapshot(SubscriptionPath::Cache).state,
            SubscriptionPathState::Failed
        );
    }

    #[tokio::test]
    async fn shutdown_joins_every_task_after_aborting_timed_out_work() {
        let cancel = CancellationToken::new();
        let health = Arc::new(SubscriptionHealth::new());
        let event_dropped = Arc::new(AtomicBool::new(false));
        let cache_dropped = Arc::new(AtomicBool::new(false));

        let (event_started_tx, event_started_rx) = tokio::sync::oneshot::channel();
        let event_flag = Arc::clone(&event_dropped);
        let event_task = spawn_owned(
            SubscriptionPath::Event,
            TaskKind::Network,
            Arc::clone(&health),
            cancel.clone(),
            async move {
                let _drop_flag = DropFlag(event_flag);
                event_started_tx
                    .send(())
                    .expect("signal event task started");
                std::future::pending::<()>().await;
            },
        );

        let (cache_started_tx, cache_started_rx) = tokio::sync::oneshot::channel();
        let cache_flag = Arc::clone(&cache_dropped);
        let cache_task = spawn_owned(
            SubscriptionPath::Cache,
            TaskKind::Consumer,
            Arc::clone(&health),
            cancel.clone(),
            async move {
                let _drop_flag = DropFlag(cache_flag);
                cache_started_tx
                    .send(())
                    .expect("signal cache task started");
                std::future::pending::<()>().await;
            },
        );

        event_started_rx.await.expect("event task started");
        cache_started_rx.await.expect("cache task started");

        RedisSubscriptionRuntime {
            cancel,
            health: Arc::clone(&health),
            tasks: vec![event_task, cache_task],
            shutdown_complete: false,
        }
        .shutdown(Duration::ZERO)
        .await;

        assert!(
            event_dropped.load(Ordering::SeqCst),
            "event resource must be released before shutdown returns"
        );
        assert!(
            cache_dropped.load(Ordering::SeqCst),
            "cache resource must be released before shutdown returns"
        );
        assert_eq!(
            health.snapshot(SubscriptionPath::Event).state,
            SubscriptionPathState::Failed
        );
        assert_eq!(
            health.snapshot(SubscriptionPath::Cache).state,
            SubscriptionPathState::Failed
        );
    }
}
