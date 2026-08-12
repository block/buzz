//! Relay-authenticated client binding status.
//!
//! Kind `24244` is a short-lived, ephemeral envelope whose JSON content names
//! the exact authorization domain and event-author key to which presentation
//! applies. The status is display-only: it is not identity proof, membership,
//! an authorization decision, or an access lease. Consumers must obtain a
//! value through
//! [`validate_client_binding_status_event`](crate::client_binding_status::validate_client_binding_status_event)
//! or [`ClientBindingStatusTracker`](crate::client_binding_status::ClientBindingStatusTracker)
//! rather than mutable profile fields or client-supplied claims.

use std::fmt;

use nostr::{Event, EventBuilder, EventId, Keys, Kind, PublicKey, Timestamp};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    kind::KIND_CLIENT_BINDING_STATUS, verify_event, CanonicalCurrentBindingEvidence, CommunityId,
};

/// Wire version accepted by this module.
pub const CLIENT_BINDING_STATUS_VERSION: u64 = 1;

/// Maximum lifetime of a client binding status, in seconds.
///
/// Producers may choose a shorter lifetime. A longer lifetime fails closed.
pub const MAX_CLIENT_BINDING_STATUS_LIFETIME_SECS: u64 = 300;

/// Explicit client-status clock-skew allowance.
///
/// Status is presentation-only and issued from centrally injected relay time,
/// so the portable profile permits no future issue-time skew.
pub const CLIENT_BINDING_STATUS_CLOCK_SKEW_SECS: u64 = 0;

/// Maximum encoded payload length.
pub const MAX_CLIENT_BINDING_STATUS_PAYLOAD_BYTES: usize = 4096;

/// Maximum encoded length of the opaque policy revision.
pub const MAX_CLIENT_BINDING_STATUS_POLICY_VERSION_BYTES: usize = 256;

/// Server-selected, display-only status disposition.
///
/// V1 deliberately exposes only current verification or an opaque withdrawal.
/// Revocation, rotation, lineage, retirement, and other lifecycle causes are
/// durable server-side history and are never part of the client contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientBindingStatusDisposition {
    /// The client may display current verification while the envelope is fresh.
    DisplayCurrent,
    /// Clear current presentation and advance only the scoped replay floor.
    Withdrawn,
}

/// A validated v1 client binding status.
///
/// Construction proves that one correctly signed event from the expected
/// relay matched the caller's server-resolved authorization domain and exact
/// message-author key at the injected validation time. It does not grant or
/// deny any capability.
#[derive(Clone, PartialEq, Eq)]
pub struct ClientBindingStatusV1 {
    event_id: EventId,
    authorization_domain: CommunityId,
    event_author_pubkey: PublicKey,
    binding_version: Option<u64>,
    policy_version: Option<String>,
    status_revision: u64,
    issued_at: u64,
    fresh_until: u64,
    disposition: ClientBindingStatusDisposition,
}

impl fmt::Debug for ClientBindingStatusV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientBindingStatusV1")
            .field("event_id", &"[redacted]")
            .field("authorization_domain", &"[redacted]")
            .field("event_author_pubkey", &"[redacted]")
            .field("binding_version", &"[redacted]")
            .field("policy_version", &"[redacted]")
            .field("status_revision", &"[redacted]")
            .field("issued_at", &"[redacted]")
            .field("fresh_until", &"[redacted]")
            .field("disposition", &self.disposition)
            .finish()
    }
}

impl ClientBindingStatusV1 {
    /// Signed event identifier used for equal-revision idempotency.
    pub const fn event_id(&self) -> EventId {
        self.event_id
    }

    /// Server-resolved authorization domain for which this status is valid.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Exact event-author key whose messages may consume this status.
    pub const fn event_author_pubkey(&self) -> PublicKey {
        self.event_author_pubkey
    }

    /// Positive version of the identity-to-key binding.
    pub const fn binding_version(&self) -> Option<u64> {
        self.binding_version
    }

    /// Opaque provider-neutral policy revision.
    pub fn policy_version(&self) -> Option<&str> {
        self.policy_version.as_deref()
    }

    /// Positive, monotonically increasing revision for this scoped status.
    pub const fn status_revision(&self) -> u64 {
        self.status_revision
    }

    /// Relay issue time as Unix seconds.
    pub const fn issued_at(&self) -> u64 {
        self.issued_at
    }

    /// Exclusive freshness bound as Unix seconds.
    pub const fn fresh_until(&self) -> u64 {
        self.fresh_until
    }

    /// Server-selected, display-only disposition.
    pub const fn disposition(&self) -> ClientBindingStatusDisposition {
        self.disposition
    }

    /// Returns `true` only for an active, current presentation disposition.
    ///
    /// Freshness and relay authentication have already been checked by
    /// [`validate_client_binding_status_event`]. This method must not be used
    /// for access control.
    pub const fn displays_current_binding(&self) -> bool {
        matches!(
            self.disposition,
            ClientBindingStatusDisposition::DisplayCurrent
        )
    }
}

/// Validated producer input for one v1 client binding status.
///
/// This is a serialization/signing input only. It intentionally carries no
/// authorization context, capability set, membership state, or access lease.
pub struct ClientBindingStatusInputV1 {
    authorization_domain: CommunityId,
    event_author_pubkey: PublicKey,
    binding_version: Option<u64>,
    policy_version: Option<String>,
    status_revision: u64,
    issued_at: u64,
    fresh_until: u64,
    disposition: ClientBindingStatusDisposition,
}

impl fmt::Debug for ClientBindingStatusInputV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientBindingStatusInputV1")
            .field("authorization_domain", &"[redacted]")
            .field("event_author_pubkey", &"[redacted]")
            .field("binding_version", &"[redacted]")
            .field("policy_version", &"[redacted]")
            .field("status_revision", &"[redacted]")
            .field("issued_at", &"[redacted]")
            .field("fresh_until", &"[redacted]")
            .field("disposition", &self.disposition)
            .finish()
    }
}

