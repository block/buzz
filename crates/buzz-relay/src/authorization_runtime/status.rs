//! Connection-local current-binding status production.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use buzz_auth::{
    BoundedAuthorizationLease, CurrentBindingStatusEvidenceRequest, LocalStatusEvidenceResolver,
    RouteCapability,
};
use buzz_core::client_binding_status::ClientBindingStatusInputV1;
use buzz_core::{AuthorizationLeaseFence, CanonicalCurrentBindingEvidence, CommunityId};
use chrono::{DateTime, Utc};
use nostr::{Event, Keys, PublicKey};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

const MAX_RENEWAL_SECONDS: u64 = 120;
const MAX_PRESENTATION_SECONDS: u64 = 300;
/// Connection-local proof that the exact unchanged bootstrap blob was
/// delivered successfully before current-status production was enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnchangedBootstrapDelivery {
    _private: (),
}

impl UnchangedBootstrapDelivery {
    /// Constructed only by the relay connection adapter after the validated
    /// bootstrap event has entered that connection's outbound queue.
    pub(crate) const fn delivered() -> Self {
        Self { _private: () }
    }
}

/// Fixed upper bounds for current presentation and renewal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusCadence {
    renewal: Duration,
    maximum_lifetime: Duration,
}

impl StatusCadence {
    /// Validate renewal no later than 120 seconds and lifetime no longer than
    /// 300 seconds. The live authorization lease can only shorten either.
    pub fn new(renewal: Duration, maximum_lifetime: Duration) -> Result<Self, StatusSessionError> {
        if renewal.is_zero()
            || renewal > Duration::from_secs(MAX_RENEWAL_SECONDS)
            || maximum_lifetime.is_zero()
            || maximum_lifetime > Duration::from_secs(MAX_PRESENTATION_SECONDS)
            || renewal > maximum_lifetime
        {
            return Err(StatusSessionError::InvalidCadence);
        }
        Ok(Self {
            renewal,
            maximum_lifetime,
        })
    }

    /// Production cadence required by the public contract.
    pub fn production() -> Self {
        Self {
            renewal: Duration::from_secs(MAX_RENEWAL_SECONDS),
            maximum_lifetime: Duration::from_secs(MAX_PRESENTATION_SECONDS),
        }
    }
}

/// Credential-free live authorization coordinates used by status production.
/// Construction is only from a finalized authorization lease.
#[derive(Clone)]
pub struct CurrentStatusAuthorization {
    domain: CommunityId,
    author: PublicKey,
    binding_id: uuid::Uuid,
    binding_version: u64,
    policy_revision: u64,
    invalidation_generation: u64,
    authority_epoch: u64,
    fence: AuthorizationLeaseFence,
    expires_at: DateTime<Utc>,
}

impl CurrentStatusAuthorization {
    /// Capture the exact domain, actor, and exclusive lease bound.
    pub fn from_lease(lease: &BoundedAuthorizationLease) -> Result<Self, StatusSessionError> {
        if lease.capability() != RouteCapability::BindingStatus || lease.owner_pubkey().is_some() {
            return Err(StatusSessionError::EvidenceUnavailable);
        }
        let (binding_id, binding_version) = lease.binding();
        let (policy_revision, invalidation_generation, authority_epoch) =
            lease.dependency_versions();
        Ok(Self {
            domain: lease.authorization_domain(),
            author: lease.actor_pubkey(),
            binding_id,
            binding_version,
            policy_revision,
            invalidation_generation,
            authority_epoch,
            fence: lease.fence(),
            expires_at: lease.expires_at(),
        })
    }

    #[cfg(test)]
    pub(crate) fn from_test_parts(
        evidence: &CanonicalCurrentBindingEvidence,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            domain: evidence.authorization_domain(),
            author: evidence.event_author_pubkey(),
            binding_id: evidence.binding_id(),
            binding_version: evidence.binding_version(),
            policy_revision: evidence.policy_revision(),
            invalidation_generation: evidence.invalidation_generation(),
            authority_epoch: evidence.authority_epoch(),
            fence: evidence.fence(),
            expires_at,
        }
    }

    /// Exact final scope consumed by the connection bootstrap adapter.
    pub(crate) const fn domain_author(&self) -> (CommunityId, PublicKey) {
        (self.domain, self.author)
    }
}

impl fmt::Debug for CurrentStatusAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CurrentStatusAuthorization([REDACTED])")
    }
}

/// Database-backed read-only evidence and authoritative-time boundary.
#[async_trait]
pub trait CurrentStatusEvidenceSource: Send + Sync {
    /// Read one current local binding observation without mutation.
    async fn current(
        &self,
        request: &CurrentBindingStatusEvidenceRequest,
    ) -> Result<CanonicalCurrentBindingEvidence, StatusSessionError>;

