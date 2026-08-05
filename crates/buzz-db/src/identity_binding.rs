//! Corporate identity binding persistence.
//!
//! Bindings map an issuer-qualified IdP uid to the currently authorized Nostr
//! pubkey inside one Buzz community. The active uniqueness indexes deliberately
//! model one active pubkey per `(issuer, uid)` principal and one active principal
//! per pubkey. Explicit lifecycle operations distinguish principal disablement,
//! single-key revocation, and authorized rotation; authentication never
//! silently rewrites those states.

use std::fmt;

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::error::{DbError, Result};
use buzz_auth::context::{
    AuthoritativeBindingResolution, BindingExpiry as AuthorizedBindingExpiry,
};
use buzz_auth::{
    AuthorityAdapterError, AuthorityAdapterFuture, BindingResolutionRequest,
    BindingSource as AuthorizedBindingSource, BindingVersion as AuthorizedBindingVersion,
    CurrentPolicyRequest, CurrentPolicyResolutionSink, DirectBindingResolutionSink,
    EnrollmentMode as AuthorizedEnrollmentMode, ExistingBindingResolutionSink,
    FederatedAuthorityAdapter, FederatedIdentityRequirement, ResolvedFederatedPolicy,
};
use buzz_core::CommunityId;

#[cfg(test)]
pub(crate) mod test_lock_schedule {
    use std::cell::Cell;
    use std::future::Future;
    use std::sync::{Mutex, OnceLock};

    use sqlx::{Postgres, Transaction};
    use tokio::sync::{mpsc, oneshot};

    tokio::task_local! {
        static ACTOR: &'static str;
        static ROW_REQUEST_REPORTED: Cell<bool>;
        static ROW_ACQUIRED_REPORTED: Cell<bool>;
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum LockPhase {
        Request,
        Acquired,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum RowLockPhase {
        Request,
        Acquired,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) struct AdvisoryLockKey {
        class_id: u32,
        object_id: u32,
    }

    impl AdvisoryLockKey {
        pub(crate) const fn class_id(self) -> u32 {
            self.class_id
        }

        pub(crate) const fn object_id(self) -> u32 {
            self.object_id
        }
    }

    pub(crate) struct LockEvent {
        actor: &'static str,
        phase: LockPhase,
        isolation: Option<String>,
        transaction_id: Option<i64>,
        backend_pid: i32,
        database_oid: u32,
        lock_keys: Vec<AdvisoryLockKey>,
        resume: oneshot::Sender<()>,
    }

    impl LockEvent {
        pub(crate) const fn actor(&self) -> &'static str {
            self.actor
        }

        pub(crate) const fn phase(&self) -> LockPhase {
            self.phase
        }

        pub(crate) fn isolation(&self) -> Option<&str> {
            self.isolation.as_deref()
        }

        pub(crate) const fn transaction_id(&self) -> Option<i64> {
            self.transaction_id
        }

        pub(crate) const fn backend_pid(&self) -> i32 {
            self.backend_pid
        }

        pub(crate) const fn database_oid(&self) -> u32 {
            self.database_oid
        }

        pub(crate) fn lock_keys(&self) -> &[AdvisoryLockKey] {
            &self.lock_keys
        }

        pub(crate) const fn coordinate_count(&self) -> usize {
            self.lock_keys.len()
        }

        pub(crate) fn resume(self) {
            let _ = self.resume.send(());
        }
    }

    fn controller() -> &'static Mutex<Option<mpsc::UnboundedSender<LockEvent>>> {
        static CONTROLLER: OnceLock<Mutex<Option<mpsc::UnboundedSender<LockEvent>>>> =
            OnceLock::new();
        CONTROLLER.get_or_init(|| Mutex::new(None))
    }

    pub(crate) struct ControllerGuard;

    impl Drop for ControllerGuard {
        fn drop(&mut self) {
            *controller().lock().expect("lock test controller") = None;
        }
    }

    pub(crate) fn install() -> (mpsc::UnboundedReceiver<LockEvent>, ControllerGuard) {
        let (sender, receiver) = mpsc::unbounded_channel();
        let mut current = controller().lock().expect("lock test controller");
        assert!(
            current.is_none(),
            "only one deterministic lock controller may be active"
        );
        *current = Some(sender);
        (receiver, ControllerGuard)
    }

    pub(crate) struct RowLockEvent {
        actor: &'static str,
        phase: RowLockPhase,
        transaction_id: i64,
        backend_pid: i32,
        database_oid: u32,
        resume: oneshot::Sender<()>,
    }

    impl RowLockEvent {
        pub(crate) const fn actor(&self) -> &'static str {
            self.actor
        }

        pub(crate) const fn phase(&self) -> RowLockPhase {
            self.phase
        }

        pub(crate) const fn transaction_id(&self) -> i64 {
            self.transaction_id
        }

        pub(crate) const fn backend_pid(&self) -> i32 {
            self.backend_pid
        }

        pub(crate) const fn database_oid(&self) -> u32 {
            self.database_oid
        }

        pub(crate) fn resume(self) {
            let _ = self.resume.send(());
        }
    }

    fn row_controller() -> &'static Mutex<Option<mpsc::UnboundedSender<RowLockEvent>>> {
        static CONTROLLER: OnceLock<Mutex<Option<mpsc::UnboundedSender<RowLockEvent>>>> =
            OnceLock::new();
        CONTROLLER.get_or_init(|| Mutex::new(None))
    }

    pub(crate) struct RowControllerGuard;

    impl Drop for RowControllerGuard {
        fn drop(&mut self) {
            *row_controller().lock().expect("lock row test controller") = None;
        }
    }

    pub(crate) fn install_row() -> (mpsc::UnboundedReceiver<RowLockEvent>, RowControllerGuard) {
        let (sender, receiver) = mpsc::unbounded_channel();
        let mut current = row_controller().lock().expect("lock row test controller");
        assert!(
            current.is_none(),
            "only one deterministic row-lock controller may be active"
        );
        *current = Some(sender);
        (receiver, RowControllerGuard)
    }

    pub(crate) async fn actor_scope<F>(actor: &'static str, future: F) -> F::Output
    where
        F: Future,
    {
        ACTOR
            .scope(
                actor,
                ROW_REQUEST_REPORTED.scope(
                    Cell::new(false),
                    ROW_ACQUIRED_REPORTED.scope(Cell::new(false), future),
                ),
            )
            .await
    }

    pub(super) async fn checkpoint(
        tx: &mut Transaction<'_, Postgres>,
        phase: LockPhase,
        coordinates: &[Vec<u8>],
    ) {
        let Ok(actor) = ACTOR.try_with(|actor| *actor) else {
            return;
        };
        let sender = controller().lock().expect("lock test controller").clone();
        let Some(sender) = sender else {
            return;
        };
        let (backend_pid, database_oid): (i32, i64) = sqlx::query_as(
            "SELECT pg_backend_pid(), oid::BIGINT \
             FROM pg_database WHERE datname=current_database()",
        )
        .fetch_one(&mut **tx)
        .await
        .expect("read test lock backend identity");
        let class_id: i32 = sqlx::query_scalar("SELECT hashtext('buzz_nip_fi_v1')")
            .fetch_one(&mut **tx)
            .await
            .expect("hash test lock namespace");
        let mut lock_keys = Vec::with_capacity(coordinates.len());
        for coordinate in coordinates {
            let object_id: i32 = sqlx::query_scalar("SELECT hashtext(encode($1, 'hex'))")
                .bind(coordinate.as_slice())
                .fetch_one(&mut **tx)
                .await
                .expect("hash test lock coordinate");
            lock_keys.push(AdvisoryLockKey {
                class_id: class_id as u32,
                object_id: object_id as u32,
            });
        }
        let (isolation, transaction_id) = if phase == LockPhase::Acquired {
            let isolation = sqlx::query_scalar("SHOW transaction_isolation")
                .fetch_one(&mut **tx)
                .await
                .ok();
            let transaction_id = sqlx::query_scalar("SELECT txid_current()::BIGINT")
                .fetch_one(&mut **tx)
                .await
                .ok();
            (isolation, transaction_id)
        } else {
            (None, None)
        };
        let (resume, resumed) = oneshot::channel();
        if sender
            .send(LockEvent {
                actor,
                phase,
                isolation,
                transaction_id,
                backend_pid,
                database_oid: u32::try_from(database_oid)
                    .expect("database OID fits the PostgreSQL OID type"),
                lock_keys,
                resume,
            })
            .is_ok()
        {
            let _ = resumed.await;
        }
    }

    pub(crate) async fn row_checkpoint(tx: &mut Transaction<'_, Postgres>, phase: RowLockPhase) {
        let Ok(actor) = ACTOR.try_with(|actor| *actor) else {
            return;
        };
        let should_report = match phase {
            RowLockPhase::Request => ROW_REQUEST_REPORTED
                .try_with(|reported| !reported.replace(true))
                .unwrap_or(false),
            RowLockPhase::Acquired => ROW_ACQUIRED_REPORTED
                .try_with(|reported| !reported.replace(true))
                .unwrap_or(false),
        };
        if !should_report {
            return;
        }
        let sender = row_controller()
            .lock()
            .expect("lock row test controller")
            .clone();
        let Some(sender) = sender else {
            return;
        };
        let (backend_pid, database_oid, transaction_id): (i32, i64, i64) = sqlx::query_as(
            "SELECT pg_backend_pid(), oid::BIGINT, txid_current()::BIGINT \
             FROM pg_database WHERE datname=current_database()",
        )
        .fetch_one(&mut **tx)
        .await
        .expect("read test row-lock backend identity");
        let (resume, resumed) = oneshot::channel();
        if sender
            .send(RowLockEvent {
                actor,
                phase,
                transaction_id,
                backend_pid,
                database_oid: u32::try_from(database_oid)
                    .expect("database OID fits the PostgreSQL OID type"),
                resume,
            })
            .is_ok()
        {
            let _ = resumed.await;
        }
    }
}

/// Binding source when the IdP JWT carries the pubkey claim.
pub const SOURCE_JWT_NPUB: &str = "jwt_npub";
/// Binding source when the relay falls back to the stored uid/pubkey binding.
pub const SOURCE_DB_BINDING: &str = "db_binding";

/// PostgreSQL-backed trust root for current enrollment policy and binding state.
///
/// The adapter owns no request-selectable configuration. The application constructs one
/// long-lived instance from the writer pool at application startup and injects
/// that instance into the single authorization runtime.
#[derive(Clone)]
pub struct PostgresFederatedAuthorityAdapter {
    pool: PgPool,
}

impl PostgresFederatedAuthorityAdapter {
    /// Bind the authority adapter to the authoritative writer pool.
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Server-resolved first-enrollment policy for one authorization domain.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EnrollmentMode {
    /// Require verified issuer evidence that attests the proven key.
    AttestedKey,
    /// Require an out-of-band provisioned binding.
    Provisioned,
    /// Allow trust on first use.
    Tofu,
}

impl fmt::Debug for EnrollmentMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("EnrollmentMode")
            .field(&"[redacted]")
            .finish()
    }
}

/// Provider-neutral persisted binding provenance.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BindingProvenance {
    /// Verified issuer evidence attested the key.
    AttestedKey,
    /// The binding was provisioned out of band.
    Provisioned,
    /// The binding was established by trust on first use.
    Tofu,
}

impl BindingProvenance {
    /// Stable persistence label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AttestedKey => "attested_key",
            Self::Provisioned => "provisioned",
            Self::Tofu => "tofu",
        }
    }

    pub(crate) fn legacy_source(self) -> &'static str {
        match self {
            Self::AttestedKey => SOURCE_JWT_NPUB,
            Self::Provisioned | Self::Tofu => SOURCE_DB_BINDING,
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "attested_key" => Ok(Self::AttestedKey),
            "provisioned" => Ok(Self::Provisioned),
            "tofu" => Ok(Self::Tofu),
            _ => Err(DbError::InvalidData(
                "identity binding has invalid provenance".to_string(),
            )),
        }
    }
}

impl fmt::Debug for BindingProvenance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BindingProvenance")
            .field(&"[redacted]")
            .finish()
    }
}

/// Explicit persisted binding lifecycle state.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BindingState {
    /// The binding currently carries authority.
    Active,
    /// The binding was retired without an atomic replacement.
    Revoked,
    /// The binding was atomically replaced.
    Rotated,
    /// The inactive binding was archived with explicit attribution.
    Archived,
}

impl BindingState {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "active" => Ok(Self::Active),
            "revoked" => Ok(Self::Revoked),
            "rotated" => Ok(Self::Rotated),
            "archived" => Ok(Self::Archived),
            _ => Err(DbError::InvalidData(
                "identity binding has invalid lifecycle state".to_string(),
            )),
        }
    }
}

/// Truthful provenance for immutable binding-creation attribution.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CreationAttributionKind {
    /// The legacy row predates trustworthy actor and policy attribution.
    LegacyUnknown,
    /// A directly authenticated key created the binding under verified policy.
    AuthenticatedKey,
    /// A verified operator lifecycle action created the binding.
    Operator,
}

impl CreationAttributionKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyUnknown => "legacy_unknown",
            Self::AuthenticatedKey => "authenticated_key",
            Self::Operator => "operator",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "legacy_unknown" => Ok(Self::LegacyUnknown),
            "authenticated_key" => Ok(Self::AuthenticatedKey),
            "operator" => Ok(Self::Operator),
            _ => Err(DbError::InvalidData(
                "identity binding has invalid creation attribution".to_string(),
            )),
        }
    }
}

impl fmt::Debug for CreationAttributionKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("CreationAttributionKind")
            .field(&"[redacted]")
            .finish()
    }
}

