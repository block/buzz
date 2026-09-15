//! Order inbound applies after disk-authoritative local definition operations.
//!
//! Local saves and preparation already share the store critical section. A
//! prepared positive head owns a lease before that section releases the store,
//! through signing and retention (including transfer out of blocking tasks).
//! Inbound checks for leases UNDER that same store lock, but waits OUTSIDE it.
//! Thus there is no observable save-before-lease gap and no check/apply race.
//!
//! Local edits remain serialized by the existing store lock and exact-plan
//! fences; they do not wait on each other's signer. Inbound waits for ALL local
//! operations at its coordinate. This also handles multi-record saves without
//! ordered multi-lock acquisition or recursive-lock deadlocks. Leases contain
//! no timestamps or replay data: every deferred event is evaluated normally
//! against the durable head after the leases end. Failure/cancellation drops
//! the lease, leaving the existing disk-authoritative retry policy unchanged.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};
use tokio::sync::watch;

#[derive(Hash, PartialEq, Eq)]
struct Coordinate {
    // Retention paths include the storage root and canonical relay scope.
    db: PathBuf,
    owner: String,
    kind: u32,
    d_tag: String,
}

type Registry = HashMap<Coordinate, Weak<watch::Sender<()>>>;
static OPERATIONS: OnceLock<Mutex<Registry>> = OnceLock::new();

fn coordinate(db: &Path, owner: &str, kind: u32, d_tag: &str) -> Coordinate {
    Coordinate {
        db: db.to_owned(),
        owner: owner.to_owned(),
        kind,
        d_tag: d_tag.to_owned(),
    }
}

/// Kept in the prepared/signed head, not in the calling async stack: aborting
/// an awaiting command cannot release a still-running blocking save's lease.
#[derive(Debug)]
pub(crate) struct DefinitionMutation {
    _lease: Arc<watch::Sender<()>>,
}

impl DefinitionMutation {
    /// Call inside the same store critical section as the local save, before
    /// releasing that section. Also used by boot's read/prepare critical section.
    pub(crate) fn begin(db: &Path, owner: &str, kind: u32, d_tag: &str) -> Result<Self, String> {
        let mut registry = OPERATIONS
            .get_or_init(Mutex::default)
            .lock()
            .map_err(|e| e.to_string())?;
        registry.retain(|_, lease| lease.strong_count() != 0);
        let slot = registry
            .entry(coordinate(db, owner, kind, d_tag))
            .or_default();
        let lease = match slot.upgrade() {
            Some(lease) => lease,
            None => {
                let (sender, _) = watch::channel(());
                let lease = Arc::new(sender);
                *slot = Arc::downgrade(&lease);
                lease
            }
        };
        Ok(Self { _lease: lease })
    }
}

/// A receiver does NOT own a lease. Channel closure wakes every inbound waiter
/// when the last local operation finishes, fails, or is cancelled.
#[derive(Debug)]
pub(crate) struct DefinitionWaiter(watch::Receiver<()>);

impl DefinitionWaiter {
    pub(crate) async fn wait(mut self) {
        while self.0.changed().await.is_ok() {}
    }
}

/// Check under the store lock immediately before inbound decision + apply.
/// A caller seeing Some must release store/SQLite guards, await, then retry the
/// entire decision (including arrival scope). Never discard the inbound event.
pub(crate) fn pending(
    db: &Path,
    owner: &str,
    kind: u32,
    d_tag: &str,
) -> Result<Option<DefinitionWaiter>, String> {
    let mut registry = OPERATIONS
        .get_or_init(Mutex::default)
        .lock()
        .map_err(|e| e.to_string())?;
    registry.retain(|_, lease| lease.strong_count() != 0);
    Ok(registry
        .get(&coordinate(db, owner, kind, d_tag))
        .and_then(Weak::upgrade)
        .map(|lease| DefinitionWaiter(lease.subscribe())))
}
