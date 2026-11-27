//! Channel subscriptions and the per-channel message outbox.
//!
//! ```text
//!   delivery (queued) ── fan-out: evaluate each live subscription's filter
//!        │                and render the integration's message, then in one
//!        │                transaction,
//!        │                if those are still the live subscriptions:
//!        ▼
//!   delivery → accepted, payload dropped
//!   one message row per matching subscription (queued, or failed when its
//!   filter or the integration's message expression errored)
//!        │
//!        ▼
//!   sender: kind-9 event to the subscription's channel on the community relay
//! ```
//!
//! Every settlement names the attempt it claimed, so a worker whose lease
//! ended cannot overwrite the claim of the worker that took the row over.
//!
//! Subscriptions are written by revision alone: a write carries the revision
//! it produces, a retry of the applied write returns the stored subscription,
//! and anything older is stale. Deletion leaves a tombstone so a delayed write
//! cannot bring a deleted subscription back.
use super::{DeliveryState, ForwardOutcome, StoreError};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use uuid::Uuid;

/// Live subscriptions one integration may hold. Every delivery evaluates each
/// of them, so this bounds fan-out work per delivery.
pub const MAX_SUBSCRIPTIONS_PER_INTEGRATION: usize = 50;

/// Routes an integration's deliveries into one channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    pub id: Uuid,
    pub integration_id: Uuid,
    pub channel_id: Uuid,
    /// CEL filter source; `None` matches every delivery.
    pub filter: Option<String>,
    pub revision: i64,
    pub deleted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Subscription {
    /// Whether the subscription still receives deliveries.
    pub fn is_live(&self) -> bool {
        self.deleted_at.is_none()
    }
}

/// Public view of a subscription returned by the management API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SubscriptionView {
    pub id: Uuid,
    pub integration_id: Uuid,
    pub channel_id: Uuid,
    pub filter: Option<String>,
    pub revision: i64,
    pub deleted: bool,
    pub updated_at: DateTime<Utc>,
}

impl From<&Subscription> for SubscriptionView {
    fn from(s: &Subscription) -> Self {
        Self {
            id: s.id,
            integration_id: s.integration_id,
            channel_id: s.channel_id,
            filter: s.filter.clone(),
            revision: s.revision,
            deleted: !s.is_live(),
            updated_at: s.updated_at,
        }
    }
}

/// What a subscription write does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscriptionChange {
    /// Create or replace. The filter is checked by the caller before this.
    Put {
        channel_id: Uuid,
        filter: Option<String>,
    },
    /// Tombstone the subscription and fail its unsent messages.
    Delete,
}

/// One revisioned write addressed to a subscription of an integration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionWrite {
    pub id: Uuid,
    pub integration_id: Uuid,
    pub revision: i64,
    pub change: SubscriptionChange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    /// Written now.
    Applied(Subscription),
    /// This exact write was applied before; nothing changed.
    Unchanged(Subscription),
    /// `revision` is not newer than the stored one.
    StaleRevision { current: i64 },
    /// The integration (for a put) or the subscription (for a delete) does
    /// not exist.
    NotFound,
    /// The subscription id already belongs to another integration.
    WrongIntegration,
    /// The integration already holds [`MAX_SUBSCRIPTIONS_PER_INTEGRATION`].
    LimitReached,
}

/// Pure write transition shared by the memory and Postgres stores.
/// `live_count` is the integration's live subscriptions, this one included
/// when it is live. Only [`WriteOutcome::Applied`] is persisted by the caller.
pub fn write_transition(
    current: Option<Subscription>,
    write: &SubscriptionWrite,
    live_count: usize,
    now: DateTime<Utc>,
) -> WriteOutcome {
    if let Some(current) = &current {
        if current.integration_id != write.integration_id {
            return WriteOutcome::WrongIntegration;
        }
        if current.revision == write.revision && repeats(current, &write.change) {
            return WriteOutcome::Unchanged(current.clone());
        }
        if write.revision <= current.revision {
            return WriteOutcome::StaleRevision {
                current: current.revision,
            };
        }
    }
    match (&write.change, current) {
        (SubscriptionChange::Delete, None) => WriteOutcome::NotFound,
        (SubscriptionChange::Delete, Some(mut existing)) => {
            existing.deleted_at = existing.deleted_at.or(Some(now));
            existing.revision = write.revision;
            existing.updated_at = now;
            WriteOutcome::Applied(existing)
        }
        (SubscriptionChange::Put { channel_id, filter }, existing) => {
            let was_live = existing.as_ref().is_some_and(Subscription::is_live);
            if !was_live && live_count >= MAX_SUBSCRIPTIONS_PER_INTEGRATION {
                return WriteOutcome::LimitReached;
            }
            WriteOutcome::Applied(Subscription {
                id: write.id,
                integration_id: write.integration_id,
                channel_id: *channel_id,
                filter: filter.clone(),
                revision: write.revision,
                deleted_at: None,
                created_at: existing
                    .filter(Subscription::is_live)
                    .map_or(now, |e| e.created_at),
                updated_at: now,
            })
        }
    }
}