impl fmt::Debug for BindingState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BindingState")
            .field(&"[redacted]")
            .finish()
    }
}

/// Stable evidence returned by authoritative binding resolution.
#[derive(PartialEq, Eq)]
pub struct BindingEvidence {
    pub(crate) authorization_domain: CommunityId,
    pub(crate) issuer: String,
    pub(crate) subject: String,
    pub(crate) bound_pubkey: Vec<u8>,
    pub(crate) binding_id: Uuid,
    pub(crate) binding_version: u64,
    pub(crate) binding_state: BindingState,
    pub(crate) provenance: BindingProvenance,
    pub(crate) creation_attribution: CreationAttributionKind,
    pub(crate) created_by: Option<Vec<u8>>,
    pub(crate) created_policy_version: Option<String>,
    pub(crate) expires_at: Option<DateTime<Utc>>,
    pub(crate) created_at: DateTime<Utc>,
}

impl BindingEvidence {
    /// Authorization domain whose active binding was resolved.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Exact private issuer. Callers must not disclose it publicly.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Exact private subject. Callers must not disclose it publicly.
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// Exact bound key.
    pub fn bound_pubkey(&self) -> &[u8] {
        &self.bound_pubkey
    }

    /// Stable non-nil binding identifier.
    pub const fn binding_id(&self) -> Uuid {
        self.binding_id
    }

    /// Positive version local to the stable binding identifier.
    pub const fn binding_version(&self) -> u64 {
        self.binding_version
    }

    /// Explicit authoritative lifecycle state.
    pub const fn binding_state(&self) -> BindingState {
        self.binding_state
    }

    /// Persisted provider-neutral provenance.
    pub const fn provenance(&self) -> BindingProvenance {
        self.provenance
    }

    /// Truthful creation-attribution classification.
    pub const fn creation_attribution(&self) -> CreationAttributionKind {
        self.creation_attribution
    }

    /// Verified creation actor, absent only for explicitly unknown legacy rows.
    pub fn created_by(&self) -> Option<&[u8]> {
        self.created_by.as_deref()
    }

    /// Exact creation policy, absent only for explicitly unknown legacy rows.
    pub fn created_policy_version(&self) -> Option<&str> {
        self.created_policy_version.as_deref()
    }

    /// Optional authoritative binding-expiry boundary.
    pub const fn expires_at(&self) -> Option<&DateTime<Utc>> {
        self.expires_at.as_ref()
    }

    /// Immutable creation timestamp persisted by PostgreSQL.
    pub const fn created_at(&self) -> &DateTime<Utc> {
        &self.created_at
    }
}

impl fmt::Debug for BindingEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BindingEvidence")
            .field("binding_id", &"[redacted]")
            .field("binding_version", &"[redacted]")
            .field("binding_state", &"[redacted]")
            .field("provenance", &"[redacted]")
            .field("authorization_domain", &"[redacted]")
            .field("principal", &"[redacted]")
            .field("bound_pubkey", &"[redacted]")
            .field("creation_attribution", &"[redacted]")
            .field("expires_at", &"[redacted]")
            .finish()
    }
}

/// Stable denial from ordinary binding resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingDenial {
    /// Another active principal or key owns the requested coordinate.
    Conflict,
    /// A lifecycle selector or migration quarantine denies authority.
    Revoked,
    /// The domain requires an out-of-band binding.
    BindingRequired,
    /// Attested enrollment lacked a verified matching key claim.
    KeyAttestationRequired,
    /// The exact active binding remains slot-occupying but is no longer
    /// authorization-eligible at database time.
    BindingExpired,
    /// Authorization or assertion evidence was no longer current at the
    /// database mutation boundary.
    StaleEvidence,
}

/// Result of resolving a binding during ordinary authorization.
#[derive(Debug, PartialEq, Eq)]
pub enum ResolveBindingResult {
    /// A new authoritative binding was atomically enrolled.
    Enrolled(BindingEvidence),
    /// The exact authoritative binding already existed.
    Existing(BindingEvidence),
    /// Resolution denied without authority mutation.
    Denied(BindingDenial),
}

/// Typed input to ordinary binding resolution.
pub(crate) struct ResolveBindingInput<'a> {
    pub(crate) authorization_domain: CommunityId,
    pub(crate) issuer: &'a str,
    pub(crate) subject: &'a str,
    pub(crate) pubkey: &'a [u8],
    pub(crate) display_name: Option<&'a str>,
    pub(crate) enrollment_mode: EnrollmentMode,
    pub(crate) key_attested: bool,
    pub(crate) policy_version: &'a str,
    pub(crate) evidence_valid_from: u64,
    pub(crate) evidence_valid_until: u64,
}

impl fmt::Debug for ResolveBindingInput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolveBindingInput")
            .field("issuer", &"[redacted]")
            .field("subject", &"[redacted]")
            .field("pubkey", &"[redacted]")
            .field("display_name", &"[redacted]")
            .field("enrollment_mode", &"[redacted]")
            .field("key_attested", &"[redacted]")
            .field("policy_version", &"[redacted]")
            .field("evidence_valid_from", &"[redacted]")
            .field("evidence_valid_until", &"[redacted]")
            .finish()
    }
}

/// Active corporate identity binding row.
#[derive(Clone, PartialEq, Eq)]
pub struct IdentityBinding {
    /// Stable non-nil binding identifier.
    pub binding_id: Uuid,
    /// Validated identity-provider issuer.
    pub issuer: String,
    /// Corporate IdP subject or configured stable uid claim.
    pub uid: String,
    /// Bound Nostr pubkey bytes.
    pub pubkey: Vec<u8>,
    /// Positive authorization-relevant version.
    pub binding_version: u64,
    /// Explicit lifecycle state.
    pub binding_state: BindingState,
    /// Provider-neutral provenance.
    pub binding_provenance: BindingProvenance,
    /// Truthful creation-attribution classification.
    pub creation_attribution: CreationAttributionKind,
    /// Verified creation actor, absent only for legacy-unknown attribution.
    pub created_by: Option<Vec<u8>>,
    /// Verified creation policy, absent only for legacy-unknown attribution.
    pub created_policy_version: Option<String>,
    /// Optional authoritative authorization-eligibility expiry.
    pub expires_at: Option<DateTime<Utc>>,
    /// Human-readable display claim captured from the latest accepted JWT.
    pub display_name: Option<String>,
    /// Source that established or last strengthened the active binding.
    pub source: String,
    /// When the binding was first created.
    pub created_at: DateTime<Utc>,
    /// When the binding row was last updated.
    pub updated_at: DateTime<Utc>,
    /// When the binding was last seen during authentication.
    pub last_seen_at: DateTime<Utc>,
}

impl fmt::Debug for IdentityBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IdentityBinding")
            .field("binding_id", &"[redacted]")
            .field("issuer", &"[redacted]")
            .field("uid", &"[redacted]")
            .field("pubkey", &"[redacted]")
            .field("binding_version", &"[redacted]")
            .field("binding_state", &"[redacted]")
            .field("binding_provenance", &"[redacted]")
            .field("display_name", &"[redacted]")
            .field("source", &"[redacted]")
            .field("timestamps", &"[redacted]")
            .finish()
    }
}

/// Party-data-free marker for an active binding conflict.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct IdentityBindingConflict;

impl fmt::Debug for IdentityBindingConflict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("IdentityBindingConflict")
            .field(&"[redacted]")
            .finish()
    }
}

/// Outcome of creating or validating a corporate identity binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindIdentityResult {
    /// A new active binding was created.
    Created,
    /// The requested binding matched an existing active binding.
    Matched,
    /// Another active binding already owns the uid or pubkey.
    Conflict(IdentityBindingConflict),
    /// The requested uid/pubkey pair was previously revoked.
    Revoked,
    /// No active binding exists; sealed authorized-domain evidence is required
    /// before first enrollment.
    BindingRequired,
}

/// Corporate identity data staged for an atomic admission transaction.
#[derive(Clone, Copy)]
pub struct IdentityBindingInput<'a> {
    /// Validated identity-provider issuer.
    pub issuer: &'a str,
    /// Stable issuer-qualified principal identifier.
    pub uid: &'a str,
    /// Authenticated Nostr pubkey bytes.
    pub pubkey: &'a [u8],
    /// Private display attribute retained in the binding table.
    pub display_name: Option<&'a str>,
    /// Legacy verifier source (`jwt_npub` or `db_binding`). This compatibility
    /// field never grants attested provenance; only sealed authorization evidence can.
    pub source: &'a str,
}

impl fmt::Debug for IdentityBindingInput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IdentityBindingInput")
            .field("issuer", &"[redacted]")
            .field("uid", &"[redacted]")
            .field("pubkey", &"[redacted]")
            .field("display_name", &"[redacted]")
            .field("source", &"[redacted]")
            .finish()
    }
}

fn validate_inputs(issuer: &str, uid: &str, pubkey: &[u8], source: &str) -> Result<()> {
    if issuer.is_empty() {
        return Err(DbError::InvalidData(
            "identity binding issuer must not be empty".to_string(),
        ));
    }
    if uid.is_empty() {
        return Err(DbError::InvalidData(
            "identity binding uid must not be empty".to_string(),
        ));
    }
    validate_pubkey(pubkey)?;
    if !matches!(source, SOURCE_JWT_NPUB | SOURCE_DB_BINDING) {
        return Err(DbError::InvalidData(
            "invalid identity binding source".to_string(),
        ));
    }
    Ok(())
}

fn validate_pubkey(pubkey: &[u8]) -> Result<()> {
    if pubkey.len() != 32 {
        return Err(DbError::InvalidData(
            "identity binding pubkey must be 32 bytes".to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_membership_identity_key(
    member_pubkey_hex: &str,
    identity: Option<&IdentityBindingInput<'_>>,
) -> Result<()> {
    let Some(identity) = identity else {
        return Ok(());
    };
    let member_pubkey = hex::decode(member_pubkey_hex)
        .map_err(|_| DbError::InvalidData("membership pubkey must be 32-byte hex".to_string()))?;
    if member_pubkey.len() != 32 || member_pubkey.as_slice() != identity.pubkey {
        return Err(DbError::InvalidData(
            "membership pubkey does not match staged identity key".to_string(),
        ));
    }
    Ok(())
}

fn row_to_binding(row: sqlx::postgres::PgRow) -> Result<IdentityBinding> {
    let binding_id: Uuid = row.try_get("binding_id")?;
    let binding_version: i64 = row.try_get("binding_version")?;
    if binding_id.is_nil() || binding_version <= 0 {
        return Err(DbError::InvalidData(
            "identity binding has invalid stable evidence".to_string(),
        ));
    }
    let creation_attribution =
        CreationAttributionKind::parse(row.try_get("creation_attribution_kind")?)?;
    let created_by: Option<Vec<u8>> = row.try_get("created_by")?;
    let created_policy_version: Option<String> = row.try_get("created_policy_version")?;
    let attribution_is_complete = match creation_attribution {
        CreationAttributionKind::LegacyUnknown => {
            created_by.is_none() && created_policy_version.is_none()
        }
        CreationAttributionKind::AuthenticatedKey | CreationAttributionKind::Operator => {
            created_by.as_ref().is_some_and(|actor| actor.len() == 32)
                && created_policy_version
                    .as_ref()
                    .is_some_and(|version| !version.is_empty())
        }
    };
    if !attribution_is_complete {
        return Err(DbError::InvalidData(
            "identity binding has incomplete creation attribution".to_string(),
        ));
    }
    Ok(IdentityBinding {
        binding_id,
        issuer: row.try_get("issuer")?,
        uid: row.try_get("uid")?,
        pubkey: row.try_get("pubkey")?,
        binding_version: u64::try_from(binding_version).map_err(|_| {
            DbError::InvalidData("identity binding version is out of range".to_string())
        })?,
        binding_state: BindingState::parse(row.try_get("binding_state")?)?,
        binding_provenance: BindingProvenance::parse(row.try_get("binding_provenance")?)?,
        creation_attribution,
        created_by,
        created_policy_version,
        expires_at: row.try_get("expires_at")?,
        display_name: row.try_get("display_name")?,
        source: row.try_get("source")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        last_seen_at: row.try_get("last_seen_at")?,
    })
}

fn evidence_from_binding(
    authorization_domain: CommunityId,
    binding: &IdentityBinding,
    binding_version: u64,
    provenance: BindingProvenance,
) -> BindingEvidence {
    BindingEvidence {
        authorization_domain,
        issuer: binding.issuer.clone(),
        subject: binding.uid.clone(),
        bound_pubkey: binding.pubkey.clone(),
        binding_id: binding.binding_id,
        binding_version,
        binding_state: BindingState::Active,
        provenance,
        creation_attribution: binding.creation_attribution,
        created_by: binding.created_by.clone(),
        created_policy_version: binding.created_policy_version.clone(),
        expires_at: binding.expires_at,
        created_at: binding.created_at.to_owned(),
    }
}

async fn active_by_principal_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    issuer: &str,
    uid: &str,
) -> Result<Option<IdentityBinding>> {
    let row = sqlx::query(
        r#"
        SELECT binding_id, issuer, uid, pubkey, binding_version, binding_state,
               binding_provenance, creation_attribution_kind, created_by,
               created_policy_version, expires_at, display_name, source,
               created_at, updated_at, last_seen_at
        FROM identity_bindings
        WHERE community_id = $1 AND issuer = $2 AND uid = $3
          AND binding_state = 'active' AND revoked_at IS NULL
        FOR UPDATE
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(issuer)
    .bind(uid)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(row_to_binding).transpose()
}

async fn active_by_pubkey_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    pubkey: &[u8],
) -> Result<Option<IdentityBinding>> {
    let row = sqlx::query(
        r#"
        SELECT binding_id, issuer, uid, pubkey, binding_version, binding_state,
               binding_provenance, creation_attribution_kind, created_by,
               created_policy_version, expires_at, display_name, source,
               created_at, updated_at, last_seen_at
        FROM identity_bindings
        WHERE community_id = $1 AND pubkey = $2
          AND binding_state = 'active' AND revoked_at IS NULL
        FOR UPDATE
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(row_to_binding).transpose()
}

async fn principal_disabled_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    issuer: &str,
    uid: &str,
) -> Result<bool> {
    let row = sqlx::query(
        r#"
        SELECT 1 FROM identity_principals
        WHERE community_id = $1 AND issuer = $2 AND uid = $3
          AND disabled_at IS NOT NULL
        UNION ALL
        SELECT 1 FROM identity_bindings legacy
        WHERE legacy.community_id = $1 AND legacy.issuer = $2 AND legacy.uid = $3
          AND legacy.revoked_at IS NOT NULL AND legacy.revocation_scope = 'principal'
          AND NOT EXISTS (
              SELECT 1 FROM identity_principals current
              WHERE current.community_id = legacy.community_id
                AND current.issuer = legacy.issuer
                AND current.uid = legacy.uid
          )
        LIMIT 1
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(issuer)
    .bind(uid)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.is_some())
}

async fn key_revoked_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    pubkey: &[u8],
) -> Result<bool> {
    let row = sqlx::query(
        r#"
        SELECT 1 FROM identity_revoked_keys
        WHERE community_id = $1 AND pubkey = $2
        LIMIT 1
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.is_some())
}

async fn principal_requires_rotation_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    issuer: &str,
    uid: &str,
) -> Result<bool> {
    let row = sqlx::query(
        r#"
        SELECT 1
        FROM identity_pending_replacements
        WHERE community_id = $1
          AND issuer = $2
          AND subject = $3
          AND cleared_at IS NULL
        LIMIT 1
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(issuer)
    .bind(uid)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.is_some())
}

async fn retired_pair_exists_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    issuer: &str,
    subject: &str,
    pubkey: &[u8],
) -> Result<bool> {
    Ok(sqlx::query(
        "SELECT 1 FROM identity_retired_pairs \
         WHERE community_id=$1 AND issuer=$2 AND subject=$3 AND pubkey=$4 \
         UNION ALL \
         SELECT 1 FROM identity_bindings \
         WHERE community_id=$1 AND issuer=$2 AND uid=$3 AND pubkey=$4 \
           AND revoked_at IS NOT NULL \
         LIMIT 1",
    )
    .bind(community_id.as_uuid())
    .bind(issuer)
    .bind(subject)
    .bind(pubkey)
    .fetch_optional(&mut **tx)
    .await?
    .is_some())
}

