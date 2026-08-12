//! Operation-bound restore witness for protected authorization mutations.
//!
//! The external store contains fixed, immutable, bounded CAS shards of
//! owner-leased operation intents and one independently CAS-guarded record per
//! exact authority component. It never stores or compares a domain-global
//! version vector, and unrelated component advances never serialize on one
//! domain object.

use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use crate::{
    authorization_version::{
        AuthorizationOperationFence, AuthorizationOperationFenceAcquireError,
        AuthorizationOperationVersionDeltaManifest, AuthorizationVersionComponentFloor,
        AuthorizationVersionComponentKind, ProtectedPublicationCommit, ProtectedPublicationError,
        ProtectedPublicationRequest, ProtectedPublicationRestoreIdentity,
    },
    Db, DbError,
};
use async_trait::async_trait;
use buzz_core::CommunityId;
use dashmap::DashSet;
use s3::{creds::Credentials, error::S3Error, Bucket, Region};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

const FORMAT_VERSION: u8 = 1;
const OPERATION_SHARD_FORMAT_VERSION: u8 = 1;
const OPERATION_SHARD_MAPPING_VERSION: u8 = 1;
const MAX_RECORD_BYTES: u64 = 64 * 1024;
const DEFAULT_LEASE: Duration = Duration::from_secs(30);
const MAX_LEASE: Duration = Duration::from_secs(300);
const MAX_CAS_ATTEMPTS: usize = 16;
const MAX_DOMAIN_FLOORS: usize = 100_000;
const PRODUCTION_OPERATION_SHARD_COUNT: u16 = 256;
const PRODUCTION_OPERATION_SHARD_CAPACITY: u16 = 64;
const PRODUCTION_TERMINAL_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
const MIN_TERMINAL_RETENTION: Duration = Duration::from_secs(5 * 60);
const MAX_TERMINAL_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const MAX_OPERATION_SHARDS: u16 = 4_096;
const MAX_OPERATION_SHARD_CAPACITY: u16 = 128;
const CAS_RETRY_DELAY: Duration = Duration::from_millis(10);
const CAUSAL_LAG_BOUND: Duration = Duration::from_millis(150);
const OPERATION_FENCE_WAIT: Duration = Duration::from_millis(150);
const STORE_CAS_TIMEOUT: Duration = Duration::from_secs(5);
/// Fixed production capacity for detached primary sessions that may own an
/// operation fence. Changing this is a versioned production-layout change.
pub const PRODUCTION_OPERATION_FENCE_CONNECTIONS: u32 = 16;

/// Immutable per-domain external operation-store policy.
#[derive(Clone, Copy, PartialEq, Eq)]
struct OperationRestoreRetentionPolicy {
    shard_count: u16,
    entries_per_shard: u16,
    terminal_retention: Duration,
}

impl OperationRestoreRetentionPolicy {
    /// Construct a bounded policy that is sealed into the domain bootstrap.
    #[cfg(test)]
    fn new(
        shard_count: u16,
        entries_per_shard: u16,
        terminal_retention: Duration,
    ) -> Result<Self, OperationRestoreError> {
        let policy = Self {
            shard_count,
            entries_per_shard,
            terminal_retention,
        };
        policy.validate()?;
        Ok(policy)
    }

    fn production_default() -> Self {
        Self {
            shard_count: PRODUCTION_OPERATION_SHARD_COUNT,
            entries_per_shard: PRODUCTION_OPERATION_SHARD_CAPACITY,
            terminal_retention: PRODUCTION_TERMINAL_RETENTION,
        }
    }

    fn validate(self) -> Result<(), OperationRestoreError> {
        if self.shard_count == 0
            || self.shard_count > MAX_OPERATION_SHARDS
            || !self.shard_count.is_power_of_two()
            || self.entries_per_shard == 0
            || self.entries_per_shard > MAX_OPERATION_SHARD_CAPACITY
            || self.terminal_retention < MIN_TERMINAL_RETENTION
            || self.terminal_retention > MAX_TERMINAL_RETENTION
        {
            return Err(OperationRestoreError::InvalidInput);
        }
        Ok(())
    }

    fn retention_millis(self) -> Result<u64, OperationRestoreError> {
        u64::try_from(self.terminal_retention.as_millis())
            .map_err(|_| OperationRestoreError::InvalidInput)
    }

    fn layout_digest(self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(b"buzz:nip-fi-operation-restore-layout:v1");
        digest.update([OPERATION_SHARD_MAPPING_VERSION]);
        digest.update(self.shard_count.to_be_bytes());
        digest.update(self.entries_per_shard.to_be_bytes());
        digest.update(self.retention_millis().unwrap_or(u64::MAX).to_be_bytes());
        digest.update(operation_shard_record_format_hash());
        digest.finalize().into()
    }
}

impl fmt::Debug for OperationRestoreRetentionPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationRestoreRetentionPolicy")
            .field("shard_count", &self.shard_count)
            .field("entries_per_shard", &self.entries_per_shard)
            .field("terminal_retention", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct OperationIdentity {
    domain: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
}

impl OperationIdentity {
    fn validate(self) -> Result<Self, OperationRestoreError> {
        if self.domain.as_uuid().is_nil()
            || self.operation_id.is_nil()
            || self.request_fingerprint == [0; 32]
        {
            return Err(OperationRestoreError::InvalidInput);
        }
        Ok(self)
    }
}

impl fmt::Debug for OperationIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OperationIdentity([REDACTED])")
    }
}

struct OperationFenceGuard {
    capability: Option<AuthorizationOperationFence>,
    _permit: Option<OwnedSemaphorePermit>,
    #[cfg(any(test, feature = "test-utils"))]
    key: i64,
    #[cfg(any(test, feature = "test-utils"))]
    test_fences: Option<Arc<DashSet<i64>>>,
}

impl OperationFenceGuard {
    fn capability_mut(
        &mut self,
    ) -> Result<&mut AuthorizationOperationFence, OperationRestoreError> {
        self.capability
            .as_mut()
            .ok_or(OperationRestoreError::DatabaseUnavailable)
    }

    async fn unlock(mut self) -> Result<(), OperationRestoreError> {
        #[cfg(any(test, feature = "test-utils"))]
        if let Some(fences) = self.test_fences.take() {
            fences.remove(&self.key);
            return Ok(());
        }
        let Some(capability) = self.capability.take() else {
            return Err(OperationRestoreError::DatabaseUnavailable);
        };
        capability
            .release(OPERATION_FENCE_WAIT)
            .await
            .map_err(|_| OperationRestoreError::DatabaseUnavailable)
    }
}

impl Drop for OperationFenceGuard {
    fn drop(&mut self) {
        #[cfg(any(test, feature = "test-utils"))]
        if let Some(fences) = &self.test_fences {
            fences.remove(&self.key);
        }
        // A detached PgConnection is deliberately dropped rather than ever
        // returning a possibly session-locked connection to a pool.
        let _ = self.capability.take();
    }
}

impl fmt::Debug for OperationFenceGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OperationFenceGuard([REDACTED])")
    }
}

struct FencedRestoreIntent {
    restore: Option<OperationRestoreIntent>,
    fence: OperationFenceGuard,
    replay: Option<OperationRestoreCommit>,
}

impl FencedRestoreIntent {
    fn capability_mut(
        &mut self,
    ) -> Result<&mut AuthorizationOperationFence, OperationRestoreError> {
        self.fence.capability_mut()
    }
}

/// Fail-closed combined publication/database/restore result.
#[derive(Debug, Error)]
pub enum RestoredProtectedPublicationError {
    /// The canonical publication transaction denied or failed.
    #[error(transparent)]
    Publication(#[from] ProtectedPublicationError),
    /// The external operation witness failed.
    #[error(transparent)]
    Restore(#[from] OperationRestoreError),
}

/// Crate-internal consumed owner capability. Production callers receive only
/// an operation-specific sealed wrapper.
struct OperationRestoreIntent {
    identity: OperationIdentity,
    owner_token: Uuid,
    shard_id: u16,
}

impl fmt::Debug for OperationRestoreIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OperationRestoreIntent([REDACTED])")
    }
}

/// Internal result of reserving an exact external operation intent.
enum OperationRestoreBegin {
    /// The caller exclusively owns the bounded pending intent.
    Acquired(OperationRestoreIntent),
    /// The exact operation and manifest were already witnessed.
    ExactReplay(OperationRestoreCommit),
}

impl fmt::Debug for OperationRestoreBegin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Acquired(_) => formatter.write_str("OperationRestoreBegin::Acquired([REDACTED])"),
            Self::ExactReplay(_) => {
                formatter.write_str("OperationRestoreBegin::ExactReplay([REDACTED])")
            }
        }
    }
}

/// Internal durable witness result for one exact PostgreSQL operation manifest.
#[derive(Clone, Copy, PartialEq, Eq)]
struct OperationRestoreCommit {
    manifest_digest: [u8; 32],
    replay: bool,
    causally_superseded: bool,
}

/// Sealed reconciliation result for one protected-publication operation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ProtectedPublicationRestoreReconciliation {
    replay: bool,
    causally_superseded: bool,
}

impl ProtectedPublicationRestoreReconciliation {
    /// Whether the exact external witness was already terminal.
    pub const fn replay(&self) -> bool {
        self.replay
    }

    /// Whether a later, DB-proven operation owns the current component floor.
    pub const fn causally_superseded(&self) -> bool {
        self.causally_superseded
    }
}

impl fmt::Debug for ProtectedPublicationRestoreReconciliation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProtectedPublicationRestoreReconciliation")
            .field("replay", &self.replay)
            .field("causally_superseded", &self.causally_superseded)
            .finish()
    }
}

impl From<OperationRestoreCommit> for ProtectedPublicationRestoreReconciliation {
    fn from(commit: OperationRestoreCommit) -> Self {
        Self {
            replay: commit.replay,
            causally_superseded: commit.causally_superseded,
        }
    }
}

impl fmt::Debug for OperationRestoreCommit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationRestoreCommit")
            .field("manifest_digest", &"[REDACTED]")
            .field("replay", &self.replay)
            .field("causally_superseded", &self.causally_superseded)
            .finish()
    }
}

#[derive(Clone)]
struct OperationManifest {
    identity: OperationIdentity,
    manifest_digest: [u8; 32],
    components: Vec<ManifestComponent>,
}

#[derive(Clone)]
struct ManifestComponent {
    kind: AuthorizationVersionComponentKind,
    key: [u8; 32],
    before: u64,
    after: u64,
    digest: [u8; 32],
}

impl From<AuthorizationOperationVersionDeltaManifest> for OperationManifest {
    fn from(manifest: AuthorizationOperationVersionDeltaManifest) -> Self {
        Self {
            identity: OperationIdentity {
                domain: manifest.community_id(),
                operation_id: manifest.operation_id(),
                request_fingerprint: manifest.request_fingerprint(),
            },
            manifest_digest: manifest.manifest_digest(),
            components: manifest
                .components()
                .iter()
                .map(|component| ManifestComponent {
                    kind: component.component_kind(),
                    key: component.component_key(),
                    before: component.before_version(),
                    after: component.after_version(),
                    digest: component.component_digest(),
                })
                .collect(),
        }
    }
}

#[async_trait]
trait ManifestSource: Send + Sync {
    async fn authoritative_now_ms(&self) -> Result<u64, ManifestReadError>;

    async fn manifest(
        &self,
        identity: OperationIdentity,
    ) -> Result<OperationManifest, ManifestReadError>;

    async fn floors(
        &self,
        domain: CommunityId,
    ) -> Result<Vec<AuthorizationVersionComponentFloor>, ManifestReadError>;

    async fn predecessors(
        &self,
        manifest: &OperationManifest,
    ) -> Result<Vec<OperationManifest>, ManifestReadError>;

    async fn prove_supersession(
        &self,
        pending: &OperationManifest,
        component: &ManifestComponent,
        floor: &FloorRecord,
    ) -> Result<(), ManifestReadError>;
}

struct PostgresManifestSource(Db);

#[async_trait]
impl ManifestSource for PostgresManifestSource {
    async fn authoritative_now_ms(&self) -> Result<u64, ManifestReadError> {
        self.0
            .authorization_restore_clock_millis()
            .await
            .map_err(|_| ManifestReadError::Unavailable)
    }

    async fn manifest(
        &self,
        identity: OperationIdentity,
    ) -> Result<OperationManifest, ManifestReadError> {
        self.0
            .authorization_operation_version_delta(
                identity.domain,
                identity.operation_id,
                identity.request_fingerprint,
            )
            .await
            .map(OperationManifest::from)
            .map_err(|error| match error {
                DbError::NotFound(_) => ManifestReadError::Missing,
                DbError::InvalidData(_) => ManifestReadError::Invalid,
                _ => ManifestReadError::Unavailable,
            })
    }

    async fn floors(
        &self,
        domain: CommunityId,
    ) -> Result<Vec<AuthorizationVersionComponentFloor>, ManifestReadError> {
        self.0
            .authorization_version_component_floors(domain)
            .await
            .map_err(|_| ManifestReadError::Unavailable)
    }

    async fn predecessors(
        &self,
        manifest: &OperationManifest,
    ) -> Result<Vec<OperationManifest>, ManifestReadError> {
        self.0
            .authorization_operation_version_predecessors(
                manifest.identity.domain,
                &manifest
                    .components
                    .iter()
                    .map(|component| (component.kind, component.key, component.before))
                    .collect::<Vec<_>>(),
            )
            .await
            .map(|manifests| manifests.into_iter().map(OperationManifest::from).collect())
            .map_err(|error| match error {
                DbError::InvalidData(_) | DbError::NotFound(_) => ManifestReadError::Invalid,
                _ => ManifestReadError::Unavailable,
            })
    }

    async fn prove_supersession(
        &self,
        pending: &OperationManifest,
        component: &ManifestComponent,
        floor: &FloorRecord,
    ) -> Result<(), ManifestReadError> {
        let provenance = floor.provenance().map_err(|_| ManifestReadError::Invalid)?;
        self.0
            .authorization_version_proves_component_supersession(
                pending.identity.domain,
                component.kind,
                component.key,
                component.after,
                floor.version,
                provenance.operation_id,
                provenance.request_fingerprint,
                provenance.component_digest,
                provenance.manifest_digest,
            )
            .await
            .map_err(|error| match error {
                DbError::InvalidData(_) | DbError::NotFound(_) => ManifestReadError::Invalid,
                _ => ManifestReadError::Unavailable,
            })
    }
}

enum ManifestReadError {
    Missing,
    Invalid,
    Unavailable,
}

struct StoredObject {
    etag: String,
    body: Vec<u8>,
}

enum StoreCas {
    Won,
    Lost,
}

#[async_trait]
trait RestoreStore: Send + Sync {
    async fn load(&self, key: &str) -> Result<Option<StoredObject>, ()>;
    async fn list(&self, prefix: &str) -> Result<Vec<String>, ()>;
    async fn compare_and_swap(
        &self,
        key: &str,
        expected_etag: Option<&str>,
        body: &[u8],
    ) -> Result<StoreCas, ()>;
}

#[cfg(any(test, feature = "test-utils"))]
#[derive(Default)]
struct TestRestoreStore(std::sync::Mutex<std::collections::HashMap<String, (u64, Vec<u8>)>>);

#[cfg(any(test, feature = "test-utils"))]
#[async_trait]
impl RestoreStore for TestRestoreStore {
    async fn load(&self, key: &str) -> Result<Option<StoredObject>, ()> {
        Ok(self
            .0
            .lock()
            .map_err(|_| ())?
            .get(key)
            .map(|(version, body)| StoredObject {
                etag: version.to_string(),
                body: body.clone(),
            }))
    }

    async fn list(&self, prefix: &str) -> Result<Vec<String>, ()> {
        Ok(self
            .0
            .lock()
            .map_err(|_| ())?
            .keys()
            .filter(|key| key.starts_with(prefix))
            .cloned()
            .collect())
    }

    async fn compare_and_swap(
        &self,
        key: &str,
        expected_etag: Option<&str>,
        body: &[u8],
    ) -> Result<StoreCas, ()> {
        let mut records = self.0.lock().map_err(|_| ())?;
        let current = records.get(key).map(|(version, _)| version.to_string());
        if current.as_deref() != expected_etag {
            return Ok(StoreCas::Lost);
        }
        let next = records.get(key).map_or(1, |(version, _)| version + 1);
        records.insert(key.to_owned(), (next, body.to_vec()));
        Ok(StoreCas::Won)
    }
}

struct S3RestoreStore {
    bucket: Arc<Bucket>,
}

impl S3RestoreStore {
    fn new(config: &buzz_media::MediaConfig) -> Result<Self, ()> {
        let region = Region::Custom {
            region: config.s3_region.clone(),
            endpoint: config.s3_endpoint.clone(),
        };
        let credentials = match (
            config.s3_access_key.is_empty(),
            config.s3_secret_key.is_empty(),
        ) {
            (false, false) => Credentials::new(
                Some(&config.s3_access_key),
                Some(&config.s3_secret_key),
                None,
                None,
                None,
            ),
            (true, true) => Credentials::default(),
            _ => return Err(()),
        }
        .map_err(|_| ())?;
        let bucket = Bucket::new(&config.s3_bucket, region, credentials).map_err(|_| ())?;
        let bucket = match config.s3_addressing_style {
            buzz_media::config::S3AddressingStyle::Path => bucket.with_path_style(),
            buzz_media::config::S3AddressingStyle::Virtual => bucket,
        };
        Ok(Self {
            bucket: Arc::from(bucket),
        })
    }
}

#[async_trait]
impl RestoreStore for S3RestoreStore {
    async fn load(&self, key: &str) -> Result<Option<StoredObject>, ()> {
        let (head, status) = match self.bucket.head_object(key).await {
            Ok(result) => result,
            Err(S3Error::HttpFailWithBody(404, _)) => return Ok(None),
            Err(_) => return Err(()),
        };
        if status == 404 {
            return Ok(None);
        }
        if !(200..300).contains(&status)
            || head
                .content_length
                .is_some_and(|length| u64::try_from(length).unwrap_or(u64::MAX) > MAX_RECORD_BYTES)
        {
            return Err(());
        }
        match self.bucket.get_object(key).await {
            Ok(response) => {
                let headers = response.headers();
                let etag = headers
                    .get("etag")
                    .or_else(|| headers.get("ETag"))
                    .cloned()
                    .ok_or(())?;
                let body = response.to_vec();
                if u64::try_from(body.len()).unwrap_or(u64::MAX) > MAX_RECORD_BYTES {
                    return Err(());
                }
                Ok(Some(StoredObject { etag, body }))
            }
            Err(S3Error::HttpFailWithBody(404, _)) => Ok(None),
            Err(_) => Err(()),
        }
    }

    async fn list(&self, prefix: &str) -> Result<Vec<String>, ()> {
        let pages = self
            .bucket
            .list(prefix.to_owned(), None)
            .await
            .map_err(|_| ())?;
        let mut keys = Vec::new();
        for page in pages {
            for object in page.contents {
                if keys.len() >= MAX_DOMAIN_FLOORS {
                    return Err(());
                }
                keys.push(object.key);
            }
        }
        Ok(keys)
    }

    async fn compare_and_swap(
        &self,
        key: &str,
        expected_etag: Option<&str>,
        body: &[u8],
    ) -> Result<StoreCas, ()> {
        if u64::try_from(body.len()).unwrap_or(u64::MAX) > MAX_RECORD_BYTES {
            return Err(());
        }
        let mut headers = http::HeaderMap::new();
        match expected_etag {
            Some(etag) => {
                headers.insert(http::header::IF_MATCH, etag.parse().map_err(|_| ())?);
            }
            None => {
                headers.insert(http::header::IF_NONE_MATCH, "*".parse().map_err(|_| ())?);
            }
        }
        match self
            .bucket
            .put_object_with_content_type_and_headers(key, body, "application/json", Some(headers))
            .await
        {
            Ok(response) if (200..300).contains(&response.status_code()) => Ok(StoreCas::Won),
            Err(S3Error::HttpFailWithBody(412, _)) => Ok(StoreCas::Lost),
            _ => Err(()),
        }
    }
}

struct RestoreInner {
    writer_db: Db,
    source: Arc<dyn ManifestSource>,
    store: Arc<dyn RestoreStore>,
    bootstrap_ids: Arc<HashMap<CommunityId, Uuid>>,
    ready_domains: Arc<DashSet<CommunityId>>,
    lease: Duration,
    retention: OperationRestoreRetentionPolicy,
    fence_pool: PgPool,
    fence_slots: Arc<Semaphore>,
    #[cfg(any(test, feature = "test-utils"))]
    test_fences: Option<Arc<DashSet<i64>>>,
    healthy: AtomicBool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComponentReconciliation {
    Exact,
    Applied,
    CausallySuperseded,
}

/// Cloneable operation-bound restore witness runtime.
///
/// Raw owner intents are intentionally not part of the public surface:
///
/// ```compile_fail
/// use buzz_db::authorization_restore::{OperationRestoreIntent, OperationRestoreCommit};
/// ```
///
/// The lock-owning session capability is likewise crate-private:
///
/// ```compile_fail
/// use buzz_db::authorization_version::AuthorizationOperationFence;
/// ```
#[derive(Clone)]
pub struct OperationRestoreRuntime(Arc<RestoreInner>);

impl OperationRestoreRuntime {
    /// Construct with the canonical writer, a separately created witness-read
    /// handle, and the external CAS store.
    pub fn new(
        writer_db: Db,
        witness_read_db: Db,
        fence_pool: PgPool,
        store_configuration: buzz_media::MediaConfig,
        domains: impl IntoIterator<Item = (CommunityId, Uuid)>,
        lease: Duration,
    ) -> Result<Self, OperationRestoreError> {
        Self::new_with_retention_policy(
            writer_db,
            witness_read_db,
            fence_pool,
            store_configuration,
            domains,
            lease,
            OperationRestoreRetentionPolicy::production_default(),
        )
    }

