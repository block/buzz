//! Community-admitted write transactions.

use std::collections::{BTreeMap, BTreeSet};

use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;

use crate::deletion::{DeletionStore, ServingWriteLease};
use crate::Result;

/// A transaction that has passed community write admission.
///
/// Its only constructors take the shared community admission lock (or
/// validate a serving write lease) on the transaction they wrap, for the
/// community they record, so construction and admission are one step.
/// Write helpers take `&mut AdmittedTx` and read the community from it,
/// so the compiler rejects a raw [`sqlx::Transaction`] or a transaction
/// admitted for a different community.
///
/// Outside this crate the value exposes neither the inner transaction nor its
/// connection, so code there can't swap in another connection or run
/// statements on it, and [`AdmittedTx::commit`] and [`AdmittedTx::rollback`]
/// are the only ways it can end the transaction. Inside the crate, the
/// `conn()` accessor hands statements the connection; the type can't stop
/// crate code from issuing a raw `COMMIT` through it, so the database
/// community write fences remain the backstop while deployed. Internal writers
/// must preserve this boundary and use `commit()` for staged follow-up work.
/// Dropping the value without committing rolls back, as with [`sqlx::Transaction`].
///
/// These are live regressions for the guarantee. Event-write helpers accept an
/// admitted transaction:
///
/// ```no_run
/// async fn write(tx: &mut buzz_db::AdmittedTx, event: &nostr::Event) {
///     let _ = buzz_db::event::insert_event_in_transaction(tx, event, None).await;
/// }
/// ```
///
/// but not a raw transaction, which carries no proof of admission:
///
/// ```compile_fail
/// async fn write(tx: &mut sqlx::Transaction<'static, sqlx::Postgres>, event: &nostr::Event) {
///     let _ = buzz_db::event::insert_event_in_transaction(tx, event, None).await;
/// }
/// ```
///
/// Code outside the crate cannot construct one through a conversion:
///
/// ```compile_fail
/// fn forge(tx: sqlx::Transaction<'static, sqlx::Postgres>) -> buzz_db::AdmittedTx {
///     tx.into()
/// }
/// ```
///
/// and cannot reach the connection to swap it out, through a dereference:
///
/// ```compile_fail
/// fn swap(tx: &mut buzz_db::AdmittedTx, other: &mut sqlx::PgConnection) {
///     std::mem::swap(&mut **tx, other);
/// }
/// ```
///
/// or through `AsMut`:
///
/// ```compile_fail
/// fn swap(tx: &mut buzz_db::AdmittedTx, other: &mut sqlx::PgConnection) {
///     std::mem::swap(AsMut::<sqlx::PgConnection>::as_mut(tx), other);
/// }
/// ```
#[must_use = "dropping an AdmittedTx rolls it back; call commit()"]
pub struct AdmittedTx {
    tx: Transaction<'static, Postgres>,
    community: CommunityId,
    /// Channels whose TTL deadline [`AdmittedTx::commit`] refreshes.
    ttl_refresh_channels: BTreeSet<Uuid>,
    /// Channel head rows [`AdmittedTx::commit`] hands off once committed.
    channel_heads: BTreeMap<Uuid, crate::store::channel_heads::Staged>,
    /// The pool this transaction came from, for that hand-off.
    pool: sqlx::PgPool,
    push_enqueue: Option<super::push_enqueue::PushEnqueue>,
    push_events: Vec<StagedPushEvent>,
}

struct StagedPushEvent {
    event_id: Vec<u8>,
    created_at: DateTime<Utc>,
}

impl AdmittedTx {
    /// Take the shared community admission lock on `tx` and wrap it.
    pub(crate) async fn admit(
        mut tx: Transaction<'static, Postgres>,
        store: &DeletionStore,
        community: CommunityId,
    ) -> Result<Self> {
        store.guard_transaction(&mut tx, community).await?;
        Ok(Self {
            tx,
            community,
            ttl_refresh_channels: BTreeSet::new(),
            channel_heads: BTreeMap::new(),
            pool: store.pool().clone(),
            push_enqueue: None,
            push_events: Vec::new(),
        })
    }

