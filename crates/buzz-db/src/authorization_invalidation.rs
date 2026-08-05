//! Durable provider-neutral authorization invalidation state.
//!
//! Postgres is the authority. Every committed event receives one strictly
//! increasing generation inside its authorization domain. Redis may advertise
//! that generation, but consumers always reconcile selector floors here.

use std::collections::BTreeMap;
use std::fmt;

use buzz_core::CommunityId;
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::{Db, DbError, Result};

/// Maximum selectors accepted in one atomic invalidation event.
pub const MAX_INVALIDATION_SELECTORS: usize = 64;

/// Provider-neutral selector classes understood by the authorization runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuthorizationSelectorKind {
    /// Exact issuer-qualified principal fingerprint.
    PrincipalFingerprint,
    /// Exact Nostr public key.
    NostrKey,
    /// Stable binding ID together with an invalid-through version.
    Binding,
    /// Exact runtime session ID.
    Session,
    /// Entire authorization domain.
    Domain,
    /// Exact opaque provider policy version.
    PolicyVersion,
    /// Exact delegated owner Nostr key.
    DelegatedOwner,
}

impl AuthorizationSelectorKind {
    /// Stable storage label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PrincipalFingerprint => "principal_fingerprint",
            Self::NostrKey => "nostr_key",
            Self::Binding => "binding",
            Self::Session => "session",
            Self::Domain => "domain",
            Self::PolicyVersion => "policy_version",
            Self::DelegatedOwner => "delegated_owner",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "principal_fingerprint" => Ok(Self::PrincipalFingerprint),
            "nostr_key" => Ok(Self::NostrKey),
            "binding" => Ok(Self::Binding),
            "session" => Ok(Self::Session),
            "domain" => Ok(Self::Domain),
            "policy_version" => Ok(Self::PolicyVersion),
            "delegated_owner" => Ok(Self::DelegatedOwner),
            _ => Err(DbError::InvalidData(
                "authorization invalidation selector kind is invalid".into(),
            )),
        }
    }
}

/// A typed selector for one authorization dependency.
#[derive(Clone, PartialEq, Eq)]
pub enum AuthorizationSelector {
    /// Already-derived issuer-qualified principal fingerprint.
    PrincipalFingerprint([u8; 32]),
    /// Exact Nostr actor key.
    NostrKey([u8; 32]),
    /// Stable binding ID and highest invalid version.
    Binding {
        /// Stable binding identifier.
        binding_id: Uuid,
        /// All binding versions through this value are invalid.
        invalid_through: u64,
    },
    /// Exact runtime session.
    Session(Uuid),
    /// Entire authorization domain.
    Domain,
    /// Exact opaque provider policy version.
    PolicyVersion(String),
    /// Exact delegated owner key.
    DelegatedOwner([u8; 32]),
}

impl fmt::Debug for AuthorizationSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationSelector")
            .field("kind", &self.kind())
            .field("value", &"[redacted]")
            .finish()
    }
}

impl AuthorizationSelector {
    /// Derive a fingerprint from exact validated issuer and subject bytes.
    pub fn principal(issuer: &str, subject: &str) -> Result<Self> {
        if issuer.is_empty() || subject.is_empty() {
            return Err(DbError::InvalidData(
                "authorization principal must be exact and non-empty".into(),
            ));
        }
        Ok(Self::PrincipalFingerprint(tagged_fingerprint(
            b"principal",
            &[issuer.as_bytes(), subject.as_bytes()],
        )))
    }

    /// Preserve a previously derived principal fingerprint.
    pub const fn principal_fingerprint(fingerprint: [u8; 32]) -> Self {
        Self::PrincipalFingerprint(fingerprint)
    }

    /// Select an exact Nostr actor key.
    pub const fn nostr_key(key: [u8; 32]) -> Self {
        Self::NostrKey(key)
    }

    /// Select a binding and all of its versions through `invalid_through`.
    pub fn binding(binding_id: Uuid, invalid_through: u64) -> Result<Self> {
        if binding_id.is_nil() || invalid_through == 0 {
            return Err(DbError::InvalidData(
                "authorization binding selector requires a non-nil ID and positive version".into(),
            ));
        }
        Ok(Self::Binding {
            binding_id,
            invalid_through,
        })
    }