impl ClientBindingStatusInputV1 {
    /// Construct current status from the frozen provider-free evidence value.
    ///
    /// This is the preferred producer entry point. The wire remains v1: the
    /// typed local policy revision is encoded through the existing opaque
    /// policy-version field, and no binding identifier, fence, provider data,
    /// or connection epoch crosses the status boundary.
    pub fn current_from_evidence(
        evidence: &CanonicalCurrentBindingEvidence,
        status_revision: u64,
    ) -> Result<Self, ClientBindingStatusError> {
        let issued_at = u64::try_from(evidence.observed_at().timestamp())
            .map_err(|_| ClientBindingStatusError::InvalidIssueTime)?;
        let fresh_until = u64::try_from(evidence.fresh_until().timestamp())
            .map_err(|_| ClientBindingStatusError::InvalidFreshnessBound)?;
        Self::current(
            evidence.authorization_domain(),
            evidence.event_author_pubkey(),
            evidence.binding_version(),
            evidence.policy_revision().to_string(),
            status_revision,
            issued_at,
            fresh_until,
        )
    }

    /// Construct bounded, provider-neutral current-status input.
    pub fn current(
        authorization_domain: CommunityId,
        event_author_pubkey: PublicKey,
        binding_version: u64,
        policy_version: impl Into<String>,
        status_revision: u64,
        issued_at: u64,
        fresh_until: u64,
    ) -> Result<Self, ClientBindingStatusError> {
        let value = Self {
            authorization_domain,
            event_author_pubkey,
            binding_version: Some(binding_version),
            policy_version: Some(policy_version.into()),
            status_revision,
            issued_at,
            fresh_until,
            disposition: ClientBindingStatusDisposition::DisplayCurrent,
        };
        validate_payload_fields(
            value.authorization_domain,
            value.binding_version,
            value.policy_version.as_deref(),
            value.status_revision,
            value.issued_at,
            value.fresh_until,
            value.disposition,
        )?;
        Ok(value)
    }

    /// Construct an opaque withdrawal carrying no binding or lifecycle data.
    pub fn withdrawn(
        authorization_domain: CommunityId,
        event_author_pubkey: PublicKey,
        status_revision: u64,
        issued_at: u64,
        fresh_until: u64,
    ) -> Result<Self, ClientBindingStatusError> {
        let value = Self {
            authorization_domain,
            event_author_pubkey,
            binding_version: None,
            policy_version: None,
            status_revision,
            issued_at,
            fresh_until,
            disposition: ClientBindingStatusDisposition::Withdrawn,
        };
        validate_payload_fields(
            value.authorization_domain,
            value.binding_version,
            value.policy_version.as_deref(),
            value.status_revision,
            value.issued_at,
            value.fresh_until,
            value.disposition,
        )?;
        Ok(value)
    }

    /// Sign this status with the relay key advertised through NIP-11 `self`.
    pub fn sign_with_relay_keys(
        self,
        relay_keys: &Keys,
    ) -> Result<Event, ClientBindingStatusBuildError> {
        let wire = WireClientBindingStatusV1 {
            version: CLIENT_BINDING_STATUS_VERSION,
            authorization_domain: self.authorization_domain.as_uuid().to_string(),
            event_author_pubkey: self.event_author_pubkey.to_hex(),
            status_revision: self.status_revision,
            issued_at: self.issued_at,
            fresh_until: self.fresh_until,
            status: self.disposition,
            binding_version: self.binding_version,
            policy_version: self.policy_version,
        };
        let content = serde_json::to_string(&wire)
            .map_err(|_| ClientBindingStatusBuildError::Serialization)?;
        if content.len() > MAX_CLIENT_BINDING_STATUS_PAYLOAD_BYTES {
            return Err(ClientBindingStatusBuildError::PayloadTooLarge);
        }
        EventBuilder::new(Kind::Custom(KIND_CLIENT_BINDING_STATUS as u16), content)
            .tags([])
            .custom_created_at(Timestamp::from(wire.issued_at))
            .sign_with_keys(relay_keys)
            .map_err(|_| ClientBindingStatusBuildError::Signing)
    }
}

