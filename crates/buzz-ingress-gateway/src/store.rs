//! Durable gateway state behind a trait, so the HTTP surface is tested against
//! an in-memory reference store and production runs on Postgres.
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use thiserror::Error;
use uuid::Uuid;

/// Webhook providers the gateway can verify. Stored as the wire name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Github,
    Pagerduty,
    Sentry,
    IncidentIo,
}

impl Provider {
    /// Stable storage and wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Github => "github",
            Provider::Pagerduty => "pagerduty",
            Provider::Sentry => "sentry",
            Provider::IncidentIo => "incident_io",
        }
    }

    /// Inverse of [`Provider::as_str`].
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "github" => Some(Provider::Github),
            "pagerduty" => Some(Provider::Pagerduty),
            "sentry" => Some(Provider::Sentry),
            "incident_io" => Some(Provider::IncidentIo),
            _ => None,
        }
    }
}

/// A provider integration owned by one community.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Integration {
    pub id: Uuid,
    pub community_host: String,
    pub provider: Provider,
    pub revision: i64,
    pub enabled: bool,
    pub disabled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A sealed provider secret. A retired secret keeps verifying deliveries until
/// `grace_until`, so a provider-side rotation never drops in-flight deliveries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationSecret {
    pub id: Uuid,
    pub integration_id: Uuid,
    pub key_id: String,
    pub ciphertext: Vec<u8>,
    pub created_at: DateTime<Utc>,
    pub retired_at: Option<DateTime<Utc>>,
    pub grace_until: Option<DateTime<Utc>>,
}

impl IntegrationSecret {
    /// Whether deliveries signed with this secret are still accepted at `now`.
    pub fn verifies_at(&self, now: DateTime<Utc>) -> bool {
        match (self.retired_at, self.grace_until) {
            (None, _) => true,
            (Some(_), Some(grace_until)) => now < grace_until,
            (Some(_), None) => false,
        }
    }
}

/// A management mutation. Every mutation carries the caller's `operation_id`
/// and the integration revision it produces; the store applies it at most once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mutation {
    PutIntegration {
        community_host: String,
        provider: Provider,
    },
    PutSecret {
        key_id: String,
        ciphertext: Vec<u8>,
        grace: Duration,
    },
    Disable,
    Enable,
}

/// One idempotent management mutation addressed to an integration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    pub operation_id: Uuid,
    pub integration_id: Uuid,
    pub revision: i64,
    pub mutation: Mutation,
}

/// Durable record of an applied operation and the response it produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedOperation {
    pub operation_id: Uuid,
    pub integration_id: Uuid,
    pub revision: i64,
    pub response: serde_json::Value,
    pub applied_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// Applied now; `response` is the integration view after the change.
    Applied(AppliedOperation),
    /// The same `operation_id` was applied before; the stored response is returned.
    Replayed(AppliedOperation),
    /// `revision` is not newer than the current integration revision.
    StaleRevision { current: i64 },
    /// The mutation needs an integration that does not exist.
    NotFound,
}

/// Lifecycle of a stored delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    Queued,
    Accepted,
    Failed,
}

impl DeliveryState {
    /// Stable storage and wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            DeliveryState::Queued => "queued",
            DeliveryState::Accepted => "accepted",
            DeliveryState::Failed => "failed",
        }
    }

    /// Inverse of [`DeliveryState::as_str`].
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(DeliveryState::Queued),
            "accepted" => Some(DeliveryState::Accepted),
            "failed" => Some(DeliveryState::Failed),
            _ => None,
        }
    }
}

/// A verified delivery about to be stored. The provider is acknowledged only
/// once this row is committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDelivery {
    pub integration_id: Uuid,
    pub delivery_id: String,
    pub event_type: String,
    pub provider_timestamp: Option<DateTime<Utc>>,
    pub headers: serde_json::Value,
    pub body: Vec<u8>,
    pub received_at: DateTime<Utc>,
}