    /// Atomically re-read the complete tuple and authoritative database time
    /// immediately before presentation.
    async fn recheck(
        &self,
        evidence: &CanonicalCurrentBindingEvidence,
    ) -> Result<(CanonicalCurrentBindingEvidence, DateTime<Utc>), StatusSessionError>;
}

#[async_trait]
impl<T> CurrentStatusEvidenceSource for Arc<T>
where
    T: CurrentStatusEvidenceSource + ?Sized,
{
    async fn current(
        &self,
        request: &CurrentBindingStatusEvidenceRequest,
    ) -> Result<CanonicalCurrentBindingEvidence, StatusSessionError> {
        (**self).current(request).await
    }

    async fn recheck(
        &self,
        evidence: &CanonicalCurrentBindingEvidence,
    ) -> Result<(CanonicalCurrentBindingEvidence, DateTime<Utc>), StatusSessionError> {
        (**self).recheck(evidence).await
    }
}

/// Production adapter over the read-only local binding resolver.
///
/// The resolver owns both reads. Its atomic recheck returns a PostgreSQL
/// observation, while the relay's local clock is used only to shorten (never
/// extend) that observation's accepted freshness interval.
#[derive(Clone)]
pub struct LocalBindingStatusEvidenceSource<R> {
    resolver: Arc<R>,
}

impl<R> LocalBindingStatusEvidenceSource<R> {
    /// Bind one relay producer to the configured resolver.
    pub const fn new(resolver: Arc<R>) -> Self {
        Self { resolver }
    }
}

#[async_trait]
impl<R> CurrentStatusEvidenceSource for LocalBindingStatusEvidenceSource<R>
where
    R: LocalStatusEvidenceResolver + 'static,
{
    async fn current(
        &self,
        request: &CurrentBindingStatusEvidenceRequest,
    ) -> Result<CanonicalCurrentBindingEvidence, StatusSessionError> {
        self.resolver
            .current_status_evidence(request)
            .await
            .map_err(|_| StatusSessionError::EvidenceUnavailable)
    }

    async fn recheck(
        &self,
        evidence: &CanonicalCurrentBindingEvidence,
    ) -> Result<(CanonicalCurrentBindingEvidence, DateTime<Utc>), StatusSessionError> {
        let (rechecked, authoritative_now) = self
            .resolver
            .recheck_current_status_evidence(evidence)
            .await
            .map_err(|_| StatusSessionError::EvidenceUnavailable)?;
        Ok((rechecked, authoritative_now))
    }
}

/// Relay-side needs from the typed current-only wire contract.
///
/// Associated values are opaque to this runtime. The caller supplies exact rechecked
/// evidence, a connection-local monotonic revision, and strict time bounds;
/// The core contract alone owns event shape, signing, validation, and fold semantics.
pub trait CurrentStatusContract: Send + Sync {
    /// Typed signed current presentation.
    type Current: Send + Sync;
    /// Typed signed withdrawal.
    type Withdrawal: Send + Sync;

    /// Stable identity of the exact current/withdrawal wire policy. It must
    /// exclude rotating signing keys and every connection-local revision.
    fn contract_fingerprint(&self) -> [u8; 32];

    /// Build one current presentation from exact rechecked evidence.
    fn current(
        &self,
        evidence: &CanonicalCurrentBindingEvidence,
        connection_revision: u64,
    ) -> Result<Self::Current, StatusSessionError>;

    /// Build an opaque withdrawal superseding the currently delivered value.
    fn withdrawal(
        &self,
        domain: CommunityId,
        author: PublicKey,
        connection_revision: u64,
        issued_at: DateTime<Utc>,
        fresh_until: DateTime<Utc>,
    ) -> Result<Self::Withdrawal, StatusSessionError>;
}

/// Production adapter over the core producer facade. It never builds,
/// serializes, or validates status payloads itself.
#[derive(Clone)]
pub struct ConnectionLocalStatusContract {
    relay_keys: Keys,
    contract_fingerprint: [u8; 32],
}

impl ConnectionLocalStatusContract {
    /// Bind signing to the same relay key advertised by NIP-11 `self`.
    pub fn new(
        relay_keys: Keys,
        advertised_relay_key: PublicKey,
    ) -> Result<Self, StatusSessionError> {
        if relay_keys.public_key() != advertised_relay_key {
            return Err(StatusSessionError::ContractUnavailable);
        }
        let contract_fingerprint =
            Sha256::digest(b"buzz:client-binding-status:connection-local:v1").into();
        Ok(Self {
            relay_keys,
            contract_fingerprint,
        })
    }
}

impl CurrentStatusContract for ConnectionLocalStatusContract {
    type Current = Event;
    type Withdrawal = Event;