    /// Select an exact non-nil runtime session.
    pub fn session(session_id: Uuid) -> Result<Self> {
        if session_id.is_nil() {
            return Err(DbError::InvalidData(
                "authorization session selector must not be nil".into(),
            ));
        }
        Ok(Self::Session(session_id))
    }

    /// Select the entire authorization domain.
    pub const fn domain() -> Self {
        Self::Domain
    }

    /// Select an exact non-empty provider policy version.
    pub fn policy_version(version: impl Into<String>) -> Result<Self> {
        let version = version.into();
        if version.is_empty() {
            return Err(DbError::InvalidData(
                "authorization policy version must not be empty".into(),
            ));
        }
        Ok(Self::PolicyVersion(version))
    }

    /// Select an exact delegated owner key.
    pub const fn delegated_owner(key: [u8; 32]) -> Self {
        Self::DelegatedOwner(key)
    }

    /// Selector class.
    pub const fn kind(&self) -> AuthorizationSelectorKind {
        match self {
            Self::PrincipalFingerprint(_) => AuthorizationSelectorKind::PrincipalFingerprint,
            Self::NostrKey(_) => AuthorizationSelectorKind::NostrKey,
            Self::Binding { .. } => AuthorizationSelectorKind::Binding,
            Self::Session(_) => AuthorizationSelectorKind::Session,
            Self::Domain => AuthorizationSelectorKind::Domain,
            Self::PolicyVersion(_) => AuthorizationSelectorKind::PolicyVersion,
            Self::DelegatedOwner(_) => AuthorizationSelectorKind::DelegatedOwner,
        }
    }

    /// Redaction-safe stable selector fingerprint.
    pub fn fingerprint(&self) -> [u8; 32] {
        match self {
            Self::PrincipalFingerprint(value) => *value,
            Self::NostrKey(key) => tagged_fingerprint(b"nostr-key", &[key]),
            Self::Binding { binding_id, .. } => {
                tagged_fingerprint(b"binding", &[binding_id.as_bytes()])
            }
            Self::Session(session_id) => tagged_fingerprint(b"session", &[session_id.as_bytes()]),
            Self::Domain => tagged_fingerprint(b"domain", &[]),
            Self::PolicyVersion(version) => {
                tagged_fingerprint(b"policy-version", &[version.as_bytes()])
            }
            Self::DelegatedOwner(key) => tagged_fingerprint(b"delegated-owner", &[key]),
        }
    }

    /// Invalid-through binding version, when this is a binding selector.
    pub const fn binding_version_floor(&self) -> Option<u64> {
        match self {
            Self::Binding {
                invalid_through, ..
            } => Some(*invalid_through),
            _ => None,
        }
    }
}

/// Effect retained for a selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorizationInvalidationEffect {
    /// Fence evaluations captured before this event's generation.
    Fence,
    /// Deny this selector until an explicit future recovery mechanism changes authority.
    StickyDeny,
    /// Permanently deny binding versions through the selector's version floor.
    BindingVersionFloor,
}

impl AuthorizationInvalidationEffect {
    const fn is_sticky(self) -> bool {
        matches!(self, Self::StickyDeny)
    }
}

/// One selector and effect in an invalidation event.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationInvalidationEntry {
    selector: AuthorizationSelector,
    effect: AuthorizationInvalidationEffect,
}

impl AuthorizationInvalidationEntry {
    /// Permanently deny one exact selector until a separately designed and
    /// authorized recovery mechanism changes authority.
    pub fn sticky_deny(selector: AuthorizationSelector) -> Result<Self> {
        if selector.kind() == AuthorizationSelectorKind::Binding {
            return Err(DbError::InvalidData(
                "binding invalidation requires a version-floor entry".into(),
            ));
        }
        Ok(Self {
            selector,
            effect: AuthorizationInvalidationEffect::StickyDeny,
        })
    }

    /// Fence all evaluations already in flight in one domain while allowing
    /// later evaluations to resolve fresh policy and binding state.
    pub const fn domain_fence() -> Self {
        Self {
            selector: AuthorizationSelector::Domain,
            effect: AuthorizationInvalidationEffect::Fence,
        }
    }