async fn migration_denied_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    issuer: &str,
    subject: &str,
) -> Result<bool> {
    Ok(sqlx::query(
        "SELECT 1 FROM identity_migration_denials \
         WHERE community_id=$1 AND issuer=$2 AND subject=$3",
    )
    .bind(community_id.as_uuid())
    .bind(issuer)
    .bind(subject)
    .fetch_optional(&mut **tx)
    .await?
    .is_some())
}

async fn migration_key_denied_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    pubkey: &[u8],
) -> Result<bool> {
    Ok(sqlx::query(
        "SELECT 1 FROM identity_migration_denied_keys \
         WHERE community_id=$1 AND pubkey=$2",
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .fetch_optional(&mut **tx)
    .await?
    .is_some())
}

const IDENTITY_LOCK_ENCODING_VERSION: u8 = 1;

fn identity_lock_coordinate(kind: u8, community_id: CommunityId, parts: &[&[u8]]) -> Vec<u8> {
    let mut coordinate =
        Vec::with_capacity(2 + 16 + parts.iter().map(|part| 8 + part.len()).sum::<usize>());
    coordinate.push(IDENTITY_LOCK_ENCODING_VERSION);
    coordinate.push(kind);
    coordinate.extend_from_slice(community_id.as_uuid().as_bytes());
    for part in parts {
        let length = part.len() as u64;
        coordinate.extend_from_slice(&length.to_be_bytes());
        coordinate.extend_from_slice(part);
    }
    coordinate
}

pub(crate) fn principal_lock_coordinate(
    community_id: CommunityId,
    issuer: &str,
    subject: &str,
) -> Vec<u8> {
    identity_lock_coordinate(1, community_id, &[issuer.as_bytes(), subject.as_bytes()])
}

pub(crate) fn key_lock_coordinate(community_id: CommunityId, pubkey: &[u8]) -> Vec<u8> {
    identity_lock_coordinate(2, community_id, &[pubkey])
}

pub(crate) fn binding_lock_coordinate(community_id: CommunityId, binding_id: Uuid) -> Vec<u8> {
    identity_lock_coordinate(3, community_id, &[binding_id.as_bytes()])
}

pub(crate) fn operation_lock_coordinate(community_id: CommunityId, operation_id: Uuid) -> Vec<u8> {
    identity_lock_coordinate(4, community_id, &[operation_id.as_bytes()])
}

pub(crate) async fn lock_identity_coordinates_tx(
    tx: &mut Transaction<'_, Postgres>,
    mut coordinates: Vec<Vec<u8>>,
) -> Result<()> {
    coordinates.sort();
    coordinates.dedup();
    #[cfg(test)]
    test_lock_schedule::checkpoint(tx, test_lock_schedule::LockPhase::Request, &coordinates).await;
    for coordinate in &coordinates {
        sqlx::query(
            "SELECT pg_advisory_xact_lock(\
                hashtext('buzz_nip_fi_v1'), hashtext(encode($1, 'hex'))\
            )",
        )
        .bind(coordinate.as_slice())
        .execute(&mut **tx)
        .await?;
    }
    #[cfg(test)]
    test_lock_schedule::checkpoint(tx, test_lock_schedule::LockPhase::Acquired, &coordinates).await;
    Ok(())
}

async fn lock_identity_keys_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    issuer: &str,
    uid: &str,
    pubkey: &[u8],
) -> Result<()> {
    lock_identity_coordinates_tx(
        tx,
        vec![
            principal_lock_coordinate(community_id, issuer, uid),
            key_lock_coordinate(community_id, pubkey),
        ],
    )
    .await
}

/// Validate an existing active corporate identity binding.
///
/// This compatibility operation cannot enroll or strengthen a binding because
/// it does not carry sealed authorization evidence:
/// - same issuer + uid + pubkey succeeds without mutating binding evidence;
/// - same issuer + uid with a different pubkey conflicts;
/// - same pubkey with a different issuer-qualified principal conflicts;
/// - principal disablement and unresolved key revocation reject every key;
/// - a previously revoked issuer/uid/pubkey tuple remains revoked;
/// - no active row returns [`BindIdentityResult::BindingRequired`].
pub async fn bind_or_validate_identity(
    pool: &PgPool,
    community_id: CommunityId,
    issuer: &str,
    uid: &str,
    pubkey: &[u8],
    display_name: Option<&str>,
    source: &str,
) -> Result<BindIdentityResult> {
    let mut tx = pool.begin().await?;
    let result = bind_or_validate_identity_tx(
        &mut tx,
        community_id,
        &IdentityBindingInput {
            issuer,
            uid,
            pubkey,
            display_name,
            source,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(result)
}

/// Validate an existing binding inside a caller-owned admission transaction.
pub(crate) async fn bind_or_validate_identity_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    identity: &IdentityBindingInput<'_>,
) -> Result<BindIdentityResult> {
    let IdentityBindingInput {
        issuer,
        uid,
        pubkey,
        display_name: _,
        source,
    } = *identity;
    validate_inputs(issuer, uid, pubkey, source)?;
    sqlx::query("SET LOCAL lock_timeout = '3s'")
        .execute(&mut **tx)
        .await?;
    lock_identity_keys_tx(tx, community_id, issuer, uid, pubkey).await?;

    let denied = migration_denied_tx(tx, community_id, issuer, uid).await?
        || migration_key_denied_tx(tx, community_id, pubkey).await?
        || principal_disabled_tx(tx, community_id, issuer, uid).await?
        || key_revoked_tx(tx, community_id, pubkey).await?
        || principal_requires_rotation_tx(tx, community_id, issuer, uid).await?
        || retired_pair_exists_tx(tx, community_id, issuer, uid, pubkey).await?;
    if denied {
        return Ok(BindIdentityResult::Revoked);
    }

    let active_principal = active_by_principal_tx(tx, community_id, issuer, uid).await?;
    if active_principal
        .as_ref()
        .is_some_and(|binding| binding.pubkey != pubkey)
    {
        return Ok(BindIdentityResult::Conflict(IdentityBindingConflict));
    }
    let active_key = active_by_pubkey_tx(tx, community_id, pubkey).await?;
    if active_key
        .as_ref()
        .is_some_and(|binding| binding.issuer != issuer || binding.uid != uid)
    {
        return Ok(BindIdentityResult::Conflict(IdentityBindingConflict));
    }

    Ok(if active_principal.is_some() {
        BindIdentityResult::Matched
    } else {
        BindIdentityResult::BindingRequired
    })
}

#[derive(Clone, Copy)]
struct AuthoritativeBindingRequestView<'a> {
    authorization_domain: CommunityId,
    issuer: &'a str,
    subject: &'a str,
    pubkey: [u8; 32],
    policy_id: Uuid,
    policy_epoch: u64,
    policy_requirement: FederatedIdentityRequirement,
    key_attested: bool,
    effective_from: u64,
    effective_until: u64,
}

impl<'a> AuthoritativeBindingRequestView<'a> {
    fn from_sealed(request: &'a BindingResolutionRequest) -> Self {
        Self {
            authorization_domain: request.authorization_domain(),
            issuer: request.principal().issuer(),
            subject: request.principal().subject(),
            pubkey: request.bound_pubkey().to_bytes(),
            policy_id: request.policy_id(),
            policy_epoch: request.policy_epoch(),
            policy_requirement: request.policy_requirement(),
            key_attested: request.key_attested(),
            effective_from: request.effective_from(),
            effective_until: request.effective_until(),
        }
    }
}

#[derive(Clone, Copy)]
struct CurrentEnrollmentPolicy {
    policy_id: Uuid,
    policy_epoch: u64,
    requirement: FederatedIdentityRequirement,
    effective_from: u64,
    effective_until: u64,
}

impl CurrentEnrollmentPolicy {
    fn from_row(row: sqlx::postgres::PgRow) -> Result<Self> {
        let policy_id: Uuid = row.try_get("policy_id")?;
        let policy_epoch: i64 = row.try_get("policy_epoch")?;
        let effective_from: i64 = row.try_get("effective_from")?;
        let effective_until: i64 = row.try_get("effective_until")?;
        if policy_id.is_nil()
            || policy_epoch <= 0
            || effective_from < 0
            || effective_from >= effective_until
        {
            return Err(DbError::InvalidData(
                "identity enrollment policy has invalid authoritative state".to_string(),
            ));
        }
        Ok(Self {
            policy_id,
            policy_epoch: u64::try_from(policy_epoch).map_err(|_| {
                DbError::InvalidData("identity enrollment policy epoch is out of range".to_string())
            })?,
            requirement: policy_requirement(row.try_get("requirement")?)?,
            effective_from: u64::try_from(effective_from).map_err(|_| {
                DbError::InvalidData(
                    "identity enrollment policy lower bound is out of range".to_string(),
                )
            })?,
            effective_until: u64::try_from(effective_until).map_err(|_| {
                DbError::InvalidData(
                    "identity enrollment policy upper bound is out of range".to_string(),
                )
            })?,
        })
    }

    fn matches_binding_request(&self, request: &AuthoritativeBindingRequestView<'_>) -> bool {
        self.policy_id == request.policy_id
            && self.policy_epoch == request.policy_epoch
            && self.requirement == request.policy_requirement
            && request.effective_from >= self.effective_from
            && request.effective_until <= self.effective_until
            && request.effective_from < request.effective_until
    }
}

fn policy_requirement(value: &str) -> Result<FederatedIdentityRequirement> {
    match value {
        "not_required" => Ok(FederatedIdentityRequirement::NotRequired),
        "attested_key" => Ok(FederatedIdentityRequirement::Required(
            AuthorizedEnrollmentMode::AttestedKey,
        )),
        "provisioned" => Ok(FederatedIdentityRequirement::Required(
            AuthorizedEnrollmentMode::Provisioned,
        )),
        "tofu" => Ok(FederatedIdentityRequirement::Required(
            AuthorizedEnrollmentMode::Tofu,
        )),
        _ => Err(DbError::InvalidData(
            "identity enrollment policy has invalid requirement".to_string(),
        )),
    }
}

fn storage_enrollment_mode(
    requirement: FederatedIdentityRequirement,
) -> std::result::Result<EnrollmentMode, buzz_auth::AuthContextError> {
    match requirement {
        FederatedIdentityRequirement::Required(AuthorizedEnrollmentMode::AttestedKey) => {
            Ok(EnrollmentMode::AttestedKey)
        }
        FederatedIdentityRequirement::Required(AuthorizedEnrollmentMode::Provisioned) => {
            Ok(EnrollmentMode::Provisioned)
        }
        FederatedIdentityRequirement::Required(AuthorizedEnrollmentMode::Tofu) => {
            Ok(EnrollmentMode::Tofu)
        }
        FederatedIdentityRequirement::NotRequired => {
            Err(buzz_auth::AuthContextError::UnexpectedFederatedAuthorization)
        }
    }
}