    /// Validate `lease` on `tx` under the community admission lock and wrap
    /// it for the lease's community.
    pub(super) async fn admit_with_serving_lease(
        mut tx: Transaction<'static, Postgres>,
        store: &DeletionStore,
        lease: &ServingWriteLease,
    ) -> Result<Self> {
        store
            .guard_transaction_with_serving_lease(&mut tx, lease)
            .await?;
        Ok(Self {
            tx,
            community: lease.community_id,
            ttl_refresh_channels: BTreeSet::new(),
            channel_heads: BTreeMap::new(),
            pool: store.pool().clone(),
            push_enqueue: None,
            push_events: Vec::new(),
        })
    }

    pub(crate) fn set_push_enqueue(&mut self, producer: Option<super::push_enqueue::PushEnqueue>) {
        self.push_enqueue = producer;
    }

    pub(crate) fn record_push_event(
        &mut self,
        event_id: &[u8],
        created_at: DateTime<Utc>,
        kind: i32,
    ) {
        if self.push_enqueue.is_some()
            && crate::store::event_follow_up::PUSH_MATCH_KINDS.contains(&kind)
        {
            // Bound transaction-local staging as well as the shared worker queue.
            if self.push_events.len() < super::push_enqueue::CAPACITY {
                self.push_events.push(StagedPushEvent {
                    event_id: event_id.to_vec(),
                    created_at,
                });
            } else {
                super::push_enqueue::record_drop("transaction_full");
            }
        }
    }

    /// The community this transaction was admitted for.
    pub fn community(&self) -> CommunityId {
        self.community
    }

    /// The admitted connection, for statements inside this crate.
    ///
    /// Crate-private on purpose: a public `&mut PgConnection` would let safe
    /// code `std::mem::swap` another connection in while keeping the proof of
    /// admission.
    pub(crate) fn conn(&mut self) -> &mut PgConnection {
        &mut self.tx
    }

    /// Record that an event committed with this transaction belongs to
    /// `channel_id`, so [`AdmittedTx::commit`] refreshes the channel's TTL.
    pub(crate) fn record_channel_event(&mut self, channel_id: Option<Uuid>, kind: i32) {
        if let Some(channel) = crate::store::event_follow_up::ttl_refresh_channel(channel_id, kind)
        {
            self.ttl_refresh_channels.insert(channel);
        }
    }

    /// What this transaction contributes to `channel`'s head row.
    pub(crate) fn channel_head(
        &mut self,
        channel: Uuid,
    ) -> &mut crate::store::channel_heads::Staged {
        self.channel_heads.entry(channel).or_default()
    }

    /// Commit the transaction. This is the only commit path for admitted
    /// writes.
    ///
    /// Refreshes the TTL of every channel that received an event as the last
    /// statements before COMMIT. Channel head rows are handed off only after
    /// the commit, so no message waits on another message's row.
    pub async fn commit(mut self) -> Result<()> {
        if !self.ttl_refresh_channels.is_empty() {
            crate::store::event_follow_up::refresh_channel_ttls(
                &mut self.tx,
                self.community,
                &self.ttl_refresh_channels,
            )
            .await?;
        }
        self.tx.commit().await?;
        if !self.channel_heads.is_empty() {
            crate::store::channel_heads::submit(&self.pool, self.community, self.channel_heads);
        }
        if let Some(producer) = self.push_enqueue {
            for event in self.push_events {
                producer.submit(self.community, event.event_id, event.created_at);
            }
        }
        Ok(())
    }

    /// Roll the transaction back explicitly.
    pub async fn rollback(self) -> Result<()> {
        self.tx.rollback().await?;
        Ok(())
    }
}

impl std::fmt::Debug for AdmittedTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmittedTx")
            .field("community", &self.community)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "admitted_tx/postgres_tests.rs"]
mod postgres_tests;