#[derive(Deserialize)]
struct VersionHeader {
    version: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireClientBindingStatusV1 {
    version: u64,
    authorization_domain: String,
    event_author_pubkey: String,
    status_revision: u64,
    issued_at: u64,
    fresh_until: u64,
    status: ClientBindingStatusDisposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    binding_version: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    policy_version: Option<String>,
}

/// Validate and authenticate a relay-issued v1 client binding status event.
///
/// `trusted_relay_pubkey`, `expected_authorization_domain`, and
/// `expected_event_author_pubkey` must come from connection or message context,
/// never from the status payload. `now` is injected Unix time; the exclusive
/// expiry rule means `now == fresh_until` is expired. Future-issued statuses
/// are rejected under [`CLIENT_BINDING_STATUS_CLOCK_SKEW_SECS`].
pub fn validate_client_binding_status_event(
    event: &Event,
    trusted_relay_pubkey: &PublicKey,
    expected_authorization_domain: CommunityId,
    expected_event_author_pubkey: &PublicKey,
    now: u64,
) -> Result<ClientBindingStatusV1, ClientBindingStatusError> {
    if event.kind.as_u16() as u32 != KIND_CLIENT_BINDING_STATUS {
        return Err(ClientBindingStatusError::WrongKind);
    }
    if event.content.len() > MAX_CLIENT_BINDING_STATUS_PAYLOAD_BYTES {
        return Err(ClientBindingStatusError::PayloadTooLarge);
    }
    verify_event(event).map_err(|_| ClientBindingStatusError::UnauthenticatedEvent)?;
    if event.pubkey != *trusted_relay_pubkey {
        return Err(ClientBindingStatusError::UnexpectedRelay);
    }
    if !event.tags.is_empty() {
        return Err(ClientBindingStatusError::UnexpectedTags);
    }

    let header: VersionHeader = serde_json::from_str(&event.content)
        .map_err(|_| ClientBindingStatusError::MalformedPayload)?;
    if header.version != CLIENT_BINDING_STATUS_VERSION {
        return Err(ClientBindingStatusError::UnsupportedVersion);
    }

    let wire: WireClientBindingStatusV1 = serde_json::from_str(&event.content)
        .map_err(|_| ClientBindingStatusError::MalformedPayload)?;

    let authorization_domain = Uuid::parse_str(&wire.authorization_domain)
        .map_err(|_| ClientBindingStatusError::InvalidAuthorizationDomain)?;
    if authorization_domain.is_nil()
        || authorization_domain.to_string() != wire.authorization_domain
    {
        return Err(ClientBindingStatusError::InvalidAuthorizationDomain);
    }
    let authorization_domain = CommunityId::from_uuid(authorization_domain);
    if authorization_domain != expected_authorization_domain {
        return Err(ClientBindingStatusError::AuthorizationDomainMismatch);
    }

    if wire.event_author_pubkey.len() != 64
        || !wire
            .event_author_pubkey
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ClientBindingStatusError::InvalidEventAuthorPubkey);
    }
    let event_author_pubkey = PublicKey::from_hex(&wire.event_author_pubkey)
        .map_err(|_| ClientBindingStatusError::InvalidEventAuthorPubkey)?;
    if event_author_pubkey != *expected_event_author_pubkey {
        return Err(ClientBindingStatusError::EventAuthorMismatch);
    }

    let disposition = wire.status;
    let binding_version = wire.binding_version;
    let policy_version = wire.policy_version;

    validate_payload_fields(
        authorization_domain,
        binding_version,
        policy_version.as_deref(),
        wire.status_revision,
        wire.issued_at,
        wire.fresh_until,
        disposition,
    )?;
    if event.created_at.as_secs() != wire.issued_at {
        return Err(ClientBindingStatusError::EventTimeMismatch);
    }
    if wire.issued_at > now.saturating_add(CLIENT_BINDING_STATUS_CLOCK_SKEW_SECS) {
        return Err(ClientBindingStatusError::NotYetValid);
    }
    if now >= wire.fresh_until {
        return Err(ClientBindingStatusError::Expired);
    }

    Ok(ClientBindingStatusV1 {
        event_id: event.id,
        authorization_domain,
        event_author_pubkey,
        binding_version,
        policy_version,
        status_revision: wire.status_revision,
        issued_at: wire.issued_at,
        fresh_until: wire.fresh_until,
        disposition,
    })
}

fn validate_payload_fields(
    authorization_domain: CommunityId,
    binding_version: Option<u64>,
    policy_version: Option<&str>,
    status_revision: u64,
    issued_at: u64,
    fresh_until: u64,
    disposition: ClientBindingStatusDisposition,
) -> Result<(), ClientBindingStatusError> {
    if authorization_domain.as_uuid().is_nil() {
        return Err(ClientBindingStatusError::InvalidAuthorizationDomain);
    }
    match disposition {
        ClientBindingStatusDisposition::DisplayCurrent => {
            if binding_version.is_none_or(|version| version == 0) {
                return Err(ClientBindingStatusError::InvalidBindingVersion);
            }
            let Some(policy_version) = policy_version else {
                return Err(ClientBindingStatusError::InvalidPolicyVersion);
            };
            if policy_version.is_empty()
                || policy_version.len() > MAX_CLIENT_BINDING_STATUS_POLICY_VERSION_BYTES
                || policy_version.trim() != policy_version
                || policy_version.chars().any(char::is_control)
            {
                return Err(ClientBindingStatusError::InvalidPolicyVersion);
            }
        }
        ClientBindingStatusDisposition::Withdrawn => {
            if binding_version.is_some() || policy_version.is_some() {
                return Err(ClientBindingStatusError::WithdrawalContainsCurrentState);
            }
        }
    }
    if status_revision == 0 {
        return Err(ClientBindingStatusError::InvalidStatusRevision);
    }
    if issued_at == 0 {
        return Err(ClientBindingStatusError::InvalidIssueTime);
    }
    if fresh_until <= issued_at {
        return Err(ClientBindingStatusError::InvalidFreshnessBound);
    }
    if fresh_until - issued_at > MAX_CLIENT_BINDING_STATUS_LIFETIME_SECS {
        return Err(ClientBindingStatusError::FreshnessWindowTooLong);
    }
    Ok(())
}

/// One accepted high-water update from [`ClientBindingStatusTracker`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientBindingStatusUpdate {
    /// A strictly newer revision replaced presentation state.
    Accepted,
    /// The exact same signed event was observed again.
    Duplicate,
}

#[derive(Clone, Copy)]
struct StatusHighWater {
    revision: u64,
    event_id: EventId,
    operation: ClientBindingStatusDisposition,
}

/// Client-side, scope-keyed status revision fold.
///
/// The tracker authenticates every event against one trusted relay/domain/
/// author tuple. A lower revision or a different event or operation at the
/// same revision is rejected. Expiry and disconnect clear presentation while
/// retaining the high-water mark, so a previously seen envelope cannot
/// restore a badge. The fold is deliberately RAM-only and must be instantiated
/// per connection; it has no serialization or persistence surface.
pub struct ClientBindingStatusTracker {
    trusted_relay_pubkey: PublicKey,
    authorization_domain: CommunityId,
    event_author_pubkey: PublicKey,
    high_water: Option<StatusHighWater>,
    status: Option<ClientBindingStatusV1>,
}

impl fmt::Debug for ClientBindingStatusTracker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientBindingStatusTracker")
            .field("trusted_relay_pubkey", &"[redacted]")
            .field("authorization_domain", &"[redacted]")
            .field("event_author_pubkey", &"[redacted]")
            .field("high_water", &self.high_water.map(|_| "[redacted]"))
            .field("status", &self.status.as_ref().map(|_| "[redacted]"))
            .finish()
    }
}