    fn new_with_retention_policy(
        writer_db: Db,
        witness_read_db: Db,
        fence_pool: PgPool,
        store_configuration: buzz_media::MediaConfig,
        domains: impl IntoIterator<Item = (CommunityId, Uuid)>,
        lease: Duration,
        retention: OperationRestoreRetentionPolicy,
    ) -> Result<Self, OperationRestoreError> {
        if lease.is_zero() || lease > MAX_LEASE {
            return Err(OperationRestoreError::InvalidInput);
        }
        retention.validate()?;
        let mut bootstrap_ids = HashMap::new();
        for (domain, bootstrap) in domains {
            if bootstrap_ids.insert(domain, bootstrap).is_some() {
                return Err(OperationRestoreError::InvalidBootstrap);
            }
        }
        if bootstrap_ids.is_empty()
            || bootstrap_ids
                .iter()
                .any(|(domain, bootstrap)| domain.as_uuid().is_nil() || bootstrap.is_nil())
        {
            return Err(OperationRestoreError::InvalidBootstrap);
        }
        let store = S3RestoreStore::new(&store_configuration)
            .map_err(|_| OperationRestoreError::InvalidInput)?;
        Ok(Self(Arc::new(RestoreInner {
            writer_db,
            source: Arc::new(PostgresManifestSource(witness_read_db)),
            store: Arc::new(store),
            bootstrap_ids: Arc::new(bootstrap_ids),
            ready_domains: Arc::new(DashSet::new()),
            lease,
            retention,
            fence_pool,
            fence_slots: Arc::new(Semaphore::new(
                PRODUCTION_OPERATION_FENCE_CONNECTIONS as usize,
            )),
            #[cfg(any(test, feature = "test-utils"))]
            test_fences: None,
            healthy: AtomicBool::new(true),
        })))
    }

    /// Construct with the default bounded owner lease.
    pub fn with_default_lease(
        writer_db: Db,
        witness_read_db: Db,
        fence_pool: PgPool,
        store_configuration: buzz_media::MediaConfig,
        domains: impl IntoIterator<Item = (CommunityId, Uuid)>,
    ) -> Result<Self, OperationRestoreError> {
        Self::new(
            writer_db,
            witness_read_db,
            fence_pool,
            store_configuration,
            domains,
            DEFAULT_LEASE,
        )
    }

    #[cfg(any(test, feature = "test-utils"))]
    #[doc(hidden)]
    pub fn for_tests(
        witness_read_db: Db,
        domains: impl IntoIterator<Item = (CommunityId, Uuid)>,
    ) -> Self {
        let bootstrap_ids: HashMap<_, _> = domains.into_iter().collect();
        let retention = OperationRestoreRetentionPolicy::production_default();
        let store = TestRestoreStore::default();
        {
            let mut records = store.0.lock().expect("test restore store");
            for (domain, bootstrap_id) in &bootstrap_ids {
                for shard_id in 0..retention.shard_count {
                    records.insert(
                        operation_shard_key(*domain, shard_id),
                        (
                            1,
                            serde_json::to_vec(&OperationShardRecord::empty(
                                *domain,
                                *bootstrap_id,
                                shard_id,
                                retention,
                            ))
                            .expect("encode test restore shard"),
                        ),
                    );
                }
            }
        }
        Self(Arc::new(RestoreInner {
            writer_db: witness_read_db.clone(),
            source: Arc::new(PostgresManifestSource(witness_read_db)),
            store: Arc::new(store),
            bootstrap_ids: Arc::new(bootstrap_ids),
            ready_domains: Arc::new(DashSet::new()),
            lease: DEFAULT_LEASE,
            retention,
            fence_pool: sqlx::postgres::PgPoolOptions::new()
                .max_connections(1)
                .connect_lazy("postgresql://127.0.0.1:1/unreachable")
                .expect("parse test fence database URL"),
            fence_slots: Arc::new(Semaphore::new(1)),
            test_fences: Some(Arc::new(DashSet::new())),
            healthy: AtomicBool::new(true),
        }))
    }

    #[cfg(any(test, feature = "test-utils"))]
    #[doc(hidden)]
    pub fn force_unhealthy_for_test(&self) {
        self.latch_unhealthy();
    }

    #[cfg(any(test, feature = "test-utils"))]
    #[doc(hidden)]
    pub fn mark_domain_ready_for_test(&self, domain: CommunityId) {
        self.0.ready_domains.insert(domain);
    }

    /// Explicitly provision a first immutable bootstrap and exact component baselines.
    /// Normal production startup calls [`Self::verify_domain`] and never calls this method.
    pub async fn provision_domain(&self, domain: CommunityId) -> Result<(), OperationRestoreError> {
        self.require_healthy()?;
        let bootstrap_id = self.bootstrap_id(domain)?;
        let floors = self.read_database_floors(domain).await?;
        for floor in floors {
            self.provision_floor(domain, floor).await?;
        }
        self.validate_worst_case_shard_size(domain, bootstrap_id)?;
        for shard_id in 0..self.0.retention.shard_count {
            self.provision_operation_shard(domain, bootstrap_id, shard_id)
                .await?;
        }
        let marker_key = bootstrap_key(domain);
        let marker = BootstrapRecord::new(domain, bootstrap_id, self.0.retention);
        match self.store_record(&marker_key, None, &marker).await? {
            StoreCas::Won => {}
            StoreCas::Lost => {
                let Some((_, existing)) = self.load_bootstrap(&marker_key).await? else {
                    self.latch_unhealthy();
                    return Err(OperationRestoreError::StoreUnavailable);
                };
                self.validate_bootstrap(&existing, domain, bootstrap_id)?;
            }
        }
        self.0.ready_domains.insert(domain);
        Ok(())
    }

    /// Verify a previously provisioned bootstrap. Missing state is never synthesized.
    pub async fn verify_domain(&self, domain: CommunityId) -> Result<(), OperationRestoreError> {
        self.require_healthy()?;
        let bootstrap_id = self.bootstrap_id(domain)?;
        let marker_key = bootstrap_key(domain);
        let Some((_, marker)) = self.load_bootstrap(&marker_key).await? else {
            self.latch_unhealthy();
            return Err(OperationRestoreError::MissingBootstrap);
        };
        self.validate_bootstrap(&marker, domain, bootstrap_id)?;
        self.validate_worst_case_shard_size(domain, bootstrap_id)?;
        for shard_id in 0..self.0.retention.shard_count {
            let key = operation_shard_key(domain, shard_id);
            let Some((_, shard)) = self.load_operation_shard(&key).await? else {
                self.latch_unhealthy();
                return Err(OperationRestoreError::MissingOperationShard);
            };
            self.validate_operation_shard(&shard, domain, bootstrap_id, shard_id)?;
        }
        // A process may die after PostgreSQL commits its exact manifest but
        // before the external floors and operation record are advanced.  The
        // operation namespace is therefore reconciled before comparing the
        // external component floors with the current database snapshot.
        self.reconcile_startup_operations(domain).await?;
        let mut database_floors = self
            .read_database_floors(domain)
            .await?
            .into_iter()
            .map(|floor| ((floor.component_kind, floor.component_key), floor.version))
            .collect::<HashMap<_, _>>();
        let prefix = floor_prefix(domain);
        let floor_keys = self.0.store.list(&prefix).await.map_err(|_| {
            self.latch_unhealthy();
            OperationRestoreError::StoreUnavailable
        })?;
        if floor_keys.len() > MAX_DOMAIN_FLOORS {
            self.latch_unhealthy();
            return Err(OperationRestoreError::StoreUnavailable);
        }
        for stored_key in floor_keys {
            let Some((_, existing)) = self.load_floor(&stored_key).await? else {
                self.latch_unhealthy();
                return Err(OperationRestoreError::MissingFloor);
            };
            let kind = component_kind_from_i16(existing.component_kind)?;
            let component_key = decode32(&existing.component_key)?;
            if stored_key != floor_key(domain, kind, component_key) {
                self.latch_unhealthy();
                return Err(OperationRestoreError::WrongComponentKey);
            }
            if let Err(error) =
                existing.validate_coordinate(domain, bootstrap_id, kind, component_key)
            {
                self.latch_unhealthy();
                return Err(error);
            }
            let database_version = database_floors.remove(&(kind, component_key)).unwrap_or(0);
            if existing.version != database_version {
                self.latch_unhealthy();
                return Err(if existing.version > database_version {
                    OperationRestoreError::BackwardFloor
                } else {
                    OperationRestoreError::StaleFloor
                });
            }
        }
        if !database_floors.is_empty() {
            self.latch_unhealthy();
            return Err(OperationRestoreError::MissingFloor);
        }
        self.0.ready_domains.insert(domain);
        Ok(())
    }

    async fn reconcile_startup_operations(
        &self,
        domain: CommunityId,
    ) -> Result<(), OperationRestoreError> {
        let bootstrap_id = self.bootstrap_id(domain)?;
        loop {
            let now = self.authoritative_now_ms().await?;
            let mut saw_pending = false;
            let mut saw_live_pending = false;
            let mut progressed = false;
            let mut retryable = None;

            // Scan fixed shard IDs directly.  At most one detached primary
            // session is retained at a time, so startup can reconcile more
            // operations than the bounded fence-pool size without
            // self-saturating that pool.
            for shard_id in 0..self.0.retention.shard_count {
                let key = operation_shard_key(domain, shard_id);
                let shard = self
                    .compact_collectible_terminals(domain, bootstrap_id, shard_id, &key, now)
                    .await?;
                for (operation, record) in shard.entries {
                    if record.state != IntentState::Pending {
                        continue;
                    }
                    saw_pending = true;
                    let identity =
                        self.identity_from_shard_entry(domain, shard_id, &operation, &record)?;
                    let fence = match self.acquire_operation_fence(identity).await {
                        Ok(fence) => fence,
                        Err(error) if error.is_healthy_retryable() => {
                            retryable.get_or_insert(error);
                            continue;
                        }
                        Err(error) => return Err(error),
                    };
                    match self.0.source.manifest(identity).await {
                        Ok(manifest) if manifest.identity == identity => {
                            if self.manifest_ready_for_startup(&manifest).await? {
                                let result = self
                                    .reconcile_pending_manifest(shard_id, &record, manifest)
                                    .await;
                                match result {
                                    Ok(_) => fence
                                        .unlock()
                                        .await
                                        .inspect_err(|_| self.latch_unhealthy())?,
                                    Err(error) => {
                                        drop(fence);
                                        return Err(error);
                                    }
                                }
                                progressed = true;
                            } else {
                                drop(fence);
                            }
                        }
                        Ok(_) | Err(ManifestReadError::Invalid) => {
                            drop(fence);
                            self.latch_unhealthy();
                            return Err(OperationRestoreError::InvalidAttribution);
                        }
                        Err(ManifestReadError::Missing) if now >= record.expires_at_ms => {
                            let intent = OperationRestoreIntent {
                                identity,
                                owner_token: record.owner_token,
                                shard_id,
                            };
                            let result = self.abort_owned_intent(&intent).await;
                            match result {
                                Ok(()) => fence
                                    .unlock()
                                    .await
                                    .inspect_err(|_| self.latch_unhealthy())?,
                                Err(error) => {
                                    drop(fence);
                                    return Err(error);
                                }
                            }
                            progressed = true;
                        }
                        Err(ManifestReadError::Missing) => {
                            saw_live_pending = true;
                            drop(fence);
                        }
                        Err(ManifestReadError::Unavailable) => {
                            drop(fence);
                            self.latch_unhealthy();
                            return Err(OperationRestoreError::DatabaseUnavailable);
                        }
                    }
                }
            }

            if !saw_pending {
                return Ok(());
            }
            if progressed {
                continue;
            }
            if let Some(error) = retryable {
                return Err(error);
            }
            if saw_live_pending {
                return Err(OperationRestoreError::OperationInProgress);
            }
            self.latch_unhealthy();
            return Err(OperationRestoreError::StaleFloor);
        }
    }

    async fn compact_collectible_terminals(
        &self,
        domain: CommunityId,
        bootstrap_id: Uuid,
        shard_id: u16,
        key: &str,
        now: u64,
    ) -> Result<OperationShardRecord, OperationRestoreError> {
        for _ in 0..MAX_CAS_ATTEMPTS {
            let Some((etag, shard)) = self.load_operation_shard(key).await? else {
                self.latch_unhealthy();
                return Err(OperationRestoreError::MissingOperationShard);
            };
            self.validate_operation_shard(&shard, domain, bootstrap_id, shard_id)?;
            let mut next = shard.clone();
            next.entries.retain(|_, record| !record.is_collectible(now));
            if next.entries.len() == shard.entries.len() {
                return Ok(shard);
            }
            if matches!(
                self.store_record(key, Some(&etag), &next).await?,
                StoreCas::Won
            ) {
                return Ok(next);
            }
        }
        Err(OperationRestoreError::OperationContention)
    }

    async fn manifest_ready_for_startup(
        &self,
        manifest: &OperationManifest,
    ) -> Result<bool, OperationRestoreError> {
        let bootstrap_id = self.bootstrap_id(manifest.identity.domain)?;
        for component in &manifest.components {
            if component.key == [0; 32]
                || component.after <= component.before
                || component.after > i64::MAX as u64
            {
                self.latch_unhealthy();
                return Err(OperationRestoreError::InvalidAttribution);
            }
            let key = floor_key(manifest.identity.domain, component.kind, component.key);
            let Some((_, floor)) = self.load_floor(&key).await? else {
                if component.before == 0 {
                    continue;
                }
                return Ok(false);
            };
            if let Err(error) = floor.validate_coordinate(
                manifest.identity.domain,
                bootstrap_id,
                component.kind,
                component.key,
            ) {
                self.latch_unhealthy();
                return Err(error);
            }
            if floor.exact_advance(manifest, component)? || floor.version == component.before {
                continue;
            }
            if floor.version < component.before {
                return Ok(false);
            }
            if floor.version > component.after {
                self.prove_supersession(manifest, component, &floor).await?;
                continue;
            }
            self.latch_unhealthy();
            return Err(OperationRestoreError::InvalidAttribution);
        }
        Ok(true)
    }

    async fn acquire_operation_fence(
        &self,
        identity: OperationIdentity,
    ) -> Result<OperationFenceGuard, OperationRestoreError> {
        self.acquire_operation_fence_with_timeout(identity, OPERATION_FENCE_WAIT)
            .await
    }

    async fn acquire_operation_fence_with_timeout(
        &self,
        identity: OperationIdentity,
        timeout: Duration,
    ) -> Result<OperationFenceGuard, OperationRestoreError> {
        if timeout.is_zero() {
            return Err(OperationRestoreError::OperationFenceTimeout);
        }
        #[cfg(any(test, feature = "test-utils"))]
        if let Some(fences) = &self.0.test_fences {
            let key = operation_fence_test_key(identity);
            if !fences.insert(key) {
                // Model a bounded PostgreSQL lock probe so tests prove the
                // shard-wide deadline, not merely an immediate in-memory
                // collision fast path.
                tokio::time::sleep(Duration::from_millis(20).min(timeout)).await;
                return Err(OperationRestoreError::OperationInProgress);
            }
            return Ok(OperationFenceGuard {
                key,
                capability: None,
                _permit: None,
                test_fences: Some(fences.clone()),
            });
        }

        let permit = self
            .0
            .fence_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| OperationRestoreError::OperationFenceSaturated)?;
        let capability = match AuthorizationOperationFence::acquire_for_operation_restore(
            &self.0.fence_pool,
            identity.domain,
            identity.operation_id,
            identity.request_fingerprint,
            timeout,
        )
        .await
        {
            Ok(Some(capability)) => capability,
            Ok(None) | Err(AuthorizationOperationFenceAcquireError::LockTimedOut) => {
                return Err(OperationRestoreError::OperationFenceTimeout)
            }
            Err(AuthorizationOperationFenceAcquireError::PoolSaturated) => {
                return Err(OperationRestoreError::OperationFenceSaturated)
            }
            Err(AuthorizationOperationFenceAcquireError::Database(_)) => {
                self.latch_unhealthy();
                return Err(OperationRestoreError::DatabaseUnavailable);
            }
        };
        Ok(OperationFenceGuard {
            capability: Some(capability),
            _permit: Some(permit),
            #[cfg(any(test, feature = "test-utils"))]
            key: operation_fence_test_key(identity),
            #[cfg(any(test, feature = "test-utils"))]
            test_fences: None,
        })
    }

    /// Test/internal reservation primitive. Production callers use a sealed
    /// operation-specific wrapper that acquires the PostgreSQL session fence.
    #[cfg(test)]
    async fn begin(
        &self,
        domain: CommunityId,
        operation_id: Uuid,
        request_fingerprint: [u8; 32],
    ) -> Result<OperationRestoreBegin, OperationRestoreError> {
        self.require_healthy()?;
        let identity = OperationIdentity {
            domain,
            operation_id,
            request_fingerprint,
        }
        .validate()?;
        self.require_domain_ready(domain)?;
        let bootstrap_id = self.bootstrap_id(domain)?;
        let shard_id = operation_shard_id(identity, bootstrap_id, self.0.retention.shard_count);
        self.maintain_begin_shard(domain, shard_id).await?;
        self.reserve_identity(identity, shard_id).await
    }

    async fn begin_fenced(
        &self,
        identity: OperationIdentity,
    ) -> Result<FencedRestoreIntent, OperationRestoreError> {
        self.require_healthy()?;
        let identity = identity.validate()?;
        self.require_domain_ready(identity.domain)?;
        let bootstrap_id = self.bootstrap_id(identity.domain)?;
        let shard_id = operation_shard_id(identity, bootstrap_id, self.0.retention.shard_count);
        self.maintain_begin_shard(identity.domain, shard_id).await?;
        let fence = self.acquire_operation_fence(identity).await?;
        match self.reserve_identity(identity, shard_id).await {
            Ok(OperationRestoreBegin::Acquired(intent)) => Ok(FencedRestoreIntent {
                restore: Some(intent),
                fence,
                replay: None,
            }),
            Ok(OperationRestoreBegin::ExactReplay(replay)) => Ok(FencedRestoreIntent {
                restore: None,
                fence,
                replay: Some(replay),
            }),
            Err(error) => {
                drop(fence);
                Err(error)
            }
        }
    }

    async fn reserve_identity(
        &self,
        identity: OperationIdentity,
        shard_id: u16,
    ) -> Result<OperationRestoreBegin, OperationRestoreError> {
        let domain = identity.domain;
        let operation_id = identity.operation_id;
        let bootstrap_id = self.bootstrap_id(domain)?;
        let key = operation_shard_key(domain, shard_id);
        for _ in 0..MAX_CAS_ATTEMPTS {
            let Some((etag, mut shard)) = self.load_operation_shard(&key).await? else {
                self.latch_unhealthy();
                return Err(OperationRestoreError::MissingOperationShard);
            };
            self.validate_operation_shard(&shard, domain, bootstrap_id, shard_id)?;
            let operation_key = operation_id.to_string();
            if let Some(record) = shard.entries.get(&operation_key) {
                self.validate_intent_record(record, identity)?;
                return match record.state {
                    IntentState::Committed => Ok(OperationRestoreBegin::ExactReplay(
                        self.validate_committed(identity, record).await?,
                    )),
                    IntentState::Aborted => Err(OperationRestoreError::OperationAborted),
                    IntentState::Pending => Err(OperationRestoreError::OperationInProgress),
                };
            }
            if shard.entries.len() >= usize::from(self.0.retention.entries_per_shard) {
                return Err(OperationRestoreError::OperationCapacityExceeded);
            }
            let now = self.authoritative_now_ms().await?;
            let owner_token = Uuid::new_v4();
            shard.entries.insert(
                operation_key,
                IntentRecord::pending(
                    identity,
                    owner_token,
                    bootstrap_id,
                    now,
                    lease_expiry(now, self.0.lease)?,
                ),
            );
            if matches!(
                self.store_record(&key, Some(&etag), &shard).await?,
                StoreCas::Won
            ) {
                return Ok(OperationRestoreBegin::Acquired(OperationRestoreIntent {
                    identity,
                    owner_token,
                    shard_id,
                }));
            }
        }
        Err(OperationRestoreError::OperationContention)
    }

    async fn begin_protected_publication(
        &self,
        identity: ProtectedPublicationRestoreIdentity,
    ) -> Result<FencedRestoreIntent, OperationRestoreError> {
        self.begin_fenced(OperationIdentity {
            domain: identity.authorization_domain(),
            operation_id: identity.operation_id(),
            request_fingerprint: identity.request_fingerprint(),
        })
        .await
    }

    /// Execute one sealed staged publication on its lock-owning primary
    /// session. The session is never returned to a pool.
    pub async fn commit_protected_publication(
        &self,
        request: &ProtectedPublicationRequest,
    ) -> Result<ProtectedPublicationCommit, RestoredProtectedPublicationError> {
        let mut fenced = self
            .begin_protected_publication(request.restore_identity())
            .await?;
        let result = self
            .0
            .writer_db
            .commit_staged_protected_publication_fenced(fenced.capability_mut()?, request)
            .await;
        match result {
            Ok(commit) => {
                self.commit_fenced(fenced).await?;
                Ok(commit)
            }
            Err(
                error @ (ProtectedPublicationError::Conflict
                | ProtectedPublicationError::StaleAuthorization
                | ProtectedPublicationError::AuditUnavailable),
            ) => {
                self.abort_fenced(fenced).await?;
                Err(error.into())
            }
            Err(error @ ProtectedPublicationError::Database(_)) => {
                self.abandon_ambiguous(fenced);
                Err(error.into())
            }
        }
    }

    async fn commit_fenced(
        &self,
        mut fenced: FencedRestoreIntent,
    ) -> Result<(), OperationRestoreError> {
        if let Some(intent) = fenced.restore.take() {
            self.commit(intent).await?
        } else {
            fenced
                .replay
                .take()
                .ok_or(OperationRestoreError::InvalidAttribution)?
        };
        if let Err(error) = fenced.fence.unlock().await {
            self.latch_unhealthy();
            return Err(error);
        }
        Ok(())
    }

    async fn abort_fenced(
        &self,
        mut fenced: FencedRestoreIntent,
    ) -> Result<(), OperationRestoreError> {
        if let Some(intent) = fenced.restore.take() {
            self.abort(intent).await?;
        }
        if let Err(error) = fenced.fence.unlock().await {
            self.latch_unhealthy();
            return Err(error);
        }
        Ok(())
    }

    fn abandon_ambiguous(&self, fenced: FencedRestoreIntent) {
        self.latch_unhealthy();
        drop(fenced);
    }

    /// Resolve the exact manifest through the independent read handle and advance its floors.
    async fn commit(
        &self,
        intent: OperationRestoreIntent,
    ) -> Result<OperationRestoreCommit, OperationRestoreError> {
        self.require_healthy()?;
        let record = self.load_owned_intent(&intent).await?;
        if record.state == IntentState::Committed {
            return self.validate_committed(intent.identity, &record).await;
        }
        match self.0.source.manifest(intent.identity).await {
            Ok(manifest) if manifest.identity == intent.identity => {
                self.reconcile_pending_manifest(intent.shard_id, &record, manifest)
                    .await
            }
            Ok(_) | Err(ManifestReadError::Invalid) => {
                self.latch_unhealthy();
                Err(OperationRestoreError::InvalidAttribution)
            }
            Err(ManifestReadError::Missing) => {
                self.abort_owned_intent(&intent).await?;
                self.latch_unhealthy();
                Err(OperationRestoreError::MissingAttribution)
            }
            Err(ManifestReadError::Unavailable) => {
                self.latch_unhealthy();
                Err(OperationRestoreError::DatabaseUnavailable)
            }
        }
    }

