//! Bounded, versioned authorization leases.
//!
//! A lease is issued only by the crate-owned federated finalizer after all
//! identity, provider, delegation, and binding evidence has been validated.
//! Lease consumers must supply the current typed context, lease, binding,
//! provider profile, and policy versions plus the exact capability being
//! exercised.

use std::{fmt, sync::Arc, time::SystemTime};

use buzz_core::CommunityId;
use nostr::PublicKey;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    context::{AuthContextVersion, AuthTransport, BindingVersion, VersionedBindingRef},
    provider::{AuthorizationCapability, AuthorizationProfileId, CapabilitySet, PolicyVersion},
};

/// Largest application lease duration accepted by the portable runtime.
pub const MAX_APPLICATION_LEASE_SECONDS: u64 = 86_400;
/// Largest explicit clock-skew allowance accepted by the portable runtime.
pub const MAX_AUTHORIZATION_CLOCK_SKEW_SECONDS: u64 = 300;
/// Largest renewal lead time accepted by the portable runtime.
pub const MAX_LEASE_RENEWAL_LEAD_SECONDS: u64 = 3_600;

/// Centrally supplied authorization time in Unix seconds.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AuthorizationTime(u64);

impl AuthorizationTime {
    /// Preserve a Unix timestamp supplied by an authorization clock.
    pub const fn from_unix_seconds(unix_seconds: u64) -> Self {
        Self(unix_seconds)
    }

    /// Timestamp as seconds since the Unix epoch.
    pub const fn unix_seconds(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for AuthorizationTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("AuthorizationTime")
            .field(&"[redacted]")
            .finish()
    }
}

/// Centrally injected time source for authorization decisions and lease use.
pub trait AuthorizationClock: Send + Sync {
    /// Return the current authorization time.
    fn now(&self) -> Result<AuthorizationTime, AuthorizationClockError>;
}

/// System-backed authorization clock used outside deterministic tests.
#[derive(Debug, Default)]
pub struct SystemAuthorizationClock;

impl AuthorizationClock for SystemAuthorizationClock {
    fn now(&self) -> Result<AuthorizationTime, AuthorizationClockError> {
        let elapsed = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|_| AuthorizationClockError::BeforeUnixEpoch)?;
        Ok(AuthorizationTime::from_unix_seconds(elapsed.as_secs()))
    }
}

/// Failure to obtain central authorization time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AuthorizationClockError {
    /// The system clock was earlier than the Unix epoch.
    #[error("authorization clock is earlier than the Unix epoch")]
    BeforeUnixEpoch,
    /// The injected clock could not provide a current value.
    #[error("authorization clock is unavailable")]
    Unavailable,
}

/// Shared injected authorization clock.
pub type SharedAuthorizationClock = Arc<dyn AuthorizationClock>;

/// Explicit conservative clock-skew allowance.
///
/// Skew is subtracted from every evidence bound when authority is issued; it
/// never extends a lease past an assertion, provider, delegation, binding, or
/// application expiry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorizationClockSkew(u64);

impl AuthorizationClockSkew {
    /// Build a bounded skew allowance in seconds.
    pub const fn from_seconds(seconds: u64) -> Result<Self, LeasePolicyError> {
        if seconds > MAX_AUTHORIZATION_CLOCK_SKEW_SECONDS {
            return Err(LeasePolicyError::ClockSkewTooLarge);
        }
        Ok(Self(seconds))
    }

    /// Configured conservative skew allowance in seconds.
    pub const fn seconds(self) -> u64 {
        self.0
    }
}

/// Maximum application lifetime for an access lease or verification status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplicationLeaseLimit(u64);

impl ApplicationLeaseLimit {
    /// Build a non-zero application lifetime no greater than one day.
    pub const fn from_seconds(seconds: u64) -> Result<Self, LeasePolicyError> {
        if seconds == 0 {
            return Err(LeasePolicyError::ZeroApplicationLimit);
        }
        if seconds > MAX_APPLICATION_LEASE_SECONDS {
            return Err(LeasePolicyError::ApplicationLimitTooLarge);
        }
        Ok(Self(seconds))
    }

    /// Configured maximum lifetime in seconds.
    pub const fn seconds(self) -> u64 {
        self.0
    }
}

/// Server-owned access-lease bounds for one authorization domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessLeasePolicy {
    application_limit: ApplicationLeaseLimit,
    clock_skew: AuthorizationClockSkew,
}