impl ClientBindingStatusTracker {
    /// Start an empty fold for one trusted relay/domain/author scope.
    pub const fn new(
        trusted_relay_pubkey: PublicKey,
        authorization_domain: CommunityId,
        event_author_pubkey: PublicKey,
    ) -> Self {
        Self {
            trusted_relay_pubkey,
            authorization_domain,
            event_author_pubkey,
            high_water: None,
            status: None,
        }
    }

    /// Authenticate and fold one signed status event at injected time `now`.
    pub fn accept(
        &mut self,
        event: &Event,
        now: u64,
    ) -> Result<ClientBindingStatusUpdate, ClientBindingStatusFoldError> {
        let status = validate_client_binding_status_event(
            event,
            &self.trusted_relay_pubkey,
            self.authorization_domain,
            &self.event_author_pubkey,
            now,
        )?;
        if let Some(high_water) = self.high_water {
            if status.status_revision < high_water.revision {
                return Err(ClientBindingStatusFoldError::LowerRevisionReplay);
            }
            if status.status_revision == high_water.revision {
                if status.disposition != high_water.operation
                    || status.event_id != high_water.event_id
                {
                    return Err(ClientBindingStatusFoldError::ConflictingEqualRevision);
                }
                return Ok(ClientBindingStatusUpdate::Duplicate);
            }
        }
        self.high_water = Some(StatusHighWater {
            revision: status.status_revision,
            event_id: status.event_id,
            operation: status.disposition,
        });
        self.status = status.displays_current_binding().then_some(status);
        Ok(ClientBindingStatusUpdate::Accepted)
    }

    /// Return the fresh accepted status, clearing expired presentation.
    pub fn status(&mut self, now: u64) -> Option<&ClientBindingStatusV1> {
        if self
            .status
            .as_ref()
            .is_some_and(|status| now >= status.fresh_until)
        {
            self.status = None;
        }
        self.status.as_ref()
    }

    /// Return only a fresh status allowed to display current verification.
    pub fn current_presentation(&mut self, now: u64) -> Option<&ClientBindingStatusV1> {
        self.status(now)
            .filter(|status| status.displays_current_binding())
    }

    /// Clear presentation on relay disconnect while retaining replay defense.
    pub fn on_disconnect(&mut self) {
        self.status = None;
    }

    /// Clear presentation and retain a parseable trusted-invalid revision.
    ///
    /// The event is independently authenticated against this tracker's relay
    /// before its bounded revision is considered. Retaining the revision
    /// prevents replaying the same malformed envelope as a later syntactically
    /// valid value from restoring presentation.
    pub fn retain_trusted_invalid_high_water(&mut self, event: &Event) {
        self.status = None;
        if event.kind.as_u16() as u32 != KIND_CLIENT_BINDING_STATUS
            || event.content.len() > MAX_CLIENT_BINDING_STATUS_PAYLOAD_BYTES
            || verify_event(event).is_err()
            || event.pubkey != self.trusted_relay_pubkey
            || !event.tags.is_empty()
        {
            return;
        }
        let Ok(candidate) = serde_json::from_str::<WireClientBindingStatusV1>(&event.content)
        else {
            return;
        };
        let Ok(domain) = Uuid::parse_str(&candidate.authorization_domain) else {
            return;
        };
        let canonical_domain = !domain.is_nil()
            && domain.to_string() == candidate.authorization_domain
            && CommunityId::from_uuid(domain) == self.authorization_domain;
        let canonical_author = candidate.event_author_pubkey.len() == 64
            && candidate
                .event_author_pubkey
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && PublicKey::from_hex(&candidate.event_author_pubkey)
                .is_ok_and(|pubkey| pubkey == self.event_author_pubkey);
        if candidate.version != CLIENT_BINDING_STATUS_VERSION
            || !canonical_domain
            || !canonical_author
            || candidate.status_revision == 0
            || self
                .high_water
                .is_some_and(|high_water| candidate.status_revision <= high_water.revision)
        {
            return;
        }
        self.high_water = Some(StatusHighWater {
            revision: candidate.status_revision,
            event_id: event.id,
            operation: candidate.status,
        });
    }

    /// Replace the trusted scope and clear both presentation and revision state.
    ///
    /// Call this on relay-identity, authorization-domain, or event-author
    /// changes. Evidence from the old scope is never carried into the new one.
    pub fn change_scope(
        &mut self,
        trusted_relay_pubkey: PublicKey,
        authorization_domain: CommunityId,
        event_author_pubkey: PublicKey,
    ) {
        self.trusted_relay_pubkey = trusted_relay_pubkey;
        self.authorization_domain = authorization_domain;
        self.event_author_pubkey = event_author_pubkey;
        self.high_water = None;
        self.status = None;
    }

    /// Highest revision accepted for the current scope.
    pub const fn high_water_revision(&self) -> Option<u64> {
        match self.high_water {
            Some(value) => Some(value.revision),
            None => None,
        }
    }
}

