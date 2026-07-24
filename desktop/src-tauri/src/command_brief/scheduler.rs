use std::collections::{BTreeSet, VecDeque};
use std::fmt;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};

use chrono::{SecondsFormat, Utc};
use futures_util::FutureExt;
use tokio::sync::{broadcast, Semaphore};
use tokio_util::sync::CancellationToken;

use super::types::AdviserId;

const MAX_RUN_ID_BYTES: usize = 256;
const MAX_LIFECYCLE_EVENTS: usize = 256;
const LIFECYCLE_CHANNEL_CAPACITY: usize = 256;

/// One immutable run/adviser identity admitted to the local model queue.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SchedulerJobKey {
    run_id: String,
    adviser: AdviserId,
}

impl SchedulerJobKey {
    /// Creates a bounded job identity. The closed adviser enum prevents
    /// renderer-controlled persona names from entering the scheduler.
    pub fn new(run_id: &str, adviser: AdviserId) -> Result<Self, SchedulerConfigurationError> {
        if run_id.is_empty()
            || run_id.trim() != run_id
            || run_id.len() > MAX_RUN_ID_BYTES
            || run_id.chars().any(char::is_control)
        {
            return Err(SchedulerConfigurationError);
        }
        Ok(Self {
            run_id: run_id.to_string(),
            adviser,
        })
    }

    /// Returns the trusted run identity.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Returns the closed adviser identity.
    pub const fn adviser(&self) -> AdviserId {
        self.adviser
    }
}

/// Stable scheduler lifecycle states. They contain no model content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerLifecycleState {
    Queued,
    Running,
    Completed,
    Cancelled,
    Failed,
}

/// Metadata-only lifecycle event emitted by the app-owned scheduler.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchedulerLifecycleEvent {
    key: SchedulerJobKey,
    state: SchedulerLifecycleState,
    occurred_at: String,
}

impl SchedulerLifecycleEvent {
    /// Returns the immutable job identity.
    pub fn key(&self) -> &SchedulerJobKey {
        &self.key
    }

    /// Returns the scheduler transition.
    pub const fn state(&self) -> SchedulerLifecycleState {
        self.state
    }

    /// Returns the trusted local transition timestamp.
    pub fn occurred_at(&self) -> &str {
        &self.occurred_at
    }
}

/// Invalid capacity or key configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SchedulerConfigurationError;

impl fmt::Display for SchedulerConfigurationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid local model scheduler configuration")
    }
}

impl std::error::Error for SchedulerConfigurationError {}

/// One isolated scheduler result. Panic payloads are deliberately discarded.
#[derive(Debug, Eq, PartialEq)]
pub enum SchedulerError<E> {
    Duplicate,
    Cancelled,
    Panicked,
    Task(E),
    Unavailable,
}

struct SchedulerInner {
    capacity: u8,
    semaphore: Arc<Semaphore>,
    active: Mutex<BTreeSet<SchedulerJobKey>>,
    history: Mutex<VecDeque<SchedulerLifecycleEvent>>,
    lifecycle_sender: broadcast::Sender<SchedulerLifecycleEvent>,
}

/// One app-owned FIFO scheduler for the single configured local model.
#[derive(Clone)]
pub struct LocalModelScheduler {
    inner: Arc<SchedulerInner>,
}

impl fmt::Debug for LocalModelScheduler {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalModelScheduler")
            .field(
                "available_permits",
                &self.inner.semaphore.available_permits(),
            )
            .finish_non_exhaustive()
    }
}

impl LocalModelScheduler {
    /// Creates the default capacity-one scheduler without a fallible boundary.
    pub fn sequential() -> Self {
        let (lifecycle_sender, _) = broadcast::channel(LIFECYCLE_CHANNEL_CAPACITY);
        Self {
            inner: Arc::new(SchedulerInner {
                capacity: 1,
                semaphore: Arc::new(Semaphore::new(1)),
                active: Mutex::new(BTreeSet::new()),
                history: Mutex::new(VecDeque::new()),
                lifecycle_sender,
            }),
        }
    }