impl AccessLeasePolicy {
    /// Combine an application maximum with an explicit conservative skew.
    pub const fn new(
        application_limit: ApplicationLeaseLimit,
        clock_skew: AuthorizationClockSkew,
    ) -> Self {
        Self {
            application_limit,
            clock_skew,
        }
    }

    /// Configured application maximum.
    pub const fn application_limit(self) -> ApplicationLeaseLimit {
        self.application_limit
    }

    /// Configured conservative clock skew.
    pub const fn clock_skew(self) -> AuthorizationClockSkew {
        self.clock_skew
    }
}

/// Server-owned display-status bounds for verification-only mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerificationStatusPolicy {
    application_limit: ApplicationLeaseLimit,
    clock_skew: AuthorizationClockSkew,
}

impl VerificationStatusPolicy {
    /// Combine a short display maximum with an explicit conservative skew.
    pub const fn new(
        application_limit: ApplicationLeaseLimit,
        clock_skew: AuthorizationClockSkew,
    ) -> Self {
        Self {
            application_limit,
            clock_skew,
        }
    }

    /// Configured display-status maximum.
    pub const fn application_limit(self) -> ApplicationLeaseLimit {
        self.application_limit
    }

    /// Configured conservative clock skew.
    pub const fn clock_skew(self) -> AuthorizationClockSkew {
        self.clock_skew
    }
}

/// Invalid server-owned lease policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LeasePolicyError {
    /// An application maximum was zero.
    #[error("application authorization limit must be greater than zero")]
    ZeroApplicationLimit,
    /// An application maximum exceeded the portable upper bound.
    #[error("application authorization limit exceeds the portable maximum")]
    ApplicationLimitTooLarge,
    /// The clock-skew allowance exceeded the portable upper bound.
    #[error("authorization clock skew exceeds the portable maximum")]
    ClockSkewTooLarge,
    /// A renewal lead time was zero.
    #[error("authorization lease renewal lead must be greater than zero")]
    ZeroRenewalLead,
    /// A renewal lead time exceeded the portable upper bound.
    #[error("authorization lease renewal lead exceeds the portable maximum")]
    RenewalLeadTooLarge,
}

/// Monotonic version of an issued authorization lease.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LeaseVersion(u64);

impl LeaseVersion {
    /// Initial lease version for a newly authorized session.
    pub const INITIAL: Self = Self(1);

    /// Build a positive lease version.
    pub const fn new(value: u64) -> Result<Self, LeaseIssueError> {
        if value == 0 {
            return Err(LeaseIssueError::InvalidLeaseVersion);
        }
        Ok(Self(value))
    }

    /// Numeric lease version.
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for LeaseVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("LeaseVersion")
            .field(&"[redacted]")
            .finish()
    }
}

/// Freshness bound obtained with an authoritative active binding.
///
/// The identifier and version prevent a bound from one binding snapshot from
/// being reused with another binding or lifecycle version.
#[derive(PartialEq, Eq)]
pub struct BindingLeaseBound {
    binding_id: Uuid,
    binding_version: BindingVersion,
    valid_until: u64,
}

impl BindingLeaseBound {
    /// Attach a non-zero freshness bound to an authoritative active binding.
    pub fn new(binding: &VersionedBindingRef, valid_until: u64) -> Result<Self, LeaseIssueError> {
        if valid_until == 0 {
            return Err(LeaseIssueError::InvalidBindingBound);
        }
        Ok(Self {
            binding_id: binding.binding_id(),
            binding_version: binding.binding_version(),
            valid_until,
        })
    }

    /// Stable binding identifier represented by this bound.
    pub const fn binding_id(&self) -> Uuid {
        self.binding_id
    }

    /// Exact active binding version represented by this bound.
    pub const fn binding_version(&self) -> BindingVersion {
        self.binding_version
    }

    /// Time after which active binding state must be resolved again.
    pub const fn valid_until(&self) -> u64 {
        self.valid_until
    }
}

impl fmt::Debug for BindingLeaseBound {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BindingLeaseBound")
            .field("binding_id", &"[redacted]")
            .field("binding_version", &"[redacted]")
            .field("valid_until", &"[redacted]")
            .finish()
    }
}