/// Fail-closed client binding status validation error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ClientBindingStatusError {
    /// The event did not use the dedicated ephemeral status kind.
    #[error("client binding status event has the wrong kind")]
    WrongKind,
    /// The event body exceeded the public bound.
    #[error("client binding status payload is too large")]
    PayloadTooLarge,
    /// The event ID or Schnorr signature was invalid.
    #[error("client binding status event is not authenticated")]
    UnauthenticatedEvent,
    /// The signer did not match the relay key established by the connection.
    #[error("client binding status signer is not the trusted relay")]
    UnexpectedRelay,
    /// Status events must not carry tags or private indexing material.
    #[error("client binding status event contains unexpected tags")]
    UnexpectedTags,
    /// The JSON shape, field set, or enum encoding was malformed.
    #[error("client binding status payload is malformed")]
    MalformedPayload,
    /// The payload used a version this client does not understand.
    #[error("client binding status version is unsupported")]
    UnsupportedVersion,
    /// The authorization domain was nil or not a canonical UUID.
    #[error("client binding status authorization domain is invalid")]
    InvalidAuthorizationDomain,
    /// The payload did not match the server-resolved authorization domain.
    #[error("client binding status authorization domain does not match")]
    AuthorizationDomainMismatch,
    /// The event-author key was not canonical lowercase 64-character hex.
    #[error("client binding status event-author key is invalid")]
    InvalidEventAuthorPubkey,
    /// The payload named a key other than the displayed event's author.
    #[error("client binding status event-author key does not match")]
    EventAuthorMismatch,
    /// The binding version was zero.
    #[error("client binding status binding version must be positive")]
    InvalidBindingVersion,
    /// The opaque policy revision was empty, unsafe, or exceeded its bound.
    #[error("client binding status policy version is invalid")]
    InvalidPolicyVersion,
    /// A generic withdrawal attempted to carry current binding state.
    #[error("client binding status withdrawal contains current binding state")]
    WithdrawalContainsCurrentState,
    /// The status revision was zero.
    #[error("client binding status revision must be positive")]
    InvalidStatusRevision,
    /// The issue time was zero.
    #[error("client binding status issue time must be positive")]
    InvalidIssueTime,
    /// Freshness did not strictly follow issue time.
    #[error("client binding status freshness bound is invalid")]
    InvalidFreshnessBound,
    /// The freshness window exceeded the public short-lived maximum.
    #[error("client binding status freshness window is too long")]
    FreshnessWindowTooLong,
    /// The signed Nostr timestamp did not equal the payload issue time.
    #[error("client binding status event time does not match its issue time")]
    EventTimeMismatch,
    /// The payload was issued after the explicit skew allowance.
    #[error("client binding status is not yet valid")]
    NotYetValid,
    /// The exclusive freshness bound was reached.
    #[error("client binding status has expired")]
    Expired,
}

/// Status-event serialization/signing failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ClientBindingStatusBuildError {
    /// JSON serialization failed.
    #[error("client binding status serialization failed")]
    Serialization,
    /// The serialized payload exceeded its public bound.
    #[error("client binding status payload is too large")]
    PayloadTooLarge,
    /// Nostr event signing failed.
    #[error("client binding status signing failed")]
    Signing,
}