    /// Fence authority captured before reversible admission loss for one exact
    /// principal, Nostr key, or delegated owner.
    ///
    /// This is intentionally unavailable for bindings, sessions, domains, and
    /// policy versions. Binding invalidation uses a monotonic version floor,
    /// while domain fencing remains an explicit separate operation.
    pub fn admission_loss_fence(selector: AuthorizationSelector) -> Result<Self> {
        if !matches!(
            selector.kind(),
            AuthorizationSelectorKind::PrincipalFingerprint
                | AuthorizationSelectorKind::NostrKey
                | AuthorizationSelectorKind::DelegatedOwner
        ) {
            return Err(DbError::InvalidData(
                "authorization admission loss requires a principal, key, or delegated owner".into(),
            ));
        }
        Ok(Self {
            selector,
            effect: AuthorizationInvalidationEffect::Fence,
        })
    }

    /// Permanently deny one binding ID through a positive version, while
    /// permitting a later version of that same stable binding ID.
    pub fn binding_version_floor(binding_id: Uuid, invalid_through: u64) -> Result<Self> {
        Ok(Self {
            selector: AuthorizationSelector::binding(binding_id, invalid_through)?,
            effect: AuthorizationInvalidationEffect::BindingVersionFloor,
        })
    }

    /// Selected dependency.
    pub const fn selector(&self) -> &AuthorizationSelector {
        &self.selector
    }

    /// Retained effect.
    pub const fn effect(&self) -> AuthorizationInvalidationEffect {
        self.effect
    }
}

impl fmt::Debug for AuthorizationInvalidationEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationInvalidationEntry")
            .field("selector", &self.selector)
            .field("effect", &self.effect)
            .finish()
    }
}

/// Atomic idempotent invalidation request.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationInvalidationRequest {
    event_id: Uuid,
    entries: Vec<AuthorizationInvalidationEntry>,
}

impl AuthorizationInvalidationRequest {
    /// Validate a non-empty, bounded event.
    pub fn new(event_id: Uuid, entries: Vec<AuthorizationInvalidationEntry>) -> Result<Self> {
        if event_id.is_nil() {
            return Err(DbError::InvalidData(
                "authorization invalidation event ID must not be nil".into(),
            ));
        }
        if entries.is_empty() || entries.len() > MAX_INVALIDATION_SELECTORS {
            return Err(DbError::InvalidData(
                "authorization invalidation selector count is out of bounds".into(),
            ));
        }
        if entries.iter().any(|entry| {
            let kind = entry.selector.kind();
            !match (kind, entry.effect) {
                (
                    AuthorizationSelectorKind::PrincipalFingerprint
                    | AuthorizationSelectorKind::NostrKey
                    | AuthorizationSelectorKind::DelegatedOwner
                    | AuthorizationSelectorKind::Domain,
                    AuthorizationInvalidationEffect::Fence,
                )
                | (
                    AuthorizationSelectorKind::Binding,
                    AuthorizationInvalidationEffect::BindingVersionFloor,
                ) => true,
                (_, AuthorizationInvalidationEffect::StickyDeny) => {
                    kind != AuthorizationSelectorKind::Binding
                }
                _ => false,
            }
        }) {
            return Err(DbError::InvalidData(
                "authorization invalidation selector and effect are incompatible".into(),
            ));
        }
        Ok(Self { event_id, entries })
    }

    /// Idempotency identifier.
    pub const fn event_id(&self) -> Uuid {
        self.event_id
    }

    /// Requested selectors and effects.
    pub fn entries(&self) -> &[AuthorizationInvalidationEntry] {
        &self.entries
    }
}

impl fmt::Debug for AuthorizationInvalidationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationInvalidationRequest")
            .field("event_id", &"[redacted]")
            .field("selector_count", &self.entries.len())
            .finish()
    }
}

/// Redaction-safe receipt for a committed invalidation event.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AuthorizationInvalidationReceipt {
    /// Domain in which the event committed.
    pub community_id: CommunityId,
    /// Idempotency identifier.
    pub event_id: Uuid,
    /// Strictly increasing durable generation.
    pub generation: u64,
}