/// Versioned authority for a bounded set of federated capabilities.
///
/// This type has no public constructor or deserialization path. Only the
/// crate-owned finalizer can issue it from a validated provider snapshot and
/// matching active binding evidence.
#[derive(PartialEq, Eq)]
pub struct AuthorizationLease {
    context_version: AuthContextVersion,
    lease_version: LeaseVersion,
    authorization_domain: CommunityId,
    transport: AuthTransport,
    actor_pubkey: PublicKey,
    owner_pubkey: Option<PublicKey>,
    binding_id: Uuid,
    binding_version: BindingVersion,
    profile_id: AuthorizationProfileId,
    policy_version: PolicyVersion,
    capabilities: CapabilitySet,
    issued_at: u64,
    expires_at: u64,
    correlation_id: Uuid,
}

impl AuthorizationLease {
    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)]
    pub(crate) fn issue(
        lease_version: LeaseVersion,
        authorization_domain: CommunityId,
        transport: AuthTransport,
        actor_pubkey: PublicKey,
        owner_pubkey: Option<PublicKey>,
        binding: &VersionedBindingRef,
        binding_bound: BindingLeaseBound,
        profile_id: AuthorizationProfileId,
        policy_version: PolicyVersion,
        capabilities: CapabilitySet,
        provider_effective_until: u64,
        now: AuthorizationTime,
        policy: AccessLeasePolicy,
        correlation_id: Uuid,
    ) -> Result<Self, LeaseIssueError> {
        if binding_bound.binding_id != binding.binding_id()
            || binding_bound.binding_version != binding.binding_version()
        {
            return Err(LeaseIssueError::BindingBoundMismatch);
        }
        let expires_at = conservative_expiry(
            now,
            provider_effective_until,
            binding_bound.valid_until,
            policy.application_limit,
            policy.clock_skew,
        )?;
        Ok(Self {
            context_version: AuthContextVersion::V1,
            lease_version,
            authorization_domain,
            transport,
            actor_pubkey,
            owner_pubkey,
            binding_id: binding.binding_id(),
            binding_version: binding.binding_version(),
            profile_id,
            policy_version,
            capabilities,
            issued_at: now.unix_seconds(),
            expires_at,
            correlation_id,
        })
    }

    /// Authorization-context contract version carried by the lease.
    pub const fn context_version(&self) -> AuthContextVersion {
        self.context_version
    }

    /// Monotonic lease version.
    pub const fn lease_version(&self) -> LeaseVersion {
        self.lease_version
    }

    /// Server-resolved authorization domain.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Exact transport authorized by the lease.
    pub const fn transport(&self) -> AuthTransport {
        self.transport
    }

    /// Authenticated Nostr actor authorized by the lease.
    pub const fn actor_pubkey(&self) -> PublicKey {
        self.actor_pubkey
    }

    /// Verified delegated owner, when the actor is delegated.
    pub const fn owner_pubkey(&self) -> Option<PublicKey> {
        self.owner_pubkey
    }

    /// Stable active binding identifier.
    pub const fn binding_id(&self) -> Uuid {
        self.binding_id
    }

    /// Exact active binding version.
    pub const fn binding_version(&self) -> BindingVersion {
        self.binding_version
    }

    /// Server-resolved provider profile.
    pub const fn profile_id(&self) -> &AuthorizationProfileId {
        &self.profile_id
    }

    /// Opaque provider policy version.
    pub const fn policy_version(&self) -> &PolicyVersion {
        &self.policy_version
    }

    /// Exact capabilities granted by current provider policy.
    pub const fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    /// Central issue time in Unix seconds.
    pub const fn issued_at(&self) -> u64 {
        self.issued_at
    }

    /// Conservative earliest expiry in Unix seconds.
    pub const fn expires_at(&self) -> u64 {
        self.expires_at
    }

    /// Correlation identifier for the decision that issued the lease.
    pub const fn correlation_id(&self) -> Uuid {
        self.correlation_id
    }

    /// Compute deterministic renewal and hard-expiry deadlines.
    pub fn renewal_schedule(&self, lead_time: LeaseRenewalLeadTime) -> LeaseRenewalSchedule {
        LeaseRenewalSchedule {
            renew_at: self
                .expires_at
                .saturating_sub(lead_time.seconds())
                .max(self.issued_at),
            expires_at: self.expires_at,
        }
    }
}