    fn contract_fingerprint(&self) -> [u8; 32] {
        self.contract_fingerprint
    }

    fn current(
        &self,
        evidence: &CanonicalCurrentBindingEvidence,
        connection_revision: u64,
    ) -> Result<Self::Current, StatusSessionError> {
        ClientBindingStatusInputV1::current_from_evidence(evidence, connection_revision)
            .map_err(|_| StatusSessionError::ContractUnavailable)?
            .sign_with_relay_keys(&self.relay_keys)
            .map_err(|_| StatusSessionError::ContractUnavailable)
    }

    fn withdrawal(
        &self,
        domain: CommunityId,
        author: PublicKey,
        connection_revision: u64,
        issued_at: DateTime<Utc>,
        fresh_until: DateTime<Utc>,
    ) -> Result<Self::Withdrawal, StatusSessionError> {
        let issued_at = u64::try_from(issued_at.timestamp())
            .map_err(|_| StatusSessionError::ContractUnavailable)?;
        let fresh_until = u64::try_from(fresh_until.timestamp())
            .map_err(|_| StatusSessionError::ContractUnavailable)?;
        ClientBindingStatusInputV1::withdrawn(
            domain,
            author,
            connection_revision,
            issued_at,
            fresh_until,
        )
        .map_err(|_| StatusSessionError::ContractUnavailable)?
        .sign_with_relay_keys(&self.relay_keys)
        .map_err(|_| StatusSessionError::ContractUnavailable)
    }
}

impl fmt::Debug for ConnectionLocalStatusContract {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ConnectionLocalStatusContract([REDACTED])")
    }
}

/// Same-connection delivery and mandatory-close boundary.
#[async_trait]
pub trait CurrentStatusSink<C: CurrentStatusContract>: Send + Sync {
    /// Deliver one current presentation on the authenticated connection.
    async fn send_current(&self, current: &C::Current) -> Result<(), StatusSessionError>;
    /// Deliver one withdrawal on that same connection.
    async fn send_withdrawal(&self, withdrawal: &C::Withdrawal) -> Result<(), StatusSessionError>;
    /// Close/cancel the connection after delivery cannot be proved.
    async fn close(&self);
}

/// Stable fail-closed status producer failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum StatusSessionError {
    /// Configured cadence exceeded protocol bounds.
    #[error("invalid current-status cadence")]
    InvalidCadence,
    /// Current local binding evidence was unavailable.
    #[error("current binding evidence unavailable")]
    EvidenceUnavailable,
    /// Exact evidence changed before delivery.
    #[error("current binding evidence changed")]
    EvidenceChanged,
    /// The authorization or presentation bound expired.
    #[error("current binding presentation expired")]
    Expired,
    /// The typed constructor rejected the presentation.
    #[error("current binding status contract unavailable")]
    ContractUnavailable,
    /// Same-connection delivery could not be proved.
    #[error("current binding status delivery failed")]
    DeliveryFailed,
}

struct DeliveredCurrent {
    domain: CommunityId,
    author: PublicKey,
    expires_at: DateTime<Utc>,
}

/// Entirely connection-local producer state. Dropping it forgets every
/// revision and delivered value; there is no persistence or restore port.
pub struct ConnectionStatusSession<E, C, S>
where
    E: CurrentStatusEvidenceSource,
    C: CurrentStatusContract,
    S: CurrentStatusSink<C>,
{
    evidence: E,
    contract: C,
    sink: S,
    _bootstrap: UnchangedBootstrapDelivery,
    cadence: StatusCadence,
    revision: u64,
    contract_fingerprint: Option<[u8; 32]>,
    current: Option<DeliveredCurrent>,
    next_renewal: Option<DateTime<Utc>>,
}