fn repeats(current: &Subscription, change: &SubscriptionChange) -> bool {
    match change {
        SubscriptionChange::Delete => !current.is_live(),
        SubscriptionChange::Put { channel_id, filter } => {
            current.is_live() && current.channel_id == *channel_id && current.filter == *filter
        }
    }
}

/// One subscription's share of a fanned-out delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMessage {
    pub subscription_id: Uuid,
    pub channel_id: Uuid,
    /// The rendered message text, or why the filter or message expression
    /// failed.
    /// A failed message is stored terminal so the failure stays visible.
    pub content: Result<String, String>,
}

/// Result of fanning out one claimed delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FanoutOutcome {
    /// Store these messages and mark the delivery accepted, atomically, if
    /// the integration's live subscriptions are still exactly `evaluated`.
    Fanned {
        messages: Vec<NewMessage>,
        evaluated: Vec<Evaluated>,
    },
    /// The delivery can never be fanned out.
    Failed(String),
}

/// A subscription as a fan-out evaluated it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Evaluated {
    pub id: Uuid,
    pub revision: i64,
}

impl Evaluated {
    /// The sorted identity of a set of subscriptions, for comparing the set a
    /// fan-out evaluated with the live set when it commits.
    pub fn of(subscriptions: &[Subscription]) -> Vec<Self> {
        let mut evaluated: Vec<_> = subscriptions
            .iter()
            .map(|s| Self {
                id: s.id,
                revision: s.revision,
            })
            .collect();
        evaluated.sort();
        evaluated
    }
}

/// What [`SubscriptionStore::complete_fanout`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanoutCommit {
    /// The messages are stored and the delivery is settled.
    Settled,
    /// A subscription was created, edited or deleted while the fan-out
    /// evaluated; nothing was stored and the delivery is due again now.
    Stale,
    /// The claim was no longer this worker's: its lease ended and another
    /// worker claimed the delivery, or the delivery is already settled.
    Lost,
}

/// Identity of a message: one delivery's share for one subscription.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MessageKey {
    pub integration_id: Uuid,
    pub delivery_id: String,
    pub subscription_id: Uuid,
}

/// A queued message the sender has leased, joined with its community host.
/// `attempts` already counts this attempt and fences its settlement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedMessage {
    pub key: MessageKey,
    pub community_host: String,
    pub channel_id: Uuid,
    pub content: String,
    pub received_at: DateTime<Utc>,
    pub attempts: i32,
    /// The `created_at` every attempt signs with, set by the first claim and
    /// kept until [`ForwardOutcome::Restamp`] clears it.
    pub signed_at: DateTime<Utc>,
    /// The event an earlier attempt sent, if one reported back.
    pub event_id: Option<String>,
}