    /// Consume an uncommitted owner lease on a normal caller cancellation/error path.
    async fn abort(&self, intent: OperationRestoreIntent) -> Result<(), OperationRestoreError> {
        self.require_healthy()?;
        self.abort_owned_intent(&intent).await
    }

    /// Reconcile a lost response/crash from the exact receipt-bound PostgreSQL manifest only.
    async fn reconcile(
        &self,
        domain: CommunityId,
        operation_id: Uuid,
        request_fingerprint: [u8; 32],
    ) -> Result<OperationRestoreCommit, OperationRestoreError> {
        self.require_healthy()?;
        let identity = OperationIdentity {
            domain,
            operation_id,
            request_fingerprint,
        }
        .validate()?;
        self.require_domain_ready(domain)?;
        let bootstrap_id = self.bootstrap_id(domain)?;
        let shard_id = operation_shard_id(identity, bootstrap_id, self.0.retention.shard_count);
        let key = operation_shard_key(domain, shard_id);
        let Some((_, shard)) = self.load_operation_shard(&key).await? else {
            self.latch_unhealthy();
            return Err(OperationRestoreError::MissingOperationShard);
        };
        self.validate_operation_shard(&shard, domain, bootstrap_id, shard_id)?;
        let Some(record) = shard.entries.get(&operation_id.to_string()) else {
            return Err(OperationRestoreError::MissingIntent);
        };
        self.validate_intent_record(record, identity)?;
        match record.state {
            IntentState::Committed => self.validate_committed(identity, record).await,
            IntentState::Aborted => Err(OperationRestoreError::OperationAborted),
            IntentState::Pending => match self.0.source.manifest(identity).await {
                Ok(manifest) if manifest.identity == identity => {
                    self.reconcile_pending_manifest(shard_id, record, manifest)
                        .await
                }
                Ok(_) | Err(ManifestReadError::Invalid) => {
                    self.latch_unhealthy();
                    Err(OperationRestoreError::InvalidAttribution)
                }
                Err(ManifestReadError::Missing) => Err(OperationRestoreError::MissingAttribution),
                Err(ManifestReadError::Unavailable) => {
                    self.latch_unhealthy();
                    Err(OperationRestoreError::DatabaseUnavailable)
                }
            },
        }
    }

    /// Reconcile the exact sealed protected publication request after an
    /// ambiguous database outcome.
    pub async fn reconcile_protected_publication(
        &self,
        identity: ProtectedPublicationRestoreIdentity,
    ) -> Result<ProtectedPublicationRestoreReconciliation, OperationRestoreError> {
        let operation_identity = OperationIdentity {
            domain: identity.authorization_domain(),
            operation_id: identity.operation_id(),
            request_fingerprint: identity.request_fingerprint(),
        }
        .validate()?;
        let fence = self.acquire_operation_fence(operation_identity).await?;
        let result = self
            .reconcile(
                identity.authorization_domain(),
                identity.operation_id(),
                identity.request_fingerprint(),
            )
            .await;
        match result {
            Ok(committed) => {
                fence
                    .unlock()
                    .await
                    .inspect_err(|_| self.latch_unhealthy())?;
                Ok(committed.into())
            }
            Err(
                error @ (OperationRestoreError::MissingAttribution
                | OperationRestoreError::MissingIntent
                | OperationRestoreError::OperationAborted),
            ) => {
                fence
                    .unlock()
                    .await
                    .inspect_err(|_| self.latch_unhealthy())?;
                Err(error)
            }
            Err(error) => {
                drop(fence);
                Err(error)
            }
        }
    }

    /// Sticky storage/attribution health.
    pub fn is_healthy(&self) -> bool {
        self.0.healthy.load(Ordering::Acquire)
    }

    fn owner_mismatch<T>(&self) -> Result<T, OperationRestoreError> {
        self.latch_unhealthy();
        Err(OperationRestoreError::OwnerMismatch)
    }

    async fn reconcile_pending_manifest(
        &self,
        shard_id: u16,
        operation_record: &IntentRecord,
        manifest: OperationManifest,
    ) -> Result<OperationRestoreCommit, OperationRestoreError> {
        let identity = manifest.identity;
        self.validate_intent_record(operation_record, identity)?;
        let mut causally_superseded = false;
        for component in &manifest.components {
            causally_superseded |= matches!(
                self.apply_component(&manifest, component).await?,
                ComponentReconciliation::CausallySuperseded
            );
        }
        let terminal_at = self.authoritative_now_ms().await?;
        let retain_until = self.terminal_retain_until(terminal_at)?;
        let key = operation_shard_key(identity.domain, shard_id);
        let bootstrap_id = self.bootstrap_id(identity.domain)?;
        for _ in 0..MAX_CAS_ATTEMPTS {
            let Some((etag, mut shard)) = self.load_operation_shard(&key).await? else {
                self.latch_unhealthy();
                return Err(OperationRestoreError::MissingOperationShard);
            };
            self.validate_operation_shard(&shard, identity.domain, bootstrap_id, shard_id)?;
            let Some(current) = shard.entries.get(&identity.operation_id.to_string()) else {
                self.latch_unhealthy();
                return Err(OperationRestoreError::MissingIntent);
            };
            self.validate_intent_record(current, identity)?;
            match current.state {
                IntentState::Committed => return self.validate_committed(identity, current).await,
                IntentState::Aborted => return Err(OperationRestoreError::OperationAborted),
                IntentState::Pending if current.owner_token != operation_record.owner_token => {
                    return self.owner_mismatch()
                }
                IntentState::Pending => {}
            }
            shard.entries.insert(
                identity.operation_id.to_string(),
                current.committed(
                    manifest.manifest_digest,
                    terminal_at,
                    retain_until,
                    causally_superseded,
                ),
            );
            if matches!(
                self.store_record(&key, Some(&etag), &shard).await?,
                StoreCas::Won
            ) {
                return Ok(OperationRestoreCommit {
                    manifest_digest: manifest.manifest_digest,
                    replay: false,
                    causally_superseded,
                });
            }
        }
        Err(OperationRestoreError::OperationContention)
    }

    async fn validate_committed(
        &self,
        identity: OperationIdentity,
        record: &IntentRecord,
    ) -> Result<OperationRestoreCommit, OperationRestoreError> {
        let recorded_digest = decode32(
            record
                .manifest_digest
                .as_deref()
                .ok_or(OperationRestoreError::InvalidAttribution)?,
        )?;
        let manifest = match self.0.source.manifest(identity).await {
            Ok(manifest) => manifest,
            Err(ManifestReadError::Missing) => {
                self.latch_unhealthy();
                return Err(OperationRestoreError::MissingAttribution);
            }
            Err(ManifestReadError::Invalid) => {
                self.latch_unhealthy();
                return Err(OperationRestoreError::InvalidAttribution);
            }
            Err(ManifestReadError::Unavailable) => {
                self.latch_unhealthy();
                return Err(OperationRestoreError::DatabaseUnavailable);
            }
        };
        if manifest.identity != identity || manifest.manifest_digest != recorded_digest {
            self.latch_unhealthy();
            return Err(OperationRestoreError::InvalidAttribution);
        }
        let bootstrap_id = self.bootstrap_id(identity.domain)?;
        let mut causally_superseded = false;
        for component in &manifest.components {
            let key = floor_key(identity.domain, component.kind, component.key);
            let Some((_, floor)) = self.load_floor(&key).await? else {
                self.latch_unhealthy();
                return Err(OperationRestoreError::MissingFloor);
            };
            if let Err(error) = floor.validate_coordinate(
                identity.domain,
                bootstrap_id,
                component.kind,
                component.key,
            ) {
                self.latch_unhealthy();
                return Err(error);
            }
            if floor.version < component.after {
                self.latch_unhealthy();
                return Err(OperationRestoreError::BackwardFloor);
            }
            if floor.version == component.after {
                if !floor.exact_advance(&manifest, component)? {
                    self.latch_unhealthy();
                    return Err(OperationRestoreError::InvalidAttribution);
                }
            } else {
                self.prove_supersession(&manifest, component, &floor)
                    .await?;
                causally_superseded = true;
            }
        }
        if record.causally_superseded && !causally_superseded {
            self.latch_unhealthy();
            return Err(OperationRestoreError::InvalidAttribution);
        }
        Ok(OperationRestoreCommit {
            manifest_digest: recorded_digest,
            replay: true,
            causally_superseded,
        })
    }

    async fn maintain_begin_shard(
        &self,
        domain: CommunityId,
        shard_id: u16,
    ) -> Result<(), OperationRestoreError> {
        let deadline = tokio::time::Instant::now() + CAUSAL_LAG_BOUND;
        match tokio::time::timeout(
            CAUSAL_LAG_BOUND,
            self.maintain_begin_shard_until(domain, shard_id, deadline),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(OperationRestoreError::CausalLag),
        }
    }

    async fn maintain_begin_shard_until(
        &self,
        domain: CommunityId,
        shard_id: u16,
        deadline: tokio::time::Instant,
    ) -> Result<(), OperationRestoreError> {
        let bootstrap_id = self.bootstrap_id(domain)?;
        let key = operation_shard_key(domain, shard_id);
        let max_passes = usize::from(self.0.retention.entries_per_shard).saturating_mul(2);
        for _ in 0..max_passes {
            if tokio::time::Instant::now() >= deadline {
                return Err(OperationRestoreError::CausalLag);
            }
            let now = self.authoritative_now_ms().await?;
            let shard = self
                .compact_collectible_terminals(domain, bootstrap_id, shard_id, &key, now)
                .await?;
            let mut causal_lag = false;
            for (operation, record) in &shard.entries {
                if record.state != IntentState::Pending || now < record.expires_at_ms {
                    continue;
                }
                let identity =
                    self.identity_from_shard_entry(domain, shard_id, operation, record)?;
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    return Err(OperationRestoreError::CausalLag);
                }
                let fence = match self
                    .acquire_operation_fence_with_timeout(identity, remaining)
                    .await
                {
                    Ok(fence) => fence,
                    Err(
                        OperationRestoreError::OperationInProgress
                        | OperationRestoreError::OperationFenceTimeout
                        | OperationRestoreError::OperationFenceSaturated,
                    ) => {
                        causal_lag = true;
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                match self.0.source.manifest(identity).await {
                    Ok(manifest) if manifest.identity == identity => {
                        if !self.manifest_ready_for_startup(&manifest).await? {
                            drop(fence);
                            let _progressed = self
                                .resolve_manifest_predecessors(&manifest, deadline)
                                .await?;
                            // A predecessor transition always requires a
                            // reload of this shard before it can be declared
                            // ready; an unresolved predecessor remains
                            // bounded causal lag.
                            causal_lag = true;
                            continue;
                        }
                        let result = self
                            .reconcile_pending_manifest(shard_id, record, manifest)
                            .await;
                        match result {
                            Ok(_) => fence
                                .unlock()
                                .await
                                .inspect_err(|_| self.latch_unhealthy())?,
                            Err(error) => {
                                drop(fence);
                                return Err(error);
                            }
                        }
                        // Reload after every terminal transition; no stale
                        // eligibility or ETag is carried across a CAS.
                        continue;
                    }
                    Ok(_) | Err(ManifestReadError::Invalid) => {
                        drop(fence);
                        self.latch_unhealthy();
                        return Err(OperationRestoreError::InvalidAttribution);
                    }
                    Err(ManifestReadError::Missing) => {
                        let intent = OperationRestoreIntent {
                            identity,
                            owner_token: record.owner_token,
                            shard_id,
                        };
                        let result = self.abort_owned_intent(&intent).await;
                        match result {
                            Ok(()) => fence
                                .unlock()
                                .await
                                .inspect_err(|_| self.latch_unhealthy())?,
                            Err(error) => {
                                drop(fence);
                                return Err(error);
                            }
                        }
                        continue;
                    }
                    Err(ManifestReadError::Unavailable) => {
                        drop(fence);
                        self.latch_unhealthy();
                        return Err(OperationRestoreError::DatabaseUnavailable);
                    }
                }
            }
            if !causal_lag {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(OperationRestoreError::CausalLag);
            }
            tokio::time::sleep(CAS_RETRY_DELAY.min(remaining)).await;
        }
        Err(OperationRestoreError::OperationContention)
    }

    async fn resolve_manifest_predecessors(
        &self,
        successor: &OperationManifest,
        deadline: tokio::time::Instant,
    ) -> Result<bool, OperationRestoreError> {
        let predecessors = self
            .0
            .source
            .predecessors(successor)
            .await
            .map_err(|error| {
                self.latch_unhealthy();
                match error {
                    ManifestReadError::Missing | ManifestReadError::Invalid => {
                        OperationRestoreError::InvalidAttribution
                    }
                    ManifestReadError::Unavailable => OperationRestoreError::DatabaseUnavailable,
                }
            })?;
        let mut pending = BTreeMap::new();
        let mut discovered = std::collections::BTreeSet::new();
        for predecessor in predecessors {
            let coordinate = (
                predecessor.identity.operation_id,
                predecessor.identity.request_fingerprint,
            );
            discovered.insert(coordinate);
            pending.insert(coordinate, predecessor);
        }
        let mut progressed = false;
        while !pending.is_empty() {
            if tokio::time::Instant::now() >= deadline {
                return Ok(progressed);
            }
            let coordinates: Vec<_> = pending.keys().copied().collect();
            let mut changed = false;
            for coordinate in coordinates {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    return Ok(progressed);
                }
                let predecessor = pending
                    .get(&coordinate)
                    .cloned()
                    .ok_or(OperationRestoreError::InvalidAttribution)?;
                let identity = predecessor.identity;
                let bootstrap_id = self.bootstrap_id(identity.domain)?;
                let shard_id =
                    operation_shard_id(identity, bootstrap_id, self.0.retention.shard_count);
                let key = operation_shard_key(identity.domain, shard_id);
                let Some((_, shard)) = self.load_operation_shard(&key).await? else {
                    self.latch_unhealthy();
                    return Err(OperationRestoreError::MissingOperationShard);
                };
                self.validate_operation_shard(&shard, identity.domain, bootstrap_id, shard_id)?;
                let Some(record) = shard.entries.get(&identity.operation_id.to_string()) else {
                    pending.remove(&coordinate);
                    changed = true;
                    continue;
                };
                self.validate_intent_record(record, identity)?;
                if record.state != IntentState::Pending {
                    pending.remove(&coordinate);
                    changed = true;
                    continue;
                }
                let fence = match self
                    .acquire_operation_fence_with_timeout(identity, remaining)
                    .await
                {
                    Ok(fence) => fence,
                    Err(error) if error.is_healthy_retryable() => continue,
                    Err(error) => return Err(error),
                };
                let current = match self.0.source.manifest(identity).await {
                    Ok(manifest) if manifest.identity == identity => manifest,
                    Ok(_) | Err(ManifestReadError::Invalid) => {
                        drop(fence);
                        self.latch_unhealthy();
                        return Err(OperationRestoreError::InvalidAttribution);
                    }
                    Err(ManifestReadError::Missing) => {
                        drop(fence);
                        pending.remove(&coordinate);
                        changed = true;
                        continue;
                    }
                    Err(ManifestReadError::Unavailable) => {
                        drop(fence);
                        self.latch_unhealthy();
                        return Err(OperationRestoreError::DatabaseUnavailable);
                    }
                };
                if !self.manifest_ready_for_startup(&current).await? {
                    drop(fence);
                    let earlier = self
                        .0
                        .source
                        .predecessors(&current)
                        .await
                        .map_err(|error| {
                            self.latch_unhealthy();
                            match error {
                                ManifestReadError::Missing | ManifestReadError::Invalid => {
                                    OperationRestoreError::InvalidAttribution
                                }
                                ManifestReadError::Unavailable => {
                                    OperationRestoreError::DatabaseUnavailable
                                }
                            }
                        })?;
                    for earlier in earlier {
                        let earlier_coordinate = (
                            earlier.identity.operation_id,
                            earlier.identity.request_fingerprint,
                        );
                        if discovered.insert(earlier_coordinate) {
                            pending.insert(earlier_coordinate, earlier);
                            changed = true;
                        }
                    }
                    continue;
                }
                let result = self
                    .reconcile_pending_manifest(shard_id, record, current)
                    .await;
                match result {
                    Ok(_) => fence
                        .unlock()
                        .await
                        .inspect_err(|_| self.latch_unhealthy())?,
                    Err(error) => {
                        drop(fence);
                        return Err(error);
                    }
                }
                pending.remove(&coordinate);
                progressed = true;
                changed = true;
            }
            if !changed {
                return Ok(progressed);
            }
        }
        Ok(progressed)
    }

    async fn load_owned_intent(
        &self,
        intent: &OperationRestoreIntent,
    ) -> Result<IntentRecord, OperationRestoreError> {
        let key = operation_shard_key(intent.identity.domain, intent.shard_id);
        let bootstrap_id = self.bootstrap_id(intent.identity.domain)?;
        let Some((_, shard)) = self.load_operation_shard(&key).await? else {
            self.latch_unhealthy();
            return Err(OperationRestoreError::MissingOperationShard);
        };
        self.validate_operation_shard(
            &shard,
            intent.identity.domain,
            bootstrap_id,
            intent.shard_id,
        )?;
        let Some(record) = shard.entries.get(&intent.identity.operation_id.to_string()) else {
            return self.owner_mismatch();
        };
        self.validate_intent_record(record, intent.identity)?;
        if record.owner_token != intent.owner_token {
            return self.owner_mismatch();
        }
        Ok(record.clone())
    }

    async fn abort_owned_intent(
        &self,
        intent: &OperationRestoreIntent,
    ) -> Result<(), OperationRestoreError> {
        let key = operation_shard_key(intent.identity.domain, intent.shard_id);
        let bootstrap_id = self.bootstrap_id(intent.identity.domain)?;
        for _ in 0..MAX_CAS_ATTEMPTS {
            let Some((etag, mut shard)) = self.load_operation_shard(&key).await? else {
                self.latch_unhealthy();
                return Err(OperationRestoreError::MissingOperationShard);
            };
            self.validate_operation_shard(
                &shard,
                intent.identity.domain,
                bootstrap_id,
                intent.shard_id,
            )?;
            let Some(record) = shard.entries.get(&intent.identity.operation_id.to_string()) else {
                return self.owner_mismatch();
            };
            self.validate_intent_record(record, intent.identity)?;
            match record.state {
                IntentState::Committed => return Err(OperationRestoreError::AlreadyCommitted),
                IntentState::Aborted if record.owner_token == intent.owner_token => return Ok(()),
                IntentState::Aborted => return self.owner_mismatch(),
                IntentState::Pending if record.owner_token != intent.owner_token => {
                    return self.owner_mismatch()
                }
                IntentState::Pending => {}
            }
            let now = self.authoritative_now_ms().await?;
            let aborted = record.aborted(now, self.terminal_retain_until(now)?);
            shard
                .entries
                .insert(intent.identity.operation_id.to_string(), aborted);
            if matches!(
                self.store_record(&key, Some(&etag), &shard).await?,
                StoreCas::Won
            ) {
                return Ok(());
            }
        }
        Err(OperationRestoreError::OperationContention)
    }

    async fn provision_floor(
        &self,
        domain: CommunityId,
        floor: AuthorizationVersionComponentFloor,
    ) -> Result<(), OperationRestoreError> {
        let bootstrap_id = self.bootstrap_id(domain)?;
        if floor.component_key == [0; 32] || floor.version > i64::MAX as u64 {
            self.latch_unhealthy();
            return Err(OperationRestoreError::InvalidAttribution);
        }
        let key = floor_key(domain, floor.component_kind, floor.component_key);
        let expected = FloorRecord::provisioned(domain, bootstrap_id, &floor);
        for _ in 0..MAX_CAS_ATTEMPTS {
            match self.load_floor(&key).await? {
                None => {
                    if matches!(
                        self.store_record(&key, None, &expected).await?,
                        StoreCas::Won
                    ) {
                        return Ok(());
                    }
                }
                Some((_, existing)) => {
                    if let Err(error) = existing.validate_coordinate(
                        domain,
                        bootstrap_id,
                        floor.component_kind,
                        floor.component_key,
                    ) {
                        self.latch_unhealthy();
                        return Err(error);
                    }
                    if existing.version == floor.version {
                        return Ok(());
                    }
                    self.latch_unhealthy();
                    return Err(if existing.version > floor.version {
                        OperationRestoreError::BackwardFloor
                    } else {
                        OperationRestoreError::StaleFloor
                    });
                }
            }
        }
        self.latch_unhealthy();
        Err(OperationRestoreError::StoreUnavailable)
    }

    fn validate_worst_case_shard_size(
        &self,
        domain: CommunityId,
        bootstrap_id: Uuid,
    ) -> Result<(), OperationRestoreError> {
        let mut shard = OperationShardRecord::empty(domain, bootstrap_id, 0, self.0.retention);
        let retention_ms = self.0.retention.retention_millis()?;
        let terminal_at = u64::MAX
            .checked_sub(retention_ms)
            .ok_or(OperationRestoreError::InvalidInput)?;
        let issued_at = terminal_at
            .checked_sub(2)
            .ok_or(OperationRestoreError::InvalidInput)?;
        for index in 0..self.0.retention.entries_per_shard {
            let operation_id = Uuid::from_u128(u128::from(index) + 1);
            let identity = OperationIdentity {
                domain,
                operation_id,
                request_fingerprint: [u8::try_from(index % 254 + 1).unwrap_or(1); 32],
            };
            let record = IntentRecord::pending(
                identity,
                Uuid::from_u128(u128::from(index) + 10_000),
                bootstrap_id,
                issued_at,
                issued_at + 1,
            )
            .committed([7; 32], terminal_at, u64::MAX, true);
            shard.entries.insert(operation_id.to_string(), record);
        }
        let encoded =
            serde_json::to_vec(&shard).map_err(|_| OperationRestoreError::InvalidAttribution)?;
        if u64::try_from(encoded.len()).unwrap_or(u64::MAX) > MAX_RECORD_BYTES {
            return Err(OperationRestoreError::InvalidInput);
        }
        Ok(())
    }