impl fmt::Debug for AuthorizationInvalidationReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationInvalidationReceipt")
            .field("community_id", &"[redacted]")
            .field("event_id", &"[redacted]")
            .field("generation", &self.generation)
            .finish()
    }
}

/// Whether a request committed now or replayed its identical receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorizationInvalidationResult {
    /// This transaction committed the event.
    Applied(AuthorizationInvalidationReceipt),
    /// An identical request had already committed.
    AlreadyApplied(AuthorizationInvalidationReceipt),
}

impl AuthorizationInvalidationResult {
    /// Whether this call committed a new durable invalidation.
    pub const fn committed_now(&self) -> bool {
        matches!(self, Self::Applied(_))
    }

    /// Durable receipt in either outcome.
    pub const fn receipt(self) -> AuthorizationInvalidationReceipt {
        match self {
            Self::Applied(receipt) | Self::AlreadyApplied(receipt) => receipt,
        }
    }
}

/// One durable selector floor returned by reconciliation.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationInvalidationFloor {
    /// Selector class.
    pub kind: AuthorizationSelectorKind,
    /// Redaction-safe stable selector fingerprint.
    pub fingerprint: [u8; 32],
    /// Most recent generation that touched the floor.
    pub generation: u64,
    /// Whether the selector remains denied independently of capture generation.
    pub sticky_deny: bool,
    /// Highest invalid binding version, for binding selectors only.
    pub binding_version_floor: Option<u64>,
}

impl fmt::Debug for AuthorizationInvalidationFloor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationInvalidationFloor")
            .field("kind", &self.kind)
            .field("fingerprint", &"[redacted]")
            .field("generation", &self.generation)
            .field("sticky_deny", &self.sticky_deny)
            .field("binding_version_floor", &self.binding_version_floor)
            .finish()
    }
}

/// Consistent durable generation and selector-floor view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizationInvalidationSnapshot {
    /// Domain read from the writer database.
    pub community_id: CommunityId,
    /// Durable generation at this snapshot.
    pub generation: u64,
    /// Full floors, or floors changed after the requested delta generation.
    pub floors: Vec<AuthorizationInvalidationFloor>,
}

#[derive(Clone)]
struct NormalizedEntry {
    kind: AuthorizationSelectorKind,
    fingerprint: [u8; 32],
    sticky_deny: bool,
    binding_version_floor: Option<u64>,
}

fn tagged_fingerprint(tag: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update((tag.len() as u64).to_be_bytes());
    digest.update(tag);
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part);
    }
    digest.finalize().into()
}

fn normalized_entries(request: &AuthorizationInvalidationRequest) -> Vec<NormalizedEntry> {
    let mut entries = BTreeMap::new();
    for entry in request.entries() {
        let selector = entry.selector();
        let key = (selector.kind(), selector.fingerprint());
        let value = entries.entry(key).or_insert(NormalizedEntry {
            kind: selector.kind(),
            fingerprint: selector.fingerprint(),
            sticky_deny: false,
            binding_version_floor: None,
        });
        value.sticky_deny |= entry.effect().is_sticky();
        value.binding_version_floor = match (
            value.binding_version_floor,
            selector.binding_version_floor(),
        ) {
            (Some(current), Some(candidate)) => Some(current.max(candidate)),
            (None, candidate) => candidate,
            (current, None) => current,
        };
    }
    entries.into_values().collect()
}

fn request_fingerprint(
    community_id: CommunityId,
    event_id: Uuid,
    entries: &[NormalizedEntry],
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"buzz-authorization-invalidation-v1");
    digest.update(community_id.as_uuid().as_bytes());
    digest.update(event_id.as_bytes());
    for entry in entries {
        digest.update((entry.kind.as_str().len() as u64).to_be_bytes());
        digest.update(entry.kind.as_str().as_bytes());
        digest.update(entry.fingerprint);
        digest.update([u8::from(entry.sticky_deny)]);
        digest.update(
            entry
                .binding_version_floor
                .unwrap_or_default()
                .to_be_bytes(),
        );
    }
    digest.finalize().into()
}

/// Deterministic fingerprint shared by invalidation and restore receipts.
pub fn authorization_invalidation_request_fingerprint(
    community_id: CommunityId,
    request: &AuthorizationInvalidationRequest,
) -> [u8; 32] {
    request_fingerprint(
        community_id,
        request.event_id(),
        &normalized_entries(request),
    )
}