/// Subscription configuration and the message outbox it feeds.
#[async_trait]
pub trait SubscriptionStore: Send + Sync {
    /// Apply a revisioned subscription write. Deleting also fails the
    /// subscription's queued messages in the same transaction.
    async fn write_subscription(
        &self,
        write: SubscriptionWrite,
        now: DateTime<Utc>,
    ) -> Result<WriteOutcome, StoreError>;
    /// One subscription of an integration, tombstones included.
    async fn subscription(
        &self,
        integration_id: Uuid,
        id: Uuid,
    ) -> Result<Option<Subscription>, StoreError>;
    /// An integration's live subscriptions, oldest first.
    async fn live_subscriptions(
        &self,
        integration_id: Uuid,
    ) -> Result<Vec<Subscription>, StoreError>;
    /// Store a claimed delivery's messages and settle the delivery in one
    /// transaction, holding off subscription writes while it checks that the
    /// subscriptions it evaluated are still the live ones. Only the claim
    /// that `attempts` names can settle the delivery.
    async fn complete_fanout(
        &self,
        integration_id: Uuid,
        delivery_id: &str,
        attempts: i32,
        outcome: FanoutOutcome,
        now: DateTime<Utc>,
    ) -> Result<FanoutCommit, StoreError>;
    /// Lease up to `limit` queued messages due at `now`, as
    /// [`Store::claim_due`](super::Store::claim_due) does for deliveries, and
    /// stamp `signed_at` with `now` on rows that have none.
    async fn claim_due_messages(
        &self,
        now: DateTime<Utc>,
        lease: Duration,
        limit: usize,
    ) -> Result<Vec<ClaimedMessage>, StoreError>;
    /// Record the result of a send attempt. A claim whose lease ended and
    /// was taken over, or a message cancelled meanwhile, is left alone.
    async fn complete_message(
        &self,
        message: &ClaimedMessage,
        outcome: ForwardOutcome,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError>;
}

/// The stored state of a new message.
pub(crate) fn initial_state(message: &NewMessage) -> (DeliveryState, &str, Option<String>) {
    match &message.content {
        Ok(content) => (DeliveryState::Queued, content.as_str(), None),
        Err(error) => (DeliveryState::Failed, "", Some(storable(error))),
    }
}

/// `text` as PostgreSQL `TEXT` can hold it: NUL, which provider strings and
/// relay answers may carry, becomes U+FFFD. For error text only; a message
/// whose content holds NUL fails instead of being altered.
pub fn storable(text: &str) -> String {
    text.replace('\0', "\u{FFFD}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    fn applied(outcome: WriteOutcome) -> Subscription {
        match outcome {
            WriteOutcome::Applied(subscription) => subscription,
            other => panic!("{other:?}"),
        }
    }

    fn put(id: Uuid, integration_id: Uuid, revision: i64, channel: Uuid) -> SubscriptionWrite {
        SubscriptionWrite {
            id,
            integration_id,
            revision,
            change: SubscriptionChange::Put {
                channel_id: channel,
                filter: None,
            },
        }
    }

    #[test]
    fn writes_are_revisioned_retryable_and_tombstoned() {
        let (id, integration, channel) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let created = applied(write_transition(
            None,
            &put(id, integration, 1, channel),
            0,
            at(0),
        ));
        assert!(created.is_live());

        let retry = write_transition(
            Some(created.clone()),
            &put(id, integration, 1, channel),
            1,
            at(1),
        );
        assert_eq!(retry, WriteOutcome::Unchanged(created.clone()));
        let changed_same_revision = write_transition(
            Some(created.clone()),
            &put(id, integration, 1, Uuid::new_v4()),
            1,
            at(1),
        );
        assert_eq!(
            changed_same_revision,
            WriteOutcome::StaleRevision { current: 1 }
        );
        assert_eq!(
            write_transition(
                Some(created.clone()),
                &put(id, Uuid::new_v4(), 2, channel),
                1,
                at(1)
            ),
            WriteOutcome::WrongIntegration
        );

        let delete = SubscriptionWrite {
            id,
            integration_id: integration,
            revision: 2,
            change: SubscriptionChange::Delete,
        };
        let tombstone = applied(write_transition(Some(created), &delete, 1, at(2)));
        assert_eq!(tombstone.deleted_at, Some(at(2)));
        assert_eq!(
            write_transition(Some(tombstone.clone()), &delete, 0, at(3)),
            WriteOutcome::Unchanged(tombstone.clone())
        );
        assert_eq!(
            write_transition(
                Some(tombstone.clone()),
                &put(id, integration, 2, channel),
                0,
                at(3)
            ),
            WriteOutcome::StaleRevision { current: 2 },
            "a delayed write cannot resurrect a deleted subscription"
        );
        let revived = applied(write_transition(
            Some(tombstone),
            &put(id, integration, 3, channel),
            0,
            at(4),
        ));
        assert!(revived.is_live());
        assert_eq!(revived.created_at, at(4));
        assert_eq!(
            write_transition(None, &delete, 0, at(5)),
            WriteOutcome::NotFound
        );
    }

    #[test]
    fn new_subscriptions_are_capped_but_existing_ones_stay_editable() {
        let (integration, channel) = (Uuid::new_v4(), Uuid::new_v4());
        let full = MAX_SUBSCRIPTIONS_PER_INTEGRATION;
        assert_eq!(
            write_transition(
                None,
                &put(Uuid::new_v4(), integration, 1, channel),
                full,
                at(0)
            ),
            WriteOutcome::LimitReached
        );
        let id = Uuid::new_v4();
        let existing = applied(write_transition(
            None,
            &put(id, integration, 1, channel),
            0,
            at(0),
        ));
        assert!(matches!(
            write_transition(
                Some(existing),
                &put(id, integration, 2, Uuid::new_v4()),
                full,
                at(1)
            ),
            WriteOutcome::Applied(_)
        ));
    }
}