    async fn provision_operation_shard(
        &self,
        domain: CommunityId,
        bootstrap_id: Uuid,
        shard_id: u16,
    ) -> Result<(), OperationRestoreError> {
        let key = operation_shard_key(domain, shard_id);
        let expected =
            OperationShardRecord::empty(domain, bootstrap_id, shard_id, self.0.retention);
        for _ in 0..MAX_CAS_ATTEMPTS {
            match self.load_operation_shard(&key).await? {
                None => {
                    if matches!(
                        self.store_record(&key, None, &expected).await?,
                        StoreCas::Won
                    ) {
                        return Ok(());
                    }
                }
                Some((_, existing)) => {
                    self.validate_operation_shard(&existing, domain, bootstrap_id, shard_id)?;
                    return Ok(());
                }
            }
        }
        Err(OperationRestoreError::OperationContention)
    }

    fn validate_operation_shard(
        &self,
        shard: &OperationShardRecord,
        domain: CommunityId,
        bootstrap_id: Uuid,
        shard_id: u16,
    ) -> Result<(), OperationRestoreError> {
        if shard.format != OPERATION_SHARD_FORMAT_VERSION
            || shard.mapping_version != OPERATION_SHARD_MAPPING_VERSION
            || shard.domain != domain.as_uuid().to_string()
            || shard.bootstrap_id != bootstrap_id
            || shard.bootstrap_id.is_nil()
            || shard.shard_id != shard_id
            || decode32(&shard.layout_digest)? != self.0.retention.layout_digest()
            || shard.entries.len() > usize::from(self.0.retention.entries_per_shard)
        {
            self.latch_unhealthy();
            return Err(OperationRestoreError::InvalidOperationShard);
        }
        for (operation, record) in &shard.entries {
            let operation_id = Uuid::parse_str(operation).map_err(|_| {
                self.latch_unhealthy();
                OperationRestoreError::InvalidOperationShard
            })?;
            let request_fingerprint = decode32(&record.request_fingerprint).map_err(|_| {
                self.latch_unhealthy();
                OperationRestoreError::InvalidOperationShard
            })?;
            let identity = OperationIdentity {
                domain,
                operation_id,
                request_fingerprint,
            }
            .validate()
            .map_err(|_| {
                self.latch_unhealthy();
                OperationRestoreError::InvalidOperationShard
            })?;
            if operation_shard_id(identity, bootstrap_id, self.0.retention.shard_count) != shard_id
            {
                self.latch_unhealthy();
                return Err(OperationRestoreError::InvalidOperationShard);
            }
            self.validate_intent_record(record, identity)?;
        }
        Ok(())
    }

    async fn apply_component(
        &self,
        manifest: &OperationManifest,
        component: &ManifestComponent,
    ) -> Result<ComponentReconciliation, OperationRestoreError> {
        let bootstrap_id = self.bootstrap_id(manifest.identity.domain)?;
        if component.key == [0; 32]
            || component.after <= component.before
            || component.after > i64::MAX as u64
        {
            self.latch_unhealthy();
            return Err(OperationRestoreError::InvalidAttribution);
        }
        let key = floor_key(manifest.identity.domain, component.kind, component.key);
        let started = tokio::time::Instant::now();
        for attempt in 0..MAX_CAS_ATTEMPTS {
            match self.load_floor(&key).await? {
                None if component.before == 0 => {
                    let record = FloorRecord::advanced(bootstrap_id, manifest, component);
                    if matches!(self.store_record(&key, None, &record).await?, StoreCas::Won) {
                        return Ok(ComponentReconciliation::Applied);
                    }
                }
                None => {
                    if attempt + 1 < MAX_CAS_ATTEMPTS && started.elapsed() < CAUSAL_LAG_BOUND {
                        tokio::time::sleep(CAS_RETRY_DELAY).await;
                        continue;
                    }
                    let database_version = self
                        .0
                        .source
                        .floors(manifest.identity.domain)
                        .await
                        .map_err(|error| {
                            self.latch_unhealthy();
                            match error {
                                ManifestReadError::Unavailable => {
                                    OperationRestoreError::DatabaseUnavailable
                                }
                                ManifestReadError::Missing | ManifestReadError::Invalid => {
                                    OperationRestoreError::InvalidAttribution
                                }
                            }
                        })?
                        .into_iter()
                        .find(|floor| {
                            floor.component_kind == component.kind
                                && floor.component_key == component.key
                        })
                        .map_or(0, |floor| floor.version);
                    if database_version >= component.after {
                        return Err(OperationRestoreError::CausalLag);
                    }
                    self.latch_unhealthy();
                    return Err(OperationRestoreError::MissingFloor);
                }
                Some((etag, record)) => {
                    if let Err(error) = record.validate_coordinate(
                        manifest.identity.domain,
                        bootstrap_id,
                        component.kind,
                        component.key,
                    ) {
                        self.latch_unhealthy();
                        return Err(error);
                    }
                    if record.exact_advance(manifest, component)? {
                        return Ok(ComponentReconciliation::Exact);
                    }
                    if record.version == component.before {
                        let advanced = FloorRecord::advanced(bootstrap_id, manifest, component);
                        if matches!(
                            self.store_record(&key, Some(&etag), &advanced).await?,
                            StoreCas::Won
                        ) {
                            return Ok(ComponentReconciliation::Applied);
                        }
                        continue;
                    }
                    if record.version > component.after {
                        self.prove_supersession(manifest, component, &record)
                            .await?;
                        return Ok(ComponentReconciliation::CausallySuperseded);
                    }
                    if record.version < component.before {
                        if attempt + 1 < MAX_CAS_ATTEMPTS && started.elapsed() < CAUSAL_LAG_BOUND {
                            tokio::time::sleep(CAS_RETRY_DELAY).await;
                            continue;
                        }
                        let database_version = self
                            .0
                            .source
                            .floors(manifest.identity.domain)
                            .await
                            .map_err(|error| {
                                self.latch_unhealthy();
                                match error {
                                    ManifestReadError::Unavailable => {
                                        OperationRestoreError::DatabaseUnavailable
                                    }
                                    ManifestReadError::Missing | ManifestReadError::Invalid => {
                                        OperationRestoreError::InvalidAttribution
                                    }
                                }
                            })?
                            .into_iter()
                            .find(|floor| {
                                floor.component_kind == component.kind
                                    && floor.component_key == component.key
                            })
                            .map_or(0, |floor| floor.version);
                        if database_version >= component.after {
                            return Err(OperationRestoreError::CausalLag);
                        }
                        self.latch_unhealthy();
                        return Err(OperationRestoreError::StaleFloor);
                    }
                    self.latch_unhealthy();
                    return Err(OperationRestoreError::StaleFloor);
                }
            }
        }
        Err(OperationRestoreError::OperationContention)
    }

    async fn prove_supersession(
        &self,
        manifest: &OperationManifest,
        component: &ManifestComponent,
        floor: &FloorRecord,
    ) -> Result<(), OperationRestoreError> {
        self.0
            .source
            .prove_supersession(manifest, component, floor)
            .await
            .map_err(|error| {
                self.latch_unhealthy();
                match error {
                    ManifestReadError::Unavailable => OperationRestoreError::DatabaseUnavailable,
                    ManifestReadError::Missing | ManifestReadError::Invalid => {
                        OperationRestoreError::InvalidAttribution
                    }
                }
            })
    }

    async fn load_operation_shard(
        &self,
        key: &str,
    ) -> Result<Option<(String, OperationShardRecord)>, OperationRestoreError> {
        self.load_record(key).await
    }

    async fn load_bootstrap(
        &self,
        key: &str,
    ) -> Result<Option<(String, BootstrapRecord)>, OperationRestoreError> {
        self.load_record(key).await
    }

    async fn load_floor(
        &self,
        key: &str,
    ) -> Result<Option<(String, FloorRecord)>, OperationRestoreError> {
        self.load_record(key).await
    }

    async fn load_record<T: for<'de> Deserialize<'de>>(
        &self,
        key: &str,
    ) -> Result<Option<(String, T)>, OperationRestoreError> {
        let loaded = self.0.store.load(key).await.map_err(|_| {
            self.latch_unhealthy();
            OperationRestoreError::StoreUnavailable
        })?;
        loaded
            .map(|loaded| {
                if u64::try_from(loaded.body.len()).unwrap_or(u64::MAX) > MAX_RECORD_BYTES {
                    self.latch_unhealthy();
                    return Err(OperationRestoreError::StoreUnavailable);
                }
                serde_json::from_slice(&loaded.body)
                    .map(|record| (loaded.etag, record))
                    .map_err(|_| {
                        self.latch_unhealthy();
                        OperationRestoreError::InvalidAttribution
                    })
            })
            .transpose()
    }

    async fn store_record<T: Serialize>(
        &self,
        key: &str,
        expected_etag: Option<&str>,
        record: &T,
    ) -> Result<StoreCas, OperationRestoreError> {
        let body = serde_json::to_vec(record).map_err(|_| {
            self.latch_unhealthy();
            OperationRestoreError::InvalidAttribution
        })?;
        if u64::try_from(body.len()).unwrap_or(u64::MAX) > MAX_RECORD_BYTES {
            self.latch_unhealthy();
            return Err(OperationRestoreError::StoreUnavailable);
        }
        tokio::time::timeout(
            STORE_CAS_TIMEOUT,
            self.0.store.compare_and_swap(key, expected_etag, &body),
        )
        .await
        .map_err(|_| {
            self.latch_unhealthy();
            OperationRestoreError::StoreUnavailable
        })?
        .map_err(|_| {
            self.latch_unhealthy();
            OperationRestoreError::StoreUnavailable
        })
    }

    fn require_healthy(&self) -> Result<(), OperationRestoreError> {
        if self.is_healthy() {
            Ok(())
        } else {
            Err(OperationRestoreError::Unhealthy)
        }
    }

    fn bootstrap_id(&self, domain: CommunityId) -> Result<Uuid, OperationRestoreError> {
        self.0
            .bootstrap_ids
            .get(&domain)
            .copied()
            .ok_or(OperationRestoreError::DomainNotConfigured)
    }

    fn require_domain_ready(&self, domain: CommunityId) -> Result<(), OperationRestoreError> {
        if self.0.ready_domains.contains(&domain) {
            Ok(())
        } else {
            Err(OperationRestoreError::MissingBootstrap)
        }
    }

    async fn read_database_floors(
        &self,
        domain: CommunityId,
    ) -> Result<Vec<AuthorizationVersionComponentFloor>, OperationRestoreError> {
        self.0.source.floors(domain).await.map_err(|error| {
            self.latch_unhealthy();
            match error {
                ManifestReadError::Missing | ManifestReadError::Invalid => {
                    OperationRestoreError::InvalidAttribution
                }
                ManifestReadError::Unavailable => OperationRestoreError::DatabaseUnavailable,
            }
        })
    }

    async fn authoritative_now_ms(&self) -> Result<u64, OperationRestoreError> {
        self.0.source.authoritative_now_ms().await.map_err(|error| {
            self.latch_unhealthy();
            match error {
                ManifestReadError::Unavailable => OperationRestoreError::DatabaseUnavailable,
                ManifestReadError::Missing | ManifestReadError::Invalid => {
                    OperationRestoreError::InvalidAttribution
                }
            }
        })
    }

    fn terminal_retain_until(&self, terminal_at_ms: u64) -> Result<u64, OperationRestoreError> {
        terminal_at_ms
            .checked_add(self.0.retention.retention_millis()?)
            .ok_or(OperationRestoreError::InvalidAttribution)
    }

    fn validate_bootstrap(
        &self,
        marker: &BootstrapRecord,
        domain: CommunityId,
        bootstrap_id: Uuid,
    ) -> Result<(), OperationRestoreError> {
        if marker.format == FORMAT_VERSION
            && marker.domain == domain.as_uuid().to_string()
            && marker.bootstrap_id == bootstrap_id
            && !marker.bootstrap_id.is_nil()
            && marker.operation_mapping_version == OPERATION_SHARD_MAPPING_VERSION
            && marker.operation_shard_count == self.0.retention.shard_count
            && marker.operation_entries_per_shard == self.0.retention.entries_per_shard
            && marker.terminal_retention_ms == self.0.retention.retention_millis()?
            && decode32(&marker.operation_record_format_hash)?
                == operation_shard_record_format_hash()
            && decode32(&marker.operation_layout_digest)? == self.0.retention.layout_digest()
        {
            Ok(())
        } else {
            self.latch_unhealthy();
            Err(OperationRestoreError::InvalidBootstrap)
        }
    }

    fn validate_intent_record(
        &self,
        record: &IntentRecord,
        identity: OperationIdentity,
    ) -> Result<(), OperationRestoreError> {
        let bootstrap_id = self.bootstrap_id(identity.domain)?;
        let validation = record
            .validate_identity(identity, bootstrap_id)
            .and_then(|()| {
                if let Some((terminal_at, retain_until)) =
                    record.terminal_at_ms.zip(record.retain_until_ms)
                {
                    if retain_until.checked_sub(terminal_at)
                        != Some(self.0.retention.retention_millis()?)
                    {
                        return Err(OperationRestoreError::InvalidAttribution);
                    }
                }
                Ok(())
            });
        match validation {
            Err(OperationRestoreError::InvalidAttribution) => {
                self.latch_unhealthy();
                Err(OperationRestoreError::InvalidAttribution)
            }
            other => other,
        }
    }

    fn identity_from_shard_entry(
        &self,
        domain: CommunityId,
        shard_id: u16,
        operation: &str,
        record: &IntentRecord,
    ) -> Result<OperationIdentity, OperationRestoreError> {
        let operation_id = Uuid::parse_str(&record.operation_id).map_err(|_| {
            self.latch_unhealthy();
            OperationRestoreError::InvalidAttribution
        })?;
        let request_fingerprint = decode32(&record.request_fingerprint).inspect_err(|_| {
            self.latch_unhealthy();
        })?;
        let identity = OperationIdentity {
            domain,
            operation_id,
            request_fingerprint,
        }
        .validate()
        .inspect_err(|_| {
            self.latch_unhealthy();
        })?;
        if identity.operation_id.to_string() != operation
            || operation_shard_id(
                identity,
                self.bootstrap_id(domain)?,
                self.0.retention.shard_count,
            ) != shard_id
        {
            self.latch_unhealthy();
            return Err(OperationRestoreError::InvalidAttribution);
        }
        self.validate_intent_record(record, identity)?;
        Ok(identity)
    }

    fn latch_unhealthy(&self) {
        self.0.healthy.store(false, Ordering::Release);
    }
}

impl fmt::Debug for OperationRestoreRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationRestoreRuntime")
            .field("healthy", &self.is_healthy())
            .field("coordinates", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum IntentState {
    Pending,
    Committed,
    Aborted,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BootstrapRecord {
    format: u8,
    domain: String,
    bootstrap_id: Uuid,
    operation_mapping_version: u8,
    operation_shard_count: u16,
    operation_entries_per_shard: u16,
    terminal_retention_ms: u64,
    operation_record_format_hash: String,
    operation_layout_digest: String,
}

impl BootstrapRecord {
    fn new(
        domain: CommunityId,
        bootstrap_id: Uuid,
        retention: OperationRestoreRetentionPolicy,
    ) -> Self {
        Self {
            format: FORMAT_VERSION,
            domain: domain.as_uuid().to_string(),
            bootstrap_id,
            operation_mapping_version: OPERATION_SHARD_MAPPING_VERSION,
            operation_shard_count: retention.shard_count,
            operation_entries_per_shard: retention.entries_per_shard,
            terminal_retention_ms: retention.retention_millis().unwrap_or(u64::MAX),
            operation_record_format_hash: hex::encode(operation_shard_record_format_hash()),
            operation_layout_digest: hex::encode(retention.layout_digest()),
        }
    }
}

impl fmt::Debug for BootstrapRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BootstrapRecord([REDACTED])")
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct IntentRecord {
    format: u8,
    domain: String,
    operation_id: String,
    request_fingerprint: String,
    bootstrap_id: Uuid,
    owner_token: Uuid,
    issued_at_ms: u64,
    expires_at_ms: u64,
    state: IntentState,
    manifest_digest: Option<String>,
    terminal_at_ms: Option<u64>,
    retain_until_ms: Option<u64>,
    causally_superseded: bool,
}

impl IntentRecord {
    fn pending(
        identity: OperationIdentity,
        owner_token: Uuid,
        bootstrap_id: Uuid,
        issued_at_ms: u64,
        expires_at_ms: u64,
    ) -> Self {
        Self {
            format: FORMAT_VERSION,
            domain: identity.domain.as_uuid().to_string(),
            operation_id: identity.operation_id.to_string(),
            request_fingerprint: hex::encode(identity.request_fingerprint),
            bootstrap_id,
            owner_token,
            issued_at_ms,
            expires_at_ms,
            state: IntentState::Pending,
            manifest_digest: None,
            terminal_at_ms: None,
            retain_until_ms: None,
            causally_superseded: false,
        }
    }

    fn validate_identity(
        &self,
        expected: OperationIdentity,
        bootstrap_id: Uuid,
    ) -> Result<(), OperationRestoreError> {
        let terminal_shape_valid = self.terminal_at_ms.zip(self.retain_until_ms).is_some_and(
            |(terminal_at, retain_until)| {
                terminal_at >= self.issued_at_ms && retain_until > terminal_at
            },
        );
        let state_shape_valid = match self.state {
            IntentState::Pending => {
                self.manifest_digest.is_none()
                    && self.terminal_at_ms.is_none()
                    && self.retain_until_ms.is_none()
                    && !self.causally_superseded
            }
            IntentState::Aborted => {
                self.manifest_digest.is_none() && terminal_shape_valid && !self.causally_superseded
            }
            IntentState::Committed => {
                self.manifest_digest
                    .as_deref()
                    .is_some_and(|digest| decode32(digest).is_ok())
                    && terminal_shape_valid
            }
        };
        if self.format != FORMAT_VERSION
            || self.domain != expected.domain.as_uuid().to_string()
            || self.operation_id != expected.operation_id.to_string()
            || self.bootstrap_id != bootstrap_id
            || self.bootstrap_id.is_nil()
            || self.owner_token.is_nil()
            || self.issued_at_ms == 0
            || self.expires_at_ms <= self.issued_at_ms
            || self.expires_at_ms - self.issued_at_ms
                > u64::try_from(MAX_LEASE.as_millis()).unwrap_or(u64::MAX)
            || !state_shape_valid
        {
            return Err(OperationRestoreError::InvalidAttribution);
        }
        if decode32(&self.request_fingerprint)? != expected.request_fingerprint {
            return Err(OperationRestoreError::OperationIdentityConflict);
        }
        Ok(())
    }

    fn committed(
        &self,
        manifest_digest: [u8; 32],
        terminal_at_ms: u64,
        retain_until_ms: u64,
        causally_superseded: bool,
    ) -> Self {
        Self {
            format: self.format,
            domain: self.domain.clone(),
            operation_id: self.operation_id.clone(),
            request_fingerprint: self.request_fingerprint.clone(),
            bootstrap_id: self.bootstrap_id,
            owner_token: self.owner_token,
            issued_at_ms: self.issued_at_ms,
            expires_at_ms: self.expires_at_ms,
            state: IntentState::Committed,
            manifest_digest: Some(hex::encode(manifest_digest)),
            terminal_at_ms: Some(terminal_at_ms),
            retain_until_ms: Some(retain_until_ms),
            causally_superseded,
        }
    }

    fn aborted(&self, terminal_at_ms: u64, retain_until_ms: u64) -> Self {
        Self {
            format: self.format,
            domain: self.domain.clone(),
            operation_id: self.operation_id.clone(),
            request_fingerprint: self.request_fingerprint.clone(),
            bootstrap_id: self.bootstrap_id,
            owner_token: self.owner_token,
            issued_at_ms: self.issued_at_ms,
            expires_at_ms: self.expires_at_ms,
            state: IntentState::Aborted,
            manifest_digest: None,
            terminal_at_ms: Some(terminal_at_ms),
            retain_until_ms: Some(retain_until_ms),
            causally_superseded: false,
        }
    }

    fn is_collectible(&self, authoritative_now_ms: u64) -> bool {
        matches!(self.state, IntentState::Committed | IntentState::Aborted)
            && self
                .retain_until_ms
                .is_some_and(|retain_until| authoritative_now_ms >= retain_until)
    }
}

impl fmt::Debug for IntentRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("IntentRecord([REDACTED])")
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OperationShardRecord {
    format: u8,
    mapping_version: u8,
    domain: String,
    bootstrap_id: Uuid,
    layout_digest: String,
    shard_id: u16,
    entries: BTreeMap<String, IntentRecord>,
}

impl OperationShardRecord {
    fn empty(
        domain: CommunityId,
        bootstrap_id: Uuid,
        shard_id: u16,
        retention: OperationRestoreRetentionPolicy,
    ) -> Self {
        Self {
            format: OPERATION_SHARD_FORMAT_VERSION,
            mapping_version: OPERATION_SHARD_MAPPING_VERSION,
            domain: domain.as_uuid().to_string(),
            bootstrap_id,
            layout_digest: hex::encode(retention.layout_digest()),
            shard_id,
            entries: BTreeMap::new(),
        }
    }
}

impl fmt::Debug for OperationShardRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationShardRecord")
            .field("shard_id", &self.shard_id)
            .field("entry_count", &self.entries.len())
            .field("coordinates", &"[REDACTED]")
            .finish()
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FloorRecord {
    format: u8,
    domain: String,
    bootstrap_id: Uuid,
    component_kind: i16,
    component_key: String,
    version: u64,
    operation_id: Option<String>,
    request_fingerprint: Option<String>,
    component_digest: Option<String>,
    manifest_digest: Option<String>,
}

struct FloorProvenance {
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    component_digest: [u8; 32],
    manifest_digest: [u8; 32],
}

impl FloorRecord {
    fn provisioned(
        domain: CommunityId,
        bootstrap_id: Uuid,
        floor: &AuthorizationVersionComponentFloor,
    ) -> Self {
        Self {
            format: FORMAT_VERSION,
            domain: domain.as_uuid().to_string(),
            bootstrap_id,
            component_kind: floor.component_kind as i16,
            component_key: hex::encode(floor.component_key),
            version: floor.version,
            operation_id: None,
            request_fingerprint: None,
            component_digest: None,
            manifest_digest: None,
        }
    }

    fn advanced(
        bootstrap_id: Uuid,
        manifest: &OperationManifest,
        component: &ManifestComponent,
    ) -> Self {
        Self {
            format: FORMAT_VERSION,
            domain: manifest.identity.domain.as_uuid().to_string(),
            bootstrap_id,
            component_kind: component.kind as i16,
            component_key: hex::encode(component.key),
            version: component.after,
            operation_id: Some(manifest.identity.operation_id.to_string()),
            request_fingerprint: Some(hex::encode(manifest.identity.request_fingerprint)),
            component_digest: Some(hex::encode(component.digest)),
            manifest_digest: Some(hex::encode(manifest.manifest_digest)),
        }
    }

    fn validate_coordinate(
        &self,
        domain: CommunityId,
        bootstrap_id: Uuid,
        kind: AuthorizationVersionComponentKind,
        key: [u8; 32],
    ) -> Result<(), OperationRestoreError> {
        let provenance_valid = match (
            self.operation_id.as_deref(),
            self.request_fingerprint.as_deref(),
            self.component_digest.as_deref(),
            self.manifest_digest.as_deref(),
        ) {
            (None, None, None, None) => true,
            (Some(operation), Some(request), Some(component), Some(manifest)) => {
                Uuid::parse_str(operation).is_ok_and(|operation| !operation.is_nil())
                    && decode32(request).is_ok()
                    && decode32(component).is_ok()
                    && decode32(manifest).is_ok()
            }
            _ => false,
        };
        if self.format == FORMAT_VERSION
            && self.domain == domain.as_uuid().to_string()
            && self.bootstrap_id == bootstrap_id
            && !self.bootstrap_id.is_nil()
            && self.component_kind == kind as i16
            && decode32(&self.component_key)? == key
            && self.version <= i64::MAX as u64
            && provenance_valid
        {
            Ok(())
        } else {
            Err(OperationRestoreError::WrongComponentKey)
        }
    }

    fn exact_advance(
        &self,
        manifest: &OperationManifest,
        component: &ManifestComponent,
    ) -> Result<bool, OperationRestoreError> {
        if self.version != component.after {
            return Ok(false);
        }
        Ok(self.operation_id.as_deref()
            == Some(manifest.identity.operation_id.to_string().as_str())
            && self.request_fingerprint.as_deref()
                == Some(hex::encode(manifest.identity.request_fingerprint).as_str())
            && self.component_digest.as_deref() == Some(hex::encode(component.digest).as_str())
            && self.manifest_digest.as_deref()
                == Some(hex::encode(manifest.manifest_digest).as_str()))
    }

    fn provenance(&self) -> Result<FloorProvenance, OperationRestoreError> {
        let operation_id = Uuid::parse_str(
            self.operation_id
                .as_deref()
                .ok_or(OperationRestoreError::InvalidAttribution)?,
        )
        .map_err(|_| OperationRestoreError::InvalidAttribution)?;
        if operation_id.is_nil() {
            return Err(OperationRestoreError::InvalidAttribution);
        }
        Ok(FloorProvenance {
            operation_id,
            request_fingerprint: decode32(
                self.request_fingerprint
                    .as_deref()
                    .ok_or(OperationRestoreError::InvalidAttribution)?,
            )?,
            component_digest: decode32(
                self.component_digest
                    .as_deref()
                    .ok_or(OperationRestoreError::InvalidAttribution)?,
            )?,
            manifest_digest: decode32(
                self.manifest_digest
                    .as_deref()
                    .ok_or(OperationRestoreError::InvalidAttribution)?,
            )?,
        })
    }
}

impl fmt::Debug for FloorRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FloorRecord([REDACTED])")
    }
}

