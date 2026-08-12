//! Common in-memory bounded authorization lease for protected audio.
//!
//! Audio does not persist an admission ledger. A session combines separately
//! finalized `AudioJoin` and `AudioMedia` authority, atomically subscribes to
//! and rechecks both dependency snapshots, and retains only process-local
//! cancellation state plus a monotonic deadline. Restart therefore discards
//! every session lease and requires complete fresh authorization.

use std::{
    fmt,
    future::Future,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, Weak,
    },
};

use buzz_auth::{
    AuthorizationLeaseDependencySnapshot, BoundedAuthorizationLease, ProofTransport,
    RouteCapability,
};
use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio::time::{sleep_until, Instant};
use tokio_util::sync::CancellationToken;

/// One atomic observation of both audio authorization dependencies.
///
/// The invalidation signal is cancelled when any authoritative dependency
/// changes, including a notice delivered by another relay node. The dependency
/// loss signal is cancelled when the observation stream can no longer prove
/// that it remains current. Both signals are sticky and fail closed.
pub struct AudioLeaseObservation {
    authoritative_now: DateTime<Utc>,
    invalidation: CancellationToken,
    dependency_loss: CancellationToken,
}

impl AudioLeaseObservation {
    /// Construct the result of an atomic subscribe-before-recheck operation.
    pub fn current(
        authoritative_now: DateTime<Utc>,
        invalidation: CancellationToken,
        dependency_loss: CancellationToken,
    ) -> Self {
        Self {
            authoritative_now,
            invalidation,
            dependency_loss,
        }
    }
}

impl fmt::Debug for AudioLeaseObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AudioLeaseObservation([REDACTED])")
    }
}

/// Explicit distributed observation dependency for an audio session lease.
///
/// Implementations MUST subscribe to local and multi-node invalidation/loss
/// notices before atomically rechecking both complete snapshots and database
/// time. They MUST return an error if subscription, recheck, or the observation
/// dependency is unavailable. After success they MUST cancel `invalidation`
/// for any matching authority change and `dependency_loss` if continuing
/// observation can no longer be guaranteed.
pub trait AudioLeaseObservationDependency: Send + Sync {
    /// Dependency-specific error, always mapped to a closed public result.
    type Error: Send;

    /// Subscribe first, then atomically recheck both complete dependency tuples.
    fn subscribe_before_recheck<'a>(
        &'a self,
        join: &'a AuthorizationLeaseDependencySnapshot,
        media: &'a AuthorizationLeaseDependencySnapshot,
    ) -> impl Future<Output = Result<AudioLeaseObservation, Self::Error>> + Send + 'a;
}

/// Provider-free authorizer for a pair of finalized audio capabilities.
///
/// This type owns only its explicit observation dependency. It has no durable
/// session store and cannot restore authority after process restart.
pub struct AudioLeaseAuthorizer<O> {
    observation: O,
}

impl<O> AudioLeaseAuthorizer<O> {
    /// Install the required observation and invalidation dependency.
    pub const fn new(observation: O) -> Self {
        Self { observation }
    }
}

impl<O: AudioLeaseObservationDependency> AudioLeaseAuthorizer<O> {
    /// Authorize one exact finalized join/media pair and begin observation.
    ///
    /// A bounded lease can only be produced by the frozen authorization
    /// finalizer; prepared authorization exposes no lease to this boundary.
    pub async fn authorize(
        &self,
        join: &BoundedAuthorizationLease,
        media: &BoundedAuthorizationLease,
    ) -> Result<BoundedAudioSessionLease, AudioLeaseError> {
        let join_snapshot = join.dependency_snapshot();
        let media_snapshot = media.dependency_snapshot();
        validate_pair(
            &AudioLeaseCoordinates::from_snapshot(&join_snapshot),
            &AudioLeaseCoordinates::from_snapshot(&media_snapshot),
        )?;

        // Anchor before observer I/O. Since authoritative database time is
        // sampled during that I/O, using this earlier monotonic instant cannot
        // extend either lease and is deliberately conservative.
        let local_anchor = Instant::now();
        let observation = self
            .observation
            .subscribe_before_recheck(&join_snapshot, &media_snapshot)
            .await
            .map_err(|_| AudioLeaseError::ObservationUnavailable)?;

        if !join.is_valid_at(observation.authoritative_now)
            || !media.is_valid_at(observation.authoritative_now)
        {
            return Err(AudioLeaseError::Expired);
        }
        let expires_at = join.expires_at().min(media.expires_at());
        let deadline = monotonic_deadline(local_anchor, observation.authoritative_now, expires_at)?;
        let lease = BoundedAudioSessionLease::from_observation(
            observation.invalidation,
            observation.dependency_loss,
            deadline,
            expires_at,
        )?;
        // Observation I/O may have consumed the remaining lifetime. Never
        // return a lease that is already closed at the local monotonic clock.
        lease.revalidate()?;
        Ok(lease)
    }
}