fn positive_u64(value: i64, label: &str) -> Result<u64> {
    u64::try_from(value).map_err(|_| {
        DbError::InvalidData(format!(
            "authorization invalidation {label} is outside the supported range"
        ))
    })
}

fn fingerprint_array(value: Vec<u8>) -> Result<[u8; 32]> {
    value.try_into().map_err(|_| {
        DbError::InvalidData("authorization invalidation fingerprint has invalid length".into())
    })
}

impl Db {
    /// Atomically allocate a generation and retain the strongest selector floors.
    pub async fn apply_authorization_invalidation(
        &self,
        community_id: CommunityId,
        request: &AuthorizationInvalidationRequest,
    ) -> Result<AuthorizationInvalidationResult> {
        let entries = normalized_entries(request);
        let fingerprint = request_fingerprint(community_id, request.event_id(), &entries);
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO authorization_invalidation_domains (community_id) \
             VALUES ($1) ON CONFLICT (community_id) DO NOTHING",
        )
        .bind(community_id.as_uuid())
        .execute(&mut *tx)
        .await?;

        let generation: i64 = sqlx::query_scalar(
            "SELECT generation FROM authorization_invalidation_domains \
             WHERE community_id = $1 FOR UPDATE",
        )
        .bind(community_id.as_uuid())
        .fetch_one(&mut *tx)
        .await?;

        if let Some(row) = sqlx::query(
            "SELECT generation, request_fingerprint \
             FROM authorization_invalidation_receipts \
             WHERE community_id = $1 AND event_id = $2",
        )
        .bind(community_id.as_uuid())
        .bind(request.event_id())
        .fetch_optional(&mut *tx)
        .await?
        {
            let stored: Vec<u8> = row.try_get("request_fingerprint")?;
            if stored.as_slice() != fingerprint {
                return Err(DbError::InvalidData(
                    "authorization invalidation event ID was reused with different input".into(),
                ));
            }
            let receipt = AuthorizationInvalidationReceipt {
                community_id,
                event_id: request.event_id(),
                generation: positive_u64(row.try_get("generation")?, "generation")?,
            };
            tx.commit().await?;
            return Ok(AuthorizationInvalidationResult::AlreadyApplied(receipt));
        }

