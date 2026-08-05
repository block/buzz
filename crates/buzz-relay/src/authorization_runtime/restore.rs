//! Object-store witnessed high-water protection against stale PostgreSQL restore.
//!
//! The existing object store is the independent durability domain. A protected
//! mutation writes pending before its PostgreSQL commit and advances the
//! committed vector afterward. Startup refuses any database below a witnessed
//! floor and refuses an ambiguous pending checkpoint.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use buzz_core::CommunityId;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{Mutex, OwnedMutexGuard};
use uuid::Uuid;

use crate::api::git::store::{CasOutcome, ETag, GitStore, Precond};

const FORMAT_VERSION: u32 = 2;
const BEGIN_RETRY_LIMIT: usize = 16;
const BEGIN_RETRY_DELAY: Duration = Duration::from_millis(10);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct VersionVector {
    bindings: BTreeMap<String, u64>,
    git_publications: BTreeMap<String, u64>,
    media_publications: BTreeMap<String, u64>,
    object_authority: BTreeMap<String, u64>,
    invalidation_generation: u64,
    authority_epoch: u64,
    status_revision: u64,
}

impl From<buzz_db::authorization_version::AuthorizationVersionVector> for VersionVector {
    fn from(value: buzz_db::authorization_version::AuthorizationVersionVector) -> Self {
        Self {
            bindings: value.bindings,
            git_publications: value.git_publications,
            media_publications: value.media_publications,
            object_authority: value.object_authority,
            invalidation_generation: value.invalidation_generation,
            authority_epoch: value.authority_epoch,
            status_revision: value.status_revision,
        }
    }
}

impl VersionVector {
    fn to_db(&self) -> buzz_db::authorization_version::AuthorizationVersionVector {
        buzz_db::authorization_version::AuthorizationVersionVector {
            bindings: self.bindings.clone(),
            git_publications: self.git_publications.clone(),
            media_publications: self.media_publications.clone(),
            object_authority: self.object_authority.clone(),
            invalidation_generation: self.invalidation_generation,
            authority_epoch: self.authority_epoch,
            status_revision: self.status_revision,
        }
    }