/// Result of storing a delivery keyed by `(integration_id, delivery_id)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertOutcome {
    Inserted,
    Duplicate,
}

/// A stored delivery without its payload, for the admin timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeliverySummary {
    pub delivery_id: String,
    pub event_type: String,
    pub state: DeliveryState,
    pub attempts: i32,
    pub event_id: Option<String>,
    pub received_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

/// Keyset position in the newest-first delivery timeline. Encoded as
/// `{received_at_micros}.{delivery_id}` so a client can hand it back opaquely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryCursor {
    pub received_at: DateTime<Utc>,
    pub delivery_id: String,
}

impl DeliveryCursor {
    /// Inverse of [`DeliveryCursor::encode`]; `None` for anything malformed.
    pub fn parse(value: &str) -> Option<Self> {
        let (micros, delivery_id) = value.split_once('.')?;
        if delivery_id.is_empty() || delivery_id.len() > MAX_DELIVERY_ID_CURSOR_BYTES {
            return None;
        }
        Some(Self {
            received_at: DateTime::from_timestamp_micros(micros.parse().ok()?)?,
            delivery_id: delivery_id.to_owned(),
        })
    }

    /// Opaque cursor string for the admin client.
    pub fn encode(&self) -> String {
        format!(
            "{}.{}",
            self.received_at.timestamp_micros(),
            self.delivery_id
        )
    }

    fn position(&self) -> (DateTime<Utc>, &str) {
        (self.received_at, self.delivery_id.as_str())
    }
}

const MAX_DELIVERY_ID_CURSOR_BYTES: usize = 256;

/// Page selector for [`Store::list_deliveries`]: newest first, optionally
/// bounded below by `since`, continued from `before`, filtered by `state`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeliveryPage {
    pub since: Option<DateTime<Utc>>,
    pub before: Option<DeliveryCursor>,
    pub state: Option<DeliveryState>,
    pub limit: usize,
}

