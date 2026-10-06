//! Community-admitted event-write transactions.

use std::ops::{Deref, DerefMut};

use buzz_core::CommunityId;
use sqlx::{PgConnection, Postgres, Transaction};

use crate::Result;

/// An event-write transaction that has passed community admission.
///
/// Only the admission chokepoints in this crate can construct one, and only
/// after taking the shared community admission lock (or validating a serving
/// write lease) on the transaction. Event-write helpers take `&mut AdmittedTx`
/// and read the community from it, so the compiler rejects a raw
/// [`sqlx::Transaction`] or a transaction admitted for a different community.
///
/// The value dereferences to the underlying [`PgConnection`] for statements,
/// but never exposes the inner transaction: [`AdmittedTx::commit`] and
/// [`AdmittedTx::rollback`] are the only ways to end it. Dropping it without
/// committing rolls back, as with [`sqlx::Transaction`].
#[must_use = "dropping an AdmittedTx rolls it back; call commit()"]
pub struct AdmittedTx {
    tx: Transaction<'static, Postgres>,
    community: CommunityId,
}

impl AdmittedTx {
    /// Wrap a transaction that has already been admitted for `community`.
    ///
    /// Callers must have taken the community admission lock (or validated a
    /// serving lease for `community`) on `tx` before calling this. The source
    /// policy in `tests/observability_source.rs` pins every call site.
    pub(super) fn admitted(tx: Transaction<'static, Postgres>, community: CommunityId) -> Self {
        Self { tx, community }
    }

    /// The community this transaction was admitted for.
    pub fn community(&self) -> CommunityId {
        self.community
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

impl Deref for AdmittedTx {
    type Target = PgConnection;

    fn deref(&self) -> &PgConnection {
        &self.tx
    }
}

impl DerefMut for AdmittedTx {
    fn deref_mut(&mut self) -> &mut PgConnection {
        &mut self.tx
    }
}

impl AsMut<PgConnection> for AdmittedTx {
    fn as_mut(&mut self) -> &mut PgConnection {
        &mut self.tx
    }
}

impl std::fmt::Debug for AdmittedTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmittedTx")
            .field("community", &self.community)
            .finish_non_exhaustive()
    }
}