        let next_generation = generation.checked_add(1).ok_or_else(|| {
            DbError::InvalidData("authorization invalidation generation exhausted".into())
        })?;
        sqlx::query(
            "INSERT INTO authorization_invalidation_receipts \
             (community_id, event_id, generation, request_fingerprint) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(community_id.as_uuid())
        .bind(request.event_id())
        .bind(next_generation)
        .bind(fingerprint.as_slice())
        .execute(&mut *tx)
        .await?;

        for entry in entries {
            let binding_floor = entry
                .binding_version_floor
                .map(i64::try_from)
                .transpose()
                .map_err(|_| {
                    DbError::InvalidData(
                        "authorization binding version exceeds database range".into(),
                    )
                })?;
            sqlx::query(
                "INSERT INTO authorization_invalidation_floors \
                 (community_id, selector_kind, selector_fingerprint, generation, \
                  sticky_deny, binding_version_floor) \
                 VALUES ($1, $2, $3, $4, $5, $6) \
                 ON CONFLICT (community_id, selector_kind, selector_fingerprint) DO UPDATE SET \
                   generation = EXCLUDED.generation, \
                   sticky_deny = authorization_invalidation_floors.sticky_deny \
                                 OR EXCLUDED.sticky_deny, \
                   binding_version_floor = CASE \
                     WHEN authorization_invalidation_floors.binding_version_floor IS NULL \
                       THEN EXCLUDED.binding_version_floor \
                     WHEN EXCLUDED.binding_version_floor IS NULL \
                       THEN authorization_invalidation_floors.binding_version_floor \
                     ELSE GREATEST(authorization_invalidation_floors.binding_version_floor, \
                                   EXCLUDED.binding_version_floor) \
                   END, \
                   updated_at = NOW()",
            )
            .bind(community_id.as_uuid())
            .bind(entry.kind.as_str())
            .bind(entry.fingerprint.as_slice())
            .bind(next_generation)
            .bind(entry.sticky_deny)
            .bind(binding_floor)
            .execute(&mut *tx)
            .await?;
        }

        sqlx::query(
            "UPDATE authorization_invalidation_domains \
             SET generation = $2, updated_at = NOW() WHERE community_id = $1",
        )
        .bind(community_id.as_uuid())
        .bind(next_generation)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO authorization_operation_receipts \
             (community_id, operation_id, operation_kind, request_fingerprint, \
              result_payload, lease_expires_at) \
             VALUES ($1, $2, 'authorization.invalidation', $3, $4, \
                     clock_timestamp() + INTERVAL '100 years')",
        )
        .bind(community_id.as_uuid())
        .bind(request.event_id())
        .bind(fingerprint.as_slice())
        .bind(next_generation.to_be_bytes().as_slice())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok(AuthorizationInvalidationResult::Applied(
            AuthorizationInvalidationReceipt {
                community_id,
                event_id: request.event_id(),
                generation: positive_u64(next_generation, "generation")?,
            },
        ))
    }

    /// Read a consistent full snapshot from the writer database.
    pub async fn authorization_invalidation_snapshot(
        &self,
        community_id: CommunityId,
    ) -> Result<AuthorizationInvalidationSnapshot> {
        self.authorization_invalidation_read(community_id, None)
            .await
    }

    /// Read floors changed after `after_generation` plus the current generation.
    pub async fn authorization_invalidation_delta(
        &self,
        community_id: CommunityId,
        after_generation: u64,
    ) -> Result<AuthorizationInvalidationSnapshot> {
        self.authorization_invalidation_read(community_id, Some(after_generation))
            .await
    }

    async fn authorization_invalidation_read(
        &self,
        community_id: CommunityId,
        after_generation: Option<u64>,
    ) -> Result<AuthorizationInvalidationSnapshot> {
        let after_generation = after_generation
            .map(i64::try_from)
            .transpose()
            .map_err(|_| {
                DbError::InvalidData(
                    "authorization invalidation generation exceeds database range".into(),
                )
            })?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        let generation: Option<i64> = sqlx::query_scalar(
            "SELECT generation FROM authorization_invalidation_domains WHERE community_id = $1",
        )
        .bind(community_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?;
        let generation = generation.unwrap_or_default();
        let rows = sqlx::query(
            "SELECT selector_kind, selector_fingerprint, generation, sticky_deny, \
                    binding_version_floor \
             FROM authorization_invalidation_floors \
             WHERE community_id = $1 AND generation <= $2 \
               AND ($3::BIGINT IS NULL OR generation > $3) \
             ORDER BY generation, selector_kind, selector_fingerprint",
        )
        .bind(community_id.as_uuid())
        .bind(generation)
        .bind(after_generation)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;

        let floors = rows
            .into_iter()
            .map(|row| {
                let binding_version: Option<i64> = row.try_get("binding_version_floor")?;
                Ok(AuthorizationInvalidationFloor {
                    kind: AuthorizationSelectorKind::parse(row.try_get("selector_kind")?)?,
                    fingerprint: fingerprint_array(row.try_get("selector_fingerprint")?)?,
                    generation: positive_u64(row.try_get("generation")?, "floor generation")?,
                    sticky_deny: row.try_get("sticky_deny")?,
                    binding_version_floor: binding_version
                        .map(|value| positive_u64(value, "binding version"))
                        .transpose()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(AuthorizationInvalidationSnapshot {
            community_id,
            generation: positive_u64(generation, "generation")?,
            floors,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DbConfig;

    fn request(
        event_id: Uuid,
        selector: AuthorizationSelector,
    ) -> AuthorizationInvalidationRequest {
        let entry = match selector {
            AuthorizationSelector::Binding {
                binding_id,
                invalid_through,
            } => AuthorizationInvalidationEntry::binding_version_floor(binding_id, invalid_through),
            selector => AuthorizationInvalidationEntry::sticky_deny(selector),
        }
        .expect("test invalidation entry is valid");
        AuthorizationInvalidationRequest::new(event_id, vec![entry])
            .expect("test invalidation request is valid")
    }

    #[test]
    fn principal_fingerprints_are_exact_and_domain_neutral() {
        let a = AuthorizationSelector::principal("issuer-a", "subject").expect("valid principal");
        let b = AuthorizationSelector::principal("issuer-b", "subject").expect("valid principal");
        assert_ne!(a.fingerprint(), b.fingerprint());
        assert_eq!(
            a.fingerprint(),
            AuthorizationSelector::principal("issuer-a", "subject")
                .expect("valid principal")
                .fingerprint()
        );
        assert!(!format!("{a:?}").contains("issuer-a"));
    }

    #[test]
    fn duplicate_binding_entries_retain_strongest_version_floor() {
        let binding_id = Uuid::new_v4();
        let request = AuthorizationInvalidationRequest::new(
            Uuid::new_v4(),
            vec![
                AuthorizationInvalidationEntry::binding_version_floor(binding_id, 2)
                    .expect("valid binding floor"),
                AuthorizationInvalidationEntry::binding_version_floor(binding_id, 5)
                    .expect("valid binding floor"),
            ],
        )
        .expect("valid request");
        let normalized = normalized_entries(&request);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].binding_version_floor, Some(5));
        assert!(!normalized[0].sticky_deny);
    }

    #[test]
    fn transient_admission_fences_are_exact_and_never_bindings() {
        let allowed = [
            AuthorizationSelector::principal("issuer", "subject").expect("valid principal"),
            AuthorizationSelector::nostr_key([1_u8; 32]),
            AuthorizationSelector::delegated_owner([2_u8; 32]),
        ];
        let entries = allowed
            .into_iter()
            .map(AuthorizationInvalidationEntry::admission_loss_fence)
            .collect::<Result<Vec<_>>>()
            .expect("exact admission selectors are valid");
        let request = AuthorizationInvalidationRequest::new(Uuid::new_v4(), entries)
            .expect("admission-loss request is valid");
        assert!(normalized_entries(&request)
            .iter()
            .all(|entry| !entry.sticky_deny && entry.binding_version_floor.is_none()));

        for selector in [
            AuthorizationSelector::binding(Uuid::new_v4(), 1).expect("valid binding"),
            AuthorizationSelector::session(Uuid::new_v4()).expect("valid session"),
            AuthorizationSelector::domain(),
            AuthorizationSelector::policy_version("policy").expect("valid policy"),
        ] {
            assert!(AuthorizationInvalidationEntry::admission_loss_fence(selector).is_err());
        }
        let forged_binding_fence = AuthorizationInvalidationRequest::new(
            Uuid::new_v4(),
            vec![AuthorizationInvalidationEntry {
                selector: AuthorizationSelector::binding(Uuid::new_v4(), 1).expect("valid binding"),
                effect: AuthorizationInvalidationEffect::Fence,
            }],
        );
        assert!(matches!(forged_binding_fence, Err(DbError::InvalidData(_))));
        assert!(AuthorizationInvalidationRequest::new(
            Uuid::new_v4(),
            vec![AuthorizationInvalidationEntry::domain_fence()],
        )
        .is_ok());
        assert!(AuthorizationInvalidationEntry::sticky_deny(
            AuthorizationSelector::binding(Uuid::new_v4(), 1).expect("valid binding")
        )
        .is_err());
    }

    #[test]
    fn request_debug_redacts_identifiers() {
        let event_id = Uuid::new_v4();
        let request = request(
            event_id,
            AuthorizationSelector::policy_version("private-policy").expect("valid policy"),
        );
        let debug = format!("{request:?}");
        assert!(!debug.contains(&event_id.to_string()));
        assert!(!debug.contains("private-policy"));
    }

    async fn integration_db() -> Db {
        let mut config = DbConfig::default();
        config.database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or(config.database_url);
        config.min_connections = 0;
        let db = Db::new(&config)
            .await
            .expect("connect integration database");
        db.migrate().await.expect("run integration migrations");
        db
    }

    async fn integration_community(db: &Db) -> CommunityId {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(format!(
                "authorization-invalidation-{}.example",
                id.simple()
            ))
            .execute(&db.pool)
            .await
            .expect("insert integration community");
        CommunityId::from_uuid(id)
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn durable_idempotency_concurrency_and_delta_converge() {
        let db = integration_db().await;
        let community_id = integration_community(&db).await;
        let binding_id = Uuid::new_v4();
        let first_id = Uuid::new_v4();
        let first = request(
            first_id,
            AuthorizationSelector::binding(binding_id, 1).expect("valid binding"),
        );
        let first_receipt = db
            .apply_authorization_invalidation(community_id, &first)
            .await
            .expect("first event commits");
        assert_eq!(first_receipt.receipt().generation, 1);
        assert!(matches!(
            db.apply_authorization_invalidation(community_id, &first)
                .await
                .expect("identical retry resolves"),
            AuthorizationInvalidationResult::AlreadyApplied(_)
        ));

        let conflicting_retry = request(
            first_id,
            AuthorizationSelector::policy_version("different").expect("valid policy"),
        );
        assert!(matches!(
            db.apply_authorization_invalidation(community_id, &conflicting_retry)
                .await,
            Err(DbError::InvalidData(_))
        ));

        let second = request(
            Uuid::new_v4(),
            AuthorizationSelector::session(Uuid::new_v4()).expect("valid session"),
        );
        let third = request(
            Uuid::new_v4(),
            AuthorizationSelector::policy_version("old-policy").expect("valid policy"),
        );
        let db_a = db.clone();
        let db_b = db.clone();
        let (second_result, third_result) = tokio::join!(
            db_a.apply_authorization_invalidation(community_id, &second),
            db_b.apply_authorization_invalidation(community_id, &third),
        );
        let mut generations = [
            second_result
                .expect("second event commits")
                .receipt()
                .generation,
            third_result
                .expect("third event commits")
                .receipt()
                .generation,
        ];
        generations.sort_unstable();
        assert_eq!(generations, [2, 3]);

        let binding_advance = request(
            Uuid::new_v4(),
            AuthorizationSelector::binding(binding_id, 5).expect("valid binding"),
        );
        assert_eq!(
            db.apply_authorization_invalidation(community_id, &binding_advance)
                .await
                .expect("binding floor advances")
                .receipt()
                .generation,
            4
        );
        let snapshot = db
            .authorization_invalidation_snapshot(community_id)
            .await
            .expect("full snapshot reads");
        assert_eq!(snapshot.generation, 4);
        let binding_floor = snapshot
            .floors
            .iter()
            .find(|floor| floor.kind == AuthorizationSelectorKind::Binding)
            .expect("binding floor present");
        assert_eq!(binding_floor.binding_version_floor, Some(5));
        assert!(!binding_floor.sticky_deny);

        let delta = db
            .authorization_invalidation_delta(community_id, 3)
            .await
            .expect("delta reads");
        assert_eq!(delta.generation, 4);
        assert_eq!(delta.floors.len(), 1);
        assert_eq!(delta.floors[0].kind, AuthorizationSelectorKind::Binding);

        let principal = AuthorizationSelector::principal("admission-issuer", "admission-subject")
            .expect("valid principal");
        let admission_loss = AuthorizationInvalidationRequest::new(
            Uuid::new_v4(),
            vec![
                AuthorizationInvalidationEntry::admission_loss_fence(principal.clone())
                    .expect("valid admission-loss fence"),
            ],
        )
        .expect("valid admission-loss request");
        assert_eq!(
            db.apply_authorization_invalidation(community_id, &admission_loss)
                .await
                .expect("admission-loss fence commits")
                .receipt()
                .generation,
            5
        );
        let delta = db
            .authorization_invalidation_delta(community_id, 4)
            .await
            .expect("admission-loss delta reads");
        assert_eq!(delta.generation, 5);
        assert_eq!(delta.floors.len(), 1);
        assert_eq!(
            delta.floors[0].kind,
            AuthorizationSelectorKind::PrincipalFingerprint
        );
        assert_eq!(delta.floors[0].fingerprint, principal.fingerprint());
        assert!(!delta.floors[0].sticky_deny);
        assert_eq!(delta.floors[0].binding_version_floor, None);
        // Activated domains are intentionally one-way in V1. This isolated
        // integration database is discarded by the test harness rather than
        // bypassing the production marker guard for cleanup.
    }
}
