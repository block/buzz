//! In-memory reference store: the semantics the Postgres store must match.
use super::{
    initial_message_state, transition, write_transition, AppliedOperation, ApplyOutcome,
    ClaimedDelivery, ClaimedMessage, DeliveryPage, DeliveryState, DeliverySummary, FanoutOutcome,
    ForwardOutcome, InsertOutcome, Integration, IntegrationSecret, IntegrationView, MessageKey,
    NewDelivery, Operation, Store, StoreError, Subscription, SubscriptionChange, SubscriptionStore,
    SubscriptionWrite, WriteOutcome,
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
    fn summary(&self) -> DeliverySummary {
        DeliverySummary {
            delivery_id: self.delivery.delivery_id.clone(),
            event_type: self.delivery.event_type.clone(),
            state: self.state,
            attempts: self.attempts,
            event_id: self.event_id.clone(),
            received_at: self.delivery.received_at,
            completed_at: self.completed_at,
            last_error: self.last_error.clone(),
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
            .map(StoredDelivery::summary)
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
        outcome: FanoutOutcome,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let mut s = self.0.lock().map_err(|_| StoreError::Unavailable(None))?;
        let Some(row) = s
            .deliveries
            .get_mut(&(integration_id, delivery_id.to_owned()))
            .filter(|row| row.state == DeliveryState::Queued)
        else {
            return Ok(());
        };
        row.completed_at = Some(now);
        row.next_attempt_at = None;
        let received_at = row.delivery.received_at;
        let messages = match outcome {
            FanoutOutcome::Failed(error) => {
                row.state = DeliveryState::Failed;
                row.last_error = Some(error);
                return Ok(());
            }
            FanoutOutcome::Fanned(messages) => {
                row.state = DeliveryState::Accepted;
                row.last_error = None;
                row.delivery.body.clear();
                messages
            }
        };
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
                last_error: error.map(str::to_owned),
                received_at,
                completed_at: (state != DeliveryState::Queued).then_some(now),
            };
            s.messages.entry(key).or_insert(stored);
        }
        Ok(())
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
            claimed.push(ClaimedMessage {
                community_host: host,
                channel_id: row.channel_id,
                content: row.content.clone(),
                received_at: row.received_at,
                attempts: row.attempts,
                key,
            });
        }
        Ok(claimed)
    }

    async fn complete_message(
        &self,
        key: &MessageKey,
        outcome: ForwardOutcome,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let mut s = self.0.lock().map_err(|_| StoreError::Unavailable(None))?;
        let Some(row) = s
            .messages
            .get_mut(key)
            .filter(|row| row.state == DeliveryState::Queued)
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
                row.last_error = Some(error);
            }
            ForwardOutcome::Retry {
                event_id,
                error,
                next_attempt_at,
            } => {
                row.event_id = event_id.or(row.event_id.take());
                row.next_attempt_at = Some(next_attempt_at);
                row.last_error = Some(error);
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
mod tests {
    use super::*;
    use crate::store::{DeliveryCursor, Mutation, NewMessage, Provider};

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    fn put(integration_id: Uuid, revision: i64) -> Operation {
        Operation {
            operation_id: Uuid::new_v4(),
            integration_id,
            revision,
            mutation: Mutation::PutIntegration {
                community_host: "acme.communities.example.org".into(),
                provider: Provider::Github,
            },
        }
    }

    fn secret(integration_id: Uuid, revision: i64, grace_secs: i64) -> Operation {
        Operation {
            operation_id: Uuid::new_v4(),
            integration_id,
            revision,
            mutation: Mutation::PutSecret {
                key_id: "k1".into(),
                ciphertext: vec![revision as u8; 8],
                grace: Duration::seconds(grace_secs),
            },
        }
    }

    #[tokio::test]
    async fn operations_apply_once_and_replay_the_stored_response() {
        let store = MemoryStore::default();
        let id = Uuid::new_v4();
        let op = put(id, 1);
        let first = store.apply(op.clone(), at(0)).await.unwrap();
        let ApplyOutcome::Applied(applied) = first else {
            panic!("{first:?}");
        };
        assert_eq!(applied.response["revision"], 1);
        assert_eq!(applied.response["has_secret"], false);
        let again = store.apply(op.clone(), at(5)).await.unwrap();
        assert_eq!(again, ApplyOutcome::Replayed(applied.clone()));
        let mut reused = put(id, 2);
        reused.operation_id = op.operation_id;
        assert_eq!(
            store.apply(reused, at(6)).await.unwrap(),
            ApplyOutcome::Conflict
        );
        let mut other_integration = put(Uuid::new_v4(), 1);
        other_integration.operation_id = op.operation_id;
        assert_eq!(
            store.apply(other_integration, at(7)).await.unwrap(),
            ApplyOutcome::Conflict
        );
        assert_eq!(
            store.applied_operation(applied.operation_id).await.unwrap(),
            Some(applied)
        );
    }

    #[tokio::test]
    async fn stale_revisions_and_missing_integrations_are_reported() {
        let store = MemoryStore::default();
        let id = Uuid::new_v4();
        assert_eq!(
            store.apply(secret(id, 1, 0), at(0)).await.unwrap(),
            ApplyOutcome::NotFound
        );
        store.apply(put(id, 3), at(0)).await.unwrap();
        assert_eq!(
            store.apply(put(id, 3), at(1)).await.unwrap(),
            ApplyOutcome::StaleRevision { current: 3 }
        );
        assert_eq!(
            store.apply(secret(id, 2, 0), at(1)).await.unwrap(),
            ApplyOutcome::StaleRevision { current: 3 }
        );
        assert!(store.integration(id).await.unwrap().unwrap().enabled);
    }

    #[tokio::test]
    async fn secret_rotation_keeps_the_old_secret_through_grace_only() {
        let store = MemoryStore::default();
        let id = Uuid::new_v4();
        store.apply(put(id, 1), at(0)).await.unwrap();
        store.apply(secret(id, 2, 0), at(10)).await.unwrap();
        let ApplyOutcome::Applied(rotated) = store.apply(secret(id, 3, 60), at(20)).await.unwrap()
        else {
            panic!()
        };
        assert_eq!(rotated.response["has_secret"], true);
        assert_eq!(
            rotated.response["rotating_until"],
            serde_json::json!(at(80))
        );
        let during = store.verifying_secrets(id, at(50)).await.unwrap();
        assert_eq!(during.len(), 2);
        assert_eq!(during[0].ciphertext, vec![3u8; 8]);
        let after = store.verifying_secrets(id, at(80)).await.unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].ciphertext, vec![3u8; 8]);

        let ApplyOutcome::Applied(revoked) = store.apply(secret(id, 4, 0), at(90)).await.unwrap()
        else {
            panic!()
        };
        assert_eq!(revoked.response["rotating_until"], serde_json::Value::Null);
        assert_eq!(store.verifying_secrets(id, at(90)).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn claims_lease_due_rows_oldest_first_and_hide_them_until_the_lease_ends() {
        let store = MemoryStore::default();
        let integration_id = Uuid::new_v4();
        store
            .apply(
                Operation {
                    operation_id: Uuid::new_v4(),
                    integration_id,
                    revision: 1,
                    mutation: Mutation::PutIntegration {
                        community_host: "acme.communities.example.org".into(),
                        provider: Provider::Github,
                    },
                },
                at(0),
            )
            .await
            .unwrap();
        for (id, secs) in [("late", 20), ("early", 10)] {
            store
                .insert_delivery(NewDelivery {
                    integration_id,
                    delivery_id: id.into(),
                    event_type: "ping".into(),
                    provider_timestamp: None,
                    headers: serde_json::json!({}),
                    body: b"{}".to_vec(),
                    received_at: at(secs),
                })
                .await
                .unwrap();
        }
        let lease = Duration::seconds(60);
        let first = store.claim_due(at(30), lease, 1).await.unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].delivery_id, "early");
        assert_eq!(first[0].attempts, 1);
        assert_eq!(first[0].provider, Provider::Github);
        let second = store.claim_due(at(31), lease, 10).await.unwrap();
        assert_eq!(second.len(), 1, "the leased row is hidden");
        assert_eq!(second[0].delivery_id, "late");
        assert!(store.claim_due(at(32), lease, 10).await.unwrap().is_empty());
        let reclaimed = store.claim_due(at(91), lease, 10).await.unwrap();
        assert_eq!(
            reclaimed.len(),
            2,
            "leases expired without completion return"
        );
        assert!(reclaimed.iter().all(|d| d.attempts == 2));
        store
            .complete_fanout(
                integration_id,
                "early",
                FanoutOutcome::Fanned(vec![]),
                at(92),
            )
            .await
            .unwrap();
        assert!(store
            .delivery(integration_id, "early")
            .unwrap()
            .delivery
            .body
            .is_empty());
        let still_due = store.claim_due(at(200), lease, 10).await.unwrap();
        assert_eq!(still_due.len(), 1);
        assert_eq!(still_due[0].delivery_id, "late");
    }

    #[tokio::test]
    async fn deliveries_dedupe_per_integration_and_page_newest_first() {
        let store = MemoryStore::default();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let new = |integration_id, delivery_id: &str, secs| NewDelivery {
            integration_id,
            delivery_id: delivery_id.into(),
            event_type: "ping".into(),
            provider_timestamp: None,
            headers: serde_json::json!({}),
            body: b"{}".to_vec(),
            received_at: at(secs),
        };
        assert_eq!(
            store.insert_delivery(new(a, "d1", 0)).await.unwrap(),
            InsertOutcome::Inserted
        );
        assert_eq!(
            store.insert_delivery(new(a, "d1", 5)).await.unwrap(),
            InsertOutcome::Duplicate
        );
        assert_eq!(
            store.insert_delivery(new(b, "d1", 5)).await.unwrap(),
            InsertOutcome::Inserted
        );
        store.insert_delivery(new(a, "d2", 10)).await.unwrap();
        store.insert_delivery(new(a, "d3", 20)).await.unwrap();

        let page = DeliveryPage {
            limit: 2,
            ..Default::default()
        };
        let first = store.list_deliveries(a, page.clone()).await.unwrap();
        let ids: Vec<_> = first.iter().map(|d| d.delivery_id.as_str()).collect();
        assert_eq!(ids, ["d3", "d2"]);
        assert!(first.iter().all(|d| d.state == DeliveryState::Queued));
        let older = store
            .list_deliveries(
                a,
                DeliveryPage {
                    before: first.last().map(|d| DeliveryCursor {
                        received_at: d.received_at,
                        delivery_id: d.delivery_id.clone(),
                    }),
                    ..page.clone()
                },
            )
            .await
            .unwrap();
        assert_eq!(older.len(), 1);
        assert_eq!(older[0].delivery_id, "d1");
        let recent = store
            .list_deliveries(
                a,
                DeliveryPage {
                    since: Some(at(10)),
                    ..page.clone()
                },
            )
            .await
            .unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].delivery_id, "d3");
        assert!(store
            .list_deliveries(
                a,
                DeliveryPage {
                    state: Some(DeliveryState::Failed),
                    ..page
                }
            )
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn disable_and_enable_toggle_without_losing_configuration() {
        let store = MemoryStore::default();
        let id = Uuid::new_v4();
        store.apply(put(id, 1), at(0)).await.unwrap();
        let disable = Operation {
            operation_id: Uuid::new_v4(),
            integration_id: id,
            revision: 2,
            mutation: Mutation::Disable,
        };
        store.apply(disable, at(1)).await.unwrap();
        let disabled = store.integration(id).await.unwrap().unwrap();
        assert!(!disabled.enabled);
        assert_eq!(disabled.disabled_at, Some(at(1)));
        assert_eq!(disabled.community_host, "acme.communities.example.org");
        let enable = Operation {
            operation_id: Uuid::new_v4(),
            integration_id: id,
            revision: 3,
            mutation: Mutation::Enable,
        };
        store.apply(enable, at(2)).await.unwrap();
        let enabled = store.integration(id).await.unwrap().unwrap();
        assert!(enabled.enabled && enabled.disabled_at.is_none());
        assert_eq!(enabled.revision, 3);
    }

    #[tokio::test]
    async fn fanout_queues_messages_once_and_delete_cancels_unsent_ones() {
        let store = MemoryStore::default();
        let integration_id = Uuid::new_v4();
        store.apply(put(integration_id, 1), at(0)).await.unwrap();
        let (sub_a, sub_b, channel) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        for sub in [sub_a, sub_b] {
            let write = SubscriptionWrite {
                id: sub,
                integration_id,
                revision: 1,
                change: SubscriptionChange::Put {
                    channel_id: channel,
                    filter: None,
                    message_template: None,
                },
            };
            assert!(matches!(
                store.write_subscription(write, at(1)).await.unwrap(),
                WriteOutcome::Applied(_)
            ));
        }
        assert_eq!(
            store
                .write_subscription(
                    SubscriptionWrite {
                        id: Uuid::new_v4(),
                        integration_id: Uuid::new_v4(),
                        revision: 1,
                        change: SubscriptionChange::Put {
                            channel_id: channel,
                            filter: None,
                            message_template: None,
                        },
                    },
                    at(1),
                )
                .await
                .unwrap(),
            WriteOutcome::NotFound,
            "a subscription needs an existing integration"
        );
        store
            .insert_delivery(NewDelivery {
                integration_id,
                delivery_id: "d1".into(),
                event_type: "push".into(),
                provider_timestamp: None,
                headers: serde_json::json!({}),
                body: b"{}".to_vec(),
                received_at: at(10),
            })
            .await
            .unwrap();
        let messages = vec![
            NewMessage {
                subscription_id: sub_a,
                channel_id: channel,
                content: Ok("hello".into()),
            },
            NewMessage {
                subscription_id: sub_b,
                channel_id: channel,
                content: Err("filter failed".into()),
            },
        ];
        let fanned = FanoutOutcome::Fanned(messages);
        store
            .complete_fanout(integration_id, "d1", fanned.clone(), at(11))
            .await
            .unwrap();
        store
            .complete_fanout(
                integration_id,
                "d1",
                FanoutOutcome::Failed("late".into()),
                at(12),
            )
            .await
            .unwrap();
        let delivery = store.delivery(integration_id, "d1").unwrap();
        assert_eq!(delivery.state, DeliveryState::Accepted, "settled once");
        assert!(delivery.delivery.body.is_empty());
        let key = |subscription_id| MessageKey {
            integration_id,
            delivery_id: "d1".into(),
            subscription_id,
        };
        let failed = store.message(&key(sub_b)).unwrap();
        assert_eq!(failed.state, DeliveryState::Failed);
        assert_eq!(failed.last_error.as_deref(), Some("filter failed"));

        let lease = Duration::seconds(60);
        let claimed = store.claim_due_messages(at(20), lease, 10).await.unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].key, key(sub_a));
        assert_eq!(claimed[0].content, "hello");
        assert_eq!(claimed[0].received_at, at(10));
        assert_eq!(claimed[0].community_host, "acme.communities.example.org");
        assert!(store
            .claim_due_messages(at(21), lease, 10)
            .await
            .unwrap()
            .is_empty());

        let delete = SubscriptionWrite {
            id: sub_a,
            integration_id,
            revision: 2,
            change: SubscriptionChange::Delete,
        };
        store.write_subscription(delete, at(22)).await.unwrap();
        let cancelled = store.message(&key(sub_a)).unwrap();
        assert_eq!(cancelled.state, DeliveryState::Failed);
        assert_eq!(
            cancelled.last_error.as_deref(),
            Some("subscription deleted")
        );
        store
            .complete_message(
                &key(sub_a),
                ForwardOutcome::Accepted {
                    event_id: "ab".repeat(32),
                },
                at(23),
            )
            .await
            .unwrap();
        assert_eq!(
            store.message(&key(sub_a)).unwrap(),
            cancelled,
            "an in-flight send cannot revive a cancelled message"
        );
        assert!(store
            .claim_due_messages(at(200), lease, 10)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            store
                .live_subscriptions(integration_id)
                .await
                .unwrap()
                .len(),
            1
        );
    }
}