impl fmt::Debug for AuthorizationLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationLease")
            .field("context_version", &self.context_version)
            .field("lease_version", &"[redacted]")
            .field("authorization_domain", &"[redacted]")
            .field("transport", &self.transport)
            .field("actor_pubkey", &"[redacted]")
            .field("owner_pubkey", &"[redacted]")
            .field("binding_id", &"[redacted]")
            .field("binding_version", &"[redacted]")
            .field("profile_id", &"[redacted]")
            .field("policy_version", &"[redacted]")
            .field("capabilities", &"[redacted]")
            .field("issued_at", &"[redacted]")
            .field("expires_at", &"[redacted]")
            .field("correlation_id", &"[redacted]")
            .finish()
    }
}

/// Current typed state required to consume an authorization lease.
#[derive(Clone)]
pub struct LeaseUseRequirement {
    /// Required authorization-context contract version.
    pub context_version: AuthContextVersion,
    /// Current lease version for the session.
    pub lease_version: LeaseVersion,
    /// Server-resolved authorization domain of the operation.
    pub authorization_domain: CommunityId,
    /// Transport carrying the protected operation.
    pub transport: AuthTransport,
    /// Authenticated actor performing the operation.
    pub actor_pubkey: PublicKey,
    /// Stable identifier of the current active binding.
    pub binding_id: Uuid,
    /// Current active binding version.
    pub binding_version: BindingVersion,
    /// Exact server-resolved provider profile currently selected for the operation.
    pub profile_id: AuthorizationProfileId,
    /// Current provider policy version.
    pub policy_version: PolicyVersion,
    /// Exact capability required by the operation.
    pub capability: AuthorizationCapability,
}

impl fmt::Debug for LeaseUseRequirement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LeaseUseRequirement")
            .field("context_version", &self.context_version)
            .field("lease_version", &"[redacted]")
            .field("authorization_domain", &"[redacted]")
            .field("transport", &self.transport)
            .field("actor_pubkey", &"[redacted]")
            .field("binding_id", &"[redacted]")
            .field("binding_version", &"[redacted]")
            .field("profile_id", &"[redacted]")
            .field("policy_version", &"[redacted]")
            .field("capability", &"[redacted]")
            .finish()
    }
}

/// Central validator for protected lease use.
#[derive(Clone)]
pub struct AuthorizationLeaseValidator {
    clock: SharedAuthorizationClock,
}

impl AuthorizationLeaseValidator {
    /// Create a validator that always reads from the supplied central clock.
    pub fn new(clock: SharedAuthorizationClock) -> Self {
        Self { clock }
    }

    /// Validate every typed version, subject, transport, capability, and time bound.
    pub fn authorize(
        &self,
        lease: &AuthorizationLease,
        requirement: &LeaseUseRequirement,
    ) -> Result<(), LeaseValidationError> {
        if lease.context_version != requirement.context_version {
            return Err(LeaseValidationError::ContextVersionMismatch);
        }
        if lease.lease_version != requirement.lease_version {
            return Err(LeaseValidationError::LeaseVersionMismatch);
        }
        if lease.authorization_domain != requirement.authorization_domain {
            return Err(LeaseValidationError::AuthorizationDomainMismatch);
        }
        if lease.transport != requirement.transport {
            return Err(LeaseValidationError::TransportMismatch);
        }
        if lease.actor_pubkey != requirement.actor_pubkey {
            return Err(LeaseValidationError::ActorMismatch);
        }
        if lease.binding_id != requirement.binding_id {
            return Err(LeaseValidationError::BindingIdMismatch);
        }
        if lease.binding_version != requirement.binding_version {
            return Err(LeaseValidationError::BindingVersionMismatch);
        }
        if lease.profile_id != requirement.profile_id {
            return Err(LeaseValidationError::AuthorizationProfileMismatch);
        }
        if lease.policy_version != requirement.policy_version {
            return Err(LeaseValidationError::PolicyVersionMismatch);
        }
        if !lease.capabilities.contains(requirement.capability) {
            return Err(LeaseValidationError::MissingCapability);
        }
        let now = self.clock.now()?;
        if now.unix_seconds() < lease.issued_at {
            return Err(LeaseValidationError::ClockMovedBeforeIssue);
        }
        if now.unix_seconds() >= lease.expires_at {
            return Err(LeaseValidationError::Expired);
        }
        Ok(())
    }