async fn read_current_enrollment_policy(
    pool: &PgPool,
    authorization_domain: CommunityId,
) -> Result<CurrentEnrollmentPolicy> {
    let row = sqlx::query(
        "SELECT policy_id, policy_epoch, requirement, effective_from, effective_until \
         FROM identity_enrollment_policies WHERE community_id=$1",
    )
    .bind(authorization_domain.as_uuid())
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        DbError::InvalidData("current identity enrollment policy is unavailable".to_string())
    })?;
    CurrentEnrollmentPolicy::from_row(row)
}

async fn lock_current_enrollment_policy_tx(
    tx: &mut Transaction<'_, Postgres>,
    authorization_domain: CommunityId,
) -> Result<Option<CurrentEnrollmentPolicy>> {
    sqlx::query(
        "SELECT policy_id, policy_epoch, requirement, effective_from, effective_until \
         FROM identity_enrollment_policies WHERE community_id=$1 FOR UPDATE",
    )
    .bind(authorization_domain.as_uuid())
    .fetch_optional(&mut **tx)
    .await?
    .map(CurrentEnrollmentPolicy::from_row)
    .transpose()
}

enum PolicyPreconditionState {
    Current,
    Changed,
    NotYetEffective,
    Expired,
}

async fn policy_precondition_state_tx(
    tx: &mut Transaction<'_, Postgres>,
    request: &AuthoritativeBindingRequestView<'_>,
) -> Result<PolicyPreconditionState> {
    let Some(policy) = lock_current_enrollment_policy_tx(tx, request.authorization_domain).await?
    else {
        return Ok(PolicyPreconditionState::Changed);
    };
    if !policy.matches_binding_request(request) {
        return Ok(PolicyPreconditionState::Changed);
    }
    let now: i64 =
        sqlx::query_scalar("SELECT FLOOR(EXTRACT(EPOCH FROM clock_timestamp()))::BIGINT")
            .fetch_one(&mut **tx)
            .await?;
    let effective_from = i64::try_from(request.effective_from).map_err(|_| {
        DbError::InvalidData("identity evidence lower bound is out of range".to_string())
    })?;
    let effective_until = i64::try_from(request.effective_until).map_err(|_| {
        DbError::InvalidData("identity evidence upper bound is out of range".to_string())
    })?;
    Ok(if now < effective_from {
        PolicyPreconditionState::NotYetEffective
    } else if now >= effective_until {
        PolicyPreconditionState::Expired
    } else {
        PolicyPreconditionState::Current
    })
}

fn enforce_policy_precondition(
    state: PolicyPreconditionState,
) -> std::result::Result<(), AuthorityAdapterError<DbError>> {
    match state {
        PolicyPreconditionState::Current => Ok(()),
        PolicyPreconditionState::Changed => Err(AuthorityAdapterError::policy_changed()),
        PolicyPreconditionState::NotYetEffective => {
            Err(buzz_auth::AuthContextError::FederatedPolicyNotYetEffective.into())
        }
        PolicyPreconditionState::Expired => {
            Err(buzz_auth::AuthContextError::FederatedPolicyExpired.into())
        }
    }
}

async fn resolve_authoritative_binding_request(
    pool: &PgPool,
    request: &BindingResolutionRequest,
    enrollment_allowed: bool,
) -> std::result::Result<ResolveBindingResult, AuthorityAdapterError<DbError>> {
    resolve_authoritative_binding_view(
        pool,
        &AuthoritativeBindingRequestView::from_sealed(request),
        enrollment_allowed,
    )
    .await
}

async fn resolve_authoritative_binding_view(
    pool: &PgPool,
    request: &AuthoritativeBindingRequestView<'_>,
    enrollment_allowed: bool,
) -> std::result::Result<ResolveBindingResult, AuthorityAdapterError<DbError>> {
    let storage_mode = if enrollment_allowed {
        storage_enrollment_mode(request.policy_requirement)?
    } else {
        EnrollmentMode::Provisioned
    };
    let policy_version = format!("{}:{}", request.policy_id, request.policy_epoch);
    let input = ResolveBindingInput {
        authorization_domain: request.authorization_domain,
        issuer: request.issuer,
        subject: request.subject,
        pubkey: &request.pubkey,
        display_name: None,
        enrollment_mode: storage_mode,
        key_attested: request.key_attested,
        policy_version: &policy_version,
        evidence_valid_from: request.effective_from,
        evidence_valid_until: request.effective_until,
    };
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| AuthorityAdapterError::adapter(error.into()))?;
    sqlx::query("SET LOCAL lock_timeout = '3s'")
        .execute(&mut *tx)
        .await
        .map_err(|error| AuthorityAdapterError::adapter(error.into()))?;
    let initial = policy_precondition_state_tx(&mut tx, request)
        .await
        .map_err(AuthorityAdapterError::adapter)?;
    enforce_policy_precondition(initial)?;
    let result = resolve_identity_binding_tx(&mut tx, request.authorization_domain, &input, true)
        .await
        .map_err(AuthorityAdapterError::adapter)?;
    if matches!(
        result,
        ResolveBindingResult::Existing(_) | ResolveBindingResult::Enrolled(_)
    ) {
        let final_state = policy_precondition_state_tx(&mut tx, request)
            .await
            .map_err(AuthorityAdapterError::adapter)?;
        enforce_policy_precondition(final_state)?;
        tx.commit()
            .await
            .map_err(|error| AuthorityAdapterError::adapter(error.into()))?;
    } else {
        tx.rollback()
            .await
            .map_err(|error| AuthorityAdapterError::adapter(error.into()))?;
    }
    Ok(result)
}

fn authorized_binding_parts(
    evidence: &BindingEvidence,
) -> std::result::Result<
    (
        AuthorizedBindingVersion,
        Option<AuthorizedBindingExpiry>,
        AuthorizedBindingSource,
    ),
    buzz_auth::AuthContextError,
> {
    let binding_version = AuthorizedBindingVersion::new(evidence.binding_version)?;
    let expires_at = evidence
        .expires_at
        .map(|value| {
            u64::try_from(value.timestamp())
                .map_err(|_| buzz_auth::AuthContextError::InvalidBindingExpiry)
                .and_then(AuthorizedBindingExpiry::new)
        })
        .transpose()?;
    let source = match evidence.provenance {
        BindingProvenance::AttestedKey => AuthorizedBindingSource::AttestedKey,
        BindingProvenance::Provisioned => AuthorizedBindingSource::Provisioned,
        BindingProvenance::Tofu => AuthorizedBindingSource::Tofu,
    };
    Ok((binding_version, expires_at, source))
}

fn authority_denial(denial: BindingDenial, delegated: bool) -> AuthorityAdapterError<DbError> {
    match denial {
        BindingDenial::KeyAttestationRequired => {
            buzz_auth::AuthContextError::KeyAttestationRequired.into()
        }
        BindingDenial::BindingExpired => buzz_auth::AuthContextError::BindingExpired.into(),
        BindingDenial::StaleEvidence => buzz_auth::AuthContextError::FederatedPolicyExpired.into(),
        BindingDenial::BindingRequired if delegated => {
            buzz_auth::AuthContextError::DelegatedBindingNotExistingActive.into()
        }
        BindingDenial::Conflict | BindingDenial::Revoked | BindingDenial::BindingRequired => {
            AuthorityAdapterError::adapter(DbError::InvalidData(
                "identity binding resolution denied".to_string(),
            ))
        }
    }
}

impl FederatedAuthorityAdapter for PostgresFederatedAuthorityAdapter {
    type Error = DbError;

    fn resolve_current_policy<'a>(
        &'a self,
        request: CurrentPolicyRequest,
        sink: CurrentPolicyResolutionSink,
    ) -> AuthorityAdapterFuture<
        'a,
        std::result::Result<ResolvedFederatedPolicy, AuthorityAdapterError<Self::Error>>,
    > {
        Box::pin(async move {
            let policy = read_current_enrollment_policy(&self.pool, request.authorization_domain())
                .await
                .map_err(AuthorityAdapterError::adapter)?;
            sink.resolved(
                request.authorization_domain(),
                policy.policy_id,
                policy.policy_epoch,
                policy.requirement,
                policy.effective_from,
                policy.effective_until,
            )
            .map_err(Into::into)
        })
    }

    fn resolve_direct_binding<'a>(
        &'a self,
        request: BindingResolutionRequest,
        sink: DirectBindingResolutionSink,
    ) -> AuthorityAdapterFuture<
        'a,
        std::result::Result<AuthoritativeBindingResolution, AuthorityAdapterError<Self::Error>>,
    > {
        Box::pin(async move {
            let result = resolve_authoritative_binding_request(&self.pool, &request, true).await?;
            let evidence = match result {
                ResolveBindingResult::Existing(evidence) => (evidence, false),
                ResolveBindingResult::Enrolled(evidence) => (evidence, true),
                ResolveBindingResult::Denied(denial) => {
                    return Err(authority_denial(denial, false))
                }
            };
            let (binding_version, expires_at, source) = authorized_binding_parts(&evidence.0)?;
            let seal = if evidence.1 {
                DirectBindingResolutionSink::atomically_enrolled
            } else {
                DirectBindingResolutionSink::existing_active
            };
            seal(
                sink,
                evidence.0.authorization_domain,
                evidence.0.binding_id,
                request.principal().clone(),
                request.bound_pubkey(),
                binding_version,
                expires_at,
                source,
            )
            .map_err(Into::into)
        })
    }

    fn resolve_existing_binding<'a>(
        &'a self,
        request: BindingResolutionRequest,
        sink: ExistingBindingResolutionSink,
    ) -> AuthorityAdapterFuture<
        'a,
        std::result::Result<AuthoritativeBindingResolution, AuthorityAdapterError<Self::Error>>,
    > {
        Box::pin(async move {
            let result = resolve_authoritative_binding_request(&self.pool, &request, false).await?;
            let evidence = match result {
                ResolveBindingResult::Existing(evidence) => evidence,
                ResolveBindingResult::Enrolled(_) => {
                    return Err(
                        buzz_auth::AuthContextError::DelegatedBindingNotExistingActive.into(),
                    )
                }
                ResolveBindingResult::Denied(denial) => return Err(authority_denial(denial, true)),
            };
            let (binding_version, expires_at, source) = authorized_binding_parts(&evidence)?;
            sink.existing_active(
                evidence.authorization_domain,
                evidence.binding_id,
                request.principal().clone(),
                request.bound_pubkey(),
                binding_version,
                expires_at,
                source,
            )
            .map_err(Into::into)
        })
    }
}

/// Resolve an exact issuer/subject/key binding under server-owned enrollment policy.
#[cfg(test)]
pub(crate) async fn resolve_identity_binding(
    pool: &PgPool,
    input: &ResolveBindingInput<'_>,
) -> Result<ResolveBindingResult> {
    let mut tx = pool.begin().await?;
    let result =
        resolve_identity_binding_tx(&mut tx, input.authorization_domain, input, false).await?;
    if matches!(
        &result,
        ResolveBindingResult::Enrolled(_) | ResolveBindingResult::Existing(_)
    ) && !evidence_is_current_tx(&mut tx, input).await?
    {
        tx.rollback().await?;
        return Ok(ResolveBindingResult::Denied(BindingDenial::StaleEvidence));
    }
    tx.commit().await?;
    Ok(result)
}

async fn evidence_is_current_tx(
    tx: &mut Transaction<'_, Postgres>,
    input: &ResolveBindingInput<'_>,
) -> Result<bool> {
    let evidence_valid_from = i64::try_from(input.evidence_valid_from).map_err(|_| {
        DbError::InvalidData(
            "identity evidence lower bound is outside the supported range".to_string(),
        )
    })?;
    let evidence_valid_until = i64::try_from(input.evidence_valid_until).map_err(|_| {
        DbError::InvalidData(
            "identity evidence upper bound is outside the supported range".to_string(),
        )
    })?;
    if evidence_valid_from >= evidence_valid_until {
        return Err(DbError::InvalidData(
            "identity binding authorization evidence has no valid interval".to_string(),
        ));
    }
    sqlx::query_scalar(
        "SELECT FLOOR(EXTRACT(EPOCH FROM clock_timestamp()))::BIGINT >= $1 \
                AND FLOOR(EXTRACT(EPOCH FROM clock_timestamp()))::BIGINT < $2",
    )
    .bind(evidence_valid_from)
    .bind(evidence_valid_until)
    .fetch_one(&mut **tx)
    .await
    .map_err(Into::into)
}