/// Failures of the durable store. Callers treat every variant as retryable
/// except [`StoreError::Invalid`].
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("store unavailable")]
    Unavailable(#[source] Option<sqlx::Error>),
    #[error("migration failed")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("{0}")]
    Invalid(&'static str),
}

impl From<sqlx::Error> for StoreError {
    fn from(error: sqlx::Error) -> Self {
        StoreError::Unavailable(Some(error))
    }
}

/// Public view of an integration returned by the management API and stored as
/// the idempotent response of the operation that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationView {
    pub id: Uuid,
    pub community_host: String,
    pub provider: Provider,
    pub revision: i64,
    pub enabled: bool,
    pub has_secret: bool,
    pub rotating_until: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl IntegrationView {
    /// Project an integration and its currently verifying secrets at `now`.
    pub fn new(
        integration: &Integration,
        secrets: &[IntegrationSecret],
        now: DateTime<Utc>,
    ) -> Self {
        let has_secret = secrets.iter().any(|s| s.retired_at.is_none());
        let rotating_until = secrets
            .iter()
            .filter(|s| s.retired_at.is_some() && s.verifies_at(now))
            .filter_map(|s| s.grace_until)
            .max();
        Self {
            id: integration.id,
            community_host: integration.community_host.clone(),
            provider: integration.provider,
            revision: integration.revision,
            enabled: integration.enabled,
            has_secret,
            rotating_until,
            updated_at: integration.updated_at,
        }
    }
}

#[async_trait]
pub trait Store: Send + Sync {
    /// Readiness must fail closed when durable state cannot participate.
    async fn ready(&self) -> Result<(), StoreError>;
    /// Look up an integration by id.
    async fn integration(&self, id: Uuid) -> Result<Option<Integration>, StoreError>;
    /// Secrets that still verify at `now`, newest first.
    async fn verifying_secrets(
        &self,
        integration_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Vec<IntegrationSecret>, StoreError>;
    /// The idempotency record for `operation_id`, if it was ever applied.
    async fn applied_operation(
        &self,
        operation_id: Uuid,
    ) -> Result<Option<AppliedOperation>, StoreError>;
    /// Apply a mutation exactly once, atomically with its idempotency record.
    async fn apply(
        &self,
        operation: Operation,
        now: DateTime<Utc>,
    ) -> Result<ApplyOutcome, StoreError>;
    /// Record a verified delivery exactly once per `(integration, delivery_id)`.
    async fn insert_delivery(&self, delivery: NewDelivery) -> Result<InsertOutcome, StoreError>;
    /// Delivery summaries for one integration, newest first, payloads omitted.
    async fn list_deliveries(
        &self,
        integration_id: Uuid,
        page: DeliveryPage,
    ) -> Result<Vec<DeliverySummary>, StoreError>;
}

/// Pure in-process transition shared by the memory store and the Postgres
/// implementation's tests, so both agree on revision and secret semantics.
pub fn transition(
    integration: Option<Integration>,
    secrets: &mut Vec<IntegrationSecret>,
    operation: &Operation,
    now: DateTime<Utc>,
) -> Result<Integration, ApplyOutcome> {
    if let Some(current) = &integration {
        if operation.revision <= current.revision {
            return Err(ApplyOutcome::StaleRevision {
                current: current.revision,
            });
        }
    }
    let mut next = match (&operation.mutation, integration) {
        (
            Mutation::PutIntegration {
                community_host,
                provider,
            },
            existing,
        ) => Integration {
            id: operation.integration_id,
            community_host: community_host.clone(),
            provider: *provider,
            revision: operation.revision,
            enabled: existing.as_ref().is_none_or(|e| e.enabled),
            disabled_at: existing.as_ref().and_then(|e| e.disabled_at),
            created_at: existing.as_ref().map_or(now, |e| e.created_at),
            updated_at: now,
        },
        (_, None) => return Err(ApplyOutcome::NotFound),
        (
            Mutation::PutSecret {
                key_id,
                ciphertext,
                grace,
            },
            Some(existing),
        ) => {
            for secret in secrets.iter_mut().filter(|s| s.retired_at.is_none()) {
                secret.retired_at = Some(now);
                secret.grace_until = Some(now + *grace);
            }
            secrets.insert(
                0,
                IntegrationSecret {
                    id: Uuid::new_v4(),
                    integration_id: existing.id,
                    key_id: key_id.clone(),
                    ciphertext: ciphertext.clone(),
                    created_at: now,
                    retired_at: None,
                    grace_until: None,
                },
            );
            existing
        }
        (Mutation::Disable, Some(mut existing)) => {
            existing.enabled = false;
            existing.disabled_at = Some(now);
            existing
        }
        (Mutation::Enable, Some(mut existing)) => {
            existing.enabled = true;
            existing.disabled_at = None;
            existing
        }
    };
    next.revision = operation.revision;
    next.updated_at = now;
    Ok(next)
}

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

#[derive(Default)]
struct MemoryState {
    integrations: HashMap<Uuid, Integration>,
    secrets: HashMap<Uuid, Vec<IntegrationSecret>>,
    operations: HashMap<Uuid, AppliedOperation>,
    deliveries: HashMap<(Uuid, String), StoredDelivery>,
}

/// Reference store for tests. One lock gives every transition a single
/// linearization point, as a Postgres transaction does in production.
#[derive(Default)]
pub struct MemoryStore(Mutex<MemoryState>);

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

    async fn verifying_secrets(
        &self,
        integration_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Vec<IntegrationSecret>, StoreError> {
        let s = self.0.lock().map_err(|_| StoreError::Unavailable(None))?;
        Ok(s.secrets
            .get(&integration_id)
            .map(|secrets| {
                secrets
                    .iter()
                    .filter(|secret| secret.verifies_at(now))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default())
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
            return Ok(ApplyOutcome::Replayed(applied.clone()));
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
}

impl MemoryStore {
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
        let again = store.apply(op, at(5)).await.unwrap();
        assert_eq!(again, ApplyOutcome::Replayed(applied.clone()));
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
}