    fn dominates(&self, floor: &Self) -> bool {
        self.to_db().dominates(&floor.to_db())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum CheckpointState {
    Committed,
    Pending {
        operation_id: Uuid,
        request_fingerprint: [u8; 32],
        previous: VersionVector,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Checkpoint {
    format_version: u32,
    community_id: Uuid,
    bootstrap_id: Uuid,
    vector: VersionVector,
    #[serde(flatten)]
    state: CheckpointState,
}

/// Disabled-by-default restore witness runtime.
pub struct RestoreProtectionRuntime {
    db: buzz_db::Db,
    store: GitStore,
    domains: BTreeMap<CommunityId, Arc<Mutex<()>>>,
    bootstrap_ids: BTreeMap<CommunityId, Uuid>,
}

impl RestoreProtectionRuntime {
    /// Initialize exact configured domains and verify every external floor.
    pub async fn initialize(
        db: buzz_db::Db,
        store: GitStore,
        domains: impl IntoIterator<Item = (CommunityId, Uuid)>,
    ) -> Result<Arc<Self>, RestoreProtectionError> {
        let configured = domains.into_iter().collect::<BTreeMap<_, _>>();
        if configured
            .iter()
            .any(|(domain, bootstrap)| domain.as_uuid().is_nil() || bootstrap.is_nil())
        {
            return Err(RestoreProtectionError::InvalidBootstrap);
        }
        let runtime = Arc::new(Self {
            db,
            store,
            domains: configured
                .keys()
                .copied()
                .map(|domain| (domain, Arc::new(Mutex::new(()))))
                .collect(),
            bootstrap_ids: configured,
        });
        for domain in runtime.domains.keys().copied().collect::<Vec<_>>() {
            runtime.verify_or_initialize(domain).await?;
        }
        Ok(runtime)
    }

    /// Explicitly provision the independent witness before enabling Enforce.
    /// Normal runtime startup never calls this method and never initializes a
    /// missing checkpoint from potentially restored PostgreSQL state.
    pub async fn provision_domain(
        db: &buzz_db::Db,
        store: &GitStore,
        domain: CommunityId,
        bootstrap_id: Uuid,
    ) -> Result<(), RestoreProtectionError> {
        if bootstrap_id.is_nil() {
            return Err(RestoreProtectionError::InvalidBootstrap);
        }
        let checkpoint = Checkpoint {
            format_version: FORMAT_VERSION,
            community_id: *domain.as_uuid(),
            bootstrap_id,
            vector: db.authorization_version_vector(domain).await?.into(),
            state: CheckpointState::Committed,
        };
        let body = serde_json::to_vec(&checkpoint)
            .map_err(|_| RestoreProtectionError::InvalidCheckpoint)?;
        match store
            .put_pointer(&checkpoint_key(domain), &body, Precond::IfNoneMatchStar)
            .await?
        {
            CasOutcome::Won(_) => Ok(()),
            CasOutcome::LostRace => Err(RestoreProtectionError::AlreadyProvisioned),
        }
    }

    fn mutex(&self, domain: CommunityId) -> Result<Arc<Mutex<()>>, RestoreProtectionError> {
        self.domains
            .get(&domain)
            .cloned()
            .ok_or(RestoreProtectionError::DomainNotConfigured)
    }

    fn bootstrap_id(&self, domain: CommunityId) -> Result<Uuid, RestoreProtectionError> {
        self.bootstrap_ids
            .get(&domain)
            .copied()
            .ok_or(RestoreProtectionError::DomainNotConfigured)
    }

    async fn current(&self, domain: CommunityId) -> Result<VersionVector, RestoreProtectionError> {
        Ok(self.db.authorization_version_vector(domain).await?.into())
    }

    async fn read(
        &self,
        domain: CommunityId,
    ) -> Result<Option<(ETag, Checkpoint)>, RestoreProtectionError> {
        let Some((etag, bytes)) = self.store.get_pointer(&checkpoint_key(domain)).await? else {
            return Ok(None);
        };
        let checkpoint: Checkpoint = serde_json::from_slice(&bytes)
            .map_err(|_| RestoreProtectionError::InvalidCheckpoint)?;
        if checkpoint.format_version != FORMAT_VERSION
            || checkpoint.community_id != *domain.as_uuid()
            || matches!(
                &checkpoint.state,
                CheckpointState::Pending {
                    operation_id,
                    previous,
                    ..
                } if operation_id.is_nil() || checkpoint.vector != *previous
            )
        {
            return Err(RestoreProtectionError::InvalidCheckpoint);
        }
        Ok(Some((etag, checkpoint)))
    }

    async fn verify_or_initialize(
        &self,
        domain: CommunityId,
    ) -> Result<(), RestoreProtectionError> {
        let _guard = self.mutex(domain)?.lock_owned().await;
        for attempt in 0..=BEGIN_RETRY_LIMIT {
            match self.read(domain).await? {
                None => return Err(RestoreProtectionError::MissingCheckpoint),
                Some((etag, checkpoint)) => {
                    if checkpoint.bootstrap_id != self.bootstrap_id(domain)? {
                        return Err(RestoreProtectionError::InvalidBootstrap);
                    }
                    if let CheckpointState::Pending {
                        operation_id,
                        request_fingerprint,
                        previous,
                    } = &checkpoint.state
                    {
                        let (fingerprint, current) = self
                            .db
                            .authorization_receipt_and_version_vector(domain, *operation_id)
                            .await?;
                        let current = VersionVector::from(current);
                        let recovered_vector = match fingerprint {
                            Some(fingerprint) if fingerprint == *request_fingerprint => {
                                validate_recovered_vector(previous, &current)?;
                                current
                            }
                            // The receipt belongs to another request that reused
                            // this operation ID. If authority never moved, this
                            // pending request provably did not commit and an older
                            // poisoned checkpoint can be cleared safely.
                            Some(_) if current == *previous => previous.clone(),
                            _ => return Err(RestoreProtectionError::AmbiguousInterruptedCommit),
                        };
                        let recovered = Checkpoint {
                            format_version: FORMAT_VERSION,
                            community_id: *domain.as_uuid(),
                            bootstrap_id: checkpoint.bootstrap_id,
                            vector: recovered_vector,
                            state: CheckpointState::Committed,
                        };
                        match put_exact(&self.store, domain, &recovered, etag).await {
                            Ok(()) => return Ok(()),
                            Err(RestoreProtectionError::ConcurrentCheckpoint)
                                if attempt < BEGIN_RETRY_LIMIT =>
                            {
                                tokio::time::sleep(BEGIN_RETRY_DELAY).await;
                                continue;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    let current = self.current(domain).await?;
                    if !current.dominates(&checkpoint.vector) {
                        return Err(RestoreProtectionError::StaleRestore);
                    }
                    if current == checkpoint.vector {
                        return Ok(());
                    }
                    return Err(RestoreProtectionError::UnwitnessedAuthorityAdvance);
                }
            }
        }
        Err(RestoreProtectionError::ConcurrentCheckpoint)
    }

    /// Witness intent before a PostgreSQL mutation that may advance protected
    /// binding or publication versions.
    pub async fn begin(
        self: &Arc<Self>,
        domain: CommunityId,
        operation_id: Uuid,
        request_fingerprint: [u8; 32],
    ) -> Result<RestoreMutationGuard, RestoreProtectionError> {
        if operation_id.is_nil() {
            return Err(RestoreProtectionError::InvalidOperation);
        }
        let lock = self.mutex(domain)?.lock_owned().await;
        for attempt in 0..=BEGIN_RETRY_LIMIT {
            let (etag, checkpoint) = self
                .read(domain)
                .await?
                .ok_or(RestoreProtectionError::InvalidCheckpoint)?;
            if checkpoint.bootstrap_id != self.bootstrap_id(domain)? {
                return Err(RestoreProtectionError::InvalidBootstrap);
            }
            if let CheckpointState::Pending {
                operation_id: pending_operation,
                request_fingerprint: pending_fingerprint,
                previous,
            } = &checkpoint.state
            {
                let (fingerprint, committed) = self
                    .db
                    .authorization_receipt_and_version_vector(domain, *pending_operation)
                    .await?;
                let committed = VersionVector::from(committed);
                match fingerprint {
                    Some(fingerprint) if fingerprint == *pending_fingerprint => {
                        validate_recovered_vector(previous, &committed)?;
                        let recovered = Checkpoint {
                            format_version: FORMAT_VERSION,
                            community_id: *domain.as_uuid(),
                            bootstrap_id: checkpoint.bootstrap_id,
                            vector: committed,
                            state: CheckpointState::Committed,
                        };
                        match put_exact_returning(&self.store, domain, &recovered, etag).await {
                            Ok(_) | Err(RestoreProtectionError::ConcurrentCheckpoint) => continue,
                            Err(error) => return Err(error),
                        }
                    }
                    Some(_) if committed == *previous => {
                        let recovered = Checkpoint {
                            format_version: FORMAT_VERSION,
                            community_id: *domain.as_uuid(),
                            bootstrap_id: checkpoint.bootstrap_id,
                            vector: previous.clone(),
                            state: CheckpointState::Committed,
                        };
                        match put_exact_returning(&self.store, domain, &recovered, etag).await {
                            Ok(_) | Err(RestoreProtectionError::ConcurrentCheckpoint) => continue,
                            Err(error) => return Err(error),
                        }
                    }
                    Some(_) => return Err(RestoreProtectionError::AmbiguousInterruptedCommit),
                    None if attempt < BEGIN_RETRY_LIMIT => {
                        tokio::time::sleep(BEGIN_RETRY_DELAY).await;
                        continue;
                    }
                    None => return Err(RestoreProtectionError::AmbiguousInterruptedCommit),
                }
            }
            let (receipt, current) = self
                .db
                .authorization_receipt_and_version_vector(domain, operation_id)
                .await?;
            let current = VersionVector::from(current);
            if self
                .read(domain)
                .await?
                .is_none_or(|(stable_etag, _)| stable_etag != etag)
            {
                if attempt < BEGIN_RETRY_LIMIT {
                    tokio::time::sleep(BEGIN_RETRY_DELAY).await;
                    continue;
                }
                return Err(RestoreProtectionError::ConcurrentCheckpoint);
            }
            if !current.dominates(&checkpoint.vector) {
                return Err(RestoreProtectionError::StaleRestore);
            }
            if current != checkpoint.vector {
                return Err(RestoreProtectionError::UnwitnessedAuthorityAdvance);
            }
            match receipt {
                Some(fingerprint) if fingerprint != request_fingerprint => {
                    return Err(RestoreProtectionError::OperationIdentityConflict)
                }
                Some(_) => {
                    return Ok(RestoreMutationGuard {
                        runtime: Arc::clone(self),
                        domain,
                        operation_id,
                        request_fingerprint,
                        pending_etag: None,
                        previous: checkpoint.vector,
                        bootstrap_id: checkpoint.bootstrap_id,
                        _lock: lock,
                    })
                }
                None => {}
            }
            let pending = Checkpoint {
                format_version: FORMAT_VERSION,
                community_id: *domain.as_uuid(),
                bootstrap_id: checkpoint.bootstrap_id,
                vector: current.clone(),
                state: CheckpointState::Pending {
                    operation_id,
                    request_fingerprint,
                    previous: current,
                },
            };
            match put_exact_returning(&self.store, domain, &pending, etag).await {
                Ok(etag) => {
                    return Ok(RestoreMutationGuard {
                        runtime: Arc::clone(self),
                        domain,
                        operation_id,
                        request_fingerprint,
                        pending_etag: Some(etag),
                        previous: checkpoint.vector,
                        bootstrap_id: checkpoint.bootstrap_id,
                        _lock: lock,
                    })
                }
                Err(RestoreProtectionError::ConcurrentCheckpoint)
                    if attempt < BEGIN_RETRY_LIMIT =>
                {
                    tokio::time::sleep(BEGIN_RETRY_DELAY).await;
                }
                Err(error) => return Err(error),
            }
        }
        Err(RestoreProtectionError::ConcurrentCheckpoint)
    }
}

fn validate_recovered_vector(
    previous: &VersionVector,
    committed: &VersionVector,
) -> Result<(), RestoreProtectionError> {
    if committed.dominates(previous) {
        Ok(())
    } else {
        Err(RestoreProtectionError::StaleRestore)
    }
}

/// Serialized pending witness held until the PostgreSQL commit is durable.
pub struct RestoreMutationGuard {
    runtime: Arc<RestoreProtectionRuntime>,
    domain: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    pending_etag: Option<ETag>,
    previous: VersionVector,
    bootstrap_id: Uuid,
    _lock: OwnedMutexGuard<()>,
}

impl RestoreMutationGuard {
    /// Advance the independent committed floor after PostgreSQL commit.
    pub async fn commit(self) -> Result<(), RestoreProtectionError> {
        let (receipt, current) = self
            .runtime
            .db
            .authorization_receipt_and_version_vector(self.domain, self.operation_id)
            .await?;
        if receipt != Some(self.request_fingerprint) {
            return Err(RestoreProtectionError::AmbiguousInterruptedCommit);
        }
        let current = VersionVector::from(current);
        if !current.dominates(&self.previous) {
            return Err(RestoreProtectionError::StaleRestore);
        }
        let checkpoint = Checkpoint {
            format_version: FORMAT_VERSION,
            community_id: *self.domain.as_uuid(),
            bootstrap_id: self.bootstrap_id,
            vector: current.clone(),
            state: CheckpointState::Committed,
        };
        let Some(etag) = self.pending_etag.clone() else {
            return Ok(());
        };
        match put_exact(&self.runtime.store, self.domain, &checkpoint, etag).await {
            Ok(()) => Ok(()),
            Err(RestoreProtectionError::ConcurrentCheckpoint) => {
                self.converge_committed(&current).await
            }
            Err(error) => Err(error),
        }
    }

    /// Advance an invalidation witness, converging after a stale CAS when a
    /// competing replica already witnessed the exact durable generation.
    pub async fn commit_invalidation(self, generation: u64) -> Result<(), RestoreProtectionError> {
        let (receipt, current) = self
            .runtime
            .db
            .authorization_receipt_and_version_vector(self.domain, self.operation_id)
            .await?;
        if receipt != Some(self.request_fingerprint) {
            return Err(RestoreProtectionError::AmbiguousInterruptedCommit);
        }
        let current = VersionVector::from(current);
        if !current.dominates(&self.previous) || current.invalidation_generation < generation {
            return Err(RestoreProtectionError::StaleRestore);
        }
        if let Some(etag) = self.pending_etag.clone() {
            let checkpoint = Checkpoint {
                format_version: FORMAT_VERSION,
                community_id: *self.domain.as_uuid(),
                bootstrap_id: self.bootstrap_id,
                vector: current.clone(),
                state: CheckpointState::Committed,
            };
            match put_exact(&self.runtime.store, self.domain, &checkpoint, etag).await {
                Ok(()) => return Ok(()),
                Err(RestoreProtectionError::ConcurrentCheckpoint) => {}
                Err(error) => return Err(error),
            }
        }
        self.converge_committed(&current).await
    }

    /// Converge a stale object-store CAS only when PostgreSQL still proves this
    /// exact operation and the competing checkpoint covers the vector observed
    /// after its durable commit. A different or regressed state never becomes
    /// committed as a side effect of reconciliation.
    async fn converge_committed(
        &self,
        committed_floor: &VersionVector,
    ) -> Result<(), RestoreProtectionError> {
        for attempt in 0..=BEGIN_RETRY_LIMIT {
            let (receipt, current) = self
                .runtime
                .db
                .authorization_receipt_and_version_vector(self.domain, self.operation_id)
                .await?;
            let current = VersionVector::from(current);
            if receipt != Some(self.request_fingerprint)
                || !current.dominates(&self.previous)
                || !current.dominates(committed_floor)
            {
                return Err(RestoreProtectionError::StaleRestore);
            }
            let (etag, checkpoint) = self
                .runtime
                .read(self.domain)
                .await?
                .ok_or(RestoreProtectionError::InvalidCheckpoint)?;
            if checkpoint.bootstrap_id != self.bootstrap_id {
                return Err(RestoreProtectionError::InvalidBootstrap);
            }
            if checkpoint_covers(&checkpoint, committed_floor) {
                let Some(effective) = effective_checkpoint_vector(&checkpoint) else {
                    return Err(RestoreProtectionError::InvalidCheckpoint);
                };
                if !current.dominates(effective) {
                    return Err(RestoreProtectionError::StaleRestore);
                }
                return Ok(());
            }
            if let CheckpointState::Pending {
                operation_id,
                request_fingerprint,
                previous,
            } = &checkpoint.state
            {
                let (receipt, current) = self
                    .runtime
                    .db
                    .authorization_receipt_and_version_vector(self.domain, *operation_id)
                    .await?;
                let current = VersionVector::from(current);
                if receipt == Some(*request_fingerprint) && current.dominates(previous) {
                    let recovered = Checkpoint {
                        format_version: FORMAT_VERSION,
                        community_id: *self.domain.as_uuid(),
                        bootstrap_id: self.bootstrap_id,
                        vector: current,
                        state: CheckpointState::Committed,
                    };
                    if !checkpoint_covers(&recovered, committed_floor) {
                        return Err(RestoreProtectionError::StaleRestore);
                    }
                    match put_exact(&self.runtime.store, self.domain, &recovered, etag).await {
                        Ok(()) => return Ok(()),
                        Err(RestoreProtectionError::ConcurrentCheckpoint)
                            if attempt < BEGIN_RETRY_LIMIT =>
                        {
                            tokio::time::sleep(BEGIN_RETRY_DELAY).await;
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
                }
                if receipt.is_none() && attempt < BEGIN_RETRY_LIMIT {
                    tokio::time::sleep(BEGIN_RETRY_DELAY).await;
                    continue;
                }
            }
            return Err(RestoreProtectionError::ConcurrentCheckpoint);
        }
        Err(RestoreProtectionError::ConcurrentCheckpoint)
    }

    /// Clear a pending witness only after PostgreSQL proved the transaction
    /// rolled back. The previous committed vector is restored under the exact
    /// pending ETag; a concurrent writer remains fail-closed.
    pub async fn abort(self) -> Result<(), RestoreProtectionError> {
        if self
            .runtime
            .db
            .authorization_operation_receipt_fingerprint(self.domain, self.operation_id)
            .await?
            .is_some_and(|fingerprint| fingerprint == self.request_fingerprint)
        {
            return Err(RestoreProtectionError::CommitAlreadyDurable);
        }
        let current = self.runtime.current(self.domain).await?;
        if current != self.previous {
            return Err(RestoreProtectionError::AmbiguousInterruptedCommit);
        }
        let checkpoint = Checkpoint {
            format_version: FORMAT_VERSION,
            community_id: *self.domain.as_uuid(),
            bootstrap_id: self.bootstrap_id,
            vector: self.previous,
            state: CheckpointState::Committed,
        };
        let Some(etag) = self.pending_etag.clone() else {
            return Err(RestoreProtectionError::CommitAlreadyDurable);
        };
        put_exact(&self.runtime.store, self.domain, &checkpoint, etag).await
    }
}

fn checkpoint_covers(checkpoint: &Checkpoint, committed_floor: &VersionVector) -> bool {
    effective_checkpoint_vector(checkpoint)
        .is_some_and(|effective| effective.dominates(committed_floor))
}

fn effective_checkpoint_vector(checkpoint: &Checkpoint) -> Option<&VersionVector> {
    Some(match &checkpoint.state {
        CheckpointState::Committed => &checkpoint.vector,
        CheckpointState::Pending {
            previous: pending_previous,
            ..
        } if checkpoint.vector == *pending_previous => pending_previous,
        CheckpointState::Pending { .. } => return None,
    })
}

async fn put_exact(
    store: &GitStore,
    domain: CommunityId,
    checkpoint: &Checkpoint,
    etag: ETag,
) -> Result<(), RestoreProtectionError> {
    put_exact_returning(store, domain, checkpoint, etag)
        .await
        .map(|_| ())
}

async fn put_exact_returning(
    store: &GitStore,
    domain: CommunityId,
    checkpoint: &Checkpoint,
    etag: ETag,
) -> Result<ETag, RestoreProtectionError> {
    let body =
        serde_json::to_vec(checkpoint).map_err(|_| RestoreProtectionError::InvalidCheckpoint)?;
    match store
        .put_pointer(&checkpoint_key(domain), &body, Precond::IfMatch(etag))
        .await?
    {
        CasOutcome::Won(etag) => Ok(etag),
        CasOutcome::LostRace => Err(RestoreProtectionError::ConcurrentCheckpoint),
    }
}

fn checkpoint_key(domain: CommunityId) -> String {
    format!("_authority/{domain}/authorization-version-v1.json")
}

/// Fail-closed restore protection error.
#[derive(Debug, Error)]
pub enum RestoreProtectionError {
    /// The exact domain was not configured for protected authorization.
    #[error("restore protection domain is not configured")]
    DomainNotConfigured,
    /// The immutable bootstrap identity was missing, nil, or mismatched.
    #[error("restore protection bootstrap identity is invalid")]
    InvalidBootstrap,
    /// A protected domain has not been explicitly provisioned.
    #[error("restore protection checkpoint is missing")]
    MissingCheckpoint,
    /// Provisioning raced an existing checkpoint.
    #[error("restore protection checkpoint is already provisioned")]
    AlreadyProvisioned,
    /// A checkpoint was malformed or belonged to another domain.
    #[error("restore protection checkpoint is invalid")]
    InvalidCheckpoint,
    /// The writer database is below an independently witnessed floor.
    #[error("stale PostgreSQL restore detected")]
    StaleRestore,
    /// PostgreSQL authority advanced without a matching pending witness.
    #[error("protected authority advanced without an independent witness")]
    UnwitnessedAuthorityAdvance,
    /// A crash left commit outcome ambiguous; serving is unsafe.
    #[error("interrupted protected commit requires reconciliation")]
    AmbiguousInterruptedCommit,
    /// A caller attempted to clear a pending witness after its exact database
    /// operation had already become durable.
    #[error("protected commit is already durable; pending witness retained")]
    CommitAlreadyDurable,
    /// A stable operation ID was reused with different request bytes.
    #[error("protected operation identity conflicts with an existing receipt")]
    OperationIdentityConflict,
    /// Another writer changed the checkpoint unexpectedly.
    #[error("restore protection checkpoint changed concurrently")]
    ConcurrentCheckpoint,
    /// Operation identity was invalid.
    #[error("restore protection operation identity is invalid")]
    InvalidOperation,
    /// Writer database access failed.
    #[error(transparent)]
    Database(#[from] buzz_db::DbError),
    /// Independent object-store access failed.
    #[error(transparent)]
    Store(#[from] crate::api::git::store::StoreError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_rejects_missing_and_backward_versions() {
        let mut floor = VersionVector {
            bindings: BTreeMap::new(),
            git_publications: BTreeMap::new(),
            media_publications: BTreeMap::new(),
            object_authority: BTreeMap::new(),
            invalidation_generation: 2,
            authority_epoch: 3,
            status_revision: 4,
        };
        floor.bindings.insert("principal".into(), 3);
        let mut current = floor.clone();
        assert!(current.dominates(&floor));
        current.bindings.insert("principal".into(), 2);
        assert!(!current.dominates(&floor));
        current.bindings.clear();
        assert!(!current.dominates(&floor));
        current = floor.clone();
        current.invalidation_generation = 1;
        assert!(!current.dominates(&floor));
    }

    #[test]
    fn every_publication_component_is_monotonic() {
        let mut floor = VersionVector {
            bindings: BTreeMap::from([("binding-fingerprint".into(), 3)]),
            git_publications: BTreeMap::from([("git-fingerprint".into(), 4)]),
            media_publications: BTreeMap::from([("media-fingerprint".into(), 5)]),
            object_authority: BTreeMap::from([("git".into(), 6), ("media".into(), 7)]),
            invalidation_generation: 8,
            authority_epoch: 9,
            status_revision: 10,
        };
        let original = floor.clone();
        for component in [
            "git",
            "media",
            "object",
            "invalidation",
            "authority",
            "status",
        ] {
            let mut current = original.clone();
            match component {
                "git" => current.git_publications.clear(),
                "media" => current.media_publications.clear(),
                "object" => {
                    current.object_authority.insert("git".into(), 5);
                }
                "invalidation" => current.invalidation_generation = 7,
                "authority" => current.authority_epoch = 8,
                "status" => current.status_revision = 9,
                _ => unreachable!(),
            }
            assert!(!current.dominates(&original), "{component} regressed");
        }
        floor.bindings.insert("new-binding".into(), 1);
        assert!(floor.dominates(&original));
    }

    #[test]
    fn checkpoint_wire_contains_only_opaque_version_selectors() {
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let checkpoint = Checkpoint {
            format_version: FORMAT_VERSION,
            community_id: *domain.as_uuid(),
            bootstrap_id: Uuid::new_v4(),
            vector: VersionVector {
                bindings: BTreeMap::from([("opaque-binding-fingerprint".into(), 2)]),
                git_publications: BTreeMap::new(),
                media_publications: BTreeMap::new(),
                object_authority: BTreeMap::new(),
                invalidation_generation: 3,
                authority_epoch: 4,
                status_revision: 5,
            },
            state: CheckpointState::Committed,
        };
        let wire = serde_json::to_string(&checkpoint).expect("checkpoint JSON");
        assert!(wire.contains("opaque-binding-fingerprint"));
        for prohibited in ["issuer", "subject", "display_name", "email", "pubkey"] {
            assert!(!wire.contains(prohibited));
        }
    }

    #[test]
    fn pending_recovery_requires_the_post_receipt_vector() {
        let previous = VersionVector {
            bindings: BTreeMap::new(),
            git_publications: BTreeMap::new(),
            media_publications: BTreeMap::new(),
            object_authority: BTreeMap::new(),
            invalidation_generation: 4,
            authority_epoch: 5,
            status_revision: 6,
        };
        let mut stale_pre_receipt = previous.clone();
        stale_pre_receipt.authority_epoch = 4;
        let mut committed_post_receipt = previous.clone();
        committed_post_receipt.authority_epoch = 6;

        assert!(matches!(
            validate_recovered_vector(&previous, &stale_pre_receipt),
            Err(RestoreProtectionError::StaleRestore)
        ));
        assert!(validate_recovered_vector(&previous, &committed_post_receipt).is_ok());
    }

    #[test]
    fn invalidation_convergence_accepts_only_monotonic_committed_or_later_pending_floors() {
        let domain = Uuid::new_v4();
        let bootstrap_id = Uuid::new_v4();
        let previous = VersionVector {
            invalidation_generation: 4,
            authority_epoch: 7,
            status_revision: 3,
            ..VersionVector {
                bindings: BTreeMap::new(),
                git_publications: BTreeMap::new(),
                media_publications: BTreeMap::new(),
                object_authority: BTreeMap::new(),
                invalidation_generation: 0,
                authority_epoch: 0,
                status_revision: 0,
            }
        };
        let mut covered = previous.clone();
        covered.invalidation_generation = 5;
        let committed = Checkpoint {
            format_version: FORMAT_VERSION,
            community_id: domain,
            bootstrap_id,
            vector: covered.clone(),
            state: CheckpointState::Committed,
        };
        assert!(checkpoint_covers(&committed, &covered));

        let mut later_floor = covered.clone();
        later_floor.invalidation_generation = 6;
        assert!(!checkpoint_covers(&committed, &later_floor));

        let later_pending = Checkpoint {
            format_version: FORMAT_VERSION,
            community_id: domain,
            bootstrap_id,
            vector: covered.clone(),
            state: CheckpointState::Pending {
                operation_id: Uuid::new_v4(),
                request_fingerprint: [8; 32],
                previous: covered.clone(),
            },
        };
        assert!(checkpoint_covers(&later_pending, &covered));

        let mut malformed = later_pending;
        malformed.vector.invalidation_generation = 6;
        assert!(!checkpoint_covers(&malformed, &covered));

        let mut regressed = committed;
        regressed.vector.authority_epoch = previous.authority_epoch - 1;
        assert!(!checkpoint_covers(&regressed, &covered));
    }
}