async fn resolve_identity_binding_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    input: &ResolveBindingInput<'_>,
    existing_read_only: bool,
) -> Result<ResolveBindingResult> {
    if community_id != input.authorization_domain || input.policy_version.is_empty() {
        return Err(DbError::InvalidData(
            "identity binding authorization domain or policy is invalid".to_string(),
        ));
    }
    validate_inputs(input.issuer, input.subject, input.pubkey, SOURCE_DB_BINDING)?;
    let evidence_valid_from = i64::try_from(input.evidence_valid_from).map_err(|_| {
        DbError::InvalidData(
            "identity evidence lower bound is outside the supported range".to_string(),
        )
    })?;
    let evidence_valid_until = i64::try_from(input.evidence_valid_until).map_err(|_| {
        DbError::InvalidData(
            "identity evidence upper bound is outside the supported range".to_string(),
        )
    })?;
    if evidence_valid_from >= evidence_valid_until {
        return Err(DbError::InvalidData(
            "identity binding authorization evidence has no valid interval".to_string(),
        ));
    }
    sqlx::query("SET LOCAL lock_timeout = '3s'")
        .execute(&mut **tx)
        .await?;
    lock_identity_keys_tx(tx, community_id, input.issuer, input.subject, input.pubkey).await?;
    if !evidence_is_current_tx(tx, input).await? {
        return Ok(ResolveBindingResult::Denied(BindingDenial::StaleEvidence));
    }

    let denied = migration_denied_tx(tx, community_id, input.issuer, input.subject).await?
        || migration_key_denied_tx(tx, community_id, input.pubkey).await?
        || principal_disabled_tx(tx, community_id, input.issuer, input.subject).await?
        || key_revoked_tx(tx, community_id, input.pubkey).await?
        || principal_requires_rotation_tx(tx, community_id, input.issuer, input.subject).await?
        || retired_pair_exists_tx(tx, community_id, input.issuer, input.subject, input.pubkey)
            .await?;
    let active_principal =
        active_by_principal_tx(tx, community_id, input.issuer, input.subject).await?;
    let active_key = active_by_pubkey_tx(tx, community_id, input.pubkey).await?;

    // Authorization evidence is a lease only for this mutation. Recheck the
    // database clock after every potentially blocking lock/read so a waiter
    // cannot enroll or refresh after expiry, and reject future-dated evidence.
    if !evidence_is_current_tx(tx, input).await? {
        return Ok(ResolveBindingResult::Denied(BindingDenial::StaleEvidence));
    }
    if denied {
        return Ok(ResolveBindingResult::Denied(BindingDenial::Revoked));
    }
    if active_principal
        .as_ref()
        .is_some_and(|binding| binding.pubkey != input.pubkey)
        || active_key
            .as_ref()
            .is_some_and(|binding| binding.issuer != input.issuer || binding.uid != input.subject)
    {
        return Ok(ResolveBindingResult::Denied(BindingDenial::Conflict));
    }

    if let Some(binding) = active_principal {
        let database_now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut **tx)
            .await?;
        if binding
            .expires_at
            .is_some_and(|expires_at| expires_at <= database_now)
        {
            return Ok(ResolveBindingResult::Denied(BindingDenial::BindingExpired));
        }
        if existing_read_only {
            return Ok(ResolveBindingResult::Existing(evidence_from_binding(
                community_id,
                &binding,
                binding.binding_version,
                binding.binding_provenance,
            )));
        }
        let strengthen =
            binding.binding_provenance == BindingProvenance::Tofu && input.key_attested;
        let version = binding
            .binding_version
            .checked_add(u64::from(strengthen))
            .ok_or_else(|| {
                DbError::InvalidData("identity binding version exhausted".to_string())
            })?;
        let updated = sqlx::query(
            r#"
            UPDATE identity_bindings
            SET display_name=$5,
                source=CASE WHEN $6 THEN 'jwt_npub' ELSE source END,
                binding_provenance=CASE WHEN $6 THEN 'attested_key' ELSE binding_provenance END,
                binding_version=CASE WHEN $6 THEN binding_version + 1 ELSE binding_version END,
                updated_at=NOW(), last_seen_at=NOW()
            WHERE community_id=$1 AND issuer=$2 AND uid=$3 AND pubkey=$4
              AND binding_state='active' AND revoked_at IS NULL
              AND FLOOR(EXTRACT(EPOCH FROM clock_timestamp()))::BIGINT >= $7
              AND FLOOR(EXTRACT(EPOCH FROM clock_timestamp()))::BIGINT < $8
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(input.issuer)
        .bind(input.subject)
        .bind(input.pubkey)
        .bind(input.display_name)
        .bind(strengthen)
        .bind(evidence_valid_from)
        .bind(evidence_valid_until)
        .execute(&mut **tx)
        .await?;
        if updated.rows_affected() != 1 {
            return Ok(ResolveBindingResult::Denied(BindingDenial::StaleEvidence));
        }
        if strengthen {
            sqlx::query(
                r#"
                INSERT INTO identity_binding_history
                    (community_id, binding_id, binding_version, issuer, subject,
                     pubkey, binding_state, binding_provenance, transition_kind, actor, reason)
                VALUES ($1, $2, $3, $4, $5, $6, 'active', 'attested_key',
                        'provenance_strengthened', $6, 'verified key attestation')
                "#,
            )
            .bind(community_id.as_uuid())
            .bind(binding.binding_id)
            .bind(i64::try_from(version).map_err(|_| {
                DbError::InvalidData("identity binding version is out of range".to_string())
            })?)
            .bind(input.issuer)
            .bind(input.subject)
            .bind(input.pubkey)
            .execute(&mut **tx)
            .await?;
        }
        return Ok(ResolveBindingResult::Existing(evidence_from_binding(
            community_id,
            &binding,
            version,
            if strengthen {
                BindingProvenance::AttestedKey
            } else {
                binding.binding_provenance
            },
        )));
    }

    let provenance = match input.enrollment_mode {
        EnrollmentMode::AttestedKey if !input.key_attested => {
            return Ok(ResolveBindingResult::Denied(
                BindingDenial::KeyAttestationRequired,
            ))
        }
        EnrollmentMode::AttestedKey => BindingProvenance::AttestedKey,
        EnrollmentMode::Provisioned => {
            return Ok(ResolveBindingResult::Denied(BindingDenial::BindingRequired))
        }
        EnrollmentMode::Tofu if input.key_attested => BindingProvenance::AttestedKey,
        EnrollmentMode::Tofu => BindingProvenance::Tofu,
    };
    let binding_id = Uuid::new_v4();
    let created_at: Option<DateTime<Utc>> = sqlx::query_scalar(
        r#"
        INSERT INTO identity_bindings
            (community_id, issuer, uid, pubkey, display_name, source, binding_id,
             binding_version, binding_state, binding_provenance, created_by,
             created_policy_version, expires_at, creation_attribution_kind)
        SELECT $1, $2, $3, $4, $5, $6, $7, 1, 'active', $8, $4, $9, NULL, $10
        WHERE FLOOR(EXTRACT(EPOCH FROM clock_timestamp()))::BIGINT >= $11
          AND FLOOR(EXTRACT(EPOCH FROM clock_timestamp()))::BIGINT < $12
        RETURNING created_at
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(input.issuer)
    .bind(input.subject)
    .bind(input.pubkey)
    .bind(input.display_name)
    .bind(provenance.legacy_source())
    .bind(binding_id)
    .bind(provenance.as_str())
    .bind(input.policy_version)
    .bind(CreationAttributionKind::AuthenticatedKey.as_str())
    .bind(evidence_valid_from)
    .bind(evidence_valid_until)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(created_at) = created_at else {
        return Ok(ResolveBindingResult::Denied(BindingDenial::StaleEvidence));
    };
    sqlx::query(
        r#"
        INSERT INTO identity_binding_history
            (community_id, binding_id, binding_version, issuer, subject, pubkey,
             binding_state, binding_provenance, transition_kind, actor, reason)
        VALUES ($1, $2, $3, $4, $5, $6, 'active', $7, 'enroll', $6, 'first enrollment')
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(binding_id)
    .bind(1_i64)
    .bind(input.issuer)
    .bind(input.subject)
    .bind(input.pubkey)
    .bind(provenance.as_str())
    .execute(&mut **tx)
    .await?;
    Ok(ResolveBindingResult::Enrolled(BindingEvidence {
        authorization_domain: community_id,
        issuer: input.issuer.to_owned(),
        subject: input.subject.to_owned(),
        bound_pubkey: input.pubkey.to_vec(),
        binding_id,
        binding_version: 1,
        binding_state: BindingState::Active,
        provenance,
        creation_attribution: CreationAttributionKind::AuthenticatedKey,
        created_by: Some(input.pubkey.to_vec()),
        created_policy_version: Some(input.policy_version.to_owned()),
        expires_at: None,
        created_at,
    }))
}

/// Return the active binding for `pubkey`, if one exists.
pub async fn get_active_identity_binding_by_pubkey(
    pool: &PgPool,
    community_id: CommunityId,
    pubkey: &[u8],
) -> Result<Option<IdentityBinding>> {
    validate_pubkey(pubkey)?;
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '3s'")
        .execute(&mut *tx)
        .await?;
    lock_identity_coordinates_tx(&mut tx, vec![key_lock_coordinate(community_id, pubkey)]).await?;
    let binding = active_by_pubkey_tx(&mut tx, community_id, pubkey).await?;
    if let Some(binding) = binding.as_ref() {
        let database_now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await?;
        if binding
            .expires_at
            .is_some_and(|expires_at| expires_at <= database_now)
        {
            tx.commit().await?;
            return Ok(None);
        }
        let denied = migration_key_denied_tx(&mut tx, community_id, pubkey).await?
            || migration_denied_tx(&mut tx, community_id, &binding.issuer, &binding.uid).await?
            || principal_disabled_tx(&mut tx, community_id, &binding.issuer, &binding.uid).await?
            || key_revoked_tx(&mut tx, community_id, pubkey).await?
            || principal_requires_rotation_tx(&mut tx, community_id, &binding.issuer, &binding.uid)
                .await?
            || retired_pair_exists_tx(&mut tx, community_id, &binding.issuer, &binding.uid, pubkey)
                .await?;
        if denied {
            return Err(DbError::InvalidData(
                "active identity binding conflicts with lifecycle state".to_string(),
            ));
        }
    }
    tx.commit().await?;
    Ok(binding)
}

/// Disable an issuer-qualified principal and revoke its active key.
///
/// A principal disablement is durable: normal authentication with any new key
/// returns [`BindIdentityResult::Revoked`]. Re-enablement requires a separate,
/// explicit operator lifecycle operation rather than first-use enrollment.
pub async fn revoke_identity_principal(
    pool: &PgPool,
    community_id: CommunityId,
    operation_id: crate::identity_lifecycle::LifecycleOperationId,
    issuer: &str,
    uid: &str,
    revoked_by: &[u8],
    reason: &str,
) -> Result<bool> {
    crate::identity_lifecycle::disable_identity_principal(
        pool,
        community_id,
        crate::identity_lifecycle::LifecycleContext {
            operation_id,
            actor: revoked_by,
            reason,
        },
        crate::identity_lifecycle::IdentityPrincipal {
            issuer,
            subject: uid,
        },
    )
    .await?;
    Ok(true)
}

/// Revoke one active key without disabling the issuer-qualified principal.
/// A replacement key must still be installed through [`rotate_identity_binding`].
pub async fn revoke_identity_key(
    pool: &PgPool,
    community_id: CommunityId,
    operation_id: crate::identity_lifecycle::LifecycleOperationId,
    pubkey: &[u8],
    revoked_by: &[u8],
    reason: &str,
) -> Result<bool> {
    crate::identity_lifecycle::revoke_identity_key(
        pool,
        community_id,
        crate::identity_lifecycle::LifecycleContext {
            operation_id,
            actor: revoked_by,
            reason,
        },
        pubkey,
    )
    .await?;
    Ok(true)
}

/// Atomically retire an active key and install an operator-authorized replacement.
#[allow(clippy::too_many_arguments)]
pub async fn rotate_identity_binding(
    pool: &PgPool,
    community_id: CommunityId,
    operation_id: crate::identity_lifecycle::LifecycleOperationId,
    issuer: &str,
    uid: &str,
    old_pubkey: &[u8],
    replacement: crate::identity_lifecycle::VerifiedReplacementKey<'_>,
    rotated_by: &[u8],
    reason: &str,
) -> Result<()> {
    validate_inputs(issuer, uid, old_pubkey, SOURCE_DB_BINDING)?;
    let principal = crate::identity_lifecycle::IdentityPrincipal {
        issuer,
        subject: uid,
    };
    let context = crate::identity_lifecycle::LifecycleContext {
        operation_id,
        actor: rotated_by,
        reason,
    };
    crate::identity_lifecycle::rotate_identity_binding(
        pool,
        community_id,
        context,
        principal,
        old_pubkey,
        replacement,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;
    use uuid::Uuid;

    #[test]
    fn postgres_authority_adapter_implements_the_sealed_provider_contract() {
        fn assert_adapter<T: FederatedAuthorityAdapter<Error = DbError>>() {}

        assert_adapter::<PostgresFederatedAuthorityAdapter>();
    }

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";
    const TEST_ISSUER: &str = "https://idp.example";

    async fn setup_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect to test DB");
        crate::migration::run_migrations(&pool)
            .await
            .expect("run migrations");
        pool
    }

    async fn make_community(pool: &PgPool) -> CommunityId {
        let id = Uuid::new_v4();
        let host = format!("identity-binding-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert test community");
        CommunityId::from_uuid(id)
    }

    async fn database_now(pool: &PgPool) -> u64 {
        let now: i64 =
            sqlx::query_scalar("SELECT FLOOR(EXTRACT(EPOCH FROM clock_timestamp()))::BIGINT")
                .fetch_one(pool)
                .await
                .expect("read database clock");
        u64::try_from(now).expect("database clock is non-negative")
    }

    async fn install_policy(
        pool: &PgPool,
        community: CommunityId,
        policy_id: Uuid,
        epoch: u64,
        requirement: &str,
        effective_from: u64,
        effective_until: u64,
    ) {
        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id, policy_id, policy_epoch, requirement, effective_from, effective_until) \
             VALUES ($1,$2,$3,$4,$5,$6)",
        )
        .bind(community.as_uuid())
        .bind(policy_id)
        .bind(i64::try_from(epoch).expect("test epoch fits BIGINT"))
        .bind(requirement)
        .bind(i64::try_from(effective_from).expect("test lower bound fits BIGINT"))
        .bind(i64::try_from(effective_until).expect("test upper bound fits BIGINT"))
        .execute(pool)
        .await
        .expect("install test enrollment policy");
    }

    // Keep each authority input explicit so the negative-test matrix can vary
    // one security-relevant field at a time without hiding defaults.
    #[allow(clippy::too_many_arguments)]
    fn authoritative_request<'a>(
        community: CommunityId,
        issuer: &'a str,
        subject: &'a str,
        pubkey: [u8; 32],
        policy_id: Uuid,
        policy_epoch: u64,
        requirement: FederatedIdentityRequirement,
        key_attested: bool,
        effective_from: u64,
        effective_until: u64,
    ) -> AuthoritativeBindingRequestView<'a> {
        AuthoritativeBindingRequestView {
            authorization_domain: community,
            issuer,
            subject,
            pubkey,
            policy_id,
            policy_epoch,
            policy_requirement: requirement,
            key_attested,
            effective_from,
            effective_until,
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn current_policy_adapter_returns_exact_authoritative_lineage() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let policy_id = Uuid::new_v4();
        let now = database_now(&pool).await;
        install_policy(&pool, community, policy_id, 7, "tofu", now - 1, now + 60).await;
        let correlation_id = Uuid::new_v4();
        let adapter = PostgresFederatedAuthorityAdapter::new(pool);

        let resolved =
            buzz_auth::resolve_current_federated_policy(&adapter, community, correlation_id, now)
                .await
                .expect("authoritative current policy resolves");

        assert_eq!(resolved.authorization_domain(), community);
        assert_eq!(resolved.stamp().policy_id(), policy_id);
        assert_eq!(resolved.stamp().epoch(), 7);
        assert_eq!(resolved.stamp().correlation_id(), correlation_id);
        assert_eq!(resolved.stamp().effective_from(), now - 1);
        assert_eq!(resolved.stamp().effective_until(), now + 60);
        assert_eq!(
            resolved.requirement(),
            FederatedIdentityRequirement::Required(AuthorizedEnrollmentMode::Tofu)
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn stale_policy_epoch_fails_closed_without_binding_mutation() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let policy_id = Uuid::new_v4();
        let now = database_now(&pool).await;
        install_policy(&pool, community, policy_id, 1, "tofu", now - 1, now + 60).await;
        sqlx::query(
            "UPDATE identity_enrollment_policies \
             SET policy_epoch=2, requirement='provisioned', updated_at=NOW() \
             WHERE community_id=$1",
        )
        .bind(community.as_uuid())
        .execute(&pool)
        .await
        .expect("advance test policy epoch");
        let request = authoritative_request(
            community,
            TEST_ISSUER,
            "stale-policy-user",
            random_pubkey().try_into().expect("test key is 32 bytes"),
            policy_id,
            1,
            FederatedIdentityRequirement::Required(AuthorizedEnrollmentMode::Tofu),
            false,
            now - 1,
            now + 60,
        );

        let error = resolve_authoritative_binding_view(&pool, &request, true)
            .await
            .expect_err("stale policy epoch must fail closed");

        assert!(matches!(error, AuthorityAdapterError::PolicyChanged));
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM identity_bindings WHERE community_id=$1")
                .bind(community.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("count bindings after stale policy");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn enrollment_policy_lineage_is_stable_and_monotonic() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let policy_id = Uuid::new_v4();
        let now = database_now(&pool).await;
        install_policy(&pool, community, policy_id, 1, "tofu", now - 1, now + 120).await;

        let unchanged_epoch = sqlx::query(
            "UPDATE identity_enrollment_policies SET requirement='attested_key' \
             WHERE community_id=$1",
        )
        .bind(community.as_uuid())
        .execute(&pool)
        .await;
        assert!(
            unchanged_epoch.is_err(),
            "policy changes must advance the epoch"
        );

        let changed_id = sqlx::query(
            "UPDATE identity_enrollment_policies SET policy_id=$1, policy_epoch=2 \
             WHERE community_id=$2",
        )
        .bind(Uuid::new_v4())
        .bind(community.as_uuid())
        .execute(&pool)
        .await;
        assert!(changed_id.is_err(), "a community's policy ID is immutable");

        sqlx::query(
            "UPDATE identity_enrollment_policies \
             SET policy_epoch=2, requirement='attested_key', updated_at=NOW() \
             WHERE community_id=$1",
        )
        .bind(community.as_uuid())
        .execute(&pool)
        .await
        .expect("strictly monotonic policy update succeeds");
        let lineage: (Uuid, i64, String) = sqlx::query_as(
            "SELECT policy_id,policy_epoch,requirement FROM identity_enrollment_policies \
             WHERE community_id=$1",
        )
        .bind(community.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("read stable policy lineage");
        assert_eq!(lineage, (policy_id, 2, "attested_key".to_owned()));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn policy_lock_wait_rechecks_database_time_before_any_binding_mutation() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let policy_id = Uuid::new_v4();
        let now = database_now(&pool).await;
        install_policy(&pool, community, policy_id, 1, "tofu", now - 1, now + 1).await;

        let mut blocker = pool.begin().await.expect("start policy lock blocker");
        sqlx::query("SELECT 1 FROM identity_enrollment_policies WHERE community_id=$1 FOR UPDATE")
            .bind(community.as_uuid())
            .fetch_one(&mut *blocker)
            .await
            .expect("lock current policy");

        let request = authoritative_request(
            community,
            TEST_ISSUER,
            "lock-wait-user",
            random_pubkey().try_into().expect("test key is 32 bytes"),
            policy_id,
            1,
            FederatedIdentityRequirement::Required(AuthorizedEnrollmentMode::Tofu),
            false,
            now - 1,
            now + 1,
        );
        let request_pool = pool.clone();
        let waiter = tokio::spawn(async move {
            resolve_authoritative_binding_view(&request_pool, &request, true).await
        });
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        blocker.commit().await.expect("release policy lock blocker");

        let error = waiter
            .await
            .expect("policy waiter task completes")
            .expect_err("expired policy after lock wait must fail closed");
        assert!(matches!(
            error,
            AuthorityAdapterError::Contract(buzz_auth::AuthContextError::FederatedPolicyExpired)
        ));
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM identity_bindings WHERE community_id=$1")
                .bind(community.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("count bindings after expired policy wait");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn authoritative_modes_and_expiry_are_fail_closed_and_non_mutating() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let policy_id = Uuid::new_v4();
        let now = database_now(&pool).await;
        install_policy(&pool, community, policy_id, 1, "tofu", now - 1, now + 120).await;
        let key: [u8; 32] = random_pubkey().try_into().expect("test key is 32 bytes");
        let request = authoritative_request(
            community,
            TEST_ISSUER,
            "authoritative-user",
            key,
            policy_id,
            1,
            FederatedIdentityRequirement::Required(AuthorizedEnrollmentMode::Tofu),
            false,
            now - 1,
            now + 120,
        );

        let enrolled = resolve_authoritative_binding_view(&pool, &request, true)
            .await
            .expect("current TOFU policy resolves");
        let ResolveBindingResult::Enrolled(evidence) = enrolled else {
            panic!("expected atomic enrollment, got {enrolled:?}");
        };
        assert_eq!(evidence.provenance(), BindingProvenance::Tofu);
        assert_eq!(
            evidence.created_policy_version(),
            Some(format!("{policy_id}:1").as_str())
        );

        let before: (DateTime<Utc>, DateTime<Utc>, i64) = sqlx::query_as(
            "SELECT updated_at, last_seen_at, \
                    (SELECT COUNT(*) FROM identity_binding_history WHERE community_id=$1) \
             FROM identity_bindings WHERE community_id=$1 AND binding_id=$2",
        )
        .bind(community.as_uuid())
        .bind(evidence.binding_id())
        .fetch_one(&pool)
        .await
        .expect("read binding before existing resolution");
        assert!(matches!(
            resolve_authoritative_binding_view(&pool, &request, true)
                .await
                .expect("existing binding resolves read-only"),
            ResolveBindingResult::Existing(_)
        ));
        let after: (DateTime<Utc>, DateTime<Utc>, i64) = sqlx::query_as(
            "SELECT updated_at, last_seen_at, \
                    (SELECT COUNT(*) FROM identity_binding_history WHERE community_id=$1) \
             FROM identity_bindings WHERE community_id=$1 AND binding_id=$2",
        )
        .bind(community.as_uuid())
        .bind(evidence.binding_id())
        .fetch_one(&pool)
        .await
        .expect("read binding after existing resolution");
        assert_eq!(after, before, "existing resolution must be read-only");

        sqlx::query(
            "UPDATE identity_bindings SET expires_at=clock_timestamp() \
             WHERE community_id=$1 AND binding_id=$2",
        )
        .bind(community.as_uuid())
        .bind(evidence.binding_id())
        .execute(&pool)
        .await
        .expect("expire authoritative binding");
        assert_eq!(
            resolve_authoritative_binding_view(&pool, &request, true)
                .await
                .expect("expired binding is a typed denial"),
            ResolveBindingResult::Denied(BindingDenial::BindingExpired)
        );
        let active_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_bindings \
             WHERE community_id=$1 AND binding_id=$2 \
               AND binding_state='active' AND revoked_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(evidence.binding_id())
        .fetch_one(&pool)
        .await
        .expect("count slot-occupying expired binding");
        assert_eq!(active_count, 1, "expiry must not free lifecycle slots");
        assert!(
            get_active_identity_binding_by_pubkey(&pool, community, &key)
                .await
                .expect("legacy authorization lookup handles expiry")
                .is_none(),
            "expired bindings cannot authorize delegated or revalidated sessions"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn provisioned_attested_and_delegated_absence_never_enroll() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let policy_id = Uuid::new_v4();
        let now = database_now(&pool).await;
        install_policy(
            &pool,
            community,
            policy_id,
            1,
            "provisioned",
            now - 1,
            now + 120,
        )
        .await;
        let key: [u8; 32] = random_pubkey().try_into().expect("test key is 32 bytes");
        let provisioned = authoritative_request(
            community,
            TEST_ISSUER,
            "mode-user",
            key,
            policy_id,
            1,
            FederatedIdentityRequirement::Required(AuthorizedEnrollmentMode::Provisioned),
            false,
            now - 1,
            now + 120,
        );
        assert_eq!(
            resolve_authoritative_binding_view(&pool, &provisioned, true)
                .await
                .expect("provisioned absence is a typed denial"),
            ResolveBindingResult::Denied(BindingDenial::BindingRequired)
        );
        assert_eq!(
            resolve_authoritative_binding_view(&pool, &provisioned, false)
                .await
                .expect("delegated absence is a typed denial"),
            ResolveBindingResult::Denied(BindingDenial::BindingRequired)
        );

        sqlx::query(
            "UPDATE identity_enrollment_policies \
             SET policy_epoch=2, requirement='attested_key', updated_at=NOW() \
             WHERE community_id=$1",
        )
        .bind(community.as_uuid())
        .execute(&pool)
        .await
        .expect("advance policy to attested-key mode");
        let unattested = authoritative_request(
            community,
            TEST_ISSUER,
            "mode-user",
            key,
            policy_id,
            2,
            FederatedIdentityRequirement::Required(AuthorizedEnrollmentMode::AttestedKey),
            false,
            now - 1,
            now + 120,
        );
        assert_eq!(
            resolve_authoritative_binding_view(&pool, &unattested, true)
                .await
                .expect("missing attestation is a typed denial"),
            ResolveBindingResult::Denied(BindingDenial::KeyAttestationRequired)
        );
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM identity_bindings WHERE community_id=$1")
                .bind(community.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("count bindings after denied modes");
        assert_eq!(count, 0);
    }

    fn random_pubkey() -> Vec<u8> {
        Keys::generate().public_key().to_bytes().to_vec()
    }

    async fn enroll_for_test(
        pool: &PgPool,
        community: CommunityId,
        issuer: &str,
        subject: &str,
        pubkey: &[u8],
        display_name: Option<&str>,
    ) -> BindingEvidence {
        match resolve_identity_binding(
            pool,
            &ResolveBindingInput {
                authorization_domain: community,
                issuer,
                subject,
                pubkey,
                display_name,
                enrollment_mode: EnrollmentMode::Tofu,
                key_attested: false,
                policy_version: "test-policy-v1",
                evidence_valid_from: 0,
                evidence_valid_until: i64::MAX as u64,
            },
        )
        .await
        .expect("resolve test binding")
        {
            ResolveBindingResult::Enrolled(evidence) => evidence,
            other => panic!("expected enrolled binding, got {other:?}"),
        }
    }

    #[test]
    fn identity_lock_coordinates_are_typed_length_prefixed_and_domain_scoped() {
        let first_domain = CommunityId::from_uuid(Uuid::from_u128(1));
        let second_domain = CommunityId::from_uuid(Uuid::from_u128(2));
        let principal_ab_c = principal_lock_coordinate(first_domain, "ab", "c");
        let principal_a_bc = principal_lock_coordinate(first_domain, "a", "bc");

        assert_ne!(
            principal_ab_c, principal_a_bc,
            "lengths must be unambiguous"
        );
        assert_ne!(
            principal_ab_c,
            principal_lock_coordinate(second_domain, "ab", "c"),
            "domains must not share lock coordinates"
        );
        assert_ne!(
            principal_ab_c,
            key_lock_coordinate(first_domain, &[0_u8; 32]),
            "coordinate kinds must be disjoint"
        );
    }

    #[test]
    fn identity_debug_formatting_redacts_private_coordinates() {
        let input = IdentityBindingInput {
            issuer: "private-issuer",
            uid: "private-subject",
            pubkey: &[42_u8; 32],
            display_name: Some("private-display"),
            source: SOURCE_JWT_NPUB,
        };
        let formatted = format!("{input:?}");

        assert!(formatted.contains("[redacted]"));
        for secret in ["private-issuer", "private-subject", "private-display"] {
            assert!(!formatted.contains(secret));
        }
    }

    #[test]
    fn complete_binding_evidence_debug_is_party_data_free() {
        let evidence = BindingEvidence {
            authorization_domain: CommunityId::from_uuid(Uuid::from_u128(7)),
            issuer: "private-evidence-issuer".to_string(),
            subject: "private-evidence-subject".to_string(),
            bound_pubkey: vec![0xBC; 32],
            binding_id: Uuid::from_u128(8),
            binding_version: 3,
            binding_state: BindingState::Active,
            provenance: BindingProvenance::AttestedKey,
            creation_attribution: CreationAttributionKind::AuthenticatedKey,
            created_by: Some(vec![0xBD; 32]),
            created_policy_version: Some("private-policy-version".to_string()),
            expires_at: Some(DateTime::<Utc>::from_timestamp(2, 0).expect("test expiry timestamp")),
            created_at: DateTime::<Utc>::from_timestamp(1, 0).expect("test timestamp"),
        };
        let formatted = format!("{evidence:?}");
        let encoded_key = hex::encode([0xBC; 32]);
        let binding_id = evidence.binding_id().to_string();
        assert!(formatted.contains("[redacted]"));
        for secret in [
            "private-evidence-issuer",
            "private-evidence-subject",
            "private-policy-version",
            encoded_key.as_str(),
            binding_id.as_str(),
        ] {
            assert!(!formatted.contains(secret));
        }
        assert_eq!(
            evidence.creation_attribution(),
            CreationAttributionKind::AuthenticatedKey
        );
        assert_eq!(evidence.created_by(), Some(&[0xBD; 32][..]));
    }

    #[test]
    fn conflict_marker_cannot_carry_party_data() {
        assert_eq!(std::mem::size_of::<IdentityBindingConflict>(), 0);
        assert_eq!(
            format!(
                "{:?}",
                BindIdentityResult::Conflict(IdentityBindingConflict)
            ),
            "Conflict(IdentityBindingConflict(\"[redacted]\"))"
        );
    }

    #[test]
    fn staged_identity_key_must_match_membership_key() {
        let identity_key = [7_u8; 32];
        let other_key = [8_u8; 32];
        let identity = IdentityBindingInput {
            issuer: TEST_ISSUER,
            uid: "user-1",
            pubkey: &identity_key,
            display_name: None,
            source: SOURCE_JWT_NPUB,
        };

        validate_membership_identity_key(&hex::encode(identity_key), Some(&identity))
            .expect("matching key");
        assert!(
            validate_membership_identity_key(&hex::encode(other_key), Some(&identity)).is_err()
        );
        assert!(validate_membership_identity_key("not-hex", Some(&identity)).is_err());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn legacy_binding_api_cannot_enroll_without_authorized_domain_evidence() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let pubkey = random_pubkey();

        let denied = bind_or_validate_identity(
            &pool,
            community,
            TEST_ISSUER,
            "user-1",
            &pubkey,
            Some("first@example.com"),
            SOURCE_DB_BINDING,
        )
        .await
        .expect("fail closed without authorized domain evidence");
        assert_eq!(denied, BindIdentityResult::BindingRequired);
        assert!(
            get_active_identity_binding_by_pubkey(&pool, community, &pubkey)
                .await
                .expect("lookup binding")
                .is_none()
        );
        let history_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_binding_history WHERE community_id=$1",
        )
        .bind(community.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count binding history");
        assert_eq!(history_count, 0);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn new_bindings_require_immutable_creation_attribution() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let pubkey = random_pubkey();

        enroll_for_test(
            &pool,
            community,
            TEST_ISSUER,
            "attributed-user",
            &pubkey,
            None,
        )
        .await;

        let attribution: (Option<Vec<u8>>, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT created_by, created_policy_version, creation_attribution_kind \
             FROM identity_bindings WHERE community_id=$1 AND issuer=$2 AND uid=$3",
        )
        .bind(community.as_uuid())
        .bind(TEST_ISSUER)
        .bind("attributed-user")
        .fetch_one(&pool)
        .await
        .expect("read creation attribution");
        assert_eq!(attribution.0.as_deref(), Some(pubkey.as_slice()));
        assert_eq!(attribution.1.as_deref(), Some("test-policy-v1"));
        assert_eq!(attribution.2.as_deref(), Some("authenticated_key"));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn stale_authorized_evidence_cannot_enroll_or_refresh_metadata() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let pubkey = random_pubkey();
        enroll_for_test(
            &pool,
            community,
            TEST_ISSUER,
            "stale-evidence-user",
            &pubkey,
            Some("original@example.com"),
        )
        .await;
        let before: (Option<String>, DateTime<Utc>, DateTime<Utc>) = sqlx::query_as(
            "SELECT display_name, updated_at, last_seen_at FROM identity_bindings \
             WHERE community_id=$1 AND issuer=$2 AND uid=$3",
        )
        .bind(community.as_uuid())
        .bind(TEST_ISSUER)
        .bind("stale-evidence-user")
        .fetch_one(&pool)
        .await
        .expect("read binding before stale replay");

        let stale = resolve_identity_binding(
            &pool,
            &ResolveBindingInput {
                authorization_domain: community,
                issuer: TEST_ISSUER,
                subject: "stale-evidence-user",
                pubkey: &pubkey,
                display_name: Some("changed@example.com"),
                enrollment_mode: EnrollmentMode::Tofu,
                key_attested: false,
                policy_version: "test-policy-v1",
                evidence_valid_from: 0,
                evidence_valid_until: 1,
            },
        )
        .await
        .expect("stale resolution is a typed denial");
        assert_eq!(
            stale,
            ResolveBindingResult::Denied(BindingDenial::StaleEvidence)
        );
        let after: (Option<String>, DateTime<Utc>, DateTime<Utc>) = sqlx::query_as(
            "SELECT display_name, updated_at, last_seen_at FROM identity_bindings \
             WHERE community_id=$1 AND issuer=$2 AND uid=$3",
        )
        .bind(community.as_uuid())
        .bind(TEST_ISSUER)
        .bind("stale-evidence-user")
        .fetch_one(&pool)
        .await
        .expect("read binding after stale replay");
        assert_eq!(after, before);

        let missing_key = random_pubkey();
        let missing = resolve_identity_binding(
            &pool,
            &ResolveBindingInput {
                authorization_domain: community,
                issuer: TEST_ISSUER,
                subject: "never-enrolled-stale-user",
                pubkey: &missing_key,
                display_name: None,
                enrollment_mode: EnrollmentMode::Tofu,
                key_attested: false,
                policy_version: "test-policy-v1",
                evidence_valid_from: 0,
                evidence_valid_until: 1,
            },
        )
        .await
        .expect("stale first enrollment is a typed denial");
        assert_eq!(
            missing,
            ResolveBindingResult::Denied(BindingDenial::StaleEvidence)
        );
        assert!(
            get_active_identity_binding_by_pubkey(&pool, community, &missing_key)
                .await
                .expect("lookup missing stale key")
                .is_none()
        );

        let database_now: i64 =
            sqlx::query_scalar("SELECT FLOOR(EXTRACT(EPOCH FROM clock_timestamp()))::BIGINT")
                .fetch_one(&pool)
                .await
                .expect("read database clock");
        let future_key = random_pubkey();
        let future = resolve_identity_binding(
            &pool,
            &ResolveBindingInput {
                authorization_domain: community,
                issuer: TEST_ISSUER,
                subject: "not-yet-valid-user",
                pubkey: &future_key,
                display_name: None,
                enrollment_mode: EnrollmentMode::Tofu,
                key_attested: false,
                policy_version: "test-policy-v1",
                evidence_valid_from: u64::try_from(database_now + 60).unwrap(),
                evidence_valid_until: u64::try_from(database_now + 120).unwrap(),
            },
        )
        .await
        .expect("future evidence is a typed denial");
        assert_eq!(
            future,
            ResolveBindingResult::Denied(BindingDenial::StaleEvidence)
        );
        assert!(
            get_active_identity_binding_by_pubkey(&pool, community, &future_key)
                .await
                .expect("lookup future-evidence key")
                .is_none()
        );
        let history_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_binding_history WHERE community_id=$1",
        )
        .bind(community.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count unchanged binding history");
        assert_eq!(history_count, 1, "only the verified seed may have history");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn evidence_expiring_while_waiting_for_identity_lock_cannot_enroll() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let pubkey = random_pubkey();
        let database_now: i64 =
            sqlx::query_scalar("SELECT FLOOR(EXTRACT(EPOCH FROM clock_timestamp()))::BIGINT")
                .fetch_one(&pool)
                .await
                .expect("read database clock");

        let mut blocker = pool.begin().await.expect("begin blocking transaction");
        lock_identity_keys_tx(
            &mut blocker,
            community,
            TEST_ISSUER,
            "lock-wait-expiry-user",
            &pubkey,
        )
        .await
        .expect("hold identity coordinates");

        let resolving_pool = pool.clone();
        let resolving_key = pubkey.clone();
        let resolving = tokio::spawn(async move {
            resolve_identity_binding(
                &resolving_pool,
                &ResolveBindingInput {
                    authorization_domain: community,
                    issuer: TEST_ISSUER,
                    subject: "lock-wait-expiry-user",
                    pubkey: &resolving_key,
                    display_name: None,
                    enrollment_mode: EnrollmentMode::Tofu,
                    key_attested: false,
                    policy_version: "test-policy-v1",
                    evidence_valid_from: 0,
                    evidence_valid_until: u64::try_from(database_now + 1).unwrap(),
                },
            )
            .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;
        blocker
            .rollback()
            .await
            .expect("release identity coordinates");

        assert_eq!(
            resolving
                .await
                .expect("join blocked resolver")
                .expect("resolver returns typed denial"),
            ResolveBindingResult::Denied(BindingDenial::StaleEvidence)
        );
        assert!(
            get_active_identity_binding_by_pubkey(&pool, community, &pubkey)
                .await
                .expect("lookup expired waiter")
                .is_none()
        );
        let history_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_binding_history WHERE community_id=$1",
        )
        .bind(community.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("count waiter history");
        assert_eq!(history_count, 0);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn evidence_expiring_during_existing_binding_update_rolls_back_metadata() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let pubkey = random_pubkey();
        enroll_for_test(
            &pool,
            community,
            TEST_ISSUER,
            "commit-boundary-expiry-user",
            &pubkey,
            Some("original@example.com"),
        )
        .await;
        let before: (Option<String>, DateTime<Utc>, DateTime<Utc>) = sqlx::query_as(
            "SELECT display_name, updated_at, last_seen_at FROM identity_bindings \
             WHERE community_id=$1 AND issuer=$2 AND uid=$3",
        )
        .bind(community.as_uuid())
        .bind(TEST_ISSUER)
        .bind("commit-boundary-expiry-user")
        .fetch_one(&pool)
        .await
        .expect("read pre-boundary metadata");

        let suffix = community.as_uuid().simple();
        let sequence_name = format!("buzz_test_freshness_seq_{suffix}");
        let function_name = format!("buzz_test_freshness_fn_{suffix}");
        let trigger_name = format!("buzz_test_freshness_trigger_{suffix}");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE SEQUENCE {sequence_name}"
        )))
        .execute(&pool)
        .await
        .expect("create freshness trigger sequence");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE FUNCTION {function_name}() RETURNS trigger LANGUAGE plpgsql AS $$ \
             BEGIN PERFORM nextval('{sequence_name}'); PERFORM pg_sleep(2.5); RETURN NEW; END; $$"
        )))
        .execute(&pool)
        .await
        .expect("create freshness delay function");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE TRIGGER {trigger_name} BEFORE UPDATE ON identity_bindings \
             FOR EACH ROW WHEN (OLD.community_id = '{}'::uuid) \
             EXECUTE FUNCTION {function_name}()",
            community.as_uuid()
        )))
        .execute(&pool)
        .await
        .expect("create freshness delay trigger");

        let database_now: i64 =
            sqlx::query_scalar("SELECT FLOOR(EXTRACT(EPOCH FROM clock_timestamp()))::BIGINT")
                .fetch_one(&pool)
                .await
                .expect("read database clock");
        let result = resolve_identity_binding(
            &pool,
            &ResolveBindingInput {
                authorization_domain: community,
                issuer: TEST_ISSUER,
                subject: "commit-boundary-expiry-user",
                pubkey: &pubkey,
                display_name: Some("changed@example.com"),
                enrollment_mode: EnrollmentMode::Tofu,
                key_attested: false,
                policy_version: "test-policy-v1",
                evidence_valid_from: 0,
                evidence_valid_until: u64::try_from(database_now + 2).unwrap(),
            },
        )
        .await
        .expect("commit-boundary expiry is a typed denial");
        let trigger_state: (i64, bool) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT last_value, is_called FROM {sequence_name}"
        )))
        .fetch_one(&pool)
        .await
        .expect("read freshness trigger count");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP TRIGGER {trigger_name} ON identity_bindings"
        )))
        .execute(&pool)
        .await
        .expect("drop freshness delay trigger");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP FUNCTION {function_name}()"
        )))
        .execute(&pool)
        .await
        .expect("drop freshness delay function");
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP SEQUENCE {sequence_name}"
        )))
        .execute(&pool)
        .await
        .expect("drop freshness trigger sequence");

        assert_eq!(
            trigger_state,
            (1, true),
            "the update reached the forced expiry gap"
        );
        assert_eq!(
            result,
            ResolveBindingResult::Denied(BindingDenial::StaleEvidence)
        );
        let after: (Option<String>, DateTime<Utc>, DateTime<Utc>) = sqlx::query_as(
            "SELECT display_name, updated_at, last_seen_at FROM identity_bindings \
             WHERE community_id=$1 AND issuer=$2 AND uid=$3",
        )
        .bind(community.as_uuid())
        .bind(TEST_ISSUER)
        .bind("commit-boundary-expiry-user")
        .fetch_one(&pool)
        .await
        .expect("read rolled-back metadata");
        assert_eq!(after, before);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn lifecycle_state_and_revocation_timestamp_cannot_diverge() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let pubkey = random_pubkey();

        enroll_for_test(
            &pool,
            community,
            TEST_ISSUER,
            "state-parity-user",
            &pubkey,
            None,
        )
        .await;

        let malformed = sqlx::query(
            "UPDATE identity_bindings SET binding_state='revoked' \
             WHERE community_id=$1 AND issuer=$2 AND uid=$3 \
               AND binding_state='active' AND revoked_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(TEST_ISSUER)
        .bind("state-parity-user")
        .execute(&pool)
        .await;
        assert!(
            malformed.is_err(),
            "storage must reject revoked or rotated labels with an active timestamp shape"
        );

        assert!(
            get_active_identity_binding_by_pubkey(&pool, community, &pubkey)
                .await
                .expect("read active binding")
                .is_some()
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn bind_identity_rejects_uid_conflict() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let original_pubkey = random_pubkey();
        let conflicting_pubkey = random_pubkey();

        enroll_for_test(
            &pool,
            community,
            TEST_ISSUER,
            "user-1",
            &original_pubkey,
            Some("user@example.com"),
        )
        .await;

        let result = bind_or_validate_identity(
            &pool,
            community,
            TEST_ISSUER,
            "user-1",
            &conflicting_pubkey,
            Some("user@example.com"),
            SOURCE_DB_BINDING,
        )
        .await
        .expect("uid conflict is a binding result");

        assert_eq!(
            result,
            BindIdentityResult::Conflict(IdentityBindingConflict)
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn bind_identity_rejects_pubkey_conflict() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let pubkey = random_pubkey();

        enroll_for_test(
            &pool,
            community,
            TEST_ISSUER,
            "user-1",
            &pubkey,
            Some("user@example.com"),
        )
        .await;

        let result = bind_or_validate_identity(
            &pool,
            community,
            TEST_ISSUER,
            "user-2",
            &pubkey,
            Some("other@example.com"),
            SOURCE_JWT_NPUB,
        )
        .await
        .expect("pubkey conflict is a binding result");

        assert_eq!(
            result,
            BindIdentityResult::Conflict(IdentityBindingConflict)
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn legacy_binding_bridge_cannot_manufacture_attested_provenance() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let pubkey = random_pubkey();

        enroll_for_test(
            &pool,
            community,
            TEST_ISSUER,
            "user-1",
            &pubkey,
            Some("user@example.com"),
        )
        .await;

        let matched = bind_or_validate_identity(
            &pool,
            community,
            TEST_ISSUER,
            "user-1",
            &pubkey,
            Some("user@example.com"),
            SOURCE_DB_BINDING,
        )
        .await
        .expect("match existing binding");
        assert_eq!(matched, BindIdentityResult::Matched);

        let binding = get_active_identity_binding_by_pubkey(&pool, community, &pubkey)
            .await
            .expect("lookup binding")
            .expect("binding exists");
        assert_eq!(binding.source, SOURCE_DB_BINDING);
        assert_eq!(binding.binding_provenance, BindingProvenance::Tofu);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn bind_identity_does_not_recreate_revoked_pair() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let pubkey = random_pubkey();

        enroll_for_test(
            &pool,
            community,
            TEST_ISSUER,
            "user-1",
            &pubkey,
            Some("user@example.com"),
        )
        .await;

        sqlx::query(
            r#"
            UPDATE identity_bindings
            SET binding_state = 'revoked', revoked_at = NOW(),
                revoked_reason = 'test revocation'
            WHERE community_id = $1 AND issuer = $2 AND uid = $3 AND pubkey = $4
            "#,
        )
        .bind(community.as_uuid())
        .bind(TEST_ISSUER)
        .bind("user-1")
        .bind(&pubkey)
        .execute(&pool)
        .await
        .expect("revoke binding");

        let result = bind_or_validate_identity(
            &pool,
            community,
            TEST_ISSUER,
            "user-1",
            &pubkey,
            Some("user@example.com"),
            SOURCE_JWT_NPUB,
        )
        .await
        .expect("revoked pair is a binding result");

        assert_eq!(result, BindIdentityResult::Revoked);
        assert!(
            get_active_identity_binding_by_pubkey(&pool, community, &pubkey)
                .await
                .expect("lookup binding")
                .is_none()
        );

        let replacement = random_pubkey();
        let replacement_result = bind_or_validate_identity(
            &pool,
            community,
            TEST_ISSUER,
            "user-1",
            &replacement,
            Some("user@example.com"),
            SOURCE_JWT_NPUB,
        )
        .await
        .expect("principal revocation is a binding result");
        assert_eq!(replacement_result, BindIdentityResult::Revoked);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn authorized_rotation_retires_old_key_and_installs_replacement() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let old_pubkey = random_pubkey();
        let new_pubkey = random_pubkey();
        enroll_for_test(
            &pool,
            community,
            TEST_ISSUER,
            "rotating-user",
            &old_pubkey,
            Some("user@example.com"),
        )
        .await;

        rotate_identity_binding(
            &pool,
            community,
            crate::identity_lifecycle::LifecycleOperationId::issue(),
            TEST_ISSUER,
            "rotating-user",
            &old_pubkey,
            crate::identity_lifecycle::VerifiedReplacementKey::after_verified_proof(
                &new_pubkey,
                Some("user@example.com"),
                BindingProvenance::AttestedKey,
                "test-policy-v1",
            )
            .expect("verified replacement"),
            &old_pubkey,
            "device replacement",
        )
        .await
        .expect("authorized rotation");

        assert!(
            get_active_identity_binding_by_pubkey(&pool, community, &old_pubkey)
                .await
                .expect("old lookup")
                .is_none()
        );
        assert_eq!(
            get_active_identity_binding_by_pubkey(&pool, community, &new_pubkey)
                .await
                .expect("new lookup")
                .expect("replacement active")
                .uid,
            "rotating-user"
        );
        assert_eq!(
            bind_or_validate_identity(
                &pool,
                community,
                TEST_ISSUER,
                "rotating-user",
                &old_pubkey,
                Some("user@example.com"),
                SOURCE_JWT_NPUB,
            )
            .await
            .expect("old key result"),
            BindIdentityResult::Revoked
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn key_revocation_requires_explicit_rotation_for_replacement() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let old_pubkey = random_pubkey();
        let new_pubkey = random_pubkey();
        enroll_for_test(
            &pool,
            community,
            TEST_ISSUER,
            "key-revoked-user",
            &old_pubkey,
            None,
        )
        .await;
        assert!(revoke_identity_key(
            &pool,
            community,
            crate::identity_lifecycle::LifecycleOperationId::issue(),
            &old_pubkey,
            &old_pubkey,
            "lost device",
        )
        .await
        .expect("revoke key"));

        assert_eq!(
            bind_or_validate_identity(
                &pool,
                community,
                TEST_ISSUER,
                "key-revoked-user",
                &new_pubkey,
                None,
                SOURCE_DB_BINDING,
            )
            .await
            .expect("automatic replacement result"),
            BindIdentityResult::Revoked
        );

        assert!(rotate_identity_binding(
            &pool,
            community,
            crate::identity_lifecycle::LifecycleOperationId::issue(),
            TEST_ISSUER,
            "key-revoked-user",
            &old_pubkey,
            crate::identity_lifecycle::VerifiedReplacementKey::after_verified_proof(
                &new_pubkey,
                None,
                BindingProvenance::Provisioned,
                "test-policy-v1",
            )
            .expect("verified recovery key"),
            &old_pubkey,
            "approved replacement",
        )
        .await
        .is_err());
        let principal = crate::identity_lifecycle::IdentityPrincipal {
            issuer: TEST_ISSUER,
            subject: "key-revoked-user",
        };
        let expected = crate::identity_lifecycle::get_pending_lineage(&pool, community, principal)
            .await
            .expect("read pending recovery")
            .expect("pending recovery exists");
        crate::identity_lifecycle::recover_identity_binding(
            &pool,
            community,
            crate::identity_lifecycle::LifecycleContext {
                operation_id: crate::identity_lifecycle::LifecycleOperationId::issue(),
                actor: &old_pubkey,
                reason: "approved recovery",
            },
            principal,
            &expected,
            crate::identity_lifecycle::VerifiedReplacementKey::after_verified_proof(
                &new_pubkey,
                None,
                BindingProvenance::Provisioned,
                "test-policy-v1",
            )
            .expect("verified recovery key"),
        )
        .await
        .expect("explicit recovery after key revocation");
        assert!(
            get_active_identity_binding_by_pubkey(&pool, community, &new_pubkey)
                .await
                .expect("replacement lookup")
                .is_some()
        );
        let retired = sqlx::query(
            "SELECT revoked_reason, revocation_scope, rotation_reason, rotated_to_pubkey \
             FROM identity_bindings \
             WHERE community_id = $1 AND issuer = $2 AND uid = $3 AND pubkey = $4",
        )
        .bind(community.as_uuid())
        .bind(TEST_ISSUER)
        .bind("key-revoked-user")
        .bind(&old_pubkey)
        .fetch_one(&pool)
        .await
        .expect("retired binding provenance");
        assert_eq!(
            retired.try_get::<String, _>("revoked_reason").unwrap(),
            "lost device"
        );
        assert_eq!(
            retired.try_get::<String, _>("revocation_scope").unwrap(),
            "key"
        );
        assert_eq!(
            retired.try_get::<String, _>("rotation_reason").unwrap(),
            "approved recovery"
        );
        assert_eq!(
            retired.try_get::<Vec<u8>, _>("rotated_to_pubkey").unwrap(),
            new_pubkey
        );
        let tombstone_reason: String = sqlx::query_scalar(
            "SELECT reason FROM identity_revoked_keys WHERE community_id = $1 AND pubkey = $2",
        )
        .bind(community.as_uuid())
        .bind(&old_pubkey)
        .fetch_one(&pool)
        .await
        .expect("key tombstone provenance");
        assert_eq!(tombstone_reason, "lost device");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn principal_can_be_disabled_before_first_enrollment() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        assert!(revoke_identity_principal(
            &pool,
            community,
            crate::identity_lifecycle::LifecycleOperationId::issue(),
            TEST_ISSUER,
            "never-enrolled",
            &[0xA4; 32],
            "employment ended",
        )
        .await
        .expect("persist principal tombstone"));
        assert_eq!(
            bind_or_validate_identity(
                &pool,
                community,
                TEST_ISSUER,
                "never-enrolled",
                &random_pubkey(),
                None,
                SOURCE_DB_BINDING,
            )
            .await
            .expect("disabled principal result"),
            BindIdentityResult::Revoked
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn revoked_key_cannot_rebind_to_another_principal() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let pubkey = random_pubkey();
        enroll_for_test(
            &pool,
            community,
            TEST_ISSUER,
            "first-principal",
            &pubkey,
            None,
        )
        .await;
        revoke_identity_key(
            &pool,
            community,
            crate::identity_lifecycle::LifecycleOperationId::issue(),
            &pubkey,
            &pubkey,
            "compromised key",
        )
        .await
        .expect("revoke key");

        assert_eq!(
            bind_or_validate_identity(
                &pool,
                community,
                TEST_ISSUER,
                "different-principal",
                &pubkey,
                None,
                SOURCE_DB_BINDING,
            )
            .await
            .expect("revoked key result"),
            BindIdentityResult::Revoked
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn legacy_revoked_key_history_blocks_cross_principal_rebind() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let pubkey = random_pubkey();
        enroll_for_test(
            &pool,
            community,
            TEST_ISSUER,
            "legacy-principal",
            &pubkey,
            None,
        )
        .await;
        sqlx::query(
            "UPDATE identity_bindings \
             SET binding_state = 'revoked', revoked_at = NOW(), revoked_reason = 'legacy revoke' \
             WHERE community_id = $1 AND issuer = $2 AND uid = $3 AND pubkey = $4",
        )
        .bind(community.as_uuid())
        .bind(TEST_ISSUER)
        .bind("legacy-principal")
        .bind(&pubkey)
        .execute(&pool)
        .await
        .expect("simulate pre-lifecycle revocation");
        sqlx::query(
            "INSERT INTO identity_revoked_keys (community_id, pubkey, reason) \
             VALUES ($1,$2,'legacy revoke')",
        )
        .bind(community.as_uuid())
        .bind(&pubkey)
        .execute(&pool)
        .await
        .expect("simulate legacy key tombstone projection");

        assert_eq!(
            bind_or_validate_identity(
                &pool,
                community,
                "https://other-idp.example",
                "different-principal",
                &pubkey,
                None,
                SOURCE_DB_BINDING,
            )
            .await
            .expect("legacy key tombstone result"),
            BindIdentityResult::Revoked
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn bind_identity_qualifies_same_uid_by_issuer() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let first_pubkey = random_pubkey();
        let second_pubkey = random_pubkey();

        enroll_for_test(
            &pool,
            community,
            "https://issuer-a.example",
            "shared-subject",
            &first_pubkey,
            Some("first@example.com"),
        )
        .await;
        enroll_for_test(
            &pool,
            community,
            "https://issuer-b.example",
            "shared-subject",
            &second_pubkey,
            Some("second@example.com"),
        )
        .await;

        assert_eq!(
            get_active_identity_binding_by_pubkey(&pool, community, &second_pubkey)
                .await
                .expect("lookup second binding")
                .expect("second binding exists")
                .issuer,
            "https://issuer-b.example"
        );
    }
}