/// Cloneable process-local authority retained by one live audio connection.
#[derive(Clone)]
pub struct BoundedAudioSessionLease {
    state: Arc<AudioLeaseState>,
}

impl BoundedAudioSessionLease {
    fn from_observation(
        invalidation: CancellationToken,
        dependency_loss: CancellationToken,
        deadline: Instant,
        expires_at: DateTime<Utc>,
    ) -> Result<Self, AudioLeaseError> {
        if invalidation.is_cancelled() {
            return Err(AudioLeaseError::Invalidated);
        }
        if dependency_loss.is_cancelled() {
            return Err(AudioLeaseError::DependencyLost);
        }
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| AudioLeaseError::ObservationUnavailable)?;
        let state = Arc::new(AudioLeaseState {
            invalidation,
            dependency_loss,
            cancellation: CancellationToken::new(),
            deadline,
            expires_at,
            terminal: AtomicU8::new(ACTIVE),
        });
        drop(runtime.spawn(watch_terminal_conditions(Arc::downgrade(&state))));
        Ok(Self { state })
    }

    /// Fail closed immediately before admission, media forwarding, or disclosure.
    pub fn revalidate(&self) -> Result<(), AudioLeaseError> {
        self.revalidate_at(Instant::now())
    }

    /// Sticky cancellation shared by every clone of this exact lease.
    ///
    /// The token is cancelled on expiry, local or multi-node invalidation, and
    /// continuing-observation loss.
    pub fn cancellation(&self) -> CancellationToken {
        self.state.cancellation.clone()
    }

    /// Wait until expiry, invalidation, or dependency loss closes the lease.
    pub async fn cancelled(&self) {
        self.state.cancellation.cancelled().await;
    }

    /// Earliest exclusive wall-clock expiry of the join/media pair.
    pub fn expires_at(&self) -> DateTime<Utc> {
        self.state.expires_at
    }

    fn revalidate_at(&self, now: Instant) -> Result<(), AudioLeaseError> {
        if self.state.invalidation.is_cancelled() {
            self.state.transition(INVALIDATED);
        }
        if self.state.dependency_loss.is_cancelled() {
            self.state.transition(DEPENDENCY_LOST);
        }
        if now >= self.state.deadline {
            self.state.transition(EXPIRED);
        }
        self.state.result()
    }
}

impl fmt::Debug for BoundedAudioSessionLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BoundedAudioSessionLease([REDACTED])")
    }
}

const ACTIVE: u8 = 0;
const INVALIDATED: u8 = 1;
const DEPENDENCY_LOST: u8 = 2;
const EXPIRED: u8 = 3;

struct AudioLeaseState {
    invalidation: CancellationToken,
    dependency_loss: CancellationToken,
    cancellation: CancellationToken,
    deadline: Instant,
    expires_at: DateTime<Utc>,
    terminal: AtomicU8,
}

impl AudioLeaseState {
    fn transition(&self, terminal: u8) {
        if self
            .terminal
            .compare_exchange(ACTIVE, terminal, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.cancellation.cancel();
        }
    }

    fn result(&self) -> Result<(), AudioLeaseError> {
        match self.terminal.load(Ordering::Acquire) {
            ACTIVE => Ok(()),
            INVALIDATED => Err(AudioLeaseError::Invalidated),
            DEPENDENCY_LOST => Err(AudioLeaseError::DependencyLost),
            EXPIRED => Err(AudioLeaseError::Expired),
            _ => Err(AudioLeaseError::DependencyLost),
        }
    }
}