impl<E, C, S> ConnectionStatusSession<E, C, S>
where
    E: CurrentStatusEvidenceSource,
    C: CurrentStatusContract,
    S: CurrentStatusSink<C>,
{
    /// Construct empty state for one authenticated connection.
    pub const fn new(
        evidence: E,
        contract: C,
        sink: S,
        cadence: StatusCadence,
        bootstrap: UnchangedBootstrapDelivery,
    ) -> Self {
        Self {
            evidence,
            contract,
            sink,
            _bootstrap: bootstrap,
            cadence,
            revision: 0,
            contract_fingerprint: None,
            current: None,
            next_renewal: None,
        }
    }

    /// Read, atomically recheck, construct, and deliver current status after
    /// AUTH or at renewal. Nothing is retained until delivery succeeds.
    pub async fn present(
        &mut self,
        authorization: &CurrentStatusAuthorization,
    ) -> Result<DateTime<Utc>, StatusSessionError> {
        let result = self.present_checked(authorization).await;
        if result.is_err() && self.current.take().is_some() {
            self.next_renewal = None;
            self.sink.close().await;
        }
        result
    }

    async fn present_checked(
        &mut self,
        authorization: &CurrentStatusAuthorization,
    ) -> Result<DateTime<Utc>, StatusSessionError> {
        let contract_fingerprint = self.contract.contract_fingerprint();
        if contract_fingerprint == [0; 32]
            || self
                .contract_fingerprint
                .is_some_and(|installed| installed != contract_fingerprint)
        {
            return Err(StatusSessionError::ContractUnavailable);
        }
        let request =
            CurrentBindingStatusEvidenceRequest::new(authorization.domain, authorization.author)
                .map_err(|_| StatusSessionError::EvidenceUnavailable)?;
        let evidence = self.evidence.current(&request).await?;
        let (rechecked, authoritative_now) = self.evidence.recheck(&evidence).await?;
        if evidence.authorization_domain() != authorization.domain
            || evidence.event_author_pubkey() != authorization.author
            || evidence.binding_id() != authorization.binding_id
            || evidence.binding_version() != authorization.binding_version
            || evidence.policy_revision() != authorization.policy_revision
            || evidence.invalidation_generation() != authorization.invalidation_generation
            || evidence.authority_epoch() != authorization.authority_epoch
            || evidence.fence() != authorization.fence
            || !evidence.accepts_exact_recheck(&rechecked, authoritative_now)
            || authoritative_now >= authorization.expires_at
        {
            return Err(StatusSessionError::EvidenceChanged);
        }

        let maximum_lifetime = chrono::Duration::from_std(self.cadence.maximum_lifetime)
            .map_err(|_| StatusSessionError::InvalidCadence)?;
        let protocol_expiry = authoritative_now
            .checked_add_signed(maximum_lifetime)
            .ok_or(StatusSessionError::Expired)?;
        let expires_at = [
            evidence.fresh_until(),
            authorization.expires_at,
            protocol_expiry,
        ]
        .into_iter()
        .min()
        .ok_or(StatusSessionError::Expired)?;
        if authoritative_now >= expires_at {
            return Err(StatusSessionError::Expired);
        }

        let revision = self
            .revision
            .checked_add(1)
            .ok_or(StatusSessionError::ContractUnavailable)?;
        let status_evidence = CanonicalCurrentBindingEvidence::new(
            rechecked.authorization_domain(),
            rechecked.event_author_pubkey(),
            rechecked.binding_id(),
            rechecked.binding_version(),
            rechecked.policy_revision(),
            rechecked.invalidation_generation(),
            rechecked.authority_epoch(),
            rechecked.fence(),
            rechecked.observed_at(),
            expires_at,
        )
        .map_err(|_| StatusSessionError::ContractUnavailable)?;
        let current = self.contract.current(&status_evidence, revision)?;
        if self.sink.send_current(&current).await.is_err() {
            self.sink.close().await;
            return Err(StatusSessionError::DeliveryFailed);
        }

        let renewal = chrono::Duration::from_std(self.cadence.renewal)
            .map_err(|_| StatusSessionError::InvalidCadence)?;
        let scheduled = authoritative_now
            .checked_add_signed(renewal)
            .ok_or(StatusSessionError::Expired)?;
        let next_renewal = scheduled.min(expires_at);
        self.revision = revision;
        self.contract_fingerprint = Some(contract_fingerprint);
        self.current = Some(DeliveredCurrent {
            domain: rechecked.authorization_domain(),
            author: rechecked.event_author_pubkey(),
            expires_at,
        });
        self.next_renewal = Some(next_renewal);
        Ok(next_renewal)
    }

    /// Whether renewal is due or the current presentation has reached its
    /// exclusive bound.
    pub fn renewal_due(&self, now: DateTime<Utc>) -> bool {
        self.next_renewal.is_some_and(|deadline| now >= deadline)
            || self
                .current
                .as_ref()
                .is_some_and(|current| now >= current.expires_at)
    }

    /// Withdraw the current value on invalidation, lease loss, or disconnect.
    /// Failed withdrawal closes the connection and forgets local state.
    pub async fn withdraw(&mut self, now: DateTime<Utc>) -> Result<(), StatusSessionError> {
        let Some(current) = self.current.take() else {
            self.next_renewal = None;
            return Ok(());
        };
        self.next_renewal = None;
        if self.contract_fingerprint != Some(self.contract.contract_fingerprint()) {
            self.sink.close().await;
            return Err(StatusSessionError::ContractUnavailable);
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(StatusSessionError::ContractUnavailable)?;
        let withdrawal_fresh_until = now
            .checked_add_signed(
                chrono::Duration::from_std(self.cadence.maximum_lifetime)
                    .map_err(|_| StatusSessionError::InvalidCadence)?,
            )
            .ok_or(StatusSessionError::Expired)?;
        let withdrawal = match self.contract.withdrawal(
            current.domain,
            current.author,
            revision,
            now,
            withdrawal_fresh_until,
        ) {
            Ok(withdrawal) => withdrawal,
            Err(error) => {
                self.sink.close().await;
                return Err(error);
            }
        };
        if self.sink.send_withdrawal(&withdrawal).await.is_err() {
            self.sink.close().await;
            return Err(StatusSessionError::DeliveryFailed);
        }
        self.revision = revision;
        Ok(())
    }

    /// Current connection-local revision. It has no durable meaning.
    pub const fn connection_revision(&self) -> u64 {
        self.revision
    }

    /// Run immediate presentation and bounded renewal until the owning socket
    /// or AUTH scope is cancelled. Cancellation sends a higher-revision opaque
    /// withdrawal and then discards all RAM-only state with this task.
    pub async fn run_until_cancelled(
        mut self,
        authorization: CurrentStatusAuthorization,
        cancel: CancellationToken,
    ) -> Result<(), StatusSessionError> {
        // Activation may already have presented the first authoritative value
        // while waiting to acknowledge AUTH. Preserve that value and wait for
        // its renewal instead of emitting a duplicate presentation.
        let mut presentation_is_current = self.current.is_some() && self.next_renewal.is_some();
        loop {
            let renewal = if presentation_is_current {
                presentation_is_current = false;
                self.next_renewal
                    .ok_or(StatusSessionError::EvidenceUnavailable)?
            } else {
                // Cancellation races every resolver/evidence await. A stalled
                // read therefore cannot pin AUTH replacement or socket teardown.
                let presentation = {
                    let present = self.present(&authorization);
                    tokio::pin!(present);
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => None,
                        result = &mut present => Some(result),
                    }
                };
                match presentation {
                    None => return self.withdraw(Utc::now()).await,
                    Some(Err(error)) => {
                        self.sink.close().await;
                        return Err(error);
                    }
                    Some(Ok(renewal)) => renewal,
                }
            };
            let delay = (renewal - Utc::now()).to_std().unwrap_or(Duration::ZERO);
            tokio::select! {
                _ = cancel.cancelled() => {
                    return self.withdraw(Utc::now()).await;
                }
                _ = tokio::time::sleep(delay) => {}
            }
        }
    }
}

