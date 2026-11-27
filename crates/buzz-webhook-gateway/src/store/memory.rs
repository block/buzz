//! In-memory reference store: the semantics the Postgres store must match.
use super::{
    initial_message_state, storable, transition, write_transition, AppliedOperation, ApplyOutcome,
    ClaimedDelivery, ClaimedMessage, DeliveryPage, DeliveryState, DeliverySummary, Evaluated,
    FanoutCommit, FanoutOutcome, ForwardOutcome, InsertOutcome, Integration, IntegrationSecret,
    IntegrationView, MessageCounts, MessageKey, NewDelivery, Operation, Store, StoreError,
    Subscription, SubscriptionChange, SubscriptionStore, SubscriptionWrite, WriteOutcome,
};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

/// A delivery row as the memory store holds it; exposed for tests only.
#[derive(Clone, Debug)]
pub struct StoredDelivery {
    pub delivery: NewDelivery,
    pub state: DeliveryState,
    pub event_id: Option<String>,
    pub attempts: i32,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl StoredDelivery {
    fn summary(&self, messages: MessageCounts) -> DeliverySummary {
        DeliverySummary {
            delivery_id: self.delivery.delivery_id.clone(),
            event_type: self.delivery.event_type.clone(),
            state: self.state,
            attempts: self.attempts,
            event_id: self.event_id.clone(),
            received_at: self.delivery.received_at,
            completed_at: self.completed_at,
            last_error: self.last_error.clone(),
            messages,
        }
    }
}

/// A message row as the memory store holds it; exposed for tests only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredMessage {
    pub channel_id: Uuid,
    pub content: String,
    pub state: DeliveryState,
    pub event_id: Option<String>,
    pub attempts: i32,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub received_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub signed_at: Option<DateTime<Utc>>,
}

#[derive(Default)]
struct MemoryState {
    integrations: HashMap<Uuid, Integration>,
    secrets: HashMap<Uuid, Vec<IntegrationSecret>>,
    operations: HashMap<Uuid, AppliedOperation>,
    deliveries: HashMap<(Uuid, String), StoredDelivery>,
    subscriptions: HashMap<Uuid, Subscription>,
    messages: HashMap<MessageKey, StoredMessage>,
}

impl MemoryState {
    fn live_count(&self, integration_id: Uuid) -> usize {
        self.subscriptions
            .values()
            .filter(|s| s.integration_id == integration_id && s.is_live())
            .count()
    }

    fn message_counts(&self, integration_id: Uuid, delivery_id: &str) -> MessageCounts {
        let mut counts = MessageCounts::default();
        for (_, message) in self
            .messages
            .iter()
            .filter(|(k, _)| k.integration_id == integration_id && k.delivery_id == delivery_id)
        {
            match message.state {
                DeliveryState::Queued => counts.queued += 1,
                DeliveryState::Accepted => counts.accepted += 1,
                DeliveryState::Failed => counts.failed += 1,
            }
        }
        counts
    }
}

/// Reference store for tests. One lock gives every transition a single
/// linearization point, as a Postgres transaction does in production.
#[derive(Default)]
pub struct MemoryStore(Mutex<MemoryState>);