async fn watch_terminal_conditions(state: Weak<AudioLeaseState>) {
    let Some(current) = state.upgrade() else {
        return;
    };
    let invalidation = current.invalidation.clone();
    let dependency_loss = current.dependency_loss.clone();
    let deadline = current.deadline;
    drop(current);

    let terminal = tokio::select! {
        biased;
        _ = invalidation.cancelled() => INVALIDATED,
        _ = dependency_loss.cancelled() => DEPENDENCY_LOST,
        _ = sleep_until(deadline) => EXPIRED,
    };
    if let Some(current) = state.upgrade() {
        current.transition(terminal);
    }
}

#[derive(Clone, PartialEq, Eq)]
struct AudioLeaseCoordinates {
    capability: RouteCapability,
    authorization_domain: buzz_core::CommunityId,
    actor_pubkey: nostr::PublicKey,
    owner_pubkey: Option<nostr::PublicKey>,
    binding: (uuid::Uuid, u64),
    delegated_relationship: Option<(uuid::Uuid, u64, [u8; 32])>,
    transport: ProofTransport,
    target_fingerprint: [u8; 32],
    transport_context_fingerprint: [u8; 32],
}

impl AudioLeaseCoordinates {
    fn from_snapshot(snapshot: &AuthorizationLeaseDependencySnapshot) -> Self {
        let (capability, actor_pubkey, owner_pubkey) = snapshot.authority();
        let (_, target_fingerprint, transport, transport_context_fingerprint) =
            snapshot.request_binding();
        Self {
            capability,
            authorization_domain: snapshot.identity().1,
            actor_pubkey,
            owner_pubkey,
            binding: snapshot.binding(),
            delegated_relationship: snapshot
                .delegated_relationship()
                .map(|(id, revision, conditions)| (id, revision, *conditions)),
            transport,
            target_fingerprint: *target_fingerprint,
            transport_context_fingerprint: *transport_context_fingerprint,
        }
    }
}

fn validate_pair(
    join: &AudioLeaseCoordinates,
    media: &AudioLeaseCoordinates,
) -> Result<(), AudioLeaseError> {
    if join.capability != RouteCapability::AudioJoin
        || media.capability != RouteCapability::AudioMedia
    {
        return Err(AudioLeaseError::CapabilityMismatch);
    }
    if join.authorization_domain != media.authorization_domain
        || join.actor_pubkey != media.actor_pubkey
        || join.owner_pubkey != media.owner_pubkey
        || join.binding != media.binding
        || join.delegated_relationship != media.delegated_relationship
        || join.transport != media.transport
        || join.target_fingerprint != media.target_fingerprint
        || join.transport_context_fingerprint != media.transport_context_fingerprint
    {
        return Err(AudioLeaseError::CoordinateMismatch);
    }
    Ok(())
}

fn monotonic_deadline(
    local_anchor: Instant,
    authoritative_now: DateTime<Utc>,
    expires_at: DateTime<Utc>,
) -> Result<Instant, AudioLeaseError> {
    if expires_at <= authoritative_now {
        return Err(AudioLeaseError::Expired);
    }
    let remaining = (expires_at - authoritative_now)
        .to_std()
        .map_err(|_| AudioLeaseError::Expired)?;
    local_anchor
        .checked_add(remaining)
        .ok_or(AudioLeaseError::Expired)
}

/// Stable fail-closed errors for common audio lease authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AudioLeaseError {
    /// The contexts were not exactly `AudioJoin` and `AudioMedia`.
    #[error("audio capability mismatch")]
    CapabilityMismatch,
    /// Join and media authority named different stable session coordinates.
    #[error("audio authorization coordinates mismatch")]
    CoordinateMismatch,
    /// The atomic observation dependency was unavailable.
    #[error("audio authorization observation unavailable")]
    ObservationUnavailable,
    /// The earliest exclusive join/media deadline was reached.
    #[error("audio authorization expired")]
    Expired,
    /// A local or multi-node authority dependency changed.
    #[error("audio authorization invalidated")]
    Invalidated,
    /// Continuing distributed observation could no longer be guaranteed.
    #[error("audio authorization dependency lost")]
    DependencyLost,
}