/// Status revision-fold failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ClientBindingStatusFoldError {
    /// Cryptographic or wire validation failed.
    #[error(transparent)]
    InvalidStatus(#[from] ClientBindingStatusError),
    /// A lower status revision attempted to restore older presentation.
    #[error("client binding status revision is below the accepted high-water mark")]
    LowerRevisionReplay,
    /// Another signed event reused an accepted revision.
    #[error("client binding status revision conflicts with another event")]
    ConflictingEqualRevision,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AuthorizationLeaseFence;
    use chrono::{TimeZone, Utc};
    use nostr::JsonUtil;
    use serde_json::{json, Value};

    const DOMAIN: &str = "00000000-0000-4000-8000-000000000123";
    const ISSUED_AT: u64 = 1_800_000_000;
    const FRESH_UNTIL: u64 = ISSUED_AT + 120;

    fn domain() -> CommunityId {
        CommunityId::from_uuid(Uuid::parse_str(DOMAIN).expect("synthetic domain is valid"))
    }

    fn input(
        author: PublicKey,
        revision: u64,
        disposition: ClientBindingStatusDisposition,
    ) -> ClientBindingStatusInputV1 {
        match disposition {
            ClientBindingStatusDisposition::DisplayCurrent => ClientBindingStatusInputV1::current(
                domain(),
                author,
                7,
                "synthetic-policy-v1",
                revision,
                ISSUED_AT,
                FRESH_UNTIL,
            ),
            ClientBindingStatusDisposition::Withdrawn => ClientBindingStatusInputV1::withdrawn(
                domain(),
                author,
                revision,
                ISSUED_AT,
                FRESH_UNTIL,
            ),
        }
        .expect("synthetic status input is valid")
    }

    fn signed_status(
        relay: &Keys,
        author: PublicKey,
        revision: u64,
        disposition: ClientBindingStatusDisposition,
    ) -> Event {
        input(author, revision, disposition)
            .sign_with_relay_keys(relay)
            .expect("synthetic event signs")
    }

    fn validate(
        event: &Event,
        relay: &Keys,
        author: &Keys,
        now: u64,
    ) -> Result<ClientBindingStatusV1, ClientBindingStatusError> {
        validate_client_binding_status_event(
            event,
            &relay.public_key(),
            domain(),
            &author.public_key(),
            now,
        )
    }

    #[test]
    fn validates_relay_authenticated_current_status() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let event = signed_status(
            &relay,
            author.public_key(),
            11,
            ClientBindingStatusDisposition::DisplayCurrent,
        );

        let status = validate(&event, &relay, &author, ISSUED_AT)
            .expect("synthetic current status validates");

        assert_eq!(status.event_id(), event.id);
        assert_eq!(status.authorization_domain(), domain());
        assert_eq!(status.event_author_pubkey(), author.public_key());
        assert_eq!(status.binding_version(), Some(7));
        assert_eq!(status.policy_version(), Some("synthetic-policy-v1"));
        assert_eq!(status.status_revision(), 11);
        assert_eq!(status.issued_at(), ISSUED_AT);
        assert_eq!(status.fresh_until(), FRESH_UNTIL);
        assert_eq!(
            status.disposition(),
            ClientBindingStatusDisposition::DisplayCurrent
        );
        assert!(status.displays_current_binding());
        assert!(event.tags.is_empty());
    }

    #[test]
    fn frozen_evidence_maps_to_epoch_free_private_status() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let observed_at = Utc
            .timestamp_opt(ISSUED_AT as i64, 0)
            .single()
            .expect("synthetic timestamp is valid");
        let fresh_until = Utc
            .timestamp_opt(FRESH_UNTIL as i64, 0)
            .single()
            .expect("synthetic timestamp is valid");
        let evidence = CanonicalCurrentBindingEvidence::new(
            domain(),
            author.public_key(),
            Uuid::from_u128(0x55),
            7,
            19,
            3,
            5,
            AuthorizationLeaseFence::from_bytes([9; 32]).expect("synthetic fence is valid"),
            observed_at,
            fresh_until,
        )
        .expect("synthetic evidence is valid");
        let event = ClientBindingStatusInputV1::current_from_evidence(&evidence, 11)
            .expect("evidence maps to status input")
            .sign_with_relay_keys(&relay)
            .expect("synthetic status signs");
        let status =
            validate(&event, &relay, &author, ISSUED_AT).expect("evidence-backed status validates");

        assert_eq!(status.binding_version(), Some(7));
        assert_eq!(status.policy_version(), Some("19"));
        assert_eq!(status.status_revision(), 11);
        let payload: Value = serde_json::from_str(&event.content).expect("content parses");
        for private in [
            "binding_id",
            "display_label",
            "invalidation_generation",
            "authority_epoch",
            "fence",
            "connection_epoch",
        ] {
            assert!(
                payload.get(private).is_none(),
                "leaked private field {private}"
            );
        }
    }

    #[test]
    fn wire_is_current_or_opaque_withdrawal() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let cases = [
            (
                ClientBindingStatusDisposition::DisplayCurrent,
                "display_current",
            ),
            (ClientBindingStatusDisposition::Withdrawn, "withdrawn"),
        ];
        for (disposition, expected) in cases {
            let event = signed_status(&relay, author.public_key(), 11, disposition);
            let content: Value = serde_json::from_str(&event.content).expect("content parses");
            assert_eq!(content["status"], expected);
            if disposition == ClientBindingStatusDisposition::Withdrawn {
                for forbidden in ["binding_version", "policy_version", "reason"] {
                    assert!(
                        content.get(forbidden).is_none(),
                        "unexpected field {forbidden}"
                    );
                }
            }
        }
    }

    #[test]
    fn withdrawal_removes_current_presentation_without_lifecycle_data() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let event = signed_status(
            &relay,
            author.public_key(),
            11,
            ClientBindingStatusDisposition::Withdrawn,
        );
        let status =
            validate(&event, &relay, &author, ISSUED_AT).expect("synthetic withdrawal validates");
        assert!(!status.displays_current_binding());
        assert_eq!(status.binding_version(), None);
        assert_eq!(status.policy_version(), None);
    }

    #[test]
    fn exact_expiry_and_future_issue_fail_closed() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let event = signed_status(
            &relay,
            author.public_key(),
            11,
            ClientBindingStatusDisposition::DisplayCurrent,
        );

        assert_eq!(
            validate(&event, &relay, &author, FRESH_UNTIL),
            Err(ClientBindingStatusError::Expired)
        );
        assert_eq!(
            validate(&event, &relay, &author, ISSUED_AT - 1),
            Err(ClientBindingStatusError::NotYetValid)
        );
        assert_eq!(CLIENT_BINDING_STATUS_CLOCK_SKEW_SECS, 0);
    }

    #[test]
    fn rejects_wrong_signer_tampering_kind_scope_and_tags() {
        let relay = Keys::generate();
        let wrong_relay = Keys::generate();
        let author = Keys::generate();
        let other_author = Keys::generate();
        let event = signed_status(
            &relay,
            author.public_key(),
            11,
            ClientBindingStatusDisposition::DisplayCurrent,
        );

        assert_eq!(
            validate(&event, &wrong_relay, &author, ISSUED_AT),
            Err(ClientBindingStatusError::UnexpectedRelay)
        );
        assert_eq!(
            validate(&event, &relay, &other_author, ISSUED_AT),
            Err(ClientBindingStatusError::EventAuthorMismatch)
        );
        assert_eq!(
            validate_client_binding_status_event(
                &event,
                &relay.public_key(),
                CommunityId::from_uuid(Uuid::from_u128(999)),
                &author.public_key(),
                ISSUED_AT,
            ),
            Err(ClientBindingStatusError::AuthorizationDomainMismatch)
        );

        let wrong_kind = EventBuilder::new(Kind::TextNote, event.content.clone())
            .custom_created_at(Timestamp::from(ISSUED_AT))
            .sign_with_keys(&relay)
            .expect("synthetic event signs");
        assert_eq!(
            validate(&wrong_kind, &relay, &author, ISSUED_AT),
            Err(ClientBindingStatusError::WrongKind)
        );

        let tagged = EventBuilder::new(
            Kind::Custom(KIND_CLIENT_BINDING_STATUS as u16),
            event.content.clone(),
        )
        .tags([nostr::Tag::parse(["p", &author.public_key().to_hex()])
            .expect("synthetic tag is valid")])
        .custom_created_at(Timestamp::from(ISSUED_AT))
        .sign_with_keys(&relay)
        .expect("synthetic event signs");
        assert_eq!(
            validate(&tagged, &relay, &author, ISSUED_AT),
            Err(ClientBindingStatusError::UnexpectedTags)
        );

        let mut json: Value = serde_json::from_str(&event.as_json()).expect("event parses");
        json["content"] = Value::String("{}".to_string());
        let tampered = Event::from_json(json.to_string()).expect("tampered event parses");
        assert_eq!(
            validate(&tampered, &relay, &author, ISSUED_AT),
            Err(ClientBindingStatusError::UnauthenticatedEvent)
        );
    }

    #[test]
    fn unknown_version_fields_and_enums_fail_closed() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let event = signed_status(
            &relay,
            author.public_key(),
            11,
            ClientBindingStatusDisposition::DisplayCurrent,
        );
        let mut payload: Value = serde_json::from_str(&event.content).expect("content parses");

        payload["version"] = json!(2);
        let unknown_version = EventBuilder::new(
            Kind::Custom(KIND_CLIENT_BINDING_STATUS as u16),
            payload.to_string(),
        )
        .custom_created_at(Timestamp::from(ISSUED_AT))
        .sign_with_keys(&relay)
        .expect("synthetic event signs");
        assert_eq!(
            validate(&unknown_version, &relay, &author, ISSUED_AT),
            Err(ClientBindingStatusError::UnsupportedVersion)
        );

        payload["version"] = json!(1);
        payload["synthetic_extension"] = json!(true);
        let unknown_field = EventBuilder::new(
            Kind::Custom(KIND_CLIENT_BINDING_STATUS as u16),
            payload.to_string(),
        )
        .custom_created_at(Timestamp::from(ISSUED_AT))
        .sign_with_keys(&relay)
        .expect("synthetic event signs");
        assert_eq!(
            validate(&unknown_field, &relay, &author, ISSUED_AT),
            Err(ClientBindingStatusError::MalformedPayload)
        );

        let mut payload: Value =
            serde_json::from_str(&event.content).expect("content parses again");
        payload["display_label"] = json!("Synthetic Example");
        let injected_display_label = EventBuilder::new(
            Kind::Custom(KIND_CLIENT_BINDING_STATUS as u16),
            payload.to_string(),
        )
        .custom_created_at(Timestamp::from(ISSUED_AT))
        .sign_with_keys(&relay)
        .expect("synthetic event signs");
        assert_eq!(
            validate(&injected_display_label, &relay, &author, ISSUED_AT),
            Err(ClientBindingStatusError::MalformedPayload)
        );
    }

    #[test]
    fn connection_epoch_is_rejected_as_an_unknown_status_field() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let event = signed_status(
            &relay,
            author.public_key(),
            11,
            ClientBindingStatusDisposition::DisplayCurrent,
        );

        for field in ["connection_epoch", "connectionEpoch"] {
            let mut payload: Value = serde_json::from_str(&event.content).expect("content parses");
            payload[field] = json!("22222222-2222-4222-8222-222222222222");
            let injected = EventBuilder::new(
                Kind::Custom(KIND_CLIENT_BINDING_STATUS as u16),
                payload.to_string(),
            )
            .custom_created_at(Timestamp::from(ISSUED_AT))
            .sign_with_keys(&relay)
            .expect("synthetic event signs");

            assert_eq!(
                validate(&injected, &relay, &author, ISSUED_AT),
                Err(ClientBindingStatusError::MalformedPayload),
                "accepted forbidden status field {field}"
            );
        }
    }

    #[test]
    fn historical_status_and_lifecycle_fields_cannot_reappear() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let event = signed_status(
            &relay,
            author.public_key(),
            11,
            ClientBindingStatusDisposition::DisplayCurrent,
        );
        let mut payload: Value = serde_json::from_str(&event.content).expect("content parses");
        let current_keys = payload
            .as_object()
            .expect("current payload is an object")
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            current_keys,
            std::collections::BTreeSet::from([
                "authorization_domain",
                "binding_version",
                "event_author_pubkey",
                "fresh_until",
                "issued_at",
                "policy_version",
                "status",
                "status_revision",
                "version",
            ])
        );
        let withdrawn = signed_status(
            &relay,
            author.public_key(),
            12,
            ClientBindingStatusDisposition::Withdrawn,
        );
        let withdrawn_payload: Value =
            serde_json::from_str(&withdrawn.content).expect("withdrawn content parses");
        let withdrawn_keys = withdrawn_payload
            .as_object()
            .expect("withdrawn payload is an object")
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            withdrawn_keys,
            std::collections::BTreeSet::from([
                "authorization_domain",
                "event_author_pubkey",
                "fresh_until",
                "issued_at",
                "status",
                "status_revision",
                "version",
            ])
        );
        payload["status"] = json!("historical_only");
        payload["reason"] = json!("rotated");
        let historical = EventBuilder::new(
            Kind::Custom(KIND_CLIENT_BINDING_STATUS as u16),
            payload.to_string(),
        )
        .custom_created_at(Timestamp::from(ISSUED_AT))
        .sign_with_keys(&relay)
        .expect("synthetic event signs");
        assert_eq!(
            validate(&historical, &relay, &author, ISSUED_AT),
            Err(ClientBindingStatusError::MalformedPayload)
        );

        for forbidden in [
            "history",
            "lineage",
            "predecessor",
            "replacement",
            "tombstone",
            "principal",
            "issuer",
            "subject",
            "retirement_reason",
            "historical_label",
            "employment_history",
        ] {
            let withdrawal = signed_status(
                &relay,
                author.public_key(),
                12,
                ClientBindingStatusDisposition::Withdrawn,
            );
            let mut payload: Value =
                serde_json::from_str(&withdrawal.content).expect("content parses");
            payload[forbidden] = json!("forbidden");
            let injected = EventBuilder::new(
                Kind::Custom(KIND_CLIENT_BINDING_STATUS as u16),
                payload.to_string(),
            )
            .custom_created_at(Timestamp::from(ISSUED_AT))
            .sign_with_keys(&relay)
            .expect("synthetic event signs");
            assert_eq!(
                validate(&injected, &relay, &author, ISSUED_AT),
                Err(ClientBindingStatusError::MalformedPayload),
                "accepted forbidden field {forbidden}"
            );
        }
    }

    #[test]
    fn revision_fold_rejects_lower_and_conflicting_equal_replays() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let current = signed_status(
            &relay,
            author.public_key(),
            11,
            ClientBindingStatusDisposition::DisplayCurrent,
        );
        let withdrawn = signed_status(
            &relay,
            author.public_key(),
            12,
            ClientBindingStatusDisposition::Withdrawn,
        );
        let conflicting_equal = ClientBindingStatusInputV1::current(
            domain(),
            author.public_key(),
            8,
            "synthetic-policy-v2",
            12,
            ISSUED_AT,
            FRESH_UNTIL,
        )
        .expect("conflicting current input")
        .sign_with_relay_keys(&relay)
        .expect("conflicting current signs");
        let mut tracker =
            ClientBindingStatusTracker::new(relay.public_key(), domain(), author.public_key());

        assert_eq!(
            tracker.accept(&current, ISSUED_AT),
            Ok(ClientBindingStatusUpdate::Accepted)
        );
        assert!(tracker.current_presentation(ISSUED_AT).is_some());
        assert_eq!(
            tracker.accept(&current, ISSUED_AT),
            Ok(ClientBindingStatusUpdate::Duplicate)
        );
        assert_eq!(
            tracker.accept(&withdrawn, ISSUED_AT),
            Ok(ClientBindingStatusUpdate::Accepted)
        );
        assert!(tracker.current_presentation(ISSUED_AT).is_none());
        assert_eq!(
            tracker.accept(&current, ISSUED_AT),
            Err(ClientBindingStatusFoldError::LowerRevisionReplay)
        );
        assert_eq!(
            tracker.accept(&conflicting_equal, ISSUED_AT),
            Err(ClientBindingStatusFoldError::ConflictingEqualRevision)
        );
    }

    #[test]
    fn wrong_scope_envelopes_cannot_poison_high_water() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let other_author = Keys::generate();
        let valid = signed_status(
            &relay,
            author.public_key(),
            11,
            ClientBindingStatusDisposition::DisplayCurrent,
        );
        let wrong_domain = ClientBindingStatusInputV1::current(
            CommunityId::from_uuid(Uuid::from_u128(999)),
            author.public_key(),
            7,
            "synthetic-policy-v1",
            999,
            ISSUED_AT,
            FRESH_UNTIL,
        )
        .expect("wrong-domain status input is internally valid")
        .sign_with_relay_keys(&relay)
        .expect("wrong-domain status signs");
        let wrong_author = ClientBindingStatusInputV1::current(
            domain(),
            other_author.public_key(),
            7,
            "synthetic-policy-v1",
            999,
            ISSUED_AT,
            FRESH_UNTIL,
        )
        .expect("wrong-author status input is internally valid")
        .sign_with_relay_keys(&relay)
        .expect("wrong-author status signs");
        let high_scope = ClientBindingStatusInputV1::current(
            domain(),
            author.public_key(),
            7,
            "synthetic-policy-v1",
            999,
            ISSUED_AT,
            FRESH_UNTIL,
        )
        .expect("high-revision status input is internally valid")
        .sign_with_relay_keys(&relay)
        .expect("high-revision status signs");
        let wrong_kind = EventBuilder::new(Kind::TextNote, high_scope.content.clone())
            .custom_created_at(Timestamp::from(ISSUED_AT))
            .sign_with_keys(&relay)
            .expect("wrong-kind status signs");
        let tagged = EventBuilder::new(
            Kind::Custom(KIND_CLIENT_BINDING_STATUS as u16),
            high_scope.content.clone(),
        )
        .tags([nostr::Tag::parse(["p", &author.public_key().to_hex()])
            .expect("synthetic tag is valid")])
        .custom_created_at(Timestamp::from(ISSUED_AT))
        .sign_with_keys(&relay)
        .expect("tagged status signs");

        for poison in [&wrong_domain, &wrong_author, &wrong_kind, &tagged] {
            let mut tracker =
                ClientBindingStatusTracker::new(relay.public_key(), domain(), author.public_key());
            tracker.retain_trusted_invalid_high_water(poison);
            assert_eq!(tracker.high_water_revision(), None);
            assert_eq!(
                tracker.accept(&valid, ISSUED_AT),
                Ok(ClientBindingStatusUpdate::Accepted)
            );
        }
    }

    #[test]
    fn expiry_disconnect_and_scope_change_clear_presentation() {
        let relay = Keys::generate();
        let other_relay = Keys::generate();
        let author = Keys::generate();
        let other_author = Keys::generate();
        let current = signed_status(
            &relay,
            author.public_key(),
            11,
            ClientBindingStatusDisposition::DisplayCurrent,
        );
        let mut tracker =
            ClientBindingStatusTracker::new(relay.public_key(), domain(), author.public_key());
        tracker
            .accept(&current, ISSUED_AT)
            .expect("current status accepted");

        assert!(tracker.current_presentation(FRESH_UNTIL).is_none());
        assert_eq!(tracker.high_water_revision(), Some(11));
        assert_eq!(
            tracker.accept(&current, ISSUED_AT),
            Ok(ClientBindingStatusUpdate::Duplicate)
        );
        assert!(tracker.current_presentation(ISSUED_AT).is_none());

        let newer = signed_status(
            &relay,
            author.public_key(),
            12,
            ClientBindingStatusDisposition::DisplayCurrent,
        );
        tracker
            .accept(&newer, ISSUED_AT)
            .expect("newer status accepted");
        tracker.on_disconnect();
        assert!(tracker.current_presentation(ISSUED_AT).is_none());
        assert_eq!(tracker.high_water_revision(), Some(12));

        tracker.change_scope(
            other_relay.public_key(),
            CommunityId::from_uuid(Uuid::from_u128(999)),
            other_author.public_key(),
        );
        assert!(tracker.current_presentation(ISSUED_AT).is_none());
        assert_eq!(tracker.high_water_revision(), None);
    }

    #[test]
    fn debug_output_and_wire_omit_private_identity_material() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let event = signed_status(
            &relay,
            author.public_key(),
            11,
            ClientBindingStatusDisposition::DisplayCurrent,
        );
        let status = validate(&event, &relay, &author, ISSUED_AT)
            .expect("synthetic current status validates");

        let debug = format!("{status:?}");
        assert!(!debug.contains(DOMAIN));
        assert!(!debug.contains(&author.public_key().to_hex()));
        assert!(!debug.contains("synthetic-policy-v1"));
        assert!(!debug.contains("Synthetic Example"));

        let payload: Value = serde_json::from_str(&event.content).expect("content parses");
        for forbidden in [
            "iss",
            "sub",
            "issuer",
            "audience",
            "email",
            "display_name",
            "binding_id",
            "bearer",
        ] {
            assert!(
                payload.get(forbidden).is_none(),
                "unexpected field {forbidden}"
            );
        }
    }
}