fn verifying_from(
    s: &MemoryState,
    integration_id: Uuid,
    now: DateTime<Utc>,
) -> Vec<IntegrationSecret> {
    s.secrets
        .get(&integration_id)
        .map(|secrets| {
            secrets
                .iter()
                .filter(|secret| secret.verifies_at(now))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

#[async_trait]
impl Store for MemoryStore {
    async fn ready(&self) -> Result<(), StoreError> {
        self.0
            .lock()
            .map(|_| ())
            .map_err(|_| StoreError::Unavailable(None))
    }

    async fn integration(&self, id: Uuid) -> Result<Option<Integration>, StoreError> {
        let s = self.0.lock().map_err(|_| StoreError::Unavailable(None))?;
        Ok(s.integrations.get(&id).cloned())
    }

    async fn integration_with_secrets(
        &self,
        id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Option<(Integration, Vec<IntegrationSecret>)>, StoreError> {
        let s = self.0.lock().map_err(|_| StoreError::Unavailable(None))?;
        Ok(s.integrations
            .get(&id)
            .cloned()
            .map(|integration| (integration, verifying_from(&s, id, now))))
    }

    async fn verifying_secrets(
        &self,
        integration_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Vec<IntegrationSecret>, StoreError> {
        let s = self.0.lock().map_err(|_| StoreError::Unavailable(None))?;
        Ok(verifying_from(&s, integration_id, now))
    }

    async fn applied_operation(
        &self,
        operation_id: Uuid,
    ) -> Result<Option<AppliedOperation>, StoreError> {
        let s = self.0.lock().map_err(|_| StoreError::Unavailable(None))?;
        Ok(s.operations.get(&operation_id).cloned())
    }

    async fn apply(
        &self,
        operation: Operation,
        now: DateTime<Utc>,
    ) -> Result<ApplyOutcome, StoreError> {
        let mut s = self.0.lock().map_err(|_| StoreError::Unavailable(None))?;
        if let Some(applied) = s.operations.get(&operation.operation_id) {
            return Ok(if applied.matches(&operation) {
                ApplyOutcome::Replayed(applied.clone())
            } else {
                ApplyOutcome::Conflict
            });
        }
        let mut secrets = s
            .secrets
            .get(&operation.integration_id)
            .cloned()
            .unwrap_or_default();
        let integration = s.integrations.get(&operation.integration_id).cloned();
        let next = match transition(integration, &mut secrets, &operation, now) {
            Ok(next) => next,
            Err(outcome) => return Ok(outcome),
        };
        let verifying: Vec<_> = secrets
            .iter()
            .filter(|x| x.verifies_at(now))
            .cloned()
            .collect();
        let response = serde_json::to_value(IntegrationView::new(&next, &verifying, now))
            .map_err(|_| StoreError::Unavailable(None))?;
        let applied = AppliedOperation {
            operation_id: operation.operation_id,
            integration_id: operation.integration_id,
            revision: operation.revision,
            fingerprint: operation.fingerprint(),
            response,
            applied_at: now,
        };
        s.integrations.insert(next.id, next);
        s.secrets.insert(operation.integration_id, secrets);
        s.operations.insert(operation.operation_id, applied.clone());
        Ok(ApplyOutcome::Applied(applied))
    }

    async fn insert_delivery(&self, delivery: NewDelivery) -> Result<InsertOutcome, StoreError> {
        let mut s = self.0.lock().map_err(|_| StoreError::Unavailable(None))?;
        let key = (delivery.integration_id, delivery.delivery_id.clone());
        if s.deliveries.contains_key(&key) {
            return Ok(InsertOutcome::Duplicate);
        }
        let next_attempt_at = Some(delivery.received_at);
        s.deliveries.insert(
            key,
            StoredDelivery {
                delivery,
                state: DeliveryState::Queued,
                event_id: None,
                attempts: 0,
                next_attempt_at,
                last_error: None,
                completed_at: None,
            },
        );
        Ok(InsertOutcome::Inserted)
    }

    async fn list_deliveries(
        &self,
        integration_id: Uuid,
        page: DeliveryPage,
    ) -> Result<Vec<DeliverySummary>, StoreError> {
        let s = self.0.lock().map_err(|_| StoreError::Unavailable(None))?;
        let mut rows: Vec<_> = s
            .deliveries
            .values()
            .filter(|d| d.delivery.integration_id == integration_id)
            .filter(|d| page.state.is_none_or(|state| d.state == state))
            .filter(|d| {
                page.since
                    .is_none_or(|since| d.delivery.received_at > since)
            })
            .filter(|d| {
                page.before.as_ref().is_none_or(|before| {
                    (d.delivery.received_at, d.delivery.delivery_id.as_str()) < before.position()
                })
            })
            .map(|d| d.summary(s.message_counts(integration_id, &d.delivery.delivery_id)))
            .collect();
        rows.sort_by(|a, b| {
            b.received_at
                .cmp(&a.received_at)
                .then_with(|| b.delivery_id.cmp(&a.delivery_id))
        });
        rows.truncate(page.limit);
        Ok(rows)
    }

    async fn claim_due(
        &self,
        now: DateTime<Utc>,
        lease: Duration,
        limit: usize,
    ) -> Result<Vec<ClaimedDelivery>, StoreError> {
        let mut s = self.0.lock().map_err(|_| StoreError::Unavailable(None))?;
        let mut due: Vec<(Uuid, String)> = s
            .deliveries
            .iter()
            .filter(|(_, d)| d.state == DeliveryState::Queued)
            .filter(|(_, d)| d.next_attempt_at.is_some_and(|at| at <= now))
            .map(|(key, _)| key.clone())
            .collect();
        due.sort_by(|a, b| {
            let at = |key: &(Uuid, String)| s.deliveries[key].next_attempt_at;
            at(a).cmp(&at(b)).then_with(|| a.cmp(b))
        });
        due.truncate(limit);
        let mut claimed = Vec::with_capacity(due.len());
        for key in due {
            let Some(integration) = s.integrations.get(&key.0).cloned() else {
                continue;
            };
            let Some(row) = s.deliveries.get_mut(&key) else {
                continue;
            };
            row.attempts += 1;
            row.next_attempt_at = Some(now + lease);
            claimed.push(ClaimedDelivery {
                integration_id: key.0,
                delivery_id: key.1.clone(),
                provider: integration.provider,
                message_expr: integration.config.message_expr,
                event_type: row.delivery.event_type.clone(),
                body: row.delivery.body.clone(),
                received_at: row.delivery.received_at,
                attempts: row.attempts,
            });
        }
        Ok(claimed)
    }
}

#[async_trait]
impl SubscriptionStore for MemoryStore {
    async fn write_subscription(
        &self,
        write: SubscriptionWrite,
        now: DateTime<Utc>,
    ) -> Result<WriteOutcome, StoreError> {
        let mut s = self.0.lock().map_err(|_| StoreError::Unavailable(None))?;
        if matches!(write.change, SubscriptionChange::Put { .. })
            && !s.integrations.contains_key(&write.integration_id)
        {
            return Ok(WriteOutcome::NotFound);
        }
        let current = s.subscriptions.get(&write.id).cloned();
        let live = s.live_count(write.integration_id);
        let next = match write_transition(current, &write, live, now) {
            WriteOutcome::Applied(next) => next,
            outcome => return Ok(outcome),
        };
        if !next.is_live() {
            for (_, message) in s
                .messages
                .iter_mut()
                .filter(|(k, m)| k.subscription_id == next.id && m.state == DeliveryState::Queued)
            {
                message.state = DeliveryState::Failed;
                message.next_attempt_at = None;
                message.completed_at = Some(now);
                message.last_error = Some("subscription deleted".to_owned());
            }
        }
        s.subscriptions.insert(next.id, next.clone());
        Ok(WriteOutcome::Applied(next))
    }

    async fn subscription(
        &self,
        integration_id: Uuid,
        id: Uuid,
    ) -> Result<Option<Subscription>, StoreError> {
        let s = self.0.lock().map_err(|_| StoreError::Unavailable(None))?;
        Ok(s.subscriptions
            .get(&id)
            .filter(|sub| sub.integration_id == integration_id)
            .cloned())
    }

    async fn live_subscriptions(
        &self,
        integration_id: Uuid,
    ) -> Result<Vec<Subscription>, StoreError> {
        let s = self.0.lock().map_err(|_| StoreError::Unavailable(None))?;
        let mut live: Vec<_> = s
            .subscriptions
            .values()
            .filter(|sub| sub.integration_id == integration_id && sub.is_live())
            .cloned()
            .collect();
        live.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        Ok(live)
    }

    async fn complete_fanout(
        &self,
        integration_id: Uuid,
        delivery_id: &str,
        attempts: i32,
        outcome: FanoutOutcome,
        now: DateTime<Utc>,
    ) -> Result<FanoutCommit, StoreError> {
        let mut s = self.0.lock().map_err(|_| StoreError::Unavailable(None))?;
        let live: Vec<_> = s
            .subscriptions
            .values()
            .filter(|sub| sub.integration_id == integration_id && sub.is_live())
            .cloned()
            .collect();
        let live = Evaluated::of(&live);
        let Some(row) = s
            .deliveries
            .get_mut(&(integration_id, delivery_id.to_owned()))
            .filter(|row| row.state == DeliveryState::Queued && row.attempts == attempts)
        else {
            return Ok(FanoutCommit::Lost);
        };
        let messages = match outcome {
            FanoutOutcome::Failed(error) => {
                row.state = DeliveryState::Failed;
                row.completed_at = Some(now);
                row.next_attempt_at = None;
                row.last_error = Some(storable(&error));
                return Ok(FanoutCommit::Settled);
            }
            FanoutOutcome::Fanned { evaluated, .. } if evaluated != live => {
                row.next_attempt_at = Some(now);
                return Ok(FanoutCommit::Stale);
            }
            FanoutOutcome::Fanned { messages, .. } => messages,
        };
        row.state = DeliveryState::Accepted;
        row.completed_at = Some(now);
        row.next_attempt_at = None;
        row.last_error = None;
        row.delivery.body.clear();
        let received_at = row.delivery.received_at;
        for message in messages {
            let key = MessageKey {
                integration_id,
                delivery_id: delivery_id.to_owned(),
                subscription_id: message.subscription_id,
            };
            let (state, content, error) = initial_message_state(&message);
            let stored = StoredMessage {
                channel_id: message.channel_id,
                content: content.to_owned(),
                state,
                event_id: None,
                attempts: 0,
                next_attempt_at: (state == DeliveryState::Queued).then_some(received_at),
                last_error: error,
                received_at,
                completed_at: (state != DeliveryState::Queued).then_some(now),
                signed_at: None,
            };
            s.messages.entry(key).or_insert(stored);
        }
        Ok(FanoutCommit::Settled)
    }

    async fn claim_due_messages(
        &self,
        now: DateTime<Utc>,
        lease: Duration,
        limit: usize,
    ) -> Result<Vec<ClaimedMessage>, StoreError> {
        let mut s = self.0.lock().map_err(|_| StoreError::Unavailable(None))?;
        let mut due: Vec<(Option<DateTime<Utc>>, MessageKey)> = s
            .messages
            .iter()
            .filter(|(_, m)| m.state == DeliveryState::Queued)
            .filter(|(_, m)| m.next_attempt_at.is_some_and(|at| at <= now))
            .map(|(key, m)| (m.next_attempt_at, key.clone()))
            .collect();
        due.sort();
        due.truncate(limit);
        let mut claimed = Vec::with_capacity(due.len());
        for (_, key) in due {
            let Some(host) = s
                .integrations
                .get(&key.integration_id)
                .map(|i| i.community_host.clone())
            else {
                continue;
            };
            let Some(row) = s.messages.get_mut(&key) else {
                continue;
            };
            row.attempts += 1;
            row.next_attempt_at = Some(now + lease);
            let signed_at = *row.signed_at.get_or_insert(now);
            claimed.push(ClaimedMessage {
                community_host: host,
                channel_id: row.channel_id,
                content: row.content.clone(),
                received_at: row.received_at,
                attempts: row.attempts,
                signed_at,
                event_id: row.event_id.clone(),
                key,
            });
        }
        Ok(claimed)
    }

    async fn complete_message(
        &self,
        message: &ClaimedMessage,
        outcome: ForwardOutcome,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let mut s = self.0.lock().map_err(|_| StoreError::Unavailable(None))?;
        let Some(row) = s
            .messages
            .get_mut(&message.key)
            .filter(|row| row.state == DeliveryState::Queued && row.attempts == message.attempts)
        else {
            return Ok(());
        };
        match outcome {
            ForwardOutcome::Accepted { event_id } => {
                row.state = DeliveryState::Accepted;
                row.event_id = Some(event_id);
                row.completed_at = Some(now);
                row.next_attempt_at = None;
                row.last_error = None;
            }
            ForwardOutcome::Failed { event_id, error } => {
                row.state = DeliveryState::Failed;
                row.event_id = event_id.or(row.event_id.take());
                row.completed_at = Some(now);
                row.next_attempt_at = None;
                row.last_error = Some(storable(&error));
            }
            ForwardOutcome::Retry {
                event_id,
                error,
                next_attempt_at,
            } => {
                row.event_id = event_id.or(row.event_id.take());
                row.next_attempt_at = Some(next_attempt_at);
                row.last_error = Some(storable(&error));
            }
            ForwardOutcome::Restamp { error } => {
                row.event_id = None;
                row.signed_at = None;
                row.next_attempt_at = Some(now);
                row.last_error = Some(storable(&error));
            }
        }
        Ok(())
    }
}

impl MemoryStore {
    /// Test seam: the stored message row.
    pub fn message(&self, key: &MessageKey) -> Option<StoredMessage> {
        self.0.lock().ok()?.messages.get(key).cloned()
    }

    /// Test seam: the stored delivery row, payload included.
    pub fn delivery(&self, integration_id: Uuid, delivery_id: &str) -> Option<StoredDelivery> {
        self.0
            .lock()
            .ok()?
            .deliveries
            .get(&(integration_id, delivery_id.to_owned()))
            .cloned()
    }
}

#[cfg(test)]
mod tests;