    /// Validate and retain a per-capability operation guard.
    ///
    /// Long-running streams and mutations retain this guard and call
    /// [`AuthorizationOperationGuard::revalidate`] immediately before each
    /// protected emission or commit. The guard borrows an access lease and
    /// cannot be constructed from verification-only status.
    pub fn operation_guard<'a>(
        &self,
        lease: &'a AuthorizationLease,
        requirement: LeaseUseRequirement,
    ) -> Result<AuthorizationOperationGuard<'a>, LeaseValidationError> {
        self.authorize(lease, &requirement)?;
        Ok(AuthorizationOperationGuard {
            lease,
            validator: self.clone(),
            requirement,
        })
    }

    /// Return the deterministic renewal action for WebSocket or audio authority.
    pub fn renewal_action(
        &self,
        schedule: LeaseRenewalSchedule,
    ) -> Result<LeaseRenewalAction, LeaseValidationError> {
        let now = self.clock.now()?;
        if now.unix_seconds() >= schedule.expires_at {
            return Ok(LeaseRenewalAction::Expired);
        }
        if now.unix_seconds() >= schedule.renew_at {
            return Ok(LeaseRenewalAction::RenewNow);
        }
        Ok(LeaseRenewalAction::Current)
    }

    /// Remaining whole seconds until a trusted authorization deadline.
    ///
    /// Long-lived transports use the same injected clock as lease validation;
    /// they must not schedule expiry from an independently read wall clock.
    pub fn seconds_until(&self, expires_at: u64) -> Result<u64, LeaseValidationError> {
        let now = self.clock.now()?;
        Ok(expires_at.saturating_sub(now.unix_seconds()))
    }
}

impl fmt::Debug for AuthorizationLeaseValidator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationLeaseValidator")
            .field("clock", &"[injected]")
            .finish()
    }
}

/// Retained per-capability authorization for one in-flight operation.
///
/// Initial validation is not a substitute for the mandatory pre-commit or
/// pre-emission revalidation hook. Expiry and current typed versions are
/// checked again every time [`Self::revalidate`] is called.
pub struct AuthorizationOperationGuard<'a> {
    lease: &'a AuthorizationLease,
    validator: AuthorizationLeaseValidator,
    requirement: LeaseUseRequirement,
}

impl AuthorizationOperationGuard<'_> {
    /// Revalidate the retained authority immediately before a protected effect.
    pub fn revalidate(&self) -> Result<(), LeaseValidationError> {
        self.validator.authorize(self.lease, &self.requirement)
    }

    /// Exact capability retained by this guard.
    pub const fn capability(&self) -> AuthorizationCapability {
        self.requirement.capability
    }

    /// Hard conservative expiry after which revalidation always denies.
    pub const fn expires_at(&self) -> u64 {
        self.lease.expires_at
    }
}

impl fmt::Debug for AuthorizationOperationGuard<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationOperationGuard")
            .field("lease", &"[borrowed]")
            .field("validator", &self.validator)
            .field("requirement", &self.requirement)
            .finish()
    }
}

/// Bounded lead time for renewing long-lived authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaseRenewalLeadTime(u64);

impl LeaseRenewalLeadTime {
    /// Build a non-zero renewal lead no greater than one hour.
    pub const fn from_seconds(seconds: u64) -> Result<Self, LeasePolicyError> {
        if seconds == 0 {
            return Err(LeasePolicyError::ZeroRenewalLead);
        }
        if seconds > MAX_LEASE_RENEWAL_LEAD_SECONDS {
            return Err(LeasePolicyError::RenewalLeadTooLarge);
        }
        Ok(Self(seconds))
    }

    /// Renewal lead in seconds.
    pub const fn seconds(self) -> u64 {
        self.0
    }
}

/// Deterministic renewal and fail-closed expiry deadlines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaseRenewalSchedule {
    renew_at: u64,
    expires_at: u64,
}

impl LeaseRenewalSchedule {
    /// Time at which a long-lived transport should begin renewal.
    pub const fn renew_at(self) -> u64 {
        self.renew_at
    }

    /// Hard deadline at which the transport must stop using authority.
    pub const fn expires_at(self) -> u64 {
        self.expires_at
    }
}

/// Current hook result for a long-lived transport's renewal loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseRenewalAction {
    /// Existing authority remains current before its renewal window.
    Current,
    /// Renewal is due; failure to renew before expiry must close fail-closed.
    RenewNow,
    /// The hard deadline passed; authority must no longer be used.
    Expired,
}