fn operation_shard_key(domain: CommunityId, shard_id: u16) -> String {
    format!(
        "nip-fi/restore/v1/operation-shards/{}/{shard_id:04x}",
        domain.as_uuid()
    )
}

#[cfg(any(test, feature = "test-utils"))]
fn operation_fence_test_key(identity: OperationIdentity) -> i64 {
    let mut digest = Sha256::new();
    digest.update(b"buzz:nip-fi-operation-session-fence:v1");
    digest.update(identity.domain.as_uuid().as_bytes());
    digest.update(identity.operation_id.as_bytes());
    digest.update(identity.request_fingerprint);
    let digest: [u8; 32] = digest.finalize().into();
    i64::from_be_bytes(digest[..8].try_into().expect("eight-byte fence key"))
}

fn operation_shard_id(identity: OperationIdentity, bootstrap_id: Uuid, shard_count: u16) -> u16 {
    let mut digest = Sha256::new();
    digest.update(b"buzz:nip-fi-operation-restore-shard:v1");
    digest.update(bootstrap_id.as_bytes());
    digest.update(identity.domain.as_uuid().as_bytes());
    digest.update(identity.operation_id.as_bytes());
    let bytes: [u8; 32] = digest.finalize().into();
    u16::from_be_bytes([bytes[0], bytes[1]]) & (shard_count - 1)
}

fn operation_shard_record_format_hash() -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(
        b"buzz:nip-fi-operation-shard-record:v1|format|mapping_version|domain|bootstrap_id|layout_digest|shard_id|entries|operation_id|request_fingerprint|owner_token|issued_at_ms|expires_at_ms|state|manifest_digest|terminal_at_ms|retain_until_ms|causally_superseded",
    );
    digest.finalize().into()
}

fn bootstrap_key(domain: CommunityId) -> String {
    format!("nip-fi/restore/v1/bootstrap/{}", domain.as_uuid())
}

fn floor_key(
    domain: CommunityId,
    kind: AuthorizationVersionComponentKind,
    key: [u8; 32],
) -> String {
    format!(
        "nip-fi/restore/v1/floors/{}/{}/{}",
        domain.as_uuid(),
        kind as i16,
        hex::encode(key)
    )
}

fn floor_prefix(domain: CommunityId) -> String {
    format!("nip-fi/restore/v1/floors/{}/", domain.as_uuid())
}

fn component_kind_from_i16(
    value: i16,
) -> Result<AuthorizationVersionComponentKind, OperationRestoreError> {
    match value {
        1 => Ok(AuthorizationVersionComponentKind::Binding),
        2 => Ok(AuthorizationVersionComponentKind::Policy),
        3 => Ok(AuthorizationVersionComponentKind::InvalidationGeneration),
        4 => Ok(AuthorizationVersionComponentKind::AuthorityEpoch),
        6 => Ok(AuthorizationVersionComponentKind::DelegatedRelationship),
        7 => Ok(AuthorizationVersionComponentKind::LifecycleSelector),
        _ => Err(OperationRestoreError::InvalidAttribution),
    }
}

fn lease_expiry(now_ms: u64, lease: Duration) -> Result<u64, OperationRestoreError> {
    let lease_ms =
        u64::try_from(lease.as_millis()).map_err(|_| OperationRestoreError::InvalidInput)?;
    now_ms
        .checked_add(lease_ms)
        .ok_or(OperationRestoreError::InvalidInput)
}

fn decode32(value: &str) -> Result<[u8; 32], OperationRestoreError> {
    let decoded = hex::decode(value).map_err(|_| OperationRestoreError::InvalidAttribution)?;
    decoded
        .try_into()
        .map_err(|_| OperationRestoreError::InvalidAttribution)
}

/// Closed fail-closed restore witness failures. Coordinates are never rendered.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum OperationRestoreError {
    /// Caller or deployment input is malformed.
    #[error("operation restore input is invalid")]
    InvalidInput,
    /// Domain is not part of the immutable configured restore set.
    #[error("operation restore domain is not configured")]
    DomainNotConfigured,
    /// Configured immutable bootstrap identity is malformed or mismatched.
    #[error("operation restore bootstrap is invalid")]
    InvalidBootstrap,
    /// Explicit bootstrap provisioning has not completed.
    #[error("operation restore bootstrap is missing")]
    MissingBootstrap,
    /// One immutable provisioned operation shard is missing.
    #[error("operation restore shard is missing")]
    MissingOperationShard,
    /// An operation shard is malformed or bound to another immutable layout.
    #[error("operation restore shard is invalid")]
    InvalidOperationShard,
    /// The exact mapped shard has no capacity after bounded collection.
    #[error("operation restore shard capacity is exhausted")]
    OperationCapacityExceeded,
    /// Bounded healthy CAS contention prevented reservation or terminalization.
    #[error("operation restore shard is contended")]
    OperationContention,
    /// The fixed detached-session capacity is currently saturated.
    #[error("operation restore fence capacity is saturated")]
    OperationFenceSaturated,
    /// The exact operation advisory lock was not acquired within its bound.
    #[error("operation restore fence is contended")]
    OperationFenceTimeout,
    /// A causal predecessor did not become externally visible within the
    /// bounded non-sticky reconciliation window.
    #[error("operation restore causal predecessor is pending")]
    CausalLag,
    /// The exact operation key is bound to different immutable identity.
    #[error("operation restore identity conflicts")]
    OperationIdentityConflict,
    /// Another live owner holds the operation intent.
    #[error("operation restore is already in progress")]
    OperationInProgress,
    /// The owner token does not match the pending operation.
    #[error("operation restore owner does not match")]
    OwnerMismatch,
    /// The operation was explicitly aborted.
    #[error("operation restore was aborted")]
    OperationAborted,
    /// The operation was already committed and cannot be aborted.
    #[error("operation restore is already committed")]
    AlreadyCommitted,
    /// No external intent exists for reconciliation.
    #[error("operation restore intent is missing")]
    MissingIntent,
    /// PostgreSQL has no exact operation-bound manifest.
    #[error("operation restore attribution is missing")]
    MissingAttribution,
    /// Manifest or external record data is malformed/incomplete.
    #[error("operation restore attribution is invalid")]
    InvalidAttribution,
    /// An existing component record is bound to another exact coordinate.
    #[error("operation restore component key conflicts")]
    WrongComponentKey,
    /// A nonzero first advance lacks a provisioned baseline.
    #[error("operation restore component floor is missing")]
    MissingFloor,
    /// External state does not contain the manifest's exact before value.
    #[error("operation restore component floor is stale")]
    StaleFloor,
    /// PostgreSQL is behind a previously witnessed external floor.
    #[error("operation restore detected a backward component floor")]
    BackwardFloor,
    /// Independent PostgreSQL witness reads failed.
    #[error("operation restore database is unavailable")]
    DatabaseUnavailable,
    /// External CAS/read failed or exhausted its bounded retries.
    #[error("operation restore store is unavailable")]
    StoreUnavailable,
    /// Sticky health was previously lost.
    #[error("operation restore runtime is unhealthy")]
    Unhealthy,
}

impl OperationRestoreError {
    /// Ordinary bounded contention/capacity outcomes do not poison runtime
    /// health and may be retried with a fresh server operation.
    pub const fn is_healthy_retryable(&self) -> bool {
        matches!(
            self,
            Self::OperationCapacityExceeded
                | Self::OperationContention
                | Self::OperationFenceSaturated
                | Self::OperationFenceTimeout
                | Self::OperationInProgress
                | Self::CausalLag
        )
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{atomic::AtomicU64, Mutex},
    };

    use super::*;
    use sqlx::postgres::PgPoolOptions;

    #[derive(Default)]
    struct MemoryStore {
        records: Mutex<HashMap<String, (u64, Vec<u8>)>>,
        fail: AtomicBool,
        forced_cas_losses: AtomicU64,
        floor_barrier: Option<Arc<tokio::sync::Barrier>>,
    }

    #[async_trait]
    impl RestoreStore for MemoryStore {
        async fn load(&self, key: &str) -> Result<Option<StoredObject>, ()> {
            if self.fail.load(Ordering::Acquire) {
                return Err(());
            }
            Ok(self
                .records
                .lock()
                .expect("records")
                .get(key)
                .map(|(version, body)| StoredObject {
                    etag: version.to_string(),
                    body: body.clone(),
                }))
        }

        async fn list(&self, prefix: &str) -> Result<Vec<String>, ()> {
            if self.fail.load(Ordering::Acquire) {
                return Err(());
            }
            Ok(self
                .records
                .lock()
                .expect("records")
                .keys()
                .filter(|key| key.starts_with(prefix))
                .cloned()
                .collect())
        }

