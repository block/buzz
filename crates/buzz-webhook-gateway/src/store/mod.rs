//! Durable gateway state behind a trait, so the HTTP surface is tested against
//! an in-memory reference store and production runs on Postgres.
mod integration_config;
mod memory;
mod subscriptions;

pub use integration_config::{DeliveryMode, IntegrationConfig};

pub use memory::{MemoryStore, StoredDelivery, StoredMessage};
pub(crate) use subscriptions::initial_state as initial_message_state;
pub use subscriptions::{
    storable, write_transition, ClaimedMessage, Evaluated, FanoutCommit, FanoutOutcome, MessageKey,
    NewMessage, Subscription, SubscriptionChange, SubscriptionStore, SubscriptionView,
    SubscriptionWrite, WriteOutcome, MAX_SUBSCRIPTIONS_PER_INTEGRATION,
};

use crate::config::RelayScheme;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
    pub config: IntegrationConfig,
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
    /// Create or replace. The config's expressions are checked by the
    /// caller before this.
    PutIntegration {
        community_host: String,
        provider: Provider,
        config: IntegrationConfig,
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

impl Operation {
    /// Hex SHA-256 of everything that identifies this request except secret
    /// material. A replayed `operation_id` must carry the same fingerprint to
    /// receive the stored response. The sealed secret is excluded because
    /// sealing is randomized, and the plaintext is excluded so the idempotency
    /// table never holds anything derived from a provider secret.
    pub fn fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.integration_id.as_bytes());
        hasher.update(self.revision.to_be_bytes());
        match &self.mutation {
            Mutation::PutIntegration {
                community_host,
                provider,
                config,
            } => {
                hasher.update(b"put_integration\0");
                hasher.update(community_host.as_bytes());
                hasher.update(b"\0");
                hasher.update(provider.as_str().as_bytes());
                hasher.update(b"\0");
                config.fingerprint(&mut hasher);
            }
            Mutation::PutSecret { grace, .. } => {
                hasher.update(b"put_secret\0");
                hasher.update(grace.num_seconds().to_be_bytes());
            }
            Mutation::Disable => hasher.update(b"disable\0"),
            Mutation::Enable => hasher.update(b"enable\0"),
        }
        hex::encode(hasher.finalize())
    }
}

/// Durable record of an applied operation and the response it produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedOperation {
    pub operation_id: Uuid,
    pub integration_id: Uuid,
    pub revision: i64,
    pub fingerprint: String,
    pub response: serde_json::Value,
    pub applied_at: DateTime<Utc>,
}

impl AppliedOperation {
    /// Whether `operation` is a retry of this applied operation rather than a
    /// different request reusing its `operation_id`.
    pub fn matches(&self, operation: &Operation) -> bool {
        self.integration_id == operation.integration_id
            && self.fingerprint == operation.fingerprint()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// Applied now; `response` is the integration view after the change.
    Applied(AppliedOperation),
    /// The same request was applied before; the stored response is returned.
    Replayed(AppliedOperation),
    /// `operation_id` was already used by a different request.
    Conflict,
    /// `revision` is not newer than the current integration revision.
    StaleRevision { current: i64 },
    /// The mutation needs an integration that does not exist.
    NotFound,
    /// An integration stays bound to the community it was created for:
    /// queued messages are signed for and sent to that community.
    CommunityChanged,
}

/// Lifecycle of a stored delivery, and of each message fanned out from it.
/// A delivery is `accepted` once it has been fanned out to its matching
/// subscriptions; a message is `accepted` once the relay stored it.
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
    /// Where the delivery's messages stand, so the timeline shows what
    /// reached a channel without returning any payload.
    pub messages: MessageCounts,
}

/// How many of a delivery's messages are in each state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct MessageCounts {
    pub queued: i64,
    pub accepted: i64,
    pub failed: i64,
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
    #[serde(flatten)]
    pub config: IntegrationConfig,
    pub revision: i64,
    pub enabled: bool,
    pub has_secret: bool,
    pub rotating_until: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl IntegrationView {
    /// URL the provider must deliver webhooks to for this integration.
    pub fn delivery_url(&self, scheme: RelayScheme) -> String {
        format!(
            "{}://{}/webhooks/{}",
            scheme.as_str(),
            self.community_host,
            self.id
        )
    }

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
            config: integration.config.clone(),
            revision: integration.revision,
            enabled: integration.enabled,
            has_secret,
            rotating_until,
            updated_at: integration.updated_at,
        }
    }
}

/// Integrations, secrets, idempotent operations and stored deliveries. The
/// subscription and message outbox half lives in [`SubscriptionStore`].
#[async_trait]
pub trait Store: SubscriptionStore + Send + Sync {
    /// Readiness must fail closed when durable state cannot participate.
    async fn ready(&self) -> Result<(), StoreError>;
    /// Look up an integration by id.
    async fn integration(&self, id: Uuid) -> Result<Option<Integration>, StoreError>;
    /// An integration and its verifying secrets read from one snapshot, so a
    /// concurrent write cannot pair one revision with another's secrets.
    async fn integration_with_secrets(
        &self,
        id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Option<(Integration, Vec<IntegrationSecret>)>, StoreError>;
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
    /// Lease up to `limit` queued deliveries due for fan-out at `now`, pushing
    /// their `next_attempt_at` to `now + lease` so a crashed worker's claims
    /// return.
    async fn claim_due(
        &self,
        now: DateTime<Utc>,
        lease: Duration,
        limit: usize,
    ) -> Result<Vec<ClaimedDelivery>, StoreError>;
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
    if let (Mutation::PutIntegration { community_host, .. }, Some(current)) =
        (&operation.mutation, &integration)
    {
        if *community_host != current.community_host {
            return Err(ApplyOutcome::CommunityChanged);
        }
    }
    let mut next = match (&operation.mutation, integration) {
        (
            Mutation::PutIntegration {
                community_host,
                provider,
                config,
            },
            existing,
        ) => Integration {
            id: operation.integration_id,
            community_host: community_host.clone(),
            provider: *provider,
            config: config.clone(),
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

/// A queued delivery the fan-out stage has leased, joined with its
/// integration's provider and message expression. `attempts` already counts
/// this attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedDelivery {
    pub integration_id: Uuid,
    pub delivery_id: String,
    pub provider: Provider,
    /// The integration's message expression when the delivery was claimed.
    pub message_expr: Option<String>,
    pub event_type: String,
    pub body: Vec<u8>,
    pub received_at: DateTime<Utc>,
    pub attempts: i32,
}

/// Terminal or deferred result of one attempt to send a message to a relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardOutcome {
    /// The relay stored the event.
    Accepted { event_id: String },
    /// The relay refused the event or the retry budget is spent.
    Failed {
        event_id: Option<String>,
        error: String,
    },
    /// Transient failure; the row stays queued until `next_attempt_at`.
    Retry {
        event_id: Option<String>,
        error: String,
        next_attempt_at: DateTime<Utc>,
    },
    /// The relay would now refuse `signed_at` as too old, and holds no event
    /// an earlier attempt sent. The row stays queued, due now, and its next
    /// claim signs with a fresh timestamp.
    Restamp { error: String },
}