/// Failure to issue an authorization lease or bounded display status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LeaseIssueError {
    /// A lease version was zero.
    #[error("authorization lease version must be greater than zero")]
    InvalidLeaseVersion,
    /// A binding freshness bound was zero.
    #[error("binding freshness bound must be greater than zero")]
    InvalidBindingBound,
    /// Binding freshness evidence named another binding or version.
    #[error("binding freshness bound does not match the active binding")]
    BindingBoundMismatch,
    /// Computing the configured application bound overflowed Unix seconds.
    #[error("application authorization bound overflowed")]
    ApplicationBoundOverflow,
    /// At least one required evidence bound was already expired or inside skew.
    #[error("authorization evidence is no longer current")]
    EvidenceExpired,
}

impl LeaseIssueError {
    /// Stable audit and metric code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidLeaseVersion => "authorization_lease_issue_001",
            Self::InvalidBindingBound => "authorization_lease_issue_002",
            Self::BindingBoundMismatch => "authorization_lease_issue_003",
            Self::ApplicationBoundOverflow => "authorization_lease_issue_004",
            Self::EvidenceExpired => "authorization_lease_issue_005",
        }
    }
}

/// Fail-closed protected-operation lease validation error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LeaseValidationError {
    /// The central authorization clock failed.
    #[error(transparent)]
    Clock(#[from] AuthorizationClockError),
    /// The context did not carry a federated access lease.
    #[error("authorization context does not carry an access lease")]
    MissingLease,
    /// The authorization-context contract version was not exact.
    #[error("authorization context version does not match")]
    ContextVersionMismatch,
    /// The current session lease version was not exact.
    #[error("authorization lease version does not match")]
    LeaseVersionMismatch,
    /// The operation belonged to another authorization domain.
    #[error("authorization lease domain does not match")]
    AuthorizationDomainMismatch,
    /// The operation used another transport.
    #[error("authorization lease transport does not match")]
    TransportMismatch,
    /// The operation was performed by another actor.
    #[error("authorization lease actor does not match")]
    ActorMismatch,
    /// Current binding state identifies another stable binding.
    #[error("authorization lease binding identifier does not match")]
    BindingIdMismatch,
    /// Current binding state has another version.
    #[error("authorization lease binding version does not match")]
    BindingVersionMismatch,
    /// Current server policy selected another provider profile.
    #[error("authorization lease provider profile does not match")]
    AuthorizationProfileMismatch,
    /// Current provider policy has another version.
    #[error("authorization lease policy version does not match")]
    PolicyVersionMismatch,
    /// The lease did not grant the operation's exact capability.
    #[error("authorization lease does not include the required capability")]
    MissingCapability,
    /// Central time moved before the lease issue time.
    #[error("authorization clock moved before lease issue time")]
    ClockMovedBeforeIssue,
    /// The lease reached its conservative earliest expiry.
    #[error("authorization lease has expired")]
    Expired,
}

impl LeaseValidationError {
    /// Stable audit and metric code.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Clock(_) => "authorization_lease_use_001",
            Self::MissingLease => "authorization_lease_use_002",
            Self::ContextVersionMismatch => "authorization_lease_use_003",
            Self::LeaseVersionMismatch => "authorization_lease_use_004",
            Self::AuthorizationDomainMismatch => "authorization_lease_use_005",
            Self::TransportMismatch => "authorization_lease_use_006",
            Self::ActorMismatch => "authorization_lease_use_007",
            Self::BindingIdMismatch => "authorization_lease_use_013",
            Self::BindingVersionMismatch => "authorization_lease_use_008",
            Self::AuthorizationProfileMismatch => "authorization_lease_use_014",
            Self::PolicyVersionMismatch => "authorization_lease_use_009",
            Self::MissingCapability => "authorization_lease_use_010",
            Self::ClockMovedBeforeIssue => "authorization_lease_use_011",
            Self::Expired => "authorization_lease_use_012",
        }
    }
}

#[allow(dead_code)]
pub(crate) fn conservative_expiry(
    now: AuthorizationTime,
    provider_effective_until: u64,
    binding_valid_until: u64,
    application_limit: ApplicationLeaseLimit,
    clock_skew: AuthorizationClockSkew,
) -> Result<u64, LeaseIssueError> {
    let application_until = now
        .unix_seconds()
        .checked_add(application_limit.seconds())
        .ok_or(LeaseIssueError::ApplicationBoundOverflow)?;
    let earliest = provider_effective_until
        .min(binding_valid_until)
        .min(application_until);
    let conservative = earliest.saturating_sub(clock_skew.seconds());
    if conservative <= now.unix_seconds() {
        return Err(LeaseIssueError::EvidenceExpired);
    }
    Ok(conservative)
}