        async fn compare_and_swap(
            &self,
            key: &str,
            expected_etag: Option<&str>,
            body: &[u8],
        ) -> Result<StoreCas, ()> {
            if self.fail.load(Ordering::Acquire) {
                return Err(());
            }
            if self
                .forced_cas_losses
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Ok(StoreCas::Lost);
            }
            if key.contains("/floors/") {
                if let Some(barrier) = &self.floor_barrier {
                    barrier.wait().await;
                }
            }
            let mut records = self.records.lock().expect("records");
            let current = records.get(key).map(|(version, _)| version.to_string());
            if current.as_deref() != expected_etag {
                return Ok(StoreCas::Lost);
            }
            let next = records.get(key).map_or(1, |(version, _)| version + 1);
            records.insert(key.to_owned(), (next, body.to_vec()));
            Ok(StoreCas::Won)
        }
    }

    struct MemorySource {
        manifests: Mutex<HashMap<(Uuid, [u8; 32]), OperationManifest>>,
        floors: Mutex<Vec<AuthorizationVersionComponentFloor>>,
        authoritative_now_ms: AtomicU64,
        valid_supersession: AtomicBool,
    }

    #[async_trait]
    impl ManifestSource for MemorySource {
        async fn authoritative_now_ms(&self) -> Result<u64, ManifestReadError> {
            Ok(self.authoritative_now_ms.load(Ordering::Acquire))
        }

        async fn manifest(
            &self,
            identity: OperationIdentity,
        ) -> Result<OperationManifest, ManifestReadError> {
            self.manifests
                .lock()
                .expect("manifests")
                .get(&(identity.operation_id, identity.request_fingerprint))
                .cloned()
                .ok_or(ManifestReadError::Missing)
        }

        async fn floors(
            &self,
            _domain: CommunityId,
        ) -> Result<Vec<AuthorizationVersionComponentFloor>, ManifestReadError> {
            Ok(self.floors.lock().expect("floors").clone())
        }

        async fn predecessors(
            &self,
            manifest: &OperationManifest,
        ) -> Result<Vec<OperationManifest>, ManifestReadError> {
            let manifests = self.manifests.lock().expect("manifests");
            let mut predecessors = BTreeMap::new();
            for candidate in manifests.values() {
                if candidate.identity == manifest.identity {
                    continue;
                }
                if candidate.components.iter().any(|candidate_component| {
                    manifest.components.iter().any(|component| {
                        candidate_component.kind == component.kind
                            && candidate_component.key == component.key
                            && candidate_component.after == component.before
                    })
                }) {
                    predecessors.insert(
                        (
                            candidate.identity.operation_id,
                            candidate.identity.request_fingerprint,
                        ),
                        candidate.clone(),
                    );
                }
            }
            Ok(predecessors.into_values().collect())
        }

        async fn prove_supersession(
            &self,
            _pending: &OperationManifest,
            _component: &ManifestComponent,
            _floor: &FloorRecord,
        ) -> Result<(), ManifestReadError> {
            if self.valid_supersession.load(Ordering::Acquire) {
                Ok(())
            } else {
                Err(ManifestReadError::Invalid)
            }
        }
    }

    fn test_retention() -> OperationRestoreRetentionPolicy {
        OperationRestoreRetentionPolicy::new(4, 8, MIN_TERMINAL_RETENTION)
            .expect("valid test retention")
    }

    fn runtime(
        source: Arc<MemorySource>,
        store: Arc<MemoryStore>,
        domain: CommunityId,
    ) -> OperationRestoreRuntime {
        runtime_with_lease(source, store, domain, Duration::from_secs(30))
    }

    fn runtime_with_lease(
        source: Arc<MemorySource>,
        store: Arc<MemoryStore>,
        domain: CommunityId,
        lease: Duration,
    ) -> OperationRestoreRuntime {
        runtime_with_policy(source, store, domain, lease, test_retention())
    }

    fn runtime_with_policy(
        source: Arc<MemorySource>,
        store: Arc<MemoryStore>,
        domain: CommunityId,
        lease: Duration,
        retention: OperationRestoreRetentionPolicy,
    ) -> OperationRestoreRuntime {
        let bootstrap_id = Uuid::new_v4();
        {
            let mut records = store.records.lock().expect("records");
            for shard_id in 0..retention.shard_count {
                records.insert(
                    operation_shard_key(domain, shard_id),
                    (
                        1,
                        serde_json::to_vec(&OperationShardRecord::empty(
                            domain,
                            bootstrap_id,
                            shard_id,
                            retention,
                        ))
                        .expect("encode shard"),
                    ),
                );
            }
        }
        let ready_domains = Arc::new(DashSet::new());
        ready_domains.insert(domain);
        OperationRestoreRuntime(Arc::new(RestoreInner {
            writer_db: test_db(),
            source,
            store,
            bootstrap_ids: Arc::new(HashMap::from([(domain, bootstrap_id)])),
            ready_domains,
            lease,
            retention,
            fence_pool: test_fence_pool(),
            fence_slots: Arc::new(Semaphore::new(1)),
            test_fences: Some(Arc::new(DashSet::new())),
            healthy: AtomicBool::new(true),
        }))
    }

    fn unready_runtime(
        source: Arc<MemorySource>,
        store: Arc<MemoryStore>,
        domain: CommunityId,
        bootstrap_id: Uuid,
    ) -> OperationRestoreRuntime {
        unready_runtime_with_policy(source, store, domain, bootstrap_id, test_retention())
    }

    fn unready_runtime_with_policy(
        source: Arc<MemorySource>,
        store: Arc<MemoryStore>,
        domain: CommunityId,
        bootstrap_id: Uuid,
        retention: OperationRestoreRetentionPolicy,
    ) -> OperationRestoreRuntime {
        OperationRestoreRuntime(Arc::new(RestoreInner {
            writer_db: test_db(),
            source,
            store,
            bootstrap_ids: Arc::new(HashMap::from([(domain, bootstrap_id)])),
            ready_domains: Arc::new(DashSet::new()),
            lease: Duration::from_secs(30),
            retention,
            fence_pool: test_fence_pool(),
            fence_slots: Arc::new(Semaphore::new(1)),
            test_fences: Some(Arc::new(DashSet::new())),
            healthy: AtomicBool::new(true),
        }))
    }

    fn test_fence_pool() -> PgPool {
        PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgresql://127.0.0.1:1/unreachable")
            .expect("test fence URL")
    }

    fn test_db() -> Db {
        Db::from_pool(test_fence_pool())
    }

    fn manifest(
        domain: CommunityId,
        operation_id: Uuid,
        request: [u8; 32],
        key: [u8; 32],
        before: u64,
        after: u64,
    ) -> OperationManifest {
        manifest_for_kind(
            domain,
            operation_id,
            request,
            AuthorizationVersionComponentKind::Binding,
            key,
            before,
            after,
        )
    }

    fn manifest_for_kind(
        domain: CommunityId,
        operation_id: Uuid,
        request: [u8; 32],
        kind: AuthorizationVersionComponentKind,
        key: [u8; 32],
        before: u64,
        after: u64,
    ) -> OperationManifest {
        OperationManifest {
            identity: OperationIdentity {
                domain,
                operation_id,
                request_fingerprint: request,
            },
            manifest_digest: [8; 32],
            components: vec![ManifestComponent {
                kind,
                key,
                before,
                after,
                digest: [9; 32],
            }],
        }
    }

    fn fixture() -> (
        OperationRestoreRuntime,
        Arc<MemorySource>,
        Arc<MemoryStore>,
        CommunityId,
    ) {
        let source = Arc::new(MemorySource {
            manifests: Mutex::new(HashMap::new()),
            floors: Mutex::new(Vec::new()),
            authoritative_now_ms: AtomicU64::new(1_000_000),
            valid_supersession: AtomicBool::new(false),
        });
        let store = Arc::new(MemoryStore::default());
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        (
            runtime(source.clone(), store.clone(), domain),
            source,
            store,
            domain,
        )
    }

    #[tokio::test]
    async fn exact_replay_is_operation_bound_and_wrong_request_conflicts() {
        let (runtime, source, _, domain) = fixture();
        let operation = Uuid::new_v4();
        let request = [1; 32];
        source.manifests.lock().expect("manifests").insert(
            (operation, request),
            manifest(domain, operation, request, [2; 32], 0, 1),
        );
        let OperationRestoreBegin::Acquired(intent) = runtime
            .begin(domain, operation, request)
            .await
            .expect("begin")
        else {
            panic!("first begin acquired");
        };
        assert!(!runtime.commit(intent).await.expect("commit").replay);
        assert!(matches!(
            runtime
                .begin(domain, operation, request)
                .await
                .expect("replay"),
            OperationRestoreBegin::ExactReplay(_)
        ));
        assert!(matches!(
            runtime.begin(domain, operation, [3; 32]).await,
            Err(OperationRestoreError::OperationIdentityConflict)
        ));
    }

    #[tokio::test]
    async fn startup_never_synthesizes_a_missing_or_mismatched_bootstrap() {
        let source = Arc::new(MemorySource {
            manifests: Mutex::new(HashMap::new()),
            floors: Mutex::new(Vec::new()),
            authoritative_now_ms: AtomicU64::new(1_000_000),
            valid_supersession: AtomicBool::new(false),
        });
        let store = Arc::new(MemoryStore::default());
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let missing = unready_runtime(source.clone(), store.clone(), domain, Uuid::new_v4());
        assert_eq!(
            missing.verify_domain(domain).await,
            Err(OperationRestoreError::MissingBootstrap)
        );
        assert!(!missing.is_healthy());

        let bootstrap_id = Uuid::new_v4();
        let provisioner = unready_runtime(source.clone(), store.clone(), domain, bootstrap_id);
        provisioner
            .provision_domain(domain)
            .await
            .expect("explicit bootstrap provisioning");
        let mismatched = unready_runtime(source, store, domain, Uuid::new_v4());
        assert_eq!(
            mismatched.verify_domain(domain).await,
            Err(OperationRestoreError::InvalidBootstrap)
        );
        assert!(!mismatched.is_healthy());
    }

    #[tokio::test]
    async fn startup_detects_a_component_removed_by_database_restore() {
        let source = Arc::new(MemorySource {
            manifests: Mutex::new(HashMap::new()),
            floors: Mutex::new(vec![AuthorizationVersionComponentFloor {
                component_kind: AuthorizationVersionComponentKind::Binding,
                component_key: [4; 32],
                version: 7,
            }]),
            authoritative_now_ms: AtomicU64::new(1_000_000),
            valid_supersession: AtomicBool::new(false),
        });
        let store = Arc::new(MemoryStore::default());
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let bootstrap_id = Uuid::new_v4();
        let provisioner = unready_runtime(source.clone(), store.clone(), domain, bootstrap_id);
        provisioner
            .provision_domain(domain)
            .await
            .expect("provision exact component");
        source.floors.lock().expect("floors").clear();
        let verifier = unready_runtime(source, store, domain, bootstrap_id);
        assert_eq!(
            verifier.verify_domain(domain).await,
            Err(OperationRestoreError::BackwardFloor)
        );
        assert!(!verifier.is_healthy());
    }

    #[tokio::test]
    async fn startup_reconciles_pending_postgres_commit_before_floor_comparison() {
        let source = Arc::new(MemorySource {
            manifests: Mutex::new(HashMap::new()),
            floors: Mutex::new(Vec::new()),
            authoritative_now_ms: AtomicU64::new(1_000_000),
            valid_supersession: AtomicBool::new(false),
        });
        let store = Arc::new(MemoryStore::default());
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let bootstrap_id = Uuid::new_v4();
        let original = unready_runtime(source.clone(), store.clone(), domain, bootstrap_id);
        original
            .provision_domain(domain)
            .await
            .expect("provision initial absent baseline");
        let operation = Uuid::new_v4();
        let request = [1; 32];
        let component_key = [2; 32];
        let OperationRestoreBegin::Acquired(_lost_owner) = original
            .begin(domain, operation, request)
            .await
            .expect("persist pending owner before writer transaction")
        else {
            panic!("acquired");
        };

        // Model a durable PostgreSQL commit followed by process death before
        // restore.commit(): the exact manifest and current DB floor advanced,
        // while the external intent is still Pending and its floor is absent.
        source.manifests.lock().expect("manifests").insert(
            (operation, request),
            manifest(domain, operation, request, component_key, 0, 1),
        );
        *source.floors.lock().expect("floors") = vec![AuthorizationVersionComponentFloor {
            component_kind: AuthorizationVersionComponentKind::Binding,
            component_key,
            version: 1,
        }];

        let restarted = unready_runtime(source, store, domain, bootstrap_id);
        restarted
            .verify_domain(domain)
            .await
            .expect("restart reconciles exact pending manifest first");
        assert!(matches!(
            restarted.begin(domain, operation, request).await,
            Ok(OperationRestoreBegin::ExactReplay(OperationRestoreCommit {
                replay: true,
                ..
            }))
        ));
        assert!(restarted.is_healthy());
    }

    #[tokio::test]
    async fn startup_preserves_live_missing_owner_and_aborts_expired_missing_owner() {
        let source = Arc::new(MemorySource {
            manifests: Mutex::new(HashMap::new()),
            floors: Mutex::new(Vec::new()),
            authoritative_now_ms: AtomicU64::new(1_000_000),
            valid_supersession: AtomicBool::new(false),
        });
        let store = Arc::new(MemoryStore::default());
        let live_domain = CommunityId::from_uuid(Uuid::new_v4());
        let live_bootstrap = Uuid::new_v4();
        let live = unready_runtime(source.clone(), store.clone(), live_domain, live_bootstrap);
        live.provision_domain(live_domain)
            .await
            .expect("provision live domain");
        live.begin(live_domain, Uuid::new_v4(), [1; 32])
            .await
            .expect("acquire live pending owner");
        let live_restart =
            unready_runtime(source.clone(), store.clone(), live_domain, live_bootstrap);
        assert_eq!(
            live_restart.verify_domain(live_domain).await,
            Err(OperationRestoreError::OperationInProgress)
        );
        assert!(live_restart.is_healthy());

        let expired_domain = CommunityId::from_uuid(Uuid::new_v4());
        let expired_bootstrap = Uuid::new_v4();
        let expired_ready = Arc::new(DashSet::new());
        let expired = OperationRestoreRuntime(Arc::new(RestoreInner {
            writer_db: test_db(),
            source: source.clone(),
            store: store.clone(),
            bootstrap_ids: Arc::new(HashMap::from([(expired_domain, expired_bootstrap)])),
            ready_domains: expired_ready,
            lease: Duration::from_millis(1),
            retention: test_retention(),
            fence_pool: test_fence_pool(),
            fence_slots: Arc::new(Semaphore::new(1)),
            test_fences: Some(Arc::new(DashSet::new())),
            healthy: AtomicBool::new(true),
        }));
        expired
            .provision_domain(expired_domain)
            .await
            .expect("provision expired domain");
        let operation = Uuid::new_v4();
        let request = [2; 32];
        expired
            .begin(expired_domain, operation, request)
            .await
            .expect("acquire expiring owner");
        source
            .authoritative_now_ms
            .store(1_000_002, Ordering::Release);
        let expired_restart = OperationRestoreRuntime(Arc::new(RestoreInner {
            writer_db: test_db(),
            source,
            store,
            bootstrap_ids: Arc::new(HashMap::from([(expired_domain, expired_bootstrap)])),
            ready_domains: Arc::new(DashSet::new()),
            lease: Duration::from_millis(1),
            retention: test_retention(),
            fence_pool: test_fence_pool(),
            fence_slots: Arc::new(Semaphore::new(1)),
            test_fences: Some(Arc::new(DashSet::new())),
            healthy: AtomicBool::new(true),
        }));
        expired_restart
            .verify_domain(expired_domain)
            .await
            .expect("expired missing attribution is consumed as abort");
        assert_eq!(
            expired_restart
                .reconcile(expired_domain, operation, request)
                .await,
            Err(OperationRestoreError::OperationAborted)
        );
        assert!(expired_restart.is_healthy());
    }

    #[tokio::test]
    async fn startup_rejects_malformed_operation_records_and_latches_health() {
        let source = Arc::new(MemorySource {
            manifests: Mutex::new(HashMap::new()),
            floors: Mutex::new(Vec::new()),
            authoritative_now_ms: AtomicU64::new(1_000_000),
            valid_supersession: AtomicBool::new(false),
        });
        let store = Arc::new(MemoryStore::default());
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let bootstrap_id = Uuid::new_v4();
        let provisioner = unready_runtime(source.clone(), store.clone(), domain, bootstrap_id);
        provisioner
            .provision_domain(domain)
            .await
            .expect("provision bootstrap");
        store.records.lock().expect("records").insert(
            operation_shard_key(domain, 0),
            (1, br#"{"unexpected":"record"}"#.to_vec()),
        );
        let restarted = unready_runtime(source, store, domain, bootstrap_id);
        assert_eq!(
            restarted.verify_domain(domain).await,
            Err(OperationRestoreError::InvalidAttribution)
        );
        assert!(!restarted.is_healthy());
    }

    #[tokio::test]
    async fn immutable_shards_reject_missing_remapped_and_oversized_state() {
        let source = Arc::new(MemorySource {
            manifests: Mutex::new(HashMap::new()),
            floors: Mutex::new(Vec::new()),
            authoritative_now_ms: AtomicU64::new(1_000_000),
            valid_supersession: AtomicBool::new(false),
        });
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let bootstrap_id = Uuid::new_v4();

        let missing_store = Arc::new(MemoryStore::default());
        let provisioner =
            unready_runtime(source.clone(), missing_store.clone(), domain, bootstrap_id);
        provisioner
            .provision_domain(domain)
            .await
            .expect("provision every immutable shard");
        missing_store
            .records
            .lock()
            .expect("records")
            .remove(&operation_shard_key(domain, 2));
        let missing = unready_runtime(source.clone(), missing_store, domain, bootstrap_id);
        assert_eq!(
            missing.verify_domain(domain).await,
            Err(OperationRestoreError::MissingOperationShard)
        );
        assert!(!missing.is_healthy());

        let remap_store = Arc::new(MemoryStore::default());
        let original = unready_runtime(source.clone(), remap_store.clone(), domain, bootstrap_id);
        original
            .provision_domain(domain)
            .await
            .expect("provision original layout");
        let remapped_policy = OperationRestoreRetentionPolicy::new(8, 8, MIN_TERMINAL_RETENTION)
            .expect("valid alternative layout");
        let remapped = unready_runtime_with_policy(
            source.clone(),
            remap_store,
            domain,
            bootstrap_id,
            remapped_policy,
        );
        assert_eq!(
            remapped.verify_domain(domain).await,
            Err(OperationRestoreError::InvalidBootstrap)
        );
        assert!(!remapped.is_healthy());

        let oversized_store = Arc::new(MemoryStore::default());
        let oversized_provisioner = unready_runtime(
            source.clone(),
            oversized_store.clone(),
            domain,
            bootstrap_id,
        );
        oversized_provisioner
            .provision_domain(domain)
            .await
            .expect("provision bounded records");
        oversized_store.records.lock().expect("records").insert(
            operation_shard_key(domain, 0),
            (
                1,
                vec![b'x'; usize::try_from(MAX_RECORD_BYTES + 1).expect("bounded")],
            ),
        );
        let oversized = unready_runtime(source, oversized_store, domain, bootstrap_id);
        assert_eq!(
            oversized.verify_domain(domain).await,
            Err(OperationRestoreError::StoreUnavailable)
        );
        assert!(!oversized.is_healthy());
    }

    #[tokio::test]
    async fn hot_shard_capacity_and_cas_contention_are_healthy_pre_db_denials() {
        let source = Arc::new(MemorySource {
            manifests: Mutex::new(HashMap::new()),
            floors: Mutex::new(Vec::new()),
            authoritative_now_ms: AtomicU64::new(1_000_000),
            valid_supersession: AtomicBool::new(false),
        });
        let policy = OperationRestoreRetentionPolicy::new(1, 1, MIN_TERMINAL_RETENTION)
            .expect("one-slot policy");
        let store = Arc::new(MemoryStore::default());
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let runtime = runtime_with_policy(
            source.clone(),
            store.clone(),
            domain,
            Duration::from_secs(30),
            policy,
        );
        let first_operation = Uuid::new_v4();
        let OperationRestoreBegin::Acquired(first) = runtime
            .begin(domain, first_operation, [1; 32])
            .await
            .expect("reserve only slot")
        else {
            panic!("acquired")
        };
        assert!(matches!(
            runtime.begin(domain, Uuid::new_v4(), [2; 32]).await,
            Err(OperationRestoreError::OperationCapacityExceeded)
        ));
        assert!(runtime.is_healthy());
        assert!(source.manifests.lock().expect("manifests").is_empty());
        assert!(store
            .records
            .lock()
            .expect("records")
            .keys()
            .all(|key| !key.contains("/floors/")));

        runtime.abort(first).await.expect("terminalize first slot");
        assert!(matches!(
            runtime.begin(domain, Uuid::new_v4(), [3; 32]).await,
            Err(OperationRestoreError::OperationCapacityExceeded)
        ));
        source.authoritative_now_ms.store(
            1_000_000 + policy.retention_millis().expect("retention"),
            Ordering::Release,
        );
        store.forced_cas_losses.store(1, Ordering::Release);
        assert!(matches!(
            runtime
                .begin(domain, Uuid::new_v4(), [4; 32])
                .await
                .expect("reload after lost compaction CAS"),
            OperationRestoreBegin::Acquired(_)
        ));
        assert!(runtime.is_healthy());

        let contention_store = Arc::new(MemoryStore::default());
        let contention_domain = CommunityId::from_uuid(Uuid::new_v4());
        let contention = runtime_with_policy(
            source,
            contention_store.clone(),
            contention_domain,
            Duration::from_secs(30),
            policy,
        );
        contention_store
            .forced_cas_losses
            .store(MAX_CAS_ATTEMPTS as u64, Ordering::Release);
        assert!(matches!(
            contention
                .begin(contention_domain, Uuid::new_v4(), [5; 32])
                .await,
            Err(OperationRestoreError::OperationContention)
        ));
        assert!(contention.is_healthy());

        let concurrent_source = Arc::new(MemorySource {
            manifests: Mutex::new(HashMap::new()),
            floors: Mutex::new(Vec::new()),
            authoritative_now_ms: AtomicU64::new(2_000_000),
            valid_supersession: AtomicBool::new(false),
        });
        let concurrent_store = Arc::new(MemoryStore::default());
        let concurrent_domain = CommunityId::from_uuid(Uuid::new_v4());
        let two_shards = OperationRestoreRetentionPolicy::new(2, 1, MIN_TERMINAL_RETENTION)
            .expect("two one-slot shards");
        let concurrent = runtime_with_policy(
            concurrent_source,
            concurrent_store,
            concurrent_domain,
            Duration::from_secs(30),
            two_shards,
        );
        let bootstrap = concurrent
            .bootstrap_id(concurrent_domain)
            .expect("bootstrap");
        let first = Uuid::new_v4();
        let first_shard = operation_shard_id(
            OperationIdentity {
                domain: concurrent_domain,
                operation_id: first,
                request_fingerprint: [6; 32],
            },
            bootstrap,
            two_shards.shard_count,
        );
        let (same_shard, free_shard) = loop {
            let same = Uuid::new_v4();
            let free = Uuid::new_v4();
            let same_id = OperationIdentity {
                domain: concurrent_domain,
                operation_id: same,
                request_fingerprint: [7; 32],
            };
            let free_id = OperationIdentity {
                domain: concurrent_domain,
                operation_id: free,
                request_fingerprint: [8; 32],
            };
            if operation_shard_id(same_id, bootstrap, two_shards.shard_count) == first_shard
                && operation_shard_id(free_id, bootstrap, two_shards.shard_count) != first_shard
            {
                break (same, free);
            }
        };
        let first_begin = concurrent.begin(concurrent_domain, first, [6; 32]);
        let same_begin = concurrent.begin(concurrent_domain, same_shard, [7; 32]);
        let (first_result, same_result) = tokio::join!(first_begin, same_begin);
        assert!(matches!(
            (&first_result, &same_result),
            (
                Ok(OperationRestoreBegin::Acquired(_)),
                Err(OperationRestoreError::OperationCapacityExceeded)
            ) | (
                Err(OperationRestoreError::OperationCapacityExceeded),
                Ok(OperationRestoreBegin::Acquired(_))
            )
        ));
        assert!(matches!(
            concurrent
                .begin(concurrent_domain, free_shard, [8; 32])
                .await
                .expect("independently mapped free shard remains available"),
            OperationRestoreBegin::Acquired(_)
        ));
        assert!(concurrent.is_healthy());
    }

    #[tokio::test]
    async fn startup_resolves_reverse_shard_multistep_chain_without_supersession_guessing() {
        let source = Arc::new(MemorySource {
            manifests: Mutex::new(HashMap::new()),
            floors: Mutex::new(Vec::new()),
            authoritative_now_ms: AtomicU64::new(1_000_000),
            valid_supersession: AtomicBool::new(true),
        });
        let store = Arc::new(MemoryStore::default());
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let bootstrap_id = Uuid::new_v4();
        let original = unready_runtime(source.clone(), store.clone(), domain, bootstrap_id);
        original
            .provision_domain(domain)
            .await
            .expect("provision domain");

        let mut by_shard = BTreeMap::new();
        while by_shard.len() < 3 {
            let operation = Uuid::new_v4();
            let identity = OperationIdentity {
                domain,
                operation_id: operation,
                request_fingerprint: [u8::try_from(by_shard.len() + 1).expect("small"); 32],
            };
            by_shard
                .entry(operation_shard_id(
                    identity,
                    bootstrap_id,
                    test_retention().shard_count,
                ))
                .or_insert(operation);
        }
        let operations: Vec<_> = by_shard.into_values().rev().collect();
        let key = [44; 32];
        for (index, operation) in operations.iter().copied().enumerate() {
            let request = [u8::try_from(index + 1).expect("small"); 32];
            assert!(matches!(
                original
                    .begin(domain, operation, request)
                    .await
                    .expect("reserve pending chain member"),
                OperationRestoreBegin::Acquired(_)
            ));
            source.manifests.lock().expect("manifests").insert(
                (operation, request),
                manifest(
                    domain,
                    operation,
                    request,
                    key,
                    index as u64,
                    index as u64 + 1,
                ),
            );
        }
        *source.floors.lock().expect("floors") = vec![AuthorizationVersionComponentFloor {
            component_kind: AuthorizationVersionComponentKind::Binding,
            component_key: key,
            version: 3,
        }];
        let restarted = unready_runtime(source, store, domain, bootstrap_id);
        restarted
            .verify_domain(domain)
            .await
            .expect("causal passes reconcile reverse shard scan order");
        for (index, operation) in operations.iter().copied().enumerate() {
            let request = [u8::try_from(index + 1).expect("small"); 32];
            assert!(matches!(
                restarted.begin(domain, operation, request).await,
                Ok(OperationRestoreBegin::ExactReplay(_))
            ));
        }
    }

    #[tokio::test]
    async fn startup_resolves_reverse_same_shard_predecessor_order() {
        let source = Arc::new(MemorySource {
            manifests: Mutex::new(HashMap::new()),
            floors: Mutex::new(Vec::new()),
            authoritative_now_ms: AtomicU64::new(1_000_000),
            valid_supersession: AtomicBool::new(true),
        });
        let store = Arc::new(MemoryStore::default());
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let bootstrap = Uuid::new_v4();
        let policy = OperationRestoreRetentionPolicy::new(1, 8, MIN_TERMINAL_RETENTION)
            .expect("single shard policy");
        let original =
            unready_runtime_with_policy(source.clone(), store.clone(), domain, bootstrap, policy);
        original.provision_domain(domain).await.expect("provision");
        let successor = Uuid::from_u128(1);
        let predecessor = Uuid::from_u128(2);
        let key = [61_u8; 32];
        original
            .begin(domain, successor, [1_u8; 32])
            .await
            .expect("reserve lexically first successor");
        original
            .begin(domain, predecessor, [2_u8; 32])
            .await
            .expect("reserve predecessor");
        source.manifests.lock().expect("manifests").extend([
            (
                (successor, [1_u8; 32]),
                manifest(domain, successor, [1_u8; 32], key, 1, 2),
            ),
            (
                (predecessor, [2_u8; 32]),
                manifest(domain, predecessor, [2_u8; 32], key, 0, 1),
            ),
        ]);
        *source.floors.lock().expect("floors") = vec![AuthorizationVersionComponentFloor {
            component_kind: AuthorizationVersionComponentKind::Binding,
            component_key: key,
            version: 2,
        }];
        let restarted = unready_runtime_with_policy(source, store, domain, bootstrap, policy);
        restarted
            .verify_domain(domain)
            .await
            .expect("fixed-point scan finds predecessor after successor");
        assert!(restarted.is_healthy());
    }

    #[tokio::test]
    async fn begin_recovers_a_three_operation_chain_across_three_shards() {
        let source = Arc::new(MemorySource {
            manifests: Mutex::new(HashMap::new()),
            floors: Mutex::new(Vec::new()),
            authoritative_now_ms: AtomicU64::new(1_000_000),
            valid_supersession: AtomicBool::new(true),
        });
        let store = Arc::new(MemoryStore::default());
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let policy = OperationRestoreRetentionPolicy::new(8, 8, MIN_TERMINAL_RETENTION)
            .expect("multi-shard policy");
        let runtime = runtime_with_policy(
            source.clone(),
            store.clone(),
            domain,
            Duration::from_millis(1),
            policy,
        );
        let bootstrap = runtime.bootstrap_id(domain).expect("bootstrap");
        let request_a = [66_u8; 32];
        let request_b = [67_u8; 32];
        let request_c = [68_u8; 32];
        let operation_a = Uuid::from_u128(1);
        let shard_a = operation_shard_id(
            OperationIdentity {
                domain,
                operation_id: operation_a,
                request_fingerprint: request_a,
            },
            bootstrap,
            policy.shard_count,
        );
        let mut nonce = 2_u128;
        let operation_b = loop {
            let candidate = Uuid::from_u128(nonce);
            nonce += 1;
            let shard = operation_shard_id(
                OperationIdentity {
                    domain,
                    operation_id: candidate,
                    request_fingerprint: request_b,
                },
                bootstrap,
                policy.shard_count,
            );
            if shard != shard_a {
                break candidate;
            }
        };
        let shard_b = operation_shard_id(
            OperationIdentity {
                domain,
                operation_id: operation_b,
                request_fingerprint: request_b,
            },
            bootstrap,
            policy.shard_count,
        );
        let operation_c = loop {
            let candidate = Uuid::from_u128(nonce);
            nonce += 1;
            let shard = operation_shard_id(
                OperationIdentity {
                    domain,
                    operation_id: candidate,
                    request_fingerprint: request_c,
                },
                bootstrap,
                policy.shard_count,
            );
            if shard != shard_a && shard != shard_b {
                break candidate;
            }
        };
        let shard_c = operation_shard_id(
            OperationIdentity {
                domain,
                operation_id: operation_c,
                request_fingerprint: request_c,
            },
            bootstrap,
            policy.shard_count,
        );
        runtime
            .begin(domain, operation_a, request_a)
            .await
            .expect("reserve predecessor");
        runtime
            .begin(domain, operation_b, request_b)
            .await
            .expect("reserve successor");
        runtime
            .begin(domain, operation_c, request_c)
            .await
            .expect("reserve final successor");
        let key = [69_u8; 32];
        source.manifests.lock().expect("manifests").extend([
            (
                (operation_a, request_a),
                manifest(domain, operation_a, request_a, key, 0, 1),
            ),
            (
                (operation_b, request_b),
                manifest(domain, operation_b, request_b, key, 1, 2),
            ),
            (
                (operation_c, request_c),
                manifest(domain, operation_c, request_c, key, 2, 3),
            ),
        ]);
        source
            .authoritative_now_ms
            .store(1_000_002, Ordering::Release);

        let successor = runtime.begin(domain, operation_c, request_c).await;
        assert!(
            matches!(successor, Ok(OperationRestoreBegin::ExactReplay(_))),
            "unexpected successor result: {successor:?}"
        );
        assert!(matches!(
            runtime.begin(domain, operation_b, request_b).await,
            Ok(OperationRestoreBegin::ExactReplay(_))
        ));
        assert!(matches!(
            runtime.begin(domain, operation_a, request_a).await,
            Ok(OperationRestoreBegin::ExactReplay(_))
        ));
        let records = store.records.lock().expect("records");
        for (operation, shard) in [
            (operation_a, shard_a),
            (operation_b, shard_b),
            (operation_c, shard_c),
        ] {
            let encoded = &records
                .get(&operation_shard_key(domain, shard))
                .expect("operation shard")
                .1;
            let shard: OperationShardRecord =
                serde_json::from_slice(encoded).expect("decode operation shard");
            assert_eq!(
                shard
                    .entries
                    .get(&operation.to_string())
                    .expect("intent")
                    .state,
                IntentState::Committed
            );
        }
        let encoded = &records
            .get(&floor_key(
                domain,
                AuthorizationVersionComponentKind::Binding,
                key,
            ))
            .expect("component floor")
            .1;
        let floor: FloorRecord = serde_json::from_slice(encoded).expect("decode floor");
        assert_eq!(floor.version, 3);
        assert_eq!(floor.operation_id, Some(operation_c.to_string()));
        drop(records);
        assert!(runtime.is_healthy());
    }

    #[tokio::test]
    async fn expired_writer_fence_blocks_reaper_without_poisoning_health() {
        let (runtime, source, _, domain) = fixture();
        let identity = OperationIdentity {
            domain,
            operation_id: Uuid::new_v4(),
            request_fingerprint: [62_u8; 32],
        };
        let fenced = runtime
            .begin_fenced(identity)
            .await
            .expect("writer owns fence before Pending");
        source.authoritative_now_ms.store(
            1_000_000 + u64::try_from(DEFAULT_LEASE.as_millis()).expect("lease") + 1,
            Ordering::Release,
        );
        let shard = operation_shard_id(
            identity,
            runtime.bootstrap_id(domain).expect("bootstrap"),
            runtime.0.retention.shard_count,
        );
        let started = tokio::time::Instant::now();
        assert_eq!(
            runtime.maintain_begin_shard(domain, shard).await,
            Err(OperationRestoreError::CausalLag)
        );
        assert!(started.elapsed() >= Duration::from_millis(140));
        assert!(runtime.is_healthy());
        drop(fenced);
        let reacquired = runtime
            .acquire_operation_fence(identity)
            .await
            .expect("cancelled writer drop releases the fence");
        reacquired.unlock().await.expect("explicit test unlock");
    }

    #[tokio::test]
    async fn many_contended_fences_obey_one_shard_deadline() {
        let source = Arc::new(MemorySource {
            manifests: Mutex::new(HashMap::new()),
            floors: Mutex::new(Vec::new()),
            authoritative_now_ms: AtomicU64::new(1_000_000),
            valid_supersession: AtomicBool::new(false),
        });
        let store = Arc::new(MemoryStore::default());
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let policy = OperationRestoreRetentionPolicy::new(1, 64, MIN_TERMINAL_RETENTION)
            .expect("one full production-sized shard");
        let runtime = runtime_with_policy(
            source.clone(),
            store,
            domain,
            Duration::from_millis(1),
            policy,
        );
        let fences = runtime.0.test_fences.as_ref().expect("test fences");
        for nonce in 1..=u128::from(policy.entries_per_shard) {
            let identity = OperationIdentity {
                domain,
                operation_id: Uuid::from_u128(nonce),
                request_fingerprint: [65_u8; 32],
            };
            runtime
                .begin(domain, identity.operation_id, identity.request_fingerprint)
                .await
                .expect("reserve pending contender");
            assert!(fences.insert(operation_fence_test_key(identity)));
        }
        source
            .authoritative_now_ms
            .store(1_000_002, Ordering::Release);

        let started = tokio::time::Instant::now();
        assert_eq!(
            runtime.maintain_begin_shard(domain, 0).await,
            Err(OperationRestoreError::CausalLag)
        );
        assert!(started.elapsed() >= Duration::from_millis(120));
        assert!(started.elapsed() < Duration::from_millis(300));
        assert!(runtime.is_healthy());
    }

    #[tokio::test]
    async fn fixed_production_shard_saturation_is_healthy_and_pre_database() {
        let source = Arc::new(MemorySource {
            manifests: Mutex::new(HashMap::new()),
            floors: Mutex::new(Vec::new()),
            authoritative_now_ms: AtomicU64::new(1_000_000),
            valid_supersession: AtomicBool::new(false),
        });
        let store = Arc::new(MemoryStore::default());
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let policy = OperationRestoreRetentionPolicy::production_default();
        let runtime = runtime_with_policy(source.clone(), store, domain, DEFAULT_LEASE, policy);
        let bootstrap = runtime.bootstrap_id(domain).expect("bootstrap");
        let target_shard = 0;
        let mut filled = 0_u16;
        let mut nonce = 1_u128;
        while filled < policy.entries_per_shard {
            let operation = Uuid::from_u128(nonce);
            nonce += 1;
            let identity = OperationIdentity {
                domain,
                operation_id: operation,
                request_fingerprint: [63_u8; 32],
            };
            if operation_shard_id(identity, bootstrap, policy.shard_count) != target_shard {
                continue;
            }
            runtime
                .begin(domain, operation, identity.request_fingerprint)
                .await
                .expect("fill exact production shard");
            filled += 1;
        }
        let saturated = loop {
            let operation = Uuid::from_u128(nonce);
            nonce += 1;
            let identity = OperationIdentity {
                domain,
                operation_id: operation,
                request_fingerprint: [64_u8; 32],
            };
            if operation_shard_id(identity, bootstrap, policy.shard_count) == target_shard {
                break identity;
            }
        };
        assert!(matches!(
            runtime
                .begin(
                    domain,
                    saturated.operation_id,
                    saturated.request_fingerprint,
                )
                .await,
            Err(OperationRestoreError::OperationCapacityExceeded)
        ));
        assert!(runtime.is_healthy());
        assert!(source.manifests.lock().expect("manifests").is_empty());
    }

    #[tokio::test]
    async fn pending_operation_terminalizes_only_after_authoritative_supersession_proof() {
        let source = Arc::new(MemorySource {
            manifests: Mutex::new(HashMap::new()),
            floors: Mutex::new(Vec::new()),
            authoritative_now_ms: AtomicU64::new(1_000_000),
            valid_supersession: AtomicBool::new(true),
        });
        let store = Arc::new(MemoryStore::default());
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let bootstrap_id = Uuid::new_v4();
        let original = unready_runtime(source.clone(), store.clone(), domain, bootstrap_id);
        original
            .provision_domain(domain)
            .await
            .expect("provision domain");
        let pending_operation = Uuid::new_v4();
        let pending_request = [61; 32];
        let key = [62; 32];
        assert!(matches!(
            original
                .begin(domain, pending_operation, pending_request)
                .await
                .expect("reserve pending operation"),
            OperationRestoreBegin::Acquired(_)
        ));
        source.manifests.lock().expect("manifests").insert(
            (pending_operation, pending_request),
            manifest(domain, pending_operation, pending_request, key, 0, 1),
        );
        let latest = manifest(domain, Uuid::new_v4(), [63; 32], key, 2, 3);
        let latest_component = latest.components[0].clone();
        store.records.lock().expect("records").insert(
            floor_key(domain, AuthorizationVersionComponentKind::Binding, key),
            (
                1,
                serde_json::to_vec(&FloorRecord::advanced(
                    bootstrap_id,
                    &latest,
                    &latest_component,
                ))
                .expect("encode proven latest floor"),
            ),
        );
        *source.floors.lock().expect("floors") = vec![AuthorizationVersionComponentFloor {
            component_kind: AuthorizationVersionComponentKind::Binding,
            component_key: key,
            version: 3,
        }];

        let restarted = unready_runtime(source, store, domain, bootstrap_id);
        restarted
            .verify_domain(domain)
            .await
            .expect("complete provenance proves supersession");
        let OperationRestoreBegin::ExactReplay(replay) = restarted
            .begin(domain, pending_operation, pending_request)
            .await
            .expect("replay superseded operation")
        else {
            panic!("exact replay")
        };
        assert!(replay.causally_superseded);
    }

    #[tokio::test]
    async fn exact_replay_fails_closed_when_postgres_attribution_disappears() {
        let (runtime, source, _, domain) = fixture();
        let operation = Uuid::new_v4();
        let request = [1; 32];
        source.manifests.lock().expect("manifests").insert(
            (operation, request),
            manifest(domain, operation, request, [2; 32], 0, 1),
        );
        let OperationRestoreBegin::Acquired(intent) = runtime
            .begin(domain, operation, request)
            .await
            .expect("begin")
        else {
            panic!("acquired");
        };
        runtime.commit(intent).await.expect("commit");
        source
            .manifests
            .lock()
            .expect("manifests")
            .remove(&(operation, request));
        assert!(matches!(
            runtime.begin(domain, operation, request).await,
            Err(OperationRestoreError::MissingAttribution)
        ));
        assert!(!runtime.is_healthy());
    }

    #[tokio::test]
    async fn exact_replay_revalidates_its_external_component_floor() {
        let (runtime, source, store, domain) = fixture();
        let operation = Uuid::new_v4();
        let request = [1; 32];
        let component_key = [2; 32];
        source.manifests.lock().expect("manifests").insert(
            (operation, request),
            manifest(domain, operation, request, component_key, 0, 1),
        );
        let OperationRestoreBegin::Acquired(intent) = runtime
            .begin(domain, operation, request)
            .await
            .expect("begin")
        else {
            panic!("acquired");
        };
        runtime.commit(intent).await.expect("commit");
        store.records.lock().expect("records").remove(&floor_key(
            domain,
            AuthorizationVersionComponentKind::Binding,
            component_key,
        ));

        assert!(matches!(
            runtime.begin(domain, operation, request).await,
            Err(OperationRestoreError::MissingFloor)
        ));
        assert!(!runtime.is_healthy());
    }

    #[tokio::test]
    async fn lost_response_reconcile_commits_and_consumes_the_original_owner() {
        let (runtime, source, _, domain) = fixture();
        let operation = Uuid::new_v4();
        let request = [1; 32];
        source.manifests.lock().expect("manifests").insert(
            (operation, request),
            manifest(domain, operation, request, [2; 32], 0, 1),
        );
        let OperationRestoreBegin::Acquired(intent) = runtime
            .begin(domain, operation, request)
            .await
            .expect("begin")
        else {
            panic!("acquired");
        };
        let reconciled = runtime
            .reconcile(domain, operation, request)
            .await
            .expect("reconcile exact committed PostgreSQL operation");
        assert!(!reconciled.replay);
        assert_eq!(
            runtime.abort(intent).await,
            Err(OperationRestoreError::AlreadyCommitted)
        );
        assert!(matches!(
            runtime
                .reconcile(domain, operation, request)
                .await
                .expect("exact reconcile replay"),
            OperationRestoreCommit { replay: true, .. }
        ));
    }

    #[tokio::test]
    async fn missing_nonzero_floor_and_backward_floor_fail_closed() {
        let (runtime, source, _, domain) = fixture();
        let operation = Uuid::new_v4();
        let request = [1; 32];
        source.manifests.lock().expect("manifests").insert(
            (operation, request),
            manifest(domain, operation, request, [2; 32], 7, 8),
        );
        let OperationRestoreBegin::Acquired(intent) = runtime
            .begin(domain, operation, request)
            .await
            .expect("begin")
        else {
            panic!("acquired");
        };
        assert_eq!(
            runtime.commit(intent).await,
            Err(OperationRestoreError::MissingFloor)
        );
        assert!(!runtime.is_healthy());

        let (runtime, source, _, domain) = fixture();
        *source.floors.lock().expect("floors") = vec![AuthorizationVersionComponentFloor {
            component_kind: AuthorizationVersionComponentKind::Binding,
            component_key: [2; 32],
            version: 9,
        }];
        runtime.provision_domain(domain).await.expect("provision");
        source.floors.lock().expect("floors")[0].version = 8;
        assert_eq!(
            runtime.provision_domain(domain).await,
            Err(OperationRestoreError::BackwardFloor)
        );
        assert!(!runtime.is_healthy());
    }

    #[tokio::test]
    async fn every_component_kind_anchors_first_advance_to_absent_zero() {
        let kinds = [
            AuthorizationVersionComponentKind::Binding,
            AuthorizationVersionComponentKind::Policy,
            AuthorizationVersionComponentKind::InvalidationGeneration,
            AuthorizationVersionComponentKind::AuthorityEpoch,
            AuthorizationVersionComponentKind::DelegatedRelationship,
            AuthorizationVersionComponentKind::LifecycleSelector,
        ];
        for (index, kind) in kinds.into_iter().enumerate() {
            let (runtime, source, _, domain) = fixture();
            let operation = Uuid::new_v4();
            let mut request = [1; 32];
            request[0] = u8::try_from(index + 1).expect("bounded kind index");
            let mut key = [2; 32];
            key[0] = request[0];
            source.manifests.lock().expect("manifests").insert(
                (operation, request),
                manifest_for_kind(domain, operation, request, kind, key, 1, 2),
            );
            let OperationRestoreBegin::Acquired(intent) = runtime
                .begin(domain, operation, request)
                .await
                .expect("begin gap attempt")
            else {
                panic!("acquired");
            };
            assert_eq!(
                runtime.commit(intent).await,
                Err(OperationRestoreError::MissingFloor),
                "{kind:?} accepted an unprovisioned nonzero first floor"
            );

            let (runtime, source, _, domain) = fixture();
            let operation = Uuid::new_v4();
            source.manifests.lock().expect("manifests").insert(
                (operation, request),
                manifest_for_kind(domain, operation, request, kind, key, 0, 1),
            );
            let OperationRestoreBegin::Acquired(intent) = runtime
                .begin(domain, operation, request)
                .await
                .expect("begin exact absent baseline")
            else {
                panic!("acquired");
            };
            assert!(runtime.commit(intent).await.is_ok(), "{kind:?}");
        }
    }

    #[tokio::test]
    async fn wrong_key_record_is_rejected_and_health_is_sticky() {
        let (runtime, source, store, domain) = fixture();
        let operation = Uuid::new_v4();
        let request = [1; 32];
        let key = [2; 32];
        source.manifests.lock().expect("manifests").insert(
            (operation, request),
            manifest(domain, operation, request, key, 0, 1),
        );
        let wrong = FloorRecord {
            component_key: hex::encode([3; 32]),
            ..FloorRecord::advanced(
                runtime.bootstrap_id(domain).expect("configured bootstrap"),
                source
                    .manifests
                    .lock()
                    .expect("manifests")
                    .get(&(operation, request))
                    .expect("manifest"),
                &ManifestComponent {
                    kind: AuthorizationVersionComponentKind::Binding,
                    key,
                    before: 0,
                    after: 1,
                    digest: [9; 32],
                },
            )
        };
        store.records.lock().expect("records").insert(
            floor_key(domain, AuthorizationVersionComponentKind::Binding, key),
            (1, serde_json::to_vec(&wrong).expect("encode")),
        );
        let OperationRestoreBegin::Acquired(intent) = runtime
            .begin(domain, operation, request)
            .await
            .expect("begin")
        else {
            panic!("acquired");
        };
        assert_eq!(
            runtime.commit(intent).await,
            Err(OperationRestoreError::WrongComponentKey)
        );
        assert!(!runtime.is_healthy());
        assert!(matches!(
            runtime.begin(domain, Uuid::new_v4(), [4; 32]).await,
            Err(OperationRestoreError::Unhealthy)
        ));
    }

    #[tokio::test]
    async fn abort_is_owner_checked_and_missing_manifest_is_consumed() {
        let (runtime, _, _, domain) = fixture();
        let operation = Uuid::new_v4();
        let request = [1; 32];
        let OperationRestoreBegin::Acquired(intent) = runtime
            .begin(domain, operation, request)
            .await
            .expect("begin")
        else {
            panic!("acquired");
        };
        let wrong_owner = OperationRestoreIntent {
            identity: intent.identity,
            owner_token: Uuid::new_v4(),
            shard_id: intent.shard_id,
        };
        assert_eq!(
            runtime.abort(wrong_owner).await,
            Err(OperationRestoreError::OwnerMismatch)
        );
        assert!(!runtime.is_healthy());
        assert_eq!(
            runtime.abort(intent).await,
            Err(OperationRestoreError::Unhealthy)
        );

        let (runtime, _, _, domain) = fixture();
        let operation = Uuid::new_v4();
        let OperationRestoreBegin::Acquired(intent) = runtime
            .begin(domain, operation, request)
            .await
            .expect("begin")
        else {
            panic!("acquired");
        };
        assert_eq!(
            runtime.commit(intent).await,
            Err(OperationRestoreError::MissingAttribution)
        );
        assert!(!runtime.is_healthy());
    }

    #[tokio::test]
    async fn expired_missing_intent_is_retained_then_reacquired_after_pg_retention() {
        let source = Arc::new(MemorySource {
            manifests: Mutex::new(HashMap::new()),
            floors: Mutex::new(Vec::new()),
            authoritative_now_ms: AtomicU64::new(1_000_000),
            valid_supersession: AtomicBool::new(false),
        });
        let store = Arc::new(MemoryStore::default());
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let runtime = runtime_with_lease(source.clone(), store, domain, Duration::from_millis(1));
        let operation = Uuid::new_v4();
        let request = [1; 32];
        let OperationRestoreBegin::Acquired(first) = runtime
            .begin(domain, operation, request)
            .await
            .expect("first owner")
        else {
            panic!("acquired");
        };
        source
            .authoritative_now_ms
            .store(1_000_002, Ordering::Release);
        assert!(matches!(
            runtime.begin(domain, operation, request).await,
            Err(OperationRestoreError::OperationAborted)
        ));
        source.authoritative_now_ms.store(
            1_000_002 + test_retention().retention_millis().expect("retention"),
            Ordering::Release,
        );
        let OperationRestoreBegin::Acquired(second) = runtime
            .begin(domain, operation, request)
            .await
            .expect("replacement owner after terminal retention")
        else {
            panic!("reacquired");
        };
        assert_ne!(first.owner_token, second.owner_token);
        assert_eq!(
            runtime.abort(first).await,
            Err(OperationRestoreError::OwnerMismatch)
        );
        assert!(!runtime.is_healthy());
        assert_eq!(
            runtime.abort(second).await,
            Err(OperationRestoreError::Unhealthy)
        );
    }

    #[test]
    fn restore_debug_surfaces_are_redacted() {
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let operation_id = Uuid::new_v4();
        let owner_token = Uuid::new_v4();
        let intent = OperationRestoreIntent {
            identity: OperationIdentity {
                domain,
                operation_id,
                request_fingerprint: [7; 32],
            },
            owner_token,
            shard_id: 0,
        };
        let rendered = format!("{intent:?}");
        assert!(!rendered.contains(&domain.as_uuid().to_string()));
        assert!(!rendered.contains(&operation_id.to_string()));
        assert!(!rendered.contains(&owner_token.to_string()));
        assert!(!rendered.contains(&hex::encode([7; 32])));
    }

    #[tokio::test]
    async fn unrelated_operations_advance_independent_components_concurrently() {
        let source = Arc::new(MemorySource {
            manifests: Mutex::new(HashMap::new()),
            floors: Mutex::new(Vec::new()),
            authoritative_now_ms: AtomicU64::new(1_000_000),
            valid_supersession: AtomicBool::new(false),
        });
        let store = Arc::new(MemoryStore {
            records: Mutex::new(HashMap::new()),
            fail: AtomicBool::new(false),
            forced_cas_losses: AtomicU64::new(0),
            floor_barrier: Some(Arc::new(tokio::sync::Barrier::new(2))),
        });
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let runtime = runtime(source.clone(), store, domain);
        let first_operation = Uuid::new_v4();
        let second_operation = Uuid::new_v4();
        let first_request = [1; 32];
        let second_request = [2; 32];
        source.manifests.lock().expect("manifests").extend([
            (
                (first_operation, first_request),
                manifest(domain, first_operation, first_request, [3; 32], 0, 1),
            ),
            (
                (second_operation, second_request),
                manifest(domain, second_operation, second_request, [4; 32], 0, 1),
            ),
        ]);
        let OperationRestoreBegin::Acquired(first) = runtime
            .begin(domain, first_operation, first_request)
            .await
            .expect("first begin")
        else {
            panic!("first acquired");
        };
        let OperationRestoreBegin::Acquired(second) = runtime
            .begin(domain, second_operation, second_request)
            .await
            .expect("second begin")
        else {
            panic!("second acquired");
        };
        let (first_result, second_result) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(runtime.commit(first), runtime.commit(second))
        })
        .await
        .expect("unrelated components reach their independent CAS without a domain mutex");
        assert!(first_result.is_ok());
        assert!(second_result.is_ok());
        assert!(runtime.is_healthy());
    }

    #[tokio::test]
    async fn later_same_component_commit_waits_for_earlier_witness_floor() {
        let (runtime, source, _, domain) = fixture();
        let key = [3; 32];
        let first_operation = Uuid::new_v4();
        let second_operation = Uuid::new_v4();
        let first_request = [1; 32];
        let second_request = [2; 32];
        source.manifests.lock().expect("manifests").extend([
            (
                (first_operation, first_request),
                manifest(domain, first_operation, first_request, key, 0, 1),
            ),
            (
                (second_operation, second_request),
                manifest(domain, second_operation, second_request, key, 1, 2),
            ),
        ]);
        *source.floors.lock().expect("floors") = vec![AuthorizationVersionComponentFloor {
            component_kind: AuthorizationVersionComponentKind::Binding,
            component_key: key,
            version: 2,
        }];
        let OperationRestoreBegin::Acquired(first) = runtime
            .begin(domain, first_operation, first_request)
            .await
            .expect("first begin")
        else {
            panic!("first acquired");
        };
        let OperationRestoreBegin::Acquired(second) = runtime
            .begin(domain, second_operation, second_request)
            .await
            .expect("second begin")
        else {
            panic!("second acquired");
        };
        let (second_result, first_result) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(runtime.commit(second), async {
                tokio::time::sleep(Duration::from_millis(25)).await;
                runtime.commit(first).await
            })
        })
        .await
        .expect("bounded same-component reconciliation");
        assert!(first_result.is_ok());
        assert!(second_result.is_ok());
        assert!(runtime.is_healthy());
    }

    #[tokio::test]
    async fn stale_gap_is_not_laundered_by_a_later_operation() {
        let (runtime, source, _, domain) = fixture();
        *source.floors.lock().expect("floors") = vec![AuthorizationVersionComponentFloor {
            component_kind: AuthorizationVersionComponentKind::Binding,
            component_key: [2; 32],
            version: 7,
        }];
        runtime.provision_domain(domain).await.expect("provision");
        let operation = Uuid::new_v4();
        let request = [1; 32];
        source.manifests.lock().expect("manifests").insert(
            (operation, request),
            manifest(domain, operation, request, [2; 32], 8, 9),
        );
        let OperationRestoreBegin::Acquired(intent) = runtime
            .begin(domain, operation, request)
            .await
            .expect("begin")
        else {
            panic!("acquired");
        };
        assert_eq!(
            runtime.commit(intent).await,
            Err(OperationRestoreError::StaleFloor)
        );
        assert!(!runtime.is_healthy());
    }

    #[tokio::test]
    async fn causal_lag_past_the_bound_is_healthy_and_retains_pending() {
        let (runtime, source, _, domain) = fixture();
        let key = [27_u8; 32];
        *source.floors.lock().expect("floors") = vec![AuthorizationVersionComponentFloor {
            component_kind: AuthorizationVersionComponentKind::Binding,
            component_key: key,
            version: 7,
        }];
        runtime.provision_domain(domain).await.expect("provision");
        let operation = Uuid::new_v4();
        let request = [28_u8; 32];
        source.manifests.lock().expect("manifests").insert(
            (operation, request),
            manifest(domain, operation, request, key, 8, 9),
        );
        source.floors.lock().expect("floors")[0].version = 9;
        let OperationRestoreBegin::Acquired(intent) = runtime
            .begin(domain, operation, request)
            .await
            .expect("begin")
        else {
            panic!("acquired");
        };
        let started = tokio::time::Instant::now();
        assert_eq!(
            runtime.commit(intent).await,
            Err(OperationRestoreError::CausalLag)
        );
        assert!(started.elapsed() >= Duration::from_millis(140));
        assert!(runtime.is_healthy());
        assert!(matches!(
            runtime.begin(domain, operation, request).await,
            Err(OperationRestoreError::OperationInProgress)
        ));
    }

    #[tokio::test]
    async fn production_shard_capacity_fits_maximum_width_terminal_records() {
        let source = Arc::new(MemorySource {
            manifests: Mutex::new(HashMap::new()),
            floors: Mutex::new(Vec::new()),
            authoritative_now_ms: AtomicU64::new(1_000_000),
            valid_supersession: AtomicBool::new(false),
        });
        let store = Arc::new(MemoryStore::default());
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let runtime = runtime_with_policy(
            source,
            store,
            domain,
            DEFAULT_LEASE,
            OperationRestoreRetentionPolicy::production_default(),
        );
        let bootstrap = runtime.bootstrap_id(domain).expect("bootstrap");
        assert_eq!(runtime.0.retention.shard_count, 256);
        assert_eq!(runtime.0.retention.entries_per_shard, 64);
        assert_eq!(
            runtime.0.retention.terminal_retention,
            Duration::from_secs(24 * 60 * 60)
        );
        runtime
            .validate_worst_case_shard_size(domain, bootstrap)
            .expect("maximum-width production shard remains below 64 KiB");
    }

    #[tokio::test]
    async fn external_store_failure_is_sticky_and_fail_closed() {
        let (runtime, _, store, domain) = fixture();
        store.fail.store(true, Ordering::Release);
        assert!(matches!(
            runtime.begin(domain, Uuid::new_v4(), [1; 32]).await,
            Err(OperationRestoreError::StoreUnavailable)
        ));
        store.fail.store(false, Ordering::Release);
        assert!(matches!(
            runtime.begin(domain, Uuid::new_v4(), [2; 32]).await,
            Err(OperationRestoreError::Unhealthy)
        ));
    }

    #[tokio::test]
    async fn ambiguous_database_unavailable_keeps_the_exact_intent_pending() {
        struct UnavailableManifestSource;

        #[async_trait]
        impl ManifestSource for UnavailableManifestSource {
            async fn authoritative_now_ms(&self) -> Result<u64, ManifestReadError> {
                Ok(1_000_000)
            }

            async fn manifest(
                &self,
                _identity: OperationIdentity,
            ) -> Result<OperationManifest, ManifestReadError> {
                Err(ManifestReadError::Unavailable)
            }

            async fn floors(
                &self,
                _domain: CommunityId,
            ) -> Result<Vec<AuthorizationVersionComponentFloor>, ManifestReadError> {
                Ok(Vec::new())
            }

            async fn predecessors(
                &self,
                _manifest: &OperationManifest,
            ) -> Result<Vec<OperationManifest>, ManifestReadError> {
                Err(ManifestReadError::Unavailable)
            }

            async fn prove_supersession(
                &self,
                _pending: &OperationManifest,
                _component: &ManifestComponent,
                _floor: &FloorRecord,
            ) -> Result<(), ManifestReadError> {
                Err(ManifestReadError::Unavailable)
            }
        }

        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let store = Arc::new(MemoryStore::default());
        let bootstrap_id = Uuid::new_v4();
        let retention = test_retention();
        for shard_id in 0..retention.shard_count {
            store.records.lock().expect("records").insert(
                operation_shard_key(domain, shard_id),
                (
                    1,
                    serde_json::to_vec(&OperationShardRecord::empty(
                        domain,
                        bootstrap_id,
                        shard_id,
                        retention,
                    ))
                    .expect("encode shard"),
                ),
            );
        }
        let ready = Arc::new(DashSet::new());
        ready.insert(domain);
        let runtime = OperationRestoreRuntime(Arc::new(RestoreInner {
            writer_db: test_db(),
            source: Arc::new(UnavailableManifestSource),
            store: store.clone(),
            bootstrap_ids: Arc::new(HashMap::from([(domain, bootstrap_id)])),
            ready_domains: ready,
            lease: DEFAULT_LEASE,
            retention,
            fence_pool: test_fence_pool(),
            fence_slots: Arc::new(Semaphore::new(1)),
            test_fences: Some(Arc::new(DashSet::new())),
            healthy: AtomicBool::new(true),
        }));
        let operation = Uuid::new_v4();
        let request = [1_u8; 32];
        let OperationRestoreBegin::Acquired(_intent) = runtime
            .begin(domain, operation, request)
            .await
            .expect("acquire exact intent")
        else {
            panic!("acquired");
        };
        assert_eq!(
            runtime.reconcile(domain, operation, request).await,
            Err(OperationRestoreError::DatabaseUnavailable)
        );
        let identity = OperationIdentity {
            domain,
            operation_id: operation,
            request_fingerprint: request,
        };
        let shard_id = operation_shard_id(identity, bootstrap_id, retention.shard_count);
        let stored = store
            .records
            .lock()
            .expect("records")
            .get(&operation_shard_key(domain, shard_id))
            .expect("pending record remains")
            .1
            .clone();
        let shard: OperationShardRecord = serde_json::from_slice(&stored).expect("decode shard");
        let record = shard
            .entries
            .get(&operation.to_string())
            .expect("pending record remains");
        assert_eq!(record.state, IntentState::Pending);
        assert!(!runtime.is_healthy());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn postgres_witness_read_completes_while_size_one_writer_pool_is_held() {
        use sqlx::postgres::PgPoolOptions;

        let admin_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| crate::authorization_version::loopback_test_database_url());
        let admin = sqlx::PgPool::connect(&admin_url)
            .await
            .expect("connect admin");
        let name = format!("s5_restore_pool_{}", Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .expect("create scratch database");
        let split = admin_url.rfind('/').expect("database URL path");
        let url = format!("{}/{}", &admin_url[..split], name);
        let writer = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("writer pool");
        crate::migration::run_migrations(&writer)
            .await
            .expect("migrate");
        let witness = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("independent witness pool");
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(domain.as_uuid())
            .bind("restore-pool.example")
            .execute(&writer)
            .await
            .expect("community");
        let held_writer = writer.acquire().await.expect("hold sole writer connection");
        let runtime = OperationRestoreRuntime(Arc::new(RestoreInner {
            writer_db: Db::from_pool(writer.clone()),
            source: Arc::new(PostgresManifestSource(Db::from_pool(witness.clone()))),
            store: Arc::new(MemoryStore::default()),
            bootstrap_ids: Arc::new(HashMap::from([(domain, Uuid::new_v4())])),
            ready_domains: Arc::new(DashSet::new()),
            lease: Duration::from_millis(1),
            retention: test_retention(),
            fence_pool: test_fence_pool(),
            fence_slots: Arc::new(Semaphore::new(1)),
            test_fences: Some(Arc::new(DashSet::new())),
            healthy: AtomicBool::new(true),
        }));
        tokio::time::timeout(Duration::from_secs(2), runtime.provision_domain(domain))
            .await
            .expect("witness must not wait for writer pool")
            .expect("provision exact floors");
        let operation = Uuid::new_v4();
        assert!(matches!(
            runtime
                .begin(domain, operation, [91_u8; 32])
                .await
                .expect("reserve with PostgreSQL clock"),
            OperationRestoreBegin::Acquired(_)
        ));
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(matches!(
            runtime.begin(domain, operation, [91_u8; 32]).await,
            Err(OperationRestoreError::OperationAborted)
        ));
        assert!(runtime.is_healthy());
        drop(held_writer);
        writer.close().await;
        witness.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!("DROP DATABASE {name}")))
            .execute(&admin)
            .await
            .expect("drop scratch database");
        admin.close().await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn postgres_restart_reconciles_crash_after_publication_commit() {
        use crate::authorization_version::{
            protected_publication_request_for_restore_test, ProtectedPublicationDisposition,
            ProtectedPublicationError, ProtectedPublicationOperationIdentity,
        };
        use buzz_auth::AuthorizationEventCapacityPolicy;
        use buzz_core::AuthorizationLeaseFence;
        use nostr::Keys;
        use sqlx::postgres::PgPoolOptions;

        let admin_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| crate::authorization_version::loopback_test_database_url());
        let admin = sqlx::PgPool::connect(&admin_url)
            .await
            .expect("connect admin");
        let name = format!("s5_restore_restart_{}", Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .expect("create scratch database");
        let split = admin_url.rfind('/').expect("database URL path");
        let url = format!("{}/{}", &admin_url[..split], name);
        let writer = PgPoolOptions::new()
            .max_connections(3)
            .connect(&url)
            .await
            .expect("writer pool");
        crate::migration::run_migrations(&writer)
            .await
            .expect("migrate");
        let witness = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("independent witness pool");
        let fence_probe = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("independent fence probe pool");
        let blocker_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("independent blocker pool");
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(domain.as_uuid())
            .bind(format!(
                "restore-restart-{}.example",
                domain.as_uuid().simple()
            ))
            .execute(&writer)
            .await
            .expect("community");
        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id,policy_revision,enrollment_mode,policy_digest,effective_at) \
             VALUES ($1,1,1,$2,clock_timestamp() - INTERVAL '1 second')",
        )
        .bind(domain.as_uuid())
        .bind(vec![1_u8; 32])
        .execute(&writer)
        .await
        .expect("policy");
        let actor = Keys::generate();
        let binding_id = Uuid::new_v4();
        let mut seed = writer.acquire().await.expect("seed binding connection");
        sqlx::query("SET session_replication_role=replica")
            .execute(&mut *seed)
            .await
            .expect("disable binding triggers");
        let binding_version: i64 = sqlx::query_scalar(
            "INSERT INTO identity_bindings \
             (community_id,binding_id,issuer,subject,principal_fingerprint,event_author_pubkey, \
              binding_state,lifecycle_revision,binding_provenance,policy_revision, \
              enrollment_evidence_digest,birth_history_id,creation_operation_id, \
              creation_request_fingerprint) \
             VALUES ($1,$2,'https://issuer.example','opaque-123',$3,$4,1,1,1,1,$5,$6,$7,$8) \
             RETURNING binding_version",
        )
        .bind(domain.as_uuid())
        .bind(binding_id)
        .bind(vec![2_u8; 32])
        .bind(actor.public_key().to_bytes().as_slice())
        .bind(vec![3_u8; 32])
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(vec![4_u8; 32])
        .fetch_one(&mut *seed)
        .await
        .expect("active binding");
        sqlx::query("SET session_replication_role=origin")
            .execute(&mut *seed)
            .await
            .expect("restore binding triggers");
        drop(seed);
        sqlx::query(
            "INSERT INTO authorization_invalidation_domains (community_id,current_generation) \
             VALUES ($1,0)",
        )
        .bind(domain.as_uuid())
        .execute(&writer)
        .await
        .expect("activate invalidation");
        let db = Db::from_pool(writer.clone());
        db.install_authorization_event_capacity(
            domain,
            AuthorizationEventCapacityPolicy::new(100, 1 << 20, 16 << 10).expect("capacity"),
        )
        .await
        .expect("install capacity");

        let operation = ProtectedPublicationOperationIdentity::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        )
        .expect("operation identity");
        let (coordinate, request) = protected_publication_request_for_restore_test(
            domain,
            actor.public_key().to_bytes(),
            binding_id,
            u64::try_from(binding_version).expect("binding version"),
            AuthorizationLeaseFence::from_bytes([7_u8; 32]).expect("fence"),
            0,
            1,
            operation,
            [8_u8; 32],
            None,
        )
        .expect("sealed publication request");
        let request = Arc::new(request);
        let restore_identity = request.restore_identity();

        let store = Arc::new(MemoryStore::default());
        let bootstrap_id = Uuid::new_v4();
        let original = OperationRestoreRuntime(Arc::new(RestoreInner {
            writer_db: db.clone(),
            source: Arc::new(PostgresManifestSource(Db::from_pool(witness.clone()))),
            store: store.clone(),
            bootstrap_ids: Arc::new(HashMap::from([(domain, bootstrap_id)])),
            ready_domains: Arc::new(DashSet::new()),
            lease: Duration::from_millis(25),
            retention: test_retention(),
            fence_pool: writer.clone(),
            fence_slots: Arc::new(Semaphore::new(1)),
            test_fences: None,
            healthy: AtomicBool::new(true),
        }));
        original
            .provision_domain(domain)
            .await
            .expect("provision exact DB baseline");
        let mut crash_fenced = original
            .begin_protected_publication(restore_identity)
            .await
            .expect("begin restore before writer transaction");
        let (_, saturated_request) = protected_publication_request_for_restore_test(
            domain,
            actor.public_key().to_bytes(),
            binding_id,
            u64::try_from(binding_version).expect("binding version"),
            AuthorizationLeaseFence::from_bytes([7_u8; 32]).expect("fence"),
            0,
            1,
            ProtectedPublicationOperationIdentity::new(
                Uuid::new_v4(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
            .expect("saturated operation"),
            [18_u8; 32],
            None,
        )
        .expect("sealed saturated publication request");
        let saturated_identity = saturated_request.restore_identity();
        assert!(matches!(
            original
                .begin_protected_publication(saturated_identity)
                .await,
            Err(OperationRestoreError::OperationFenceSaturated)
        ));
        assert!(original.is_healthy());
        let mut blocker = blocker_pool
            .begin()
            .await
            .expect("begin blocking transaction");
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await
            .expect("blocking backend pid");
        sqlx::query(
            "SELECT 1 FROM identity_bindings \
             WHERE community_id=$1 AND binding_id=$2 FOR UPDATE",
        )
        .bind(domain.as_uuid())
        .bind(binding_id)
        .fetch_one(&mut *blocker)
        .await
        .expect("hold binding writer lock");
        let writer_db = db.clone();
        let writer_request = request.clone();
        let writer_task = tokio::spawn(async move {
            let result = writer_db
                .commit_staged_protected_publication_fenced(
                    crash_fenced.capability_mut().expect("fenced capability"),
                    &writer_request,
                )
                .await;
            (result, crash_fenced)
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let writer_pid: Option<i32> = sqlx::query_scalar(
                    "SELECT pid FROM pg_stat_activity \
                     WHERE datname=current_database() AND wait_event_type='Lock' \
                       AND query LIKE '%FROM identity_bindings%' \
                       AND $1 = ANY(pg_blocking_pids(pid)) LIMIT 1",
                )
                .bind(blocker_pid)
                .fetch_optional(&witness)
                .await
                .expect("inspect blocked canonical writer");
                if writer_pid.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("canonical writer reaches the blocked binding re-fence");
        let crash_shard = operation_shard_id(
            OperationIdentity {
                domain: restore_identity.authorization_domain(),
                operation_id: restore_identity.operation_id(),
                request_fingerprint: restore_identity.request_fingerprint(),
            },
            bootstrap_id,
            original.0.retention.shard_count,
        );
        let shard_key = operation_shard_key(domain, crash_shard);
        let expires_at_ms = {
            let records = store.records.lock().expect("records");
            let shard: OperationShardRecord =
                serde_json::from_slice(&records.get(&shard_key).expect("operation shard").1)
                    .expect("decode operation shard");
            shard
                .entries
                .get(&restore_identity.operation_id().to_string())
                .expect("pending writer intent")
                .expires_at_ms
        };
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let now_ms: i64 = sqlx::query_scalar(
                    "SELECT floor(EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint",
                )
                .fetch_one(&witness)
                .await
                .expect("sample PostgreSQL clock");
                if u64::try_from(now_ms).expect("nonnegative PostgreSQL time") >= expires_at_ms {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("Pending lease expires by PostgreSQL time");
        let reaper = OperationRestoreRuntime(Arc::new(RestoreInner {
            writer_db: original.0.writer_db.clone(),
            source: original.0.source.clone(),
            store: original.0.store.clone(),
            bootstrap_ids: original.0.bootstrap_ids.clone(),
            ready_domains: original.0.ready_domains.clone(),
            lease: original.0.lease,
            retention: original.0.retention,
            fence_pool: fence_probe.clone(),
            fence_slots: Arc::new(Semaphore::new(1)),
            test_fences: None,
            healthy: AtomicBool::new(true),
        }));
        let reaper_result = reaper.maintain_begin_shard(domain, crash_shard).await;
        assert!(
            matches!(reaper_result, Err(OperationRestoreError::CausalLag)),
            "reaper must retain the live writer intent, got {reaper_result:?}"
        );
        assert!(reaper.is_healthy());
        let operation_identity = OperationIdentity {
            domain: restore_identity.authorization_domain(),
            operation_id: restore_identity.operation_id(),
            request_fingerprint: restore_identity.request_fingerprint(),
        };
        let exact_contention = reaper
            .acquire_operation_fence_with_timeout(operation_identity, OPERATION_FENCE_WAIT)
            .await;
        assert!(
            matches!(
                exact_contention,
                Err(OperationRestoreError::OperationFenceTimeout)
            ),
            "reaper must contend on the exact live session fence, got {exact_contention:?}"
        );
        let shard: OperationShardRecord = serde_json::from_slice(
            &store
                .records
                .lock()
                .expect("records")
                .get(&shard_key)
                .expect("operation shard")
                .1,
        )
        .expect("decode operation shard");
        assert_eq!(
            shard
                .entries
                .get(&restore_identity.operation_id().to_string())
                .expect("pending writer intent")
                .state,
            IntentState::Pending
        );
        for table in [
            "authorization_operation_receipts",
            "authorization_operation_version_delta_manifests",
        ] {
            let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT count(*) FROM {table} WHERE community_id=$1 AND operation_id=$2"
            )))
            .bind(domain.as_uuid())
            .bind(restore_identity.operation_id())
            .fetch_one(&witness)
            .await
            .expect("inspect precommit attribution");
            assert_eq!(
                count, 0,
                "{table} must remain empty while writer is blocked"
            );
        }
        let publication_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM protected_object_authority WHERE community_id=$1",
        )
        .bind(domain.as_uuid())
        .fetch_one(&witness)
        .await
        .expect("inspect precommit publication");
        assert_eq!(publication_count, 0);
        blocker
            .rollback()
            .await
            .expect("release binding writer lock");
        let (committed, crash_fenced) = tokio::time::timeout(Duration::from_secs(2), writer_task)
            .await
            .expect("canonical writer completes after blocker release")
            .expect("join fenced writer");
        let committed = committed.expect("commit publication before simulated process death");
        assert_eq!(
            committed.disposition(),
            ProtectedPublicationDisposition::Applied
        );
        drop(crash_fenced);
        assert_eq!(
            original
                .reconcile_protected_publication(saturated_identity)
                .await,
            Err(OperationRestoreError::MissingIntent)
        );

        let (_, contended_request) = protected_publication_request_for_restore_test(
            domain,
            actor.public_key().to_bytes(),
            binding_id,
            u64::try_from(binding_version).expect("binding version"),
            AuthorizationLeaseFence::from_bytes([7_u8; 32]).expect("fence"),
            0,
            1,
            ProtectedPublicationOperationIdentity::new(
                Uuid::new_v4(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
            .expect("contended operation"),
            [19_u8; 32],
            None,
        )
        .expect("sealed contended publication request");
        let contended_identity = contended_request.restore_identity();
        let held = original
            .begin_protected_publication(contended_identity)
            .await
            .expect("first runtime holds exact operation fence");
        let contender = OperationRestoreRuntime(Arc::new(RestoreInner {
            writer_db: original.0.writer_db.clone(),
            source: original.0.source.clone(),
            store: original.0.store.clone(),
            bootstrap_ids: original.0.bootstrap_ids.clone(),
            ready_domains: original.0.ready_domains.clone(),
            lease: original.0.lease,
            retention: original.0.retention,
            fence_pool: fence_probe,
            fence_slots: Arc::new(Semaphore::new(1)),
            test_fences: None,
            healthy: AtomicBool::new(true),
        }));
        let contention = contender
            .begin_protected_publication(contended_identity)
            .await;
        assert!(
            matches!(
                &contention,
                Err(OperationRestoreError::OperationFenceTimeout)
            ),
            "same-operation contender must observe the held session fence, got {:?}",
            contention.as_ref().err()
        );
        assert!(contender.is_healthy());
        original
            .abort_fenced(held)
            .await
            .expect("abort held intent");
        assert_eq!(
            original
                .reconcile_protected_publication(contended_identity)
                .await,
            Err(OperationRestoreError::OperationAborted)
        );

        let restarted = OperationRestoreRuntime(Arc::new(RestoreInner {
            writer_db: db.clone(),
            source: Arc::new(PostgresManifestSource(Db::from_pool(witness.clone()))),
            store,
            bootstrap_ids: Arc::new(HashMap::from([(domain, bootstrap_id)])),
            ready_domains: Arc::new(DashSet::new()),
            lease: Duration::from_millis(25),
            retention: test_retention(),
            fence_pool: writer.clone(),
            fence_slots: Arc::new(Semaphore::new(1)),
            test_fences: None,
            healthy: AtomicBool::new(true),
        }));
        restarted
            .verify_domain(domain)
            .await
            .expect("restart reconciles pending publication before floor comparison");
        let replay = restarted
            .commit_protected_publication(&request)
            .await
            .expect("reconstruct exact committed witness");
        assert_eq!(
            replay.disposition(),
            ProtectedPublicationDisposition::ExactReplay
        );
        assert_eq!(replay.result_digest(), committed.result_digest());

        let (_, conflict_request) = protected_publication_request_for_restore_test(
            domain,
            actor.public_key().to_bytes(),
            binding_id,
            u64::try_from(binding_version).expect("binding version"),
            AuthorizationLeaseFence::from_bytes([7_u8; 32]).expect("fence"),
            0,
            1,
            ProtectedPublicationOperationIdentity::new(
                Uuid::new_v4(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
            .expect("conflict operation"),
            [9_u8; 32],
            Some([10_u8; 32]),
        )
        .expect("sealed conflicting later publication");
        let conflict_identity = conflict_request.restore_identity();
        assert!(matches!(
            restarted
                .commit_protected_publication(&conflict_request)
                .await,
            Err(RestoredProtectedPublicationError::Publication(
                ProtectedPublicationError::Conflict
            ))
        ));
        assert_eq!(
            db.protected_publication_witness(&coordinate)
                .await
                .expect("load unchanged witness")
                .expect("publication remains")
                .result_digest(),
            committed.result_digest()
        );
        assert_eq!(
            restarted
                .reconcile_protected_publication(conflict_identity)
                .await,
            Err(OperationRestoreError::OperationAborted)
        );

        writer.close().await;
        witness.close().await;
        blocker_pool.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!("DROP DATABASE {name}")))
            .execute(&admin)
            .await
            .expect("drop scratch database");
        admin.close().await;
    }
}