impl<E, C, S> fmt::Debug for ConnectionStatusSession<E, C, S>
where
    E: CurrentStatusEvidenceSource,
    C: CurrentStatusContract,
    S: CurrentStatusSink<C>,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ConnectionStatusSession([REDACTED])")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    use buzz_auth::{
        BindingResolutionRequest, LocalBindingResolution, LocalBindingResolver,
        LocalBindingResolverCapability,
    };
    use buzz_core::AuthorizationLeaseFence;
    use nostr::Keys;
    use tokio::sync::Notify;
    use uuid::Uuid;

    use super::*;

    struct Evidence {
        value: CanonicalCurrentBindingEvidence,
        now: DateTime<Utc>,
    }

    struct Resolver {
        value: CanonicalCurrentBindingEvidence,
        now: DateTime<Utc>,
    }

    impl LocalBindingResolver for Resolver {
        type Error = ();

        fn capability(&self) -> LocalBindingResolverCapability {
            LocalBindingResolverCapability::DirectAndDelegatedOwnerBound
        }

        fn resolve<'a>(
            &'a self,
            _request: &'a BindingResolutionRequest,
        ) -> impl std::future::Future<Output = Result<LocalBindingResolution, Self::Error>> + Send + 'a
        {
            std::future::pending()
        }

        fn current_status_evidence<'a>(
            &'a self,
            _request: &'a CurrentBindingStatusEvidenceRequest,
        ) -> impl std::future::Future<Output = Result<CanonicalCurrentBindingEvidence, Self::Error>>
               + Send
               + 'a {
            let value = self.value.clone();
            async move { Ok(value) }
        }

        fn recheck_current_status_evidence<'a>(
            &'a self,
            _evidence: &'a CanonicalCurrentBindingEvidence,
        ) -> impl std::future::Future<
            Output = Result<(CanonicalCurrentBindingEvidence, DateTime<Utc>), Self::Error>,
        > + Send
               + 'a {
            let value = self.value.clone();
            let now = self.now;
            async move { Ok((value, now)) }
        }
    }

    #[async_trait]
    impl CurrentStatusEvidenceSource for Evidence {
        async fn current(
            &self,
            _request: &CurrentBindingStatusEvidenceRequest,
        ) -> Result<CanonicalCurrentBindingEvidence, StatusSessionError> {
            Ok(self.value.clone())
        }

        async fn recheck(
            &self,
            _evidence: &CanonicalCurrentBindingEvidence,
        ) -> Result<(CanonicalCurrentBindingEvidence, DateTime<Utc>), StatusSessionError> {
            Ok((self.value.clone(), self.now))
        }
    }

    struct Contract;

    impl CurrentStatusContract for Contract {
        type Current = (u64, DateTime<Utc>);
        type Withdrawal = u64;

        fn contract_fingerprint(&self) -> [u8; 32] {
            [9; 32]
        }

        fn current(
            &self,
            evidence: &CanonicalCurrentBindingEvidence,
            connection_revision: u64,
        ) -> Result<Self::Current, StatusSessionError> {
            Ok((connection_revision, evidence.fresh_until()))
        }

        fn withdrawal(
            &self,
            _domain: CommunityId,
            _author: PublicKey,
            connection_revision: u64,
            _issued_at: DateTime<Utc>,
            _fresh_until: DateTime<Utc>,
        ) -> Result<Self::Withdrawal, StatusSessionError> {
            Ok(connection_revision)
        }
    }

    #[derive(Default)]
    struct Sink {
        current: Mutex<Vec<u64>>,
        withdrawals: Mutex<Vec<u64>>,
        fail_current: AtomicBool,
        fail_withdrawal: AtomicBool,
        closed: AtomicBool,
    }

    #[derive(Clone)]
    struct SharedSink {
        current: Arc<Mutex<Vec<u64>>>,
        withdrawals: Arc<Mutex<Vec<u64>>>,
        closed: Arc<AtomicBool>,
        cancel_after_two: Option<CancellationToken>,
    }

    impl SharedSink {
        fn new(cancel_after_two: Option<CancellationToken>) -> Self {
            Self {
                current: Arc::new(Mutex::new(Vec::new())),
                withdrawals: Arc::new(Mutex::new(Vec::new())),
                closed: Arc::new(AtomicBool::new(false)),
                cancel_after_two,
            }
        }
    }

    #[async_trait]
    impl CurrentStatusSink<Contract> for SharedSink {
        async fn send_current(
            &self,
            current: &<Contract as CurrentStatusContract>::Current,
        ) -> Result<(), StatusSessionError> {
            let mut delivered = self.current.lock().unwrap();
            delivered.push(current.0);
            if delivered.len() == 2 {
                if let Some(cancel) = &self.cancel_after_two {
                    cancel.cancel();
                }
            }
            Ok(())
        }

        async fn send_withdrawal(
            &self,
            withdrawal: &<Contract as CurrentStatusContract>::Withdrawal,
        ) -> Result<(), StatusSessionError> {
            self.withdrawals.lock().unwrap().push(*withdrawal);
            Ok(())
        }

        async fn close(&self) {
            self.closed.store(true, Ordering::Release);
        }
    }

    #[async_trait]
    impl CurrentStatusSink<Contract> for Sink {
        async fn send_current(
            &self,
            current: &<Contract as CurrentStatusContract>::Current,
        ) -> Result<(), StatusSessionError> {
            if self.fail_current.load(Ordering::Acquire) {
                return Err(StatusSessionError::DeliveryFailed);
            }
            self.current.lock().unwrap().push(current.0);
            Ok(())
        }

        async fn send_withdrawal(
            &self,
            withdrawal: &<Contract as CurrentStatusContract>::Withdrawal,
        ) -> Result<(), StatusSessionError> {
            if self.fail_withdrawal.load(Ordering::Acquire) {
                return Err(StatusSessionError::DeliveryFailed);
            }
            self.withdrawals.lock().unwrap().push(*withdrawal);
            Ok(())
        }

        async fn close(&self) {
            self.closed.store(true, Ordering::Release);
        }
    }

    fn fixture() -> (Evidence, CurrentStatusAuthorization, DateTime<Utc>) {
        let now = Utc::now();
        let domain = CommunityId::from_uuid(Uuid::from_u128(1));
        let author = Keys::generate().public_key();
        let evidence = CanonicalCurrentBindingEvidence::new(
            domain,
            author,
            Uuid::from_u128(2),
            3,
            4,
            5,
            6,
            AuthorizationLeaseFence::from_bytes([7; 32]).unwrap(),
            now,
            now + chrono::Duration::seconds(300),
        )
        .unwrap();
        let authorization = CurrentStatusAuthorization::from_test_parts(
            &evidence,
            now + chrono::Duration::seconds(240),
        );
        (
            Evidence {
                value: evidence,
                now,
            },
            authorization,
            now,
        )
    }

    #[tokio::test]
    async fn local_binding_resolver_adapter_performs_both_read_only_status_reads() {
        let (evidence, authorization, now) = fixture();
        let adapter = LocalBindingStatusEvidenceSource::new(Arc::new(Resolver {
            value: evidence.value.clone(),
            now,
        }));
        let request =
            CurrentBindingStatusEvidenceRequest::new(authorization.domain, authorization.author)
                .unwrap();
        let current = adapter.current(&request).await.unwrap();
        let (rechecked, conservative_now) = adapter.recheck(&current).await.unwrap();
        assert_eq!(current, rechecked);
        assert!(conservative_now >= rechecked.observed_at());
        assert!(current.accepts_exact_recheck(&rechecked, conservative_now));
    }

    #[tokio::test]
    async fn current_renews_with_bounded_connection_local_revision_then_withdraws() {
        let (evidence, authorization, now) = fixture();
        let mut session = ConnectionStatusSession::new(
            evidence,
            Contract,
            Sink::default(),
            StatusCadence::production(),
            UnchangedBootstrapDelivery::delivered(),
        );
        let renewal = session.present(&authorization).await.unwrap();
        assert_eq!(renewal, now + chrono::Duration::seconds(120));
        assert_eq!(session.connection_revision(), 1);
        assert!(session.renewal_due(renewal));
        session
            .withdraw(now + chrono::Duration::seconds(1))
            .await
            .unwrap();
        assert_eq!(session.connection_revision(), 2);
        assert!(session.current.is_none());
    }

    #[tokio::test]
    async fn failed_withdrawal_closes_and_forgets_current_value() {
        let (evidence, authorization, now) = fixture();
        let sink = Sink::default();
        sink.fail_withdrawal.store(true, Ordering::Release);
        let mut session = ConnectionStatusSession::new(
            evidence,
            Contract,
            sink,
            StatusCadence::production(),
            UnchangedBootstrapDelivery::delivered(),
        );
        session.present(&authorization).await.unwrap();
        assert_eq!(
            session.withdraw(now).await,
            Err(StatusSessionError::DeliveryFailed)
        );
        assert!(session.sink.closed.load(Ordering::Acquire));
        assert!(session.current.is_none());
    }

    #[tokio::test]
    async fn failed_current_delivery_closes_without_retaining_state() {
        let (evidence, authorization, _) = fixture();
        let sink = Sink::default();
        sink.fail_current.store(true, Ordering::Release);
        let mut session = ConnectionStatusSession::new(
            evidence,
            Contract,
            sink,
            StatusCadence::production(),
            UnchangedBootstrapDelivery::delivered(),
        );
        assert_eq!(
            session.present(&authorization).await,
            Err(StatusSessionError::DeliveryFailed)
        );
        assert!(session.sink.closed.load(Ordering::Acquire));
        assert!(session.current.is_none());
        assert_eq!(session.connection_revision(), 0);
    }

    #[tokio::test]
    async fn run_loop_renews_then_cancels_with_higher_revision_withdrawal() {
        let (evidence, authorization, _) = fixture();
        let cancel = CancellationToken::new();
        let sink = SharedSink::new(Some(cancel.clone()));
        let observed = sink.clone();
        let session = ConnectionStatusSession::new(
            evidence,
            Contract,
            sink,
            StatusCadence::new(Duration::from_millis(5), Duration::from_secs(1)).unwrap(),
            UnchangedBootstrapDelivery::delivered(),
        );
        session
            .run_until_cancelled(authorization, cancel)
            .await
            .unwrap();
        assert_eq!(*observed.current.lock().unwrap(), vec![1, 2]);
        assert_eq!(*observed.withdrawals.lock().unwrap(), vec![3]);
        assert!(!observed.closed.load(Ordering::Acquire));
    }

    struct RecheckFailure {
        value: CanonicalCurrentBindingEvidence,
    }

    #[derive(Clone, Copy)]
    enum BlockingPhase {
        Current,
        Recheck,
    }

    struct BlockingEvidence {
        value: CanonicalCurrentBindingEvidence,
        phase: BlockingPhase,
        entered: Arc<Notify>,
    }

    #[async_trait]
    impl CurrentStatusEvidenceSource for BlockingEvidence {
        async fn current(
            &self,
            _request: &CurrentBindingStatusEvidenceRequest,
        ) -> Result<CanonicalCurrentBindingEvidence, StatusSessionError> {
            if matches!(self.phase, BlockingPhase::Current) {
                self.entered.notify_one();
                std::future::pending::<()>().await;
            }
            Ok(self.value.clone())
        }

        async fn recheck(
            &self,
            _evidence: &CanonicalCurrentBindingEvidence,
        ) -> Result<(CanonicalCurrentBindingEvidence, DateTime<Utc>), StatusSessionError> {
            if matches!(self.phase, BlockingPhase::Recheck) {
                self.entered.notify_one();
                std::future::pending::<()>().await;
            }
            Ok((self.value.clone(), self.value.observed_at()))
        }
    }

    #[async_trait]
    impl CurrentStatusEvidenceSource for RecheckFailure {
        async fn current(
            &self,
            _request: &CurrentBindingStatusEvidenceRequest,
        ) -> Result<CanonicalCurrentBindingEvidence, StatusSessionError> {
            Ok(self.value.clone())
        }

        async fn recheck(
            &self,
            _evidence: &CanonicalCurrentBindingEvidence,
        ) -> Result<(CanonicalCurrentBindingEvidence, DateTime<Utc>), StatusSessionError> {
            Err(StatusSessionError::EvidenceUnavailable)
        }
    }

    #[tokio::test]
    async fn final_recheck_failure_closes_without_delivering_current() {
        let (evidence, authorization, _) = fixture();
        let sink = SharedSink::new(None);
        let observed = sink.clone();
        let session = ConnectionStatusSession::new(
            RecheckFailure {
                value: evidence.value,
            },
            Contract,
            sink,
            StatusCadence::production(),
            UnchangedBootstrapDelivery::delivered(),
        );
        assert_eq!(
            session
                .run_until_cancelled(authorization, CancellationToken::new())
                .await,
            Err(StatusSessionError::EvidenceUnavailable)
        );
        assert!(observed.current.lock().unwrap().is_empty());
        assert!(observed.closed.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn cancellation_interrupts_stalled_current_and_recheck_reads() {
        for phase in [BlockingPhase::Current, BlockingPhase::Recheck] {
            let (evidence, authorization, _) = fixture();
            let entered = Arc::new(Notify::new());
            let sink = SharedSink::new(None);
            let observed = sink.clone();
            let cancel = CancellationToken::new();
            let session = ConnectionStatusSession::new(
                BlockingEvidence {
                    value: evidence.value,
                    phase,
                    entered: Arc::clone(&entered),
                },
                Contract,
                sink,
                StatusCadence::production(),
                UnchangedBootstrapDelivery::delivered(),
            );
            let task_cancel = cancel.clone();
            let task = tokio::spawn(async move {
                session
                    .run_until_cancelled(authorization, task_cancel)
                    .await
            });
            tokio::time::timeout(Duration::from_secs(1), entered.notified())
                .await
                .expect("evidence phase entered");
            cancel.cancel();
            let result = tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .expect("cancelled status task joined")
                .expect("status task did not panic");
            assert_eq!(result, Ok(()));
            assert!(observed.current.lock().unwrap().is_empty());
            assert!(observed.withdrawals.lock().unwrap().is_empty());
            assert!(!observed.closed.load(Ordering::Acquire));
        }
    }

    #[test]
    fn cadence_cannot_exceed_renewal_or_lifetime_bounds() {
        assert!(StatusCadence::new(Duration::from_secs(121), Duration::from_secs(300)).is_err());
        assert!(StatusCadence::new(Duration::from_secs(120), Duration::from_secs(301)).is_err());
    }

    #[test]
    fn connection_local_contract_signs_current_and_opaque_withdrawal_with_nip11_key() {
        use buzz_core::client_binding_status::validate_client_binding_status_event;

        let (evidence, _, now) = fixture();
        let relay = Keys::generate();
        let other = Keys::generate();
        assert!(ConnectionLocalStatusContract::new(relay.clone(), other.public_key()).is_err());
        let contract =
            ConnectionLocalStatusContract::new(relay.clone(), relay.public_key()).unwrap();

        let current = contract.current(&evidence.value, 1).unwrap();
        let current = validate_client_binding_status_event(
            &current,
            &relay.public_key(),
            evidence.value.authorization_domain(),
            &evidence.value.event_author_pubkey(),
            u64::try_from(now.timestamp()).unwrap(),
        )
        .unwrap();
        assert_eq!(current.status_revision(), 1);
        assert_eq!(current.binding_version(), Some(3));

        let withdrawal = contract
            .withdrawal(
                evidence.value.authorization_domain(),
                evidence.value.event_author_pubkey(),
                2,
                now,
                now + chrono::Duration::seconds(300),
            )
            .unwrap();
        let withdrawal = validate_client_binding_status_event(
            &withdrawal,
            &relay.public_key(),
            evidence.value.authorization_domain(),
            &evidence.value.event_author_pubkey(),
            u64::try_from(now.timestamp()).unwrap(),
        )
        .unwrap();
        assert_eq!(withdrawal.status_revision(), 2);
        assert_eq!(withdrawal.binding_version(), None);
        assert_eq!(withdrawal.policy_version(), None);
    }
}