    /// Creates the only supported capacities: sequential `1`, or bounded `2`.
    pub fn new(capacity: u8) -> Result<Self, SchedulerConfigurationError> {
        if !matches!(capacity, 1 | 2) {
            return Err(SchedulerConfigurationError);
        }
        if capacity == 1 {
            return Ok(Self::sequential());
        }
        let (lifecycle_sender, _) = broadcast::channel(LIFECYCLE_CHANNEL_CAPACITY);
        Ok(Self {
            inner: Arc::new(SchedulerInner {
                capacity,
                semaphore: Arc::new(Semaphore::new(capacity as usize)),
                active: Mutex::new(BTreeSet::new()),
                history: Mutex::new(VecDeque::new()),
                lifecycle_sender,
            }),
        })
    }

    /// Returns the immutable configured model concurrency.
    pub fn capacity(&self) -> u8 {
        self.inner.capacity
    }

    /// Returns the number of queued or running unique jobs.
    pub fn active_job_count(&self) -> usize {
        self.inner.active.lock().map_or(0, |active| active.len())
    }

    /// Subscribes to metadata-only queue lifecycle changes.
    pub fn subscribe(&self) -> broadcast::Receiver<SchedulerLifecycleEvent> {
        self.inner.lifecycle_sender.subscribe()
    }

    /// Returns the bounded in-memory lifecycle history.
    pub fn lifecycle_history(&self) -> Vec<SchedulerLifecycleEvent> {
        self.inner
            .history
            .lock()
            .map(|history| history.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Queues one abort-aware unit of local-model work.
    ///
    /// Tokio's semaphore wait queue is FIFO. Once running, this method never
    /// aborts the supplied future: cancellation is propagated through its
    /// token and the permit remains held until the future settles.
    pub async fn schedule<T, E, Work, WorkFuture>(
        &self,
        key: SchedulerJobKey,
        cancellation: CancellationToken,
        work: Work,
    ) -> Result<T, SchedulerError<E>>
    where
        Work: FnOnce(CancellationToken) -> WorkFuture + Send,
        WorkFuture: Future<Output = Result<T, E>> + Send,
        T: Send,
        E: Send,
    {
        self.insert_active(&key)?;
        let active_guard = ActiveJobGuard {
            inner: Arc::clone(&self.inner),
            key: key.clone(),
        };
        self.emit(key.clone(), SchedulerLifecycleState::Queued);

        let permit = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                self.emit(key, SchedulerLifecycleState::Cancelled);
                return Err(SchedulerError::Cancelled);
            }
            permit = Arc::clone(&self.inner.semaphore).acquire_owned() => {
                permit.map_err(|_| SchedulerError::Unavailable)?
            }
        };
        if cancellation.is_cancelled() {
            self.emit(key, SchedulerLifecycleState::Cancelled);
            return Err(SchedulerError::Cancelled);
        }
        self.emit(key.clone(), SchedulerLifecycleState::Running);

        let work_cancellation = cancellation.clone();
        let settled = AssertUnwindSafe(async move { work(work_cancellation).await })
            .catch_unwind()
            .await;
        drop(permit);
        drop(active_guard);

        if cancellation.is_cancelled() {
            self.emit(key, SchedulerLifecycleState::Cancelled);
            return Err(SchedulerError::Cancelled);
        }
        match settled {
            Ok(Ok(value)) => {
                self.emit(key, SchedulerLifecycleState::Completed);
                Ok(value)
            }
            Ok(Err(error)) => {
                self.emit(key, SchedulerLifecycleState::Failed);
                Err(SchedulerError::Task(error))
            }
            Err(_) => {
                self.emit(key, SchedulerLifecycleState::Failed);
                Err(SchedulerError::Panicked)
            }
        }
    }

    fn insert_active<E>(&self, key: &SchedulerJobKey) -> Result<(), SchedulerError<E>> {
        let mut active = self
            .inner
            .active
            .lock()
            .map_err(|_| SchedulerError::Unavailable)?;
        if !active.insert(key.clone()) {
            return Err(SchedulerError::Duplicate);
        }
        Ok(())
    }

    fn emit(&self, key: SchedulerJobKey, state: SchedulerLifecycleState) {
        let event = SchedulerLifecycleEvent {
            key,
            state,
            occurred_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        };
        if let Ok(mut history) = self.inner.history.lock() {
            if history.len() == MAX_LIFECYCLE_EVENTS {
                history.pop_front();
            }
            history.push_back(event.clone());
        }
        let _ = self.inner.lifecycle_sender.send(event);
    }
}

struct ActiveJobGuard {
    inner: Arc<SchedulerInner>,
    key: SchedulerJobKey,
}

impl Drop for ActiveJobGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.inner.active.lock() {
            active.remove(&self.key);
        }
    }
}
