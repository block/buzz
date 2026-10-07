//! Community-admitted event-write transactions.

use buzz_core::CommunityId;
use sqlx::{PgConnection, Postgres, Transaction};

use crate::deletion::{DeletionStore, ServingWriteLease};
use crate::Result;

/// An event-write transaction that has passed community admission.
///
/// Its only constructors take the shared community admission lock (or
/// validate a serving write lease) on the transaction they wrap, for the
/// community they record, so construction and admission are one step.
/// Event-write helpers take `&mut AdmittedTx` and read the community from it,
/// so the compiler rejects a raw [`sqlx::Transaction`] or a transaction
/// admitted for a different community.
///
/// Outside this crate the value exposes neither the inner transaction nor its
/// connection, so code there can't swap in another connection or run
/// statements on it, and [`AdmittedTx::commit`] and [`AdmittedTx::rollback`]
/// are the only ways it can end the transaction. Inside the crate, the
/// `conn()` accessor hands statements the connection; the type can't stop
/// crate code from issuing a raw `COMMIT` through it, so the database
/// community write fences remain the backstop. Dropping the value without
/// committing rolls back, as with [`sqlx::Transaction`].
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
}

impl AdmittedTx {
    /// Take the shared community admission lock on `tx` and wrap it.
    pub(super) async fn admit(
        mut tx: Transaction<'static, Postgres>,
        store: &DeletionStore,
        community: CommunityId,
    ) -> Result<Self> {
        store.guard_transaction(&mut tx, community).await?;
        Ok(Self { tx, community })
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
        })
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

    /// Commit the transaction. This is the only commit path for admitted
    /// event writes.
    pub async fn commit(self) -> Result<()> {
        self.tx.commit().await?;
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
