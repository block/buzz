//! Fail-closed compatibility contract for optional public projection.
//!
//! The client-production slice replaces this interface with durable storage
//! beside its owning migration. Until then every database entrypoint denies.

use std::fmt;

use buzz_core::{CommunityId, StoredEvent};
use nostr::Event;
use uuid::Uuid;

use crate::{Db, DbError, Result};

fn unavailable<T>() -> Result<T> {
    Err(DbError::InvalidData(
        "public projection storage is not installed".to_owned(),
    ))
}

/// Opaque source generation for one public projection.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ProjectionBindingOrigin {
    _sealed: (),
}

impl ProjectionBindingOrigin {
    /// Stable binding identifier used only for server-side fencing.
    pub const fn binding_id(self) -> Uuid {
        unreachable!()
    }

    /// Positive binding generation used only for server-side fencing.
    pub const fn binding_version(self) -> u64 {
        unreachable!()
    }
}

impl fmt::Debug for ProjectionBindingOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProjectionBindingOrigin([redacted])")
    }
}

/// Server-only disposition for a public projection head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionDisposition {
    /// A current binding owns a label-bearing assertion.
    Active,
    /// The assertion is the canonical inactive replacement.
    Inactive,
}

/// Current server-only ownership metadata for an assertion coordinate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectionHead {
    _sealed: (),
}

impl ProjectionHead {
    /// Exact signed event installed with this ownership record.
    pub const fn event_id(self) -> [u8; 32] {
        unreachable!()
    }

    /// Whether the current projection is active or inactive.
    pub const fn disposition(self) -> ProjectionDisposition {
        unreachable!()
    }

    /// Exact binding generation that created the current projection.
    pub const fn origin(self) -> Option<ProjectionBindingOrigin> {
        unreachable!()
    }
}

/// Transaction-owned active publication permit.
#[derive(Debug)]
pub struct ActivePublicProjectionPermit {
    _sealed: (),
}

impl ActivePublicProjectionPermit {
    /// Current stored projection at the locked coordinate.
    pub fn current_projection(&self) -> Option<&StoredEvent> {
        unreachable!()
    }

    /// Exact active binding generation retained through commit.
    pub const fn origin(&self) -> ProjectionBindingOrigin {
        unreachable!()
    }

    /// Commit is unavailable before the client-production migration.
    pub async fn commit(
        self,
        _event: &Event,
        _disposition: ProjectionDisposition,
    ) -> Result<StoredEvent> {
        unavailable()
    }
}

/// Begin active publication; denied before durable projection storage exists.
pub async fn begin_active_public_projection(
    _db: &Db,
    _community_id: CommunityId,
    _relay_pubkey: &[u8],
    _issuer: &str,
    _subject: &str,
    _subject_pubkey: &[u8],
) -> Result<Option<ActivePublicProjectionPermit>> {
    unavailable()
}

/// Materialization is unavailable before durable projection storage exists.
pub async fn materialize_public_projection_retirements(
    _db: &Db,
    _domains: &[CommunityId],
    _relay_pubkey: &[u8],
) -> Result<u64> {
    unavailable()
}

/// Retryable retirement work claimed by one relay replica.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionRetirementClaim {
    _sealed: (),
}

impl ProjectionRetirementClaim {
    /// Server-resolved authorization domain.
    pub const fn community_id(&self) -> CommunityId {
        unreachable!()
    }

    /// Public subject key whose assertion may need retirement.
    pub fn old_pubkey(&self) -> &[u8] {
        unreachable!()
    }

    /// Exact retired source generation, when available.
    pub const fn source_origin(&self) -> Option<ProjectionBindingOrigin> {
        unreachable!()
    }
}

/// Claiming is unavailable before durable projection storage exists.
pub async fn claim_public_projection_retirement(
    _db: &Db,
    _domains: &[CommunityId],
    _relay_pubkey: &[u8],
) -> Result<Option<ProjectionRetirementClaim>> {
    unavailable()
}

/// Transaction-owned view of one claimed retirement.
#[derive(Debug)]
pub struct ProjectionRetirementPermit {
    _sealed: (),
}

impl ProjectionRetirementPermit {
    /// Public subject key selected by the lifecycle operation.
    pub fn old_pubkey(&self) -> &[u8] {
        unreachable!()
    }

    /// Current event at the locked assertion coordinate.
    pub fn current_projection(&self) -> Option<&StoredEvent> {
        unreachable!()
    }

    /// Current private projection ownership metadata.
    pub const fn head(&self) -> Option<ProjectionHead> {
        unreachable!()
    }

    /// Current active binding for the key, if reused.
    pub const fn active_origin(&self) -> Option<ProjectionBindingOrigin> {
        unreachable!()
    }

    /// Retired binding generation carried by durable work.
    pub const fn source_origin(&self) -> Option<ProjectionBindingOrigin> {
        unreachable!()
    }

    /// Completion is unavailable before durable projection storage exists.
    pub async fn finish_no_projection(self) -> Result<()> {
        unavailable()
    }

    /// Completion is unavailable before durable projection storage exists.
    pub async fn finish_superseded(self, _current: &Event) -> Result<()> {
        unavailable()
    }

    /// Completion is unavailable before durable projection storage exists.
    pub async fn finish_newer_projection(self) -> Result<()> {
        unavailable()
    }

    /// Completion is unavailable before durable projection storage exists.
    pub async fn finish_existing_inactive(self) -> Result<()> {
        unavailable()
    }

    /// Completion is unavailable before durable projection storage exists.
    pub async fn finish_inactive(self, _inactive: &Event) -> Result<StoredEvent> {
        unavailable()
    }

    /// Retry deferral is unavailable before durable projection storage exists.
    pub async fn defer(self) -> Result<()> {
        unavailable()
    }
}

/// Begin retirement; denied before durable projection storage exists.
pub async fn begin_public_projection_retirement(
    _db: &Db,
    _claim: ProjectionRetirementClaim,
) -> Result<ProjectionRetirementPermit> {
    unavailable()
}

/// Delivery work retained until local and replicated fan-out complete.
#[derive(Debug)]
pub struct ProjectionDeliveryPermit {
    _sealed: (),
}

impl ProjectionDeliveryPermit {
    /// Exact current inactive event.
    pub const fn stored(&self) -> &StoredEvent {
        unreachable!()
    }

    /// Server-resolved authorization domain.
    pub const fn community_id(&self) -> CommunityId {
        unreachable!()
    }

    /// Completion is unavailable before durable projection storage exists.
    pub async fn complete(self) -> Result<()> {
        unavailable()
    }

    /// Retry deferral is unavailable before durable projection storage exists.
    pub async fn defer(self) -> Result<()> {
        unavailable()
    }
}

/// Begin delivery; denied before durable projection storage exists.
pub async fn begin_public_projection_delivery(
    _db: &Db,
    _domains: &[CommunityId],
    _relay_pubkey: &[u8],
) -> Result<Option<ProjectionDeliveryPermit>> {
    unavailable()
}

/// Count unfinished work; denied before durable projection storage exists.
pub async fn unfinished_public_projection_retirements(
    _db: &Db,
    _domains: &[CommunityId],
    _relay_pubkey: &[u8],
) -> Result<u64> {
    unavailable()
}