impl AudioLeaseError {
    /// Stable machine code that contains no authority or credential material.
    pub const fn code(self) -> &'static str {
        match self {
            Self::CapabilityMismatch => "nip_fi_audio_capability_mismatch",
            Self::CoordinateMismatch => "nip_fi_audio_coordinate_mismatch",
            Self::ObservationUnavailable => "nip_fi_audio_observation_unavailable",
            Self::Expired => "nip_fi_audio_expired",
            Self::Invalidated => "nip_fi_audio_invalidated",
            Self::DependencyLost => "nip_fi_audio_dependency_lost",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;
    use std::time::Duration;

    fn coordinates(capability: RouteCapability) -> AudioLeaseCoordinates {
        AudioLeaseCoordinates {
            capability,
            authorization_domain: buzz_core::CommunityId::from_uuid(uuid::Uuid::from_u128(1)),
            actor_pubkey: Keys::generate().public_key(),
            owner_pubkey: None,
            binding: (uuid::Uuid::from_u128(2), 3),
            delegated_relationship: Some((uuid::Uuid::from_u128(4), 5, [7; 32])),
            transport: ProofTransport::Nip42,
            target_fingerprint: [8; 32],
            transport_context_fingerprint: [9; 32],
        }
    }

    fn pair() -> (AudioLeaseCoordinates, AudioLeaseCoordinates) {
        let join = coordinates(RouteCapability::AudioJoin);
        let mut media = join.clone();
        media.capability = RouteCapability::AudioMedia;
        (join, media)
    }

    fn test_lease(
        invalidation: CancellationToken,
        dependency_loss: CancellationToken,
        deadline: Instant,
    ) -> BoundedAudioSessionLease {
        BoundedAudioSessionLease {
            state: Arc::new(AudioLeaseState {
                invalidation,
                dependency_loss,
                cancellation: CancellationToken::new(),
                deadline,
                expires_at: fixture_time() + chrono::TimeDelta::seconds(30),
                terminal: AtomicU8::new(ACTIVE),
            }),
        }
    }

    #[test]
    fn exact_shared_session_pair_is_required() {
        let (join, media) = pair();
        assert_eq!(validate_pair(&join, &media), Ok(()));

        let mut changed = media.clone();
        changed.actor_pubkey = Keys::generate().public_key();
        assert_eq!(
            validate_pair(&join, &changed),
            Err(AudioLeaseError::CoordinateMismatch)
        );
        let mut changed = media.clone();
        changed.binding.1 += 1;
        assert_eq!(
            validate_pair(&join, &changed),
            Err(AudioLeaseError::CoordinateMismatch)
        );
        let mut changed = media.clone();
        changed.delegated_relationship = None;
        assert_eq!(
            validate_pair(&join, &changed),
            Err(AudioLeaseError::CoordinateMismatch)
        );
        let mut changed = media.clone();
        changed.target_fingerprint = [10; 32];
        assert_eq!(
            validate_pair(&join, &changed),
            Err(AudioLeaseError::CoordinateMismatch)
        );
        let mut changed = media;
        changed.capability = RouteCapability::GitRead;
        assert_eq!(
            validate_pair(&join, &changed),
            Err(AudioLeaseError::CapabilityMismatch)
        );
    }

    #[test]
    fn authoritative_time_produces_an_exclusive_unrounded_deadline() {
        let local_anchor = Instant::now();
        let authoritative_now = fixture_time();
        let deadline = monotonic_deadline(
            local_anchor,
            authoritative_now,
            authoritative_now + chrono::TimeDelta::milliseconds(1500),
        )
        .expect("future deadline");
        let lease = test_lease(CancellationToken::new(), CancellationToken::new(), deadline);
        assert_eq!(lease.revalidate_at(local_anchor), Ok(()));
        assert_eq!(
            lease.revalidate_at(local_anchor + Duration::from_millis(1499)),
            Ok(())
        );
        assert_eq!(
            lease.revalidate_at(local_anchor + Duration::from_millis(1500)),
            Err(AudioLeaseError::Expired)
        );
        assert!(lease.cancellation().is_cancelled());
    }

    #[test]
    fn expired_authoritative_observation_fails_closed() {
        let now = fixture_time();
        assert_eq!(
            monotonic_deadline(Instant::now(), now, now),
            Err(AudioLeaseError::Expired)
        );
    }

    #[tokio::test]
    async fn multi_node_invalidation_is_sticky_across_clones() {
        let invalidation = CancellationToken::new();
        let lease = BoundedAudioSessionLease::from_observation(
            invalidation.clone(),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(30),
            fixture_time() + chrono::TimeDelta::seconds(30),
        )
        .expect("active observation");
        let clone = lease.clone();

        invalidation.cancel();
        lease.cancelled().await;
        assert_eq!(lease.revalidate(), Err(AudioLeaseError::Invalidated));
        assert_eq!(clone.revalidate(), Err(AudioLeaseError::Invalidated));
    }

    #[tokio::test]
    async fn dependency_loss_is_distinct_sticky_and_fail_closed() {
        let dependency_loss = CancellationToken::new();
        let lease = BoundedAudioSessionLease::from_observation(
            CancellationToken::new(),
            dependency_loss.clone(),
            Instant::now() + Duration::from_secs(30),
            fixture_time() + chrono::TimeDelta::seconds(30),
        )
        .expect("active observation");
        let clone = lease.clone();

        dependency_loss.cancel();
        lease.cancelled().await;
        assert_eq!(lease.revalidate(), Err(AudioLeaseError::DependencyLost));
        assert_eq!(clone.revalidate(), Err(AudioLeaseError::DependencyLost));
    }

    #[tokio::test]
    async fn expiry_wakes_waiters_and_remains_sticky() {
        let lease = BoundedAudioSessionLease::from_observation(
            CancellationToken::new(),
            CancellationToken::new(),
            Instant::now() + Duration::from_millis(5),
            fixture_time() + chrono::TimeDelta::milliseconds(5),
        )
        .expect("active observation");
        let clone = lease.clone();

        lease.cancelled().await;
        assert_eq!(lease.revalidate(), Err(AudioLeaseError::Expired));
        assert_eq!(clone.revalidate(), Err(AudioLeaseError::Expired));
    }

    #[tokio::test]
    async fn already_closed_observations_never_create_authority() {
        let invalidation = CancellationToken::new();
        invalidation.cancel();
        assert!(matches!(
            BoundedAudioSessionLease::from_observation(
                invalidation,
                CancellationToken::new(),
                Instant::now() + Duration::from_secs(30),
                fixture_time() + chrono::TimeDelta::seconds(30),
            ),
            Err(AudioLeaseError::Invalidated)
        ));

        let dependency_loss = CancellationToken::new();
        dependency_loss.cancel();
        assert!(matches!(
            BoundedAudioSessionLease::from_observation(
                CancellationToken::new(),
                dependency_loss,
                Instant::now() + Duration::from_secs(30),
                fixture_time() + chrono::TimeDelta::seconds(30),
            ),
            Err(AudioLeaseError::DependencyLost)
        ));
    }

    #[tokio::test]
    async fn a_new_process_local_observation_cannot_resurrect_an_old_lease() {
        let old_invalidation = CancellationToken::new();
        let old = BoundedAudioSessionLease::from_observation(
            old_invalidation.clone(),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(30),
            fixture_time() + chrono::TimeDelta::seconds(30),
        )
        .expect("active old observation");
        old_invalidation.cancel();
        old.cancelled().await;

        let fresh = BoundedAudioSessionLease::from_observation(
            CancellationToken::new(),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(30),
            fixture_time() + chrono::TimeDelta::seconds(30),
        )
        .expect("fresh observation");
        assert_eq!(old.revalidate(), Err(AudioLeaseError::Invalidated));
        assert_eq!(fresh.revalidate(), Ok(()));
    }

    #[test]
    fn debug_and_error_codes_disclose_no_authority() {
        let lease = test_lease(
            CancellationToken::new(),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(30),
        );
        assert_eq!(format!("{lease:?}"), "BoundedAudioSessionLease([REDACTED])");
        assert_eq!(
            AudioLeaseError::DependencyLost.code(),
            "nip_fi_audio_dependency_lost"
        );
    }

    fn fixture_time() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).expect("fixture time")
    }
}
