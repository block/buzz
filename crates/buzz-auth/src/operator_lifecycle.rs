//! Origin-sealed deployment-operator authority for identity lifecycle work.
//!
//! The verifier is the only constructor for the opaque grants in this module.
//! It requires a configured operator to sign the exact NIP-98 request and,
//! when the closed deployment policy requires it, a distinct configured
//! approver. Every accepted proof is payload- and intent-bound before the
//! verifier returns authority to a caller.

use std::{fmt, future::Future, pin::Pin};

use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use nostr::{Event, EventId, PublicKey, TagKind};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    foundation::FederatedPrincipal, verify_nip98_event, AuthError, DEFAULT_REPLAY_TTL_SECS,
};

const MAX_CONFIGURED_OPERATORS: usize = 64;
const NIP98_EXCLUSIVE_LIFETIME_SECONDS: u64 = 60;

/// Atomic all-or-none replay fence for a complete one/two/three-proof set.
///
/// This is intentionally separate from the single-event `Nip98ReplayGuard`:
/// sequential claims can burn a valid subset and cannot prove one concurrent
/// winner for a multi-signature operator action.
pub trait OperatorProofSetReplayGuard: Send + Sync {
    /// Consume one unit from a server-derived domain/action/actor quota.
    ///
    /// The verifier calls this only after every proof is complete and
    /// allowlisted, and before it consumes any replay identity.
    fn try_admit_quota<'a>(
        &'a self,
        quota_scope: &'a str,
        signer_attribution: &'a [u8; 32],
    ) -> Pin<Box<dyn Future<Output = Result<bool, AuthError>> + Send + 'a>>;

    /// Claim every distinct event id in one scope, or claim none of them.
    fn try_mark_all_in_scope<'a>(
        &'a self,
        scope: &'a str,
        event_ids: &'a [EventId],
        ttl_secs: u64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, AuthError>> + Send + 'a>>;
}

/// Closed operator actions backed by the canonical identity lifecycle engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(i16)]
pub enum OperatorLifecycleAction {
    /// Retire one active identity/key pair without revoking the key globally.
    Retire = 3,
    /// Disable one principal, retiring its active binding when present.
    Disable = 4,
    /// Permanently revoke an event-author key.
    Revoke = 5,
    /// Replace one active binding with a freshly proven key.
    Rotate = 6,
    /// Consume one pending replacement fact and install its successor.
    Recover = 7,
    /// Consume one disabled-principal fact and install a successor.
    Enable = 8,
}

impl OperatorLifecycleAction {
    /// Parse the exact action segment exposed by the deployment operator API.
    pub fn from_path_segment(value: &str) -> Option<Self> {
        match value {
            "revoke" => Some(Self::Revoke),
            "rotate" => Some(Self::Rotate),
            "recover" => Some(Self::Recover),
            "enable" => Some(Self::Enable),
            "retire" => Some(Self::Retire),
            "disable" => Some(Self::Disable),
            _ => None,
        }
    }

    /// Exact action segment covered by the signed NIP-98 URL.
    pub const fn as_path_segment(self) -> &'static str {
        match self {
            Self::Retire => "retire",
            Self::Disable => "disable",
            Self::Revoke => "revoke",
            Self::Rotate => "rotate",
            Self::Recover => "recover",
            Self::Enable => "enable",
        }
    }
}

/// Opaque authority for one exact canonical lifecycle request.
///
/// This value has no public constructor and is not serializable. A valid
/// value proves that the closed configured approval policy was satisfied for
/// the exact signed payload and semantic intent.
///
/// ```compile_fail
/// use buzz_auth::VerifiedOperatorLifecycleGrant;
/// let _forged = VerifiedOperatorLifecycleGrant {};
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct VerifiedOperatorLifecycleGrant {
    authorization_domain: CommunityId,
    action: OperatorLifecycleAction,
    operation_id: Uuid,
    intent_digest: [u8; 32],
    expected_revision: u64,
    approval_policy_digest: [u8; 32],
    signer_attribution: [u8; 32],
    approval_attribution: Option<[u8; 32]>,
    replacement_event_author_pubkey: Option<[u8; 32]>,
    replacement_expires_at: Option<DateTime<Utc>>,
    replacement_proof_attribution: Option<[u8; 32]>,
    request_attribution: [u8; 32],
    issued_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl VerifiedOperatorLifecycleGrant {
    /// Server-resolved authorization domain bound by both operator proofs.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Exact closed lifecycle action.
    pub const fn action(&self) -> OperatorLifecycleAction {
        self.action
    }

    /// Stable operation identity bound by the signed payload.
    pub const fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    /// Canonical typed-intent digest bound by both signatures.
    pub const fn intent_digest(&self) -> [u8; 32] {
        self.intent_digest
    }

    /// Exact expected lifecycle revision or selector generation.
    pub const fn expected_revision(&self) -> u64 {
        self.expected_revision
    }

    /// Digest of the immutable verifier configuration and approval policy.
    pub const fn approval_policy_digest(&self) -> [u8; 32] {
        self.approval_policy_digest
    }

    /// Pseudonymous configured signer attribution.
    pub const fn signer_attribution(&self) -> [u8; 32] {
        self.signer_attribution
    }

    /// Independently configured approver attribution.
    pub const fn approval_attribution(&self) -> Option<[u8; 32]> {
        self.approval_attribution
    }

    /// Fresh replacement key proven by the exact request, when required by
    /// Rotate, Recover, or Enable.
    pub const fn replacement_event_author_pubkey(&self) -> Option<[u8; 32]> {
        self.replacement_event_author_pubkey
    }

    /// Stable exclusive expiry requested in the signed replacement body.
    pub const fn replacement_expires_at(&self) -> Option<DateTime<Utc>> {
        self.replacement_expires_at
    }

    /// Stable attribution for the independently signed replacement proof.
    pub const fn replacement_proof_attribution(&self) -> Option<[u8; 32]> {
        self.replacement_proof_attribution
    }

    /// Stable attribution bound to semantic intent, excluding ephemeral proof
    /// IDs so a newly signed exact retry can reach canonical DB replay.
    pub const fn request_attribution(&self) -> [u8; 32] {
        self.request_attribution
    }

    /// Latest issuance time across the two exact NIP-98 proofs.
    pub const fn issued_at(&self) -> DateTime<Utc> {
        self.issued_at
    }

    /// Exclusive trusted expiry. For replacement actions this is the minimum
    /// of the primary proof, configured approver proof, replacement proof,
    /// and requested successor bound. Callers must recheck it immediately
    /// before the canonical mutation and additionally apply the authoritative
    /// database policy bound when persisting the successor.
    pub const fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }
}

impl fmt::Debug for VerifiedOperatorLifecycleGrant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VerifiedOperatorLifecycleGrant([REDACTED])")
    }
}

/// Opaque operator-approved community-provision binding intent.
///
/// Provisioning remains a separate workflow from lifecycle mutation. This
/// value lets that workflow consume the same closed operator policy without
/// accepting a caller-selected `approved` flag or raw attribution.
///
/// ```compile_fail
/// use buzz_auth::VerifiedProvisionBindingIntent;
/// let _forged = VerifiedProvisionBindingIntent {};
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct VerifiedProvisionBindingIntent {
    authorization_domain: CommunityId,
    operation_id: Uuid,
    policy_revision: u64,
    policy_digest: [u8; 32],
    principal: FederatedPrincipal,
    selected_event_author_pubkey: [u8; 32],
    intent_digest: [u8; 32],
    approval_policy_digest: [u8; 32],
    signer_attribution: [u8; 32],
    approval_attribution: Option<[u8; 32]>,
    request_attribution: [u8; 32],
    issued_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl VerifiedProvisionBindingIntent {
    /// Server-resolved domain explicitly sealed by the signed payload.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Exact operation identity explicitly sealed by the signed payload.
    pub const fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    /// Exact local enrollment-policy revision selected by the operator.
    pub const fn policy_revision(&self) -> u64 {
        self.policy_revision
    }

    /// Exact local enrollment-policy digest selected by the operator.
    pub const fn policy_digest(&self) -> [u8; 32] {
        self.policy_digest
    }

    /// Borrow the exact principal coordinates selected by the operator.
    pub fn principal_storage_key(&self) -> crate::FederatedPrincipalStorageKey<'_> {
        self.principal.storage_key()
    }

    /// Exact event-author key selected by the provision intent.
    pub const fn selected_event_author_pubkey(&self) -> [u8; 32] {
        self.selected_event_author_pubkey
    }

    /// Digest of the exact payload-bound provisioning intent.
    pub const fn intent_digest(&self) -> [u8; 32] {
        self.intent_digest
    }

    /// Stable two-party attribution for the provisioning workflow.
    pub const fn request_attribution(&self) -> [u8; 32] {
        self.request_attribution
    }

    /// Latest issuance time across the two exact NIP-98 proofs.
    pub const fn issued_at(&self) -> DateTime<Utc> {
        self.issued_at
    }

    /// Exclusive trusted expiry.
    pub const fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    /// Pseudonymous configured signer attribution.
    pub const fn signer_attribution(&self) -> [u8; 32] {
        self.signer_attribution
    }

    /// Independently configured approver attribution.
    pub const fn approval_attribution(&self) -> Option<[u8; 32]> {
        self.approval_attribution
    }

    /// Digest of the immutable verifier configuration and approval policy.
    pub const fn approval_policy_digest(&self) -> [u8; 32] {
        self.approval_policy_digest
    }
}

impl fmt::Debug for VerifiedProvisionBindingIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VerifiedProvisionBindingIntent([REDACTED])")
    }
}

/// Configuration or verification failures for the sealed operator boundary.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum OperatorLifecycleGrantError {
    /// The configured operator set cannot satisfy the closed approval policy.
    #[error("operator lifecycle verifier configuration is invalid")]
    InvalidConfiguration,
    /// The typed lifecycle/provisioning coordinates are invalid.
    #[error("operator lifecycle request is invalid")]
    InvalidRequest,
    /// One of the exact NIP-98 proofs was malformed, stale, or mismatched.
    #[error("operator lifecycle credential is invalid")]
    InvalidCredential,
    /// A signer is absent from the configured deployment operator set.
    #[error("operator lifecycle signer is not configured")]
    Unauthenticated,
    /// The two required proofs were produced by the same signer.
    #[error("operator lifecycle approval is not independent")]
    ApprovalNotIndependent,
    /// One of the exact proof identities was already consumed.
    #[error("operator lifecycle credential was replayed")]
    Replayed,
    /// The server-owned authorization-domain quota rejected the action.
    #[error("operator lifecycle quota is exhausted")]
    QuotaExceeded,
    /// Shared quota state was unavailable; verification failed closed.
    #[error("operator lifecycle quota is unavailable")]
    QuotaUnavailable,
    /// The shared replay fence was unavailable; verification failed closed.
    #[error("operator lifecycle replay fence is unavailable")]
    ReplayUnavailable,
}

/// Exact NIP-98/configuration verifier and sole mint for operator authority.
#[derive(Clone)]
pub struct OperatorLifecycleVerifier {
    configured_operators: Box<[PublicKey]>,
    approval_policy: OperatorApprovalPolicy,
    approval_policy_digest: [u8; 32],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OperatorApprovalPolicy {
    ConfiguredSigner = 1,
    IndependentConfiguredApprover = 2,
}

impl OperatorLifecycleVerifier {
    /// Install the stock policy: one exact payload-bound configured signer.
    pub fn new(configured_operators: Vec<PublicKey>) -> Result<Self, OperatorLifecycleGrantError> {
        Self::with_policy(
            configured_operators,
            OperatorApprovalPolicy::ConfiguredSigner,
        )
    }

    /// Install a deployment policy requiring one independently configured
    /// approver in addition to the exact payload-bound signer.
    pub fn requiring_independent_approval(
        configured_operators: Vec<PublicKey>,
    ) -> Result<Self, OperatorLifecycleGrantError> {
        Self::with_policy(
            configured_operators,
            OperatorApprovalPolicy::IndependentConfiguredApprover,
        )
    }

    fn with_policy(
        mut configured_operators: Vec<PublicKey>,
        approval_policy: OperatorApprovalPolicy,
    ) -> Result<Self, OperatorLifecycleGrantError> {
        configured_operators.sort_unstable_by_key(|key| key.to_bytes());
        configured_operators.dedup();
        let minimum = match approval_policy {
            OperatorApprovalPolicy::ConfiguredSigner => 1,
            OperatorApprovalPolicy::IndependentConfiguredApprover => 2,
        };
        if configured_operators.len() < minimum
            || configured_operators.len() > MAX_CONFIGURED_OPERATORS
        {
            return Err(OperatorLifecycleGrantError::InvalidConfiguration);
        }
        let approval_policy_digest = operator_policy_digest(approval_policy, &configured_operators);
        Ok(Self {
            configured_operators: configured_operators.into_boxed_slice(),
            approval_policy,
            approval_policy_digest,
        })
    }

    /// Whether this closed deployment policy requires an independent approval.
    pub const fn requires_independent_approval(&self) -> bool {
        matches!(
            self.approval_policy,
            OperatorApprovalPolicy::IndependentConfiguredApprover
        )
    }

    /// Digest of the sorted allowlist and closed approval policy.
    pub const fn approval_policy_digest(&self) -> [u8; 32] {
        self.approval_policy_digest
    }

    /// Verify exact NIP-98 authority for one signed lifecycle body.
    #[allow(clippy::too_many_arguments)]
    pub async fn verify_lifecycle(
        &self,
        authorization_event_json: &str,
        independent_approval_event_json: Option<&str>,
        replacement_proof_event_json: Option<&str>,
        expected_url: &str,
        request_body: &[u8],
        replay_guard: &dyn OperatorProofSetReplayGuard,
        authorization_domain: CommunityId,
        action: OperatorLifecycleAction,
        operation_id: Uuid,
        intent_digest: [u8; 32],
        expected_revision: u64,
        replacement_event_author_pubkey: Option<[u8; 32]>,
        replacement_expires_at: Option<DateTime<Utc>>,
    ) -> Result<VerifiedOperatorLifecycleGrant, OperatorLifecycleGrantError> {
        if authorization_domain.as_uuid().is_nil()
            || operation_id.is_nil()
            || intent_digest == [0; 32]
            || expected_revision == 0
        {
            return Err(OperatorLifecycleGrantError::InvalidRequest);
        }
        validate_lifecycle_url(expected_url, authorization_domain, action)?;
        let signed = parse_lifecycle_coordinates(request_body, action)?;
        if signed.operation_id != operation_id
            || signed.expected_revision != expected_revision
            || signed.replacement.map(|value| value.event_author_pubkey)
                != replacement_event_author_pubkey
            || signed.replacement.map(|value| value.requested_expires_at) != replacement_expires_at
        {
            return Err(OperatorLifecycleGrantError::InvalidRequest);
        }
        let replay_scope = format!(
            "operator-lifecycle:{}:{}:{}",
            authorization_domain.as_uuid(),
            action.as_path_segment(),
            operation_id
        );
        let proof = self
            .verify_operator_proofs(
                authorization_event_json,
                independent_approval_event_json,
                expected_url,
                request_body,
                Some(intent_digest),
            )
            .await?;
        let replacement = self
            .verify_replacement_proof(
                action,
                replacement_proof_event_json,
                signed.replacement,
                expected_url,
                request_body,
                authorization_domain,
                operation_id,
                intent_digest,
            )
            .await?;
        let expires_at = replacement
            .as_ref()
            .map(|replacement| {
                proof
                    .expires_at
                    .min(replacement.credential_expires_at)
                    .min(replacement.requested_expires_at)
            })
            .unwrap_or(proof.expires_at);
        let issued_at = replacement
            .as_ref()
            .map(|replacement| proof.issued_at.max(replacement.issued_at))
            .unwrap_or(proof.issued_at);
        let verification_now = Utc::now();
        if issued_at > verification_now || verification_now >= expires_at {
            return Err(OperatorLifecycleGrantError::InvalidCredential);
        }
        let mut replay_event_ids = Vec::with_capacity(3);
        replay_event_ids.push(proof.signer_event_id);
        replay_event_ids.extend(proof.approval_event_id);
        replay_event_ids.extend(replacement.as_ref().map(|value| value.event_id));
        let quota_scope = format!(
            "operator-quota:{}:{}",
            authorization_domain.as_uuid(),
            action.as_path_segment()
        );
        claim_server_quota(replay_guard, &quota_scope, &proof.signer_attribution).await?;
        claim_replay_set(replay_guard, &replay_scope, &replay_event_ids).await?;
        let request_attribution = framed_digest(
            b"buzz:verified-operator-lifecycle-grant:v1",
            &[
                authorization_domain.as_uuid().as_bytes(),
                &(action as i16).to_be_bytes(),
                operation_id.as_bytes(),
                &intent_digest,
                &expected_revision.to_be_bytes(),
                &self.approval_policy_digest,
                &proof.signer_attribution,
                &proof.approval_attribution.unwrap_or([0; 32]),
                &replacement
                    .as_ref()
                    .map(|value| value.event_author_pubkey)
                    .unwrap_or([0; 32]),
                &replacement
                    .as_ref()
                    .map(|value| value.requested_expires_at.timestamp().to_be_bytes())
                    .unwrap_or([0; 8]),
            ],
        );
        Ok(VerifiedOperatorLifecycleGrant {
            authorization_domain,
            action,
            operation_id,
            intent_digest,
            expected_revision,
            approval_policy_digest: self.approval_policy_digest,
            signer_attribution: proof.signer_attribution,
            approval_attribution: proof.approval_attribution,
            replacement_event_author_pubkey: replacement
                .as_ref()
                .map(|value| value.event_author_pubkey),
            replacement_expires_at: replacement.as_ref().map(|value| value.requested_expires_at),
            replacement_proof_attribution: replacement
                .as_ref()
                .map(|value| value.proof_attribution),
            request_attribution,
            issued_at,
            expires_at,
        })
    }

    /// Verify exact NIP-98 authority for one explicit binding provision body.
    pub async fn verify_provision_binding(
        &self,
        authorization_event_json: &str,
        independent_approval_event_json: Option<&str>,
        expected_url: &str,
        request_body: &[u8],
        replay_guard: &dyn OperatorProofSetReplayGuard,
        authorization_domain: CommunityId,
    ) -> Result<VerifiedProvisionBindingIntent, OperatorLifecycleGrantError> {
        if authorization_domain.as_uuid().is_nil() {
            return Err(OperatorLifecycleGrantError::InvalidRequest);
        }
        validate_provision_url(expected_url)?;
        let signed = parse_provision_coordinates(request_body, authorization_domain)?;
        let body_digest: [u8; 32] = Sha256::digest(request_body).into();
        let intent_digest = framed_digest(
            b"buzz:operator-provision-binding-intent:v2",
            &[
                authorization_domain.as_uuid().as_bytes(),
                signed.operation_id.as_bytes(),
                &signed.policy_revision.to_be_bytes(),
                &signed.policy_digest,
                signed.principal.storage_key().issuer().as_bytes(),
                signed.principal.storage_key().subject().as_bytes(),
                &signed.selected_event_author_pubkey,
                &body_digest,
            ],
        );
        let replay_scope = format!(
            "operator-provision:{}:{}",
            authorization_domain.as_uuid(),
            signed.operation_id
        );
        let proof = self
            .verify_operator_proofs(
                authorization_event_json,
                independent_approval_event_json,
                expected_url,
                request_body,
                None,
            )
            .await?;
        let request_attribution = framed_digest(
            b"buzz:verified-operator-provision-intent:v2",
            &[
                &intent_digest,
                &self.approval_policy_digest,
                &proof.signer_attribution,
                &proof.approval_attribution.unwrap_or([0; 32]),
            ],
        );
        let mut replay_event_ids = Vec::with_capacity(2);
        replay_event_ids.push(proof.signer_event_id);
        replay_event_ids.extend(proof.approval_event_id);
        let quota_scope = format!(
            "operator-quota:{}:provision",
            authorization_domain.as_uuid()
        );
        claim_server_quota(replay_guard, &quota_scope, &proof.signer_attribution).await?;
        claim_replay_set(replay_guard, &replay_scope, &replay_event_ids).await?;
        Ok(VerifiedProvisionBindingIntent {
            authorization_domain,
            operation_id: signed.operation_id,
            policy_revision: signed.policy_revision,
            policy_digest: signed.policy_digest,
            principal: signed.principal,
            selected_event_author_pubkey: signed.selected_event_author_pubkey,
            intent_digest,
            approval_policy_digest: self.approval_policy_digest,
            signer_attribution: proof.signer_attribution,
            approval_attribution: proof.approval_attribution,
            request_attribution,
            issued_at: proof.issued_at,
            expires_at: proof.expires_at,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn verify_operator_proofs(
        &self,
        authorization_event_json: &str,
        independent_approval_event_json: Option<&str>,
        expected_url: &str,
        request_body: &[u8],
        intent_digest: Option<[u8; 32]>,
    ) -> Result<VerifiedOperatorPair, OperatorLifecycleGrantError> {
        if expected_url.is_empty() || request_body.is_empty() {
            return Err(OperatorLifecycleGrantError::InvalidRequest);
        }
        let signer_event = strict_nip98_event(
            authorization_event_json,
            expected_url,
            request_body,
            intent_digest,
        )?;
        if !self.configured_operators.contains(&signer_event.pubkey) {
            return Err(OperatorLifecycleGrantError::Unauthenticated);
        }
        let approval_event = match (self.approval_policy, independent_approval_event_json) {
            (OperatorApprovalPolicy::ConfiguredSigner, None) => None,
            (OperatorApprovalPolicy::ConfiguredSigner, Some(_))
            | (OperatorApprovalPolicy::IndependentConfiguredApprover, None) => {
                return Err(OperatorLifecycleGrantError::InvalidRequest)
            }
            (OperatorApprovalPolicy::IndependentConfiguredApprover, Some(value)) => {
                let event = strict_nip98_event(value, expected_url, request_body, intent_digest)?;
                if event.pubkey == signer_event.pubkey {
                    return Err(OperatorLifecycleGrantError::ApprovalNotIndependent);
                }
                if !self.configured_operators.contains(&event.pubkey) {
                    return Err(OperatorLifecycleGrantError::Unauthenticated);
                }
                Some(event)
            }
        };
        let signer_seconds = signer_event.created_at.as_secs();
        let approval_seconds = approval_event
            .as_ref()
            .map(|event| event.created_at.as_secs())
            .unwrap_or(signer_seconds);
        let issued_seconds = signer_seconds.max(approval_seconds);
        let expires_seconds = signer_seconds
            .saturating_add(NIP98_EXCLUSIVE_LIFETIME_SECONDS)
            .min(approval_seconds.saturating_add(NIP98_EXCLUSIVE_LIFETIME_SECONDS));
        let issued_at = DateTime::<Utc>::from_timestamp(
            i64::try_from(issued_seconds)
                .map_err(|_| OperatorLifecycleGrantError::InvalidCredential)?,
            0,
        )
        .ok_or(OperatorLifecycleGrantError::InvalidCredential)?;
        let expires_at = DateTime::<Utc>::from_timestamp(
            i64::try_from(expires_seconds)
                .map_err(|_| OperatorLifecycleGrantError::InvalidCredential)?,
            0,
        )
        .ok_or(OperatorLifecycleGrantError::InvalidCredential)?;
        let verification_now = Utc::now();
        if issued_at > verification_now || verification_now >= expires_at {
            return Err(OperatorLifecycleGrantError::InvalidCredential);
        }
        Ok(VerifiedOperatorPair {
            signer_event_id: signer_event.id,
            approval_event_id: approval_event.as_ref().map(|event| event.id),
            signer_attribution: operator_attribution(signer_event.pubkey),
            approval_attribution: approval_event
                .as_ref()
                .map(|event| operator_attribution(event.pubkey)),
            issued_at,
            expires_at,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn verify_replacement_proof(
        &self,
        action: OperatorLifecycleAction,
        replacement_proof_event_json: Option<&str>,
        replacement: Option<SignedReplacementCoordinates>,
        expected_url: &str,
        request_body: &[u8],
        authorization_domain: CommunityId,
        operation_id: Uuid,
        intent_digest: [u8; 32],
    ) -> Result<Option<VerifiedReplacementPair>, OperatorLifecycleGrantError> {
        let required = matches!(
            action,
            OperatorLifecycleAction::Rotate
                | OperatorLifecycleAction::Recover
                | OperatorLifecycleAction::Enable
        );
        let (event_json, replacement) = match (replacement_proof_event_json, replacement) {
            (Some(event_json), Some(replacement)) if required => (event_json, replacement),
            (None, None) if !required => return Ok(None),
            _ => return Err(OperatorLifecycleGrantError::InvalidRequest),
        };
        let event =
            strict_nip98_event(event_json, expected_url, request_body, Some(intent_digest))?;
        if event.pubkey.to_bytes() != replacement.event_author_pubkey {
            return Err(OperatorLifecycleGrantError::InvalidCredential);
        }
        let expires_seconds = event
            .created_at
            .as_secs()
            .saturating_add(NIP98_EXCLUSIVE_LIFETIME_SECONDS);
        let expires_at = DateTime::<Utc>::from_timestamp(
            i64::try_from(expires_seconds)
                .map_err(|_| OperatorLifecycleGrantError::InvalidCredential)?,
            0,
        )
        .ok_or(OperatorLifecycleGrantError::InvalidCredential)?;
        let issued_at = DateTime::<Utc>::from_timestamp(
            i64::try_from(event.created_at.as_secs())
                .map_err(|_| OperatorLifecycleGrantError::InvalidCredential)?,
            0,
        )
        .ok_or(OperatorLifecycleGrantError::InvalidCredential)?;
        let verification_now = Utc::now();
        if issued_at > verification_now || verification_now >= expires_at {
            return Err(OperatorLifecycleGrantError::InvalidCredential);
        }
        Ok(Some(VerifiedReplacementPair {
            event_id: event.id,
            event_author_pubkey: replacement.event_author_pubkey,
            requested_expires_at: replacement.requested_expires_at,
            proof_attribution: framed_digest(
                b"buzz:operator-replacement-proof-attribution:v2",
                &[
                    authorization_domain.as_uuid().as_bytes(),
                    operation_id.as_bytes(),
                    &(action as i16).to_be_bytes(),
                    &intent_digest,
                    &replacement.event_author_pubkey,
                    &replacement.requested_expires_at.timestamp().to_be_bytes(),
                ],
            ),
            issued_at,
            credential_expires_at: expires_at,
        }))
    }
}

impl fmt::Debug for OperatorLifecycleVerifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OperatorLifecycleVerifier([REDACTED])")
    }
}

struct VerifiedOperatorPair {
    signer_event_id: EventId,
    approval_event_id: Option<EventId>,
    signer_attribution: [u8; 32],
    approval_attribution: Option<[u8; 32]>,
    issued_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

struct VerifiedReplacementPair {
    event_id: EventId,
    event_author_pubkey: [u8; 32],
    requested_expires_at: DateTime<Utc>,
    proof_attribution: [u8; 32],
    issued_at: DateTime<Utc>,
    credential_expires_at: DateTime<Utc>,
}

#[derive(Clone, Copy)]
struct SignedReplacementCoordinates {
    event_author_pubkey: [u8; 32],
    requested_expires_at: DateTime<Utc>,
}

struct SignedLifecycleCoordinates {
    operation_id: Uuid,
    expected_revision: u64,
    replacement: Option<SignedReplacementCoordinates>,
}

struct SignedProvisionCoordinates {
    operation_id: Uuid,
    policy_revision: u64,
    policy_digest: [u8; 32],
    principal: FederatedPrincipal,
    selected_event_author_pubkey: [u8; 32],
}

#[derive(Deserialize)]
struct LifecycleOperationWireCoordinates {
    operation_id: Uuid,
}

#[derive(Deserialize)]
struct LifecycleWireCoordinates {
    operation: LifecycleOperationWireCoordinates,
    expected_revision: u64,
    #[serde(default)]
    replacement: Option<ReplacementWireCoordinates>,
}

#[derive(Deserialize)]
struct ReplacementWireCoordinates {
    event_author_pubkey: String,
    expires_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct ProvisionWireCoordinates {
    authorization_domain: Uuid,
    operation_id: Uuid,
    policy_revision: u64,
    policy_digest: String,
    issuer: String,
    subject: String,
    selected_event_author_pubkey: String,
}

fn parse_lifecycle_coordinates(
    request_body: &[u8],
    action: OperatorLifecycleAction,
) -> Result<SignedLifecycleCoordinates, OperatorLifecycleGrantError> {
    let wire: LifecycleWireCoordinates = serde_json::from_slice(request_body)
        .map_err(|_| OperatorLifecycleGrantError::InvalidRequest)?;
    if wire.operation.operation_id.is_nil() || wire.expected_revision == 0 {
        return Err(OperatorLifecycleGrantError::InvalidRequest);
    }
    let requires_replacement = matches!(
        action,
        OperatorLifecycleAction::Rotate
            | OperatorLifecycleAction::Recover
            | OperatorLifecycleAction::Enable
    );
    let replacement = match wire.replacement {
        Some(replacement) if requires_replacement => {
            if replacement.expires_at <= Utc::now() {
                return Err(OperatorLifecycleGrantError::InvalidRequest);
            }
            Some(SignedReplacementCoordinates {
                event_author_pubkey: parse_hex_32(&replacement.event_author_pubkey)?,
                requested_expires_at: replacement.expires_at,
            })
        }
        None if !requires_replacement => None,
        _ => return Err(OperatorLifecycleGrantError::InvalidRequest),
    };
    Ok(SignedLifecycleCoordinates {
        operation_id: wire.operation.operation_id,
        expected_revision: wire.expected_revision,
        replacement,
    })
}

fn parse_provision_coordinates(
    request_body: &[u8],
    authorization_domain: CommunityId,
) -> Result<SignedProvisionCoordinates, OperatorLifecycleGrantError> {
    let wire: ProvisionWireCoordinates = serde_json::from_slice(request_body)
        .map_err(|_| OperatorLifecycleGrantError::InvalidRequest)?;
    if wire.authorization_domain != *authorization_domain.as_uuid()
        || wire.operation_id.is_nil()
        || wire.policy_revision == 0
    {
        return Err(OperatorLifecycleGrantError::InvalidRequest);
    }
    let principal = FederatedPrincipal::from_verified_parts(wire.issuer, wire.subject)
        .ok_or(OperatorLifecycleGrantError::InvalidRequest)?;
    Ok(SignedProvisionCoordinates {
        operation_id: wire.operation_id,
        policy_revision: wire.policy_revision,
        policy_digest: parse_hex_32(&wire.policy_digest)?,
        principal,
        selected_event_author_pubkey: parse_hex_32(&wire.selected_event_author_pubkey)?,
    })
}

fn validate_lifecycle_url(
    expected_url: &str,
    authorization_domain: CommunityId,
    action: OperatorLifecycleAction,
) -> Result<(), OperatorLifecycleGrantError> {
    let url =
        url::Url::parse(expected_url).map_err(|_| OperatorLifecycleGrantError::InvalidRequest)?;
    let expected_path = format!(
        "/operator/communities/{}/identity-lifecycle/{}",
        authorization_domain.as_uuid(),
        action.as_path_segment()
    );
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
        || url.path() != expected_path
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(OperatorLifecycleGrantError::InvalidRequest);
    }
    let canonical = format!("{}{}", url.origin().ascii_serialization(), expected_path);
    if expected_url != canonical {
        return Err(OperatorLifecycleGrantError::InvalidRequest);
    }
    Ok(())
}

fn validate_provision_url(expected_url: &str) -> Result<(), OperatorLifecycleGrantError> {
    const CANONICAL_PATH: &str = "/operator/identity-bindings/provision";

    let url =
        url::Url::parse(expected_url).map_err(|_| OperatorLifecycleGrantError::InvalidRequest)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
        || url.path() != CANONICAL_PATH
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(OperatorLifecycleGrantError::InvalidRequest);
    }
    let canonical = format!("{}{}", url.origin().ascii_serialization(), CANONICAL_PATH);
    if expected_url != canonical {
        return Err(OperatorLifecycleGrantError::InvalidRequest);
    }
    Ok(())
}

fn parse_hex_32(value: &str) -> Result<[u8; 32], OperatorLifecycleGrantError> {
    let decoded = hex::decode(value).map_err(|_| OperatorLifecycleGrantError::InvalidRequest)?;
    let exact: [u8; 32] = decoded
        .try_into()
        .map_err(|_| OperatorLifecycleGrantError::InvalidRequest)?;
    if exact == [0; 32] {
        return Err(OperatorLifecycleGrantError::InvalidRequest);
    }
    Ok(exact)
}

fn operator_policy_digest(policy: OperatorApprovalPolicy, keys: &[PublicKey]) -> [u8; 32] {
    let mut digest = Sha256::new();
    let domain = b"buzz:operator-approval-policy:v1";
    digest.update((domain.len() as u64).to_be_bytes());
    digest.update(domain);
    digest.update(1_u64.to_be_bytes());
    digest.update([policy as u8]);
    for key in keys {
        digest.update(32_u64.to_be_bytes());
        digest.update(key.to_bytes());
    }
    digest.finalize().into()
}

fn strict_nip98_event(
    event_json: &str,
    expected_url: &str,
    request_body: &[u8],
    intent_digest: Option<[u8; 32]>,
) -> Result<Event, OperatorLifecycleGrantError> {
    let event: Event = serde_json::from_str(event_json)
        .map_err(|_| OperatorLifecycleGrantError::InvalidCredential)?;
    let event_time = DateTime::<Utc>::from_timestamp(
        i64::try_from(event.created_at.as_secs())
            .map_err(|_| OperatorLifecycleGrantError::InvalidCredential)?,
        0,
    )
    .ok_or(OperatorLifecycleGrantError::InvalidCredential)?;
    if event_time > Utc::now() {
        return Err(OperatorLifecycleGrantError::InvalidCredential);
    }
    if event
        .tags
        .iter()
        .filter(|tag| tag.kind() == TagKind::Payload)
        .count()
        != 1
    {
        return Err(OperatorLifecycleGrantError::InvalidCredential);
    }
    if let Some(intent_digest) = intent_digest {
        let expected_intent = hex::encode(intent_digest);
        let mut intent_tags = event.tags.iter().filter(|tag| {
            tag.as_slice()
                .first()
                .is_some_and(|kind| kind == "buzz-intent")
        });
        let exact_intent = intent_tags
            .next()
            .and_then(|tag| tag.as_slice().get(1))
            .is_some_and(|value| value == &expected_intent)
            && intent_tags.next().is_none();
        if !exact_intent {
            return Err(OperatorLifecycleGrantError::InvalidCredential);
        }
    }
    verify_nip98_event(event_json, expected_url, "POST", Some(request_body))
        .map_err(|_| OperatorLifecycleGrantError::InvalidCredential)?;
    Ok(event)
}

async fn claim_replay_set(
    replay_guard: &dyn OperatorProofSetReplayGuard,
    scope: &str,
    event_ids: &[EventId],
) -> Result<(), OperatorLifecycleGrantError> {
    let valid_cardinality = (1..=3).contains(&event_ids.len());
    let distinct = event_ids
        .iter()
        .enumerate()
        .all(|(index, event_id)| !event_ids[..index].contains(event_id));
    if !valid_cardinality || !distinct {
        return Err(OperatorLifecycleGrantError::InvalidCredential);
    }
    match replay_guard
        .try_mark_all_in_scope(scope, event_ids, DEFAULT_REPLAY_TTL_SECS)
        .await
    {
        Ok(true) => Ok(()),
        Ok(false) => Err(OperatorLifecycleGrantError::Replayed),
        Err(AuthError::Nip98Replay) => Err(OperatorLifecycleGrantError::Replayed),
        Err(_) => Err(OperatorLifecycleGrantError::ReplayUnavailable),
    }
}

async fn claim_server_quota(
    guard: &dyn OperatorProofSetReplayGuard,
    quota_scope: &str,
    signer_attribution: &[u8; 32],
) -> Result<(), OperatorLifecycleGrantError> {
    match guard.try_admit_quota(quota_scope, signer_attribution).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(OperatorLifecycleGrantError::QuotaExceeded),
        Err(_) => Err(OperatorLifecycleGrantError::QuotaUnavailable),
    }
}

fn operator_attribution(public_key: PublicKey) -> [u8; 32] {
    framed_digest(
        b"buzz:configured-operator-attribution:v1",
        &[&public_key.to_bytes()],
    )
}

fn framed_digest(domain: &[u8], fields: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update((domain.len() as u64).to_be_bytes());
    digest.update(domain);
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    digest.finalize().into()
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        future::Future,
        pin::Pin,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Mutex,
        },
    };

    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    use super::*;

    #[derive(Default)]
    struct ReplaySet(Mutex<HashSet<(String, [u8; 32])>>);

    impl OperatorProofSetReplayGuard for ReplaySet {
        fn try_admit_quota<'a>(
            &'a self,
            _quota_scope: &'a str,
            _signer_attribution: &'a [u8; 32],
        ) -> Pin<Box<dyn Future<Output = Result<bool, AuthError>> + Send + 'a>> {
            Box::pin(async { Ok(true) })
        }

        fn try_mark_all_in_scope<'a>(
            &'a self,
            scope: &'a str,
            event_ids: &'a [nostr::EventId],
            _ttl_secs: u64,
        ) -> Pin<Box<dyn Future<Output = Result<bool, AuthError>> + Send + 'a>> {
            Box::pin(async move {
                let valid_cardinality = (1..=3).contains(&event_ids.len());
                let distinct = event_ids
                    .iter()
                    .enumerate()
                    .all(|(index, event_id)| !event_ids[..index].contains(event_id));
                if !valid_cardinality || !distinct {
                    return Err(AuthError::Internal(
                        "invalid test proof-set replay claim".to_owned(),
                    ));
                }
                let mut seen = self.0.lock().expect("replay lock");
                if event_ids
                    .iter()
                    .any(|event_id| seen.contains(&(scope.to_owned(), event_id.to_bytes())))
                {
                    return Ok(false);
                }
                for event_id in event_ids {
                    seen.insert((scope.to_owned(), event_id.to_bytes()));
                }
                Ok(true)
            })
        }
    }

    impl ReplaySet {
        fn len(&self) -> usize {
            self.0.lock().expect("replay lock").len()
        }
    }

    struct QuotaFence {
        quota_allowed: bool,
        quota_unavailable: bool,
        quota_scopes: Mutex<Vec<String>>,
        replay_calls: AtomicUsize,
    }

    impl QuotaFence {
        fn rejecting() -> Self {
            Self {
                quota_allowed: false,
                quota_unavailable: false,
                quota_scopes: Mutex::new(Vec::new()),
                replay_calls: AtomicUsize::new(0),
            }
        }

        fn unavailable() -> Self {
            Self {
                quota_allowed: false,
                quota_unavailable: true,
                quota_scopes: Mutex::new(Vec::new()),
                replay_calls: AtomicUsize::new(0),
            }
        }
    }

    impl OperatorProofSetReplayGuard for QuotaFence {
        fn try_admit_quota<'a>(
            &'a self,
            quota_scope: &'a str,
            _signer_attribution: &'a [u8; 32],
        ) -> Pin<Box<dyn Future<Output = Result<bool, AuthError>> + Send + 'a>> {
            Box::pin(async move {
                self.quota_scopes
                    .lock()
                    .expect("quota scope lock")
                    .push(quota_scope.to_owned());
                if self.quota_unavailable {
                    Err(AuthError::Internal("test quota unavailable".to_owned()))
                } else {
                    Ok(self.quota_allowed)
                }
            })
        }

        fn try_mark_all_in_scope<'a>(
            &'a self,
            _scope: &'a str,
            _event_ids: &'a [EventId],
            _ttl_secs: u64,
        ) -> Pin<Box<dyn Future<Output = Result<bool, AuthError>> + Send + 'a>> {
            Box::pin(async move {
                self.replay_calls.fetch_add(1, Ordering::SeqCst);
                Ok(true)
            })
        }
    }

    fn signed(keys: &Keys, url: &str, body: &[u8], intent_digest: [u8; 32]) -> String {
        signed_at(keys, url, body, intent_digest, Timestamp::now())
    }

    fn signed_at(
        keys: &Keys,
        url: &str,
        body: &[u8],
        intent_digest: [u8; 32],
        created_at: Timestamp,
    ) -> String {
        let payload = hex::encode(<[u8; 32]>::from(Sha256::digest(body)));
        signed_at_with_payloads(keys, url, body, intent_digest, created_at, &[payload])
    }

    fn signed_at_with_payloads(
        keys: &Keys,
        url: &str,
        _body: &[u8],
        intent_digest: [u8; 32],
        created_at: Timestamp,
        payloads: &[String],
    ) -> String {
        let intent = hex::encode(intent_digest);
        let mut tags = vec![
            Tag::parse(["u", url]).expect("url tag"),
            Tag::parse(["method", "POST"]).expect("method tag"),
        ];
        tags.extend(
            payloads
                .iter()
                .map(|payload| Tag::parse(["payload", payload.as_str()]).expect("payload tag")),
        );
        tags.push(Tag::parse(["buzz-intent", intent.as_str()]).expect("intent tag"));
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .custom_created_at(created_at)
            .sign_with_keys(keys)
            .expect("sign NIP-98 proof");
        serde_json::to_string(&event).expect("serialize NIP-98 proof")
    }

    fn lifecycle_body(
        operation_id: Uuid,
        intent_digest: [u8; 32],
        replacement: Option<(&Keys, DateTime<Utc>)>,
    ) -> Vec<u8> {
        let mut value = serde_json::json!({
            "operation": {
                "operation_id": operation_id,
                "correlation_id": Uuid::new_v4(),
                "attempt_id": Uuid::new_v4()
            },
            "expected_revision": 1,
            "intent_digest_for_test": hex::encode(intent_digest),
            "target": "redacted"
        });
        if let Some((keys, expires_at)) = replacement {
            value["replacement"] = serde_json::json!({
                "event_author_pubkey": hex::encode(keys.public_key().to_bytes()),
                "expires_at": expires_at
            });
        }
        serde_json::to_vec(&value).expect("serialize lifecycle body")
    }

    #[tokio::test]
    async fn stock_lifecycle_policy_requires_one_configured_exact_proof() {
        let signer = Keys::generate();
        let verifier =
            OperatorLifecycleVerifier::new(vec![signer.public_key()]).expect("configure verifier");
        let replay = ReplaySet::default();
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let operation_id = Uuid::new_v4();
        let intent_digest = [7; 32];
        let body = lifecycle_body(operation_id, intent_digest, None);
        let url = format!(
            "https://operator.example/operator/communities/{}/identity-lifecycle/retire",
            domain.as_uuid()
        );
        assert!(matches!(
            verifier
                .verify_lifecycle(
                    &signed(&signer, &url, &body, [6; 32]),
                    None,
                    None,
                    &url,
                    &body,
                    &replay,
                    domain,
                    OperatorLifecycleAction::Retire,
                    operation_id,
                    intent_digest,
                    1,
                    None,
                    None,
                )
                .await,
            Err(OperatorLifecycleGrantError::InvalidCredential)
        ));
        let grant = verifier
            .verify_lifecycle(
                &signed(&signer, &url, &body, intent_digest),
                None,
                None,
                &url,
                &body,
                &replay,
                domain,
                OperatorLifecycleAction::Retire,
                operation_id,
                intent_digest,
                1,
                None,
                None,
            )
            .await
            .expect("verify grant");
        assert_eq!(grant.authorization_domain(), domain);
        assert_eq!(grant.operation_id(), operation_id);
        assert_eq!(grant.action(), OperatorLifecycleAction::Retire);
        assert_eq!(grant.approval_attribution(), None);
        assert_ne!(grant.request_attribution(), [0; 32]);
        assert_eq!(
            format!("{grant:?}"),
            "VerifiedOperatorLifecycleGrant([REDACTED])"
        );
    }

    #[tokio::test]
    async fn lifecycle_rejects_future_created_at_before_state_claims() {
        let signer = Keys::generate();
        let verifier =
            OperatorLifecycleVerifier::new(vec![signer.public_key()]).expect("configure verifier");
        let guard = QuotaFence::rejecting();
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let operation_id = Uuid::new_v4();
        let intent_digest = [72; 32];
        let body = lifecycle_body(operation_id, intent_digest, None);
        let url = format!(
            "https://operator.example/operator/communities/{}/identity-lifecycle/retire",
            domain.as_uuid()
        );
        let future_created_at = Timestamp::from(
            Timestamp::now()
                .as_secs()
                .saturating_add(NIP98_EXCLUSIVE_LIFETIME_SECONDS),
        );

        let rejection = verifier
            .verify_lifecycle(
                &signed_at(&signer, &url, &body, intent_digest, future_created_at),
                None,
                None,
                &url,
                &body,
                &guard,
                domain,
                OperatorLifecycleAction::Retire,
                operation_id,
                intent_digest,
                1,
                None,
                None,
            )
            .await
            .expect_err("future-created proof must fail closed");

        assert_eq!(
            rejection,
            OperatorLifecycleGrantError::InvalidCredential,
            "future-created proof must retain the stable invalid-credential rejection"
        );
        assert!(
            guard
                .quota_scopes
                .lock()
                .expect("quota scope lock")
                .is_empty(),
            "future-created proof must not claim quota state"
        );
        assert_eq!(
            guard.replay_calls.load(Ordering::SeqCst),
            0,
            "future-created proof must not claim replay state"
        );
    }

    #[tokio::test]
    async fn allowlist_and_server_owned_quota_precede_replay_io() {
        let signer = Keys::generate();
        let outsider = Keys::generate();
        let verifier =
            OperatorLifecycleVerifier::new(vec![signer.public_key()]).expect("configure verifier");
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let operation_id = Uuid::new_v4();
        let intent_digest = [71; 32];
        let body = lifecycle_body(operation_id, intent_digest, None);
        let url = format!(
            "https://operator.example/operator/communities/{}/identity-lifecycle/retire",
            domain.as_uuid()
        );

        let outsider_guard = QuotaFence::rejecting();
        assert!(matches!(
            verifier
                .verify_lifecycle(
                    &signed(&outsider, &url, &body, intent_digest),
                    None,
                    None,
                    &url,
                    &body,
                    &outsider_guard,
                    domain,
                    OperatorLifecycleAction::Retire,
                    operation_id,
                    intent_digest,
                    1,
                    None,
                    None,
                )
                .await,
            Err(OperatorLifecycleGrantError::Unauthenticated)
        ));
        assert!(
            outsider_guard
                .quota_scopes
                .lock()
                .expect("quota scope lock")
                .is_empty(),
            "unconfigured signers must not consume a server quota"
        );
        assert_eq!(outsider_guard.replay_calls.load(Ordering::SeqCst), 0);

        let quota_scope = format!("operator-quota:{}:retire", domain.as_uuid());
        for (guard, expected) in [
            (
                QuotaFence::rejecting(),
                OperatorLifecycleGrantError::QuotaExceeded,
            ),
            (
                QuotaFence::unavailable(),
                OperatorLifecycleGrantError::QuotaUnavailable,
            ),
        ] {
            assert_eq!(
                verifier
                    .verify_lifecycle(
                        &signed(&signer, &url, &body, intent_digest),
                        None,
                        None,
                        &url,
                        &body,
                        &guard,
                        domain,
                        OperatorLifecycleAction::Retire,
                        operation_id,
                        intent_digest,
                        1,
                        None,
                        None,
                    )
                    .await
                    .expect_err("quota denial must fail closed"),
                expected
            );
            assert_eq!(
                guard
                    .quota_scopes
                    .lock()
                    .expect("quota scope lock")
                    .as_slice(),
                [quota_scope.as_str()]
            );
            assert_eq!(
                guard.replay_calls.load(Ordering::SeqCst),
                0,
                "quota denial must not consume replay identities"
            );
        }
    }

    #[tokio::test]
    async fn lifecycle_grant_rejects_self_approval_and_exact_replay() {
        let signer = Keys::generate();
        let approver = Keys::generate();
        let verifier = OperatorLifecycleVerifier::requiring_independent_approval(vec![
            signer.public_key(),
            approver.public_key(),
        ])
        .expect("configure verifier");
        let replay = ReplaySet::default();
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let operation_id = Uuid::new_v4();
        let intent_digest = [8; 32];
        let body = lifecycle_body(operation_id, intent_digest, None);
        let url = format!(
            "https://operator.example/operator/communities/{}/identity-lifecycle/disable",
            domain.as_uuid()
        );
        let signer_proof = signed(&signer, &url, &body, intent_digest);
        let approver_proof = signed(&approver, &url, &body, intent_digest);
        assert!(matches!(
            verifier
                .verify_lifecycle(
                    &signer_proof,
                    Some(&signer_proof),
                    None,
                    &url,
                    &body,
                    &replay,
                    domain,
                    OperatorLifecycleAction::Disable,
                    operation_id,
                    intent_digest,
                    1,
                    None,
                    None,
                )
                .await,
            Err(OperatorLifecycleGrantError::ApprovalNotIndependent)
        ));
        verifier
            .verify_lifecycle(
                &signer_proof,
                Some(&approver_proof),
                None,
                &url,
                &body,
                &replay,
                domain,
                OperatorLifecycleAction::Disable,
                operation_id,
                intent_digest,
                1,
                None,
                None,
            )
            .await
            .expect("first exact grant");
        assert!(matches!(
            verifier
                .verify_lifecycle(
                    &signer_proof,
                    Some(&approver_proof),
                    None,
                    &url,
                    &body,
                    &replay,
                    domain,
                    OperatorLifecycleAction::Disable,
                    operation_id,
                    intent_digest,
                    1,
                    None,
                    None,
                )
                .await,
            Err(OperatorLifecycleGrantError::Replayed)
        ));
    }

    #[tokio::test]
    async fn replacement_actions_require_exact_replacement_key_proof() {
        let signer = Keys::generate();
        let replacement = Keys::generate();
        let verifier =
            OperatorLifecycleVerifier::new(vec![signer.public_key()]).expect("configure verifier");
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let operation_id = Uuid::new_v4();
        let intent_digest = [9; 32];
        let replacement_expires_at = Utc::now() + chrono::Duration::seconds(30);
        let body = lifecycle_body(
            operation_id,
            intent_digest,
            Some((&replacement, replacement_expires_at)),
        );
        let url = format!(
            "https://operator.example/operator/communities/{}/identity-lifecycle/rotate",
            domain.as_uuid()
        );
        assert!(matches!(
            verifier
                .verify_lifecycle(
                    &signed(&signer, &url, &body, intent_digest),
                    None,
                    None,
                    &url,
                    &body,
                    &ReplaySet::default(),
                    domain,
                    OperatorLifecycleAction::Rotate,
                    operation_id,
                    intent_digest,
                    1,
                    Some(replacement.public_key().to_bytes()),
                    Some(replacement_expires_at + chrono::Duration::seconds(1)),
                )
                .await,
            Err(OperatorLifecycleGrantError::InvalidRequest)
        ));
        assert!(matches!(
            verifier
                .verify_lifecycle(
                    &signed(&signer, &url, &body, intent_digest),
                    None,
                    None,
                    &url,
                    &body,
                    &ReplaySet::default(),
                    domain,
                    OperatorLifecycleAction::Rotate,
                    operation_id,
                    intent_digest,
                    1,
                    Some(replacement.public_key().to_bytes()),
                    Some(replacement_expires_at),
                )
                .await,
            Err(OperatorLifecycleGrantError::InvalidRequest)
        ));
        let grant = verifier
            .verify_lifecycle(
                &signed(&signer, &url, &body, intent_digest),
                None,
                Some(&signed(&replacement, &url, &body, intent_digest)),
                &url,
                &body,
                &ReplaySet::default(),
                domain,
                OperatorLifecycleAction::Rotate,
                operation_id,
                intent_digest,
                1,
                Some(replacement.public_key().to_bytes()),
                Some(replacement_expires_at),
            )
            .await
            .expect("verify replacement-bound grant");
        assert_eq!(
            grant.replacement_event_author_pubkey(),
            Some(replacement.public_key().to_bytes())
        );
        assert!(grant.replacement_proof_attribution().is_some());
        assert_eq!(grant.replacement_expires_at(), Some(replacement_expires_at));
    }

    #[tokio::test]
    async fn invalid_complete_proof_sets_burn_no_replay_markers() {
        let signer = Keys::generate();
        let approver = Keys::generate();
        let outsider = Keys::generate();
        let replacement = Keys::generate();
        let verifier = OperatorLifecycleVerifier::requiring_independent_approval(vec![
            signer.public_key(),
            approver.public_key(),
        ])
        .expect("configure verifier");
        let replay = ReplaySet::default();
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let operation_id = Uuid::new_v4();
        let intent_digest = [31; 32];
        let requested_expiry = Utc::now() + chrono::Duration::seconds(30);
        let body = lifecycle_body(
            operation_id,
            intent_digest,
            Some((&replacement, requested_expiry)),
        );
        let url = format!(
            "https://operator.example/operator/communities/{}/identity-lifecycle/rotate",
            domain.as_uuid()
        );
        let signer_proof = signed(&signer, &url, &body, intent_digest);
        let approver_proof = signed(&approver, &url, &body, intent_digest);

        assert!(matches!(
            verifier
                .verify_lifecycle(
                    &signer_proof,
                    Some(&signed(&outsider, &url, &body, intent_digest)),
                    Some(&signed(&replacement, &url, &body, intent_digest)),
                    &url,
                    &body,
                    &replay,
                    domain,
                    OperatorLifecycleAction::Rotate,
                    operation_id,
                    intent_digest,
                    1,
                    Some(replacement.public_key().to_bytes()),
                    Some(requested_expiry),
                )
                .await,
            Err(OperatorLifecycleGrantError::Unauthenticated)
        ));
        assert_eq!(replay.len(), 0, "bad approval must burn no marker");

        assert!(matches!(
            verifier
                .verify_lifecycle(
                    &signer_proof,
                    Some(&approver_proof),
                    Some(&signed(&outsider, &url, &body, intent_digest)),
                    &url,
                    &body,
                    &replay,
                    domain,
                    OperatorLifecycleAction::Rotate,
                    operation_id,
                    intent_digest,
                    1,
                    Some(replacement.public_key().to_bytes()),
                    Some(requested_expiry),
                )
                .await,
            Err(OperatorLifecycleGrantError::InvalidCredential)
        ));
        assert_eq!(replay.len(), 0, "bad replacement must burn no marker");

        let expired_replacement = signed_at(
            &replacement,
            &url,
            &body,
            intent_digest,
            Timestamp::from(
                u64::try_from(Utc::now().timestamp() - 120).expect("positive expired timestamp"),
            ),
        );
        assert!(matches!(
            verifier
                .verify_lifecycle(
                    &signer_proof,
                    Some(&approver_proof),
                    Some(&expired_replacement),
                    &url,
                    &body,
                    &replay,
                    domain,
                    OperatorLifecycleAction::Rotate,
                    operation_id,
                    intent_digest,
                    1,
                    Some(replacement.public_key().to_bytes()),
                    Some(requested_expiry),
                )
                .await,
            Err(OperatorLifecycleGrantError::InvalidCredential)
        ));
        assert_eq!(replay.len(), 0, "expired proof must burn no marker");

        verifier
            .verify_lifecycle(
                &signer_proof,
                Some(&approver_proof),
                Some(&signed(&replacement, &url, &body, intent_digest)),
                &url,
                &body,
                &replay,
                domain,
                OperatorLifecycleAction::Rotate,
                operation_id,
                intent_digest,
                1,
                Some(replacement.public_key().to_bytes()),
                Some(requested_expiry),
            )
            .await
            .expect("valid complete set remains claimable");
        assert_eq!(replay.len(), 3);
    }

    #[tokio::test]
    async fn complete_proof_set_has_one_concurrent_winner() {
        let signer = Keys::generate();
        let approver = Keys::generate();
        let replacement = Keys::generate();
        let verifier = OperatorLifecycleVerifier::requiring_independent_approval(vec![
            signer.public_key(),
            approver.public_key(),
        ])
        .expect("configure verifier");
        let replay = ReplaySet::default();
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let operation_id = Uuid::new_v4();
        let intent_digest = [32; 32];
        let requested_expiry = Utc::now() + chrono::Duration::seconds(30);
        let body = lifecycle_body(
            operation_id,
            intent_digest,
            Some((&replacement, requested_expiry)),
        );
        let url = format!(
            "https://operator.example/operator/communities/{}/identity-lifecycle/rotate",
            domain.as_uuid()
        );
        let signer_proof = signed(&signer, &url, &body, intent_digest);
        let approver_proof = signed(&approver, &url, &body, intent_digest);
        let replacement_proof = signed(&replacement, &url, &body, intent_digest);
        let verify = || {
            verifier.verify_lifecycle(
                &signer_proof,
                Some(&approver_proof),
                Some(&replacement_proof),
                &url,
                &body,
                &replay,
                domain,
                OperatorLifecycleAction::Rotate,
                operation_id,
                intent_digest,
                1,
                Some(replacement.public_key().to_bytes()),
                Some(requested_expiry),
            )
        };
        let (first, second) = tokio::join!(verify(), verify());
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        assert!(matches!(
            first.as_ref().err().or_else(|| second.as_ref().err()),
            Some(OperatorLifecycleGrantError::Replayed)
        ));
        assert_eq!(replay.len(), 3, "one winner claims the complete set");
    }

    #[tokio::test]
    async fn lifecycle_url_is_byte_canonical_before_replay_io() {
        let signer = Keys::generate();
        let verifier =
            OperatorLifecycleVerifier::new(vec![signer.public_key()]).expect("configure verifier");
        let replay = ReplaySet::default();
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let operation_id = Uuid::new_v4();
        let intent_digest = [33; 32];
        let body = lifecycle_body(operation_id, intent_digest, None);
        let path = format!(
            "/operator/communities/{}/identity-lifecycle/retire",
            domain.as_uuid()
        );
        let aliases = [
            format!("http://operator.example{path}"),
            format!("https://operator.example/ignored/..{path}"),
            format!(
                "https://operator.example/operator/communities/{}/identity-lifecycle/%72etire",
                domain.as_uuid()
            ),
            format!("https://OPERATOR.example{path}"),
            format!("https://operator.example:443{path}"),
            format!("https://user@operator.example{path}"),
            format!("https://operator.example{path}?alias=true"),
            format!("https://operator.example{path}#alias"),
        ];
        for alias in aliases {
            assert!(matches!(
                verifier
                    .verify_lifecycle(
                        &signed(&signer, &alias, &body, intent_digest),
                        None,
                        None,
                        &alias,
                        &body,
                        &replay,
                        domain,
                        OperatorLifecycleAction::Retire,
                        operation_id,
                        intent_digest,
                        1,
                        None,
                        None,
                    )
                    .await,
                Err(OperatorLifecycleGrantError::InvalidRequest)
            ));
        }
        assert_eq!(replay.len(), 0);
        let canonical = format!("https://operator.example{path}");
        verifier
            .verify_lifecycle(
                &signed(&signer, &canonical, &body, intent_digest),
                None,
                None,
                &canonical,
                &body,
                &replay,
                domain,
                OperatorLifecycleAction::Retire,
                operation_id,
                intent_digest,
                1,
                None,
                None,
            )
            .await
            .expect("exact canonical URL");
        assert_eq!(replay.len(), 1);
    }

    #[tokio::test]
    async fn lifecycle_requires_exactly_one_payload_tag_before_replay_io() {
        let signer = Keys::generate();
        let verifier =
            OperatorLifecycleVerifier::new(vec![signer.public_key()]).expect("configure verifier");
        let replay = ReplaySet::default();
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let operation_id = Uuid::new_v4();
        let intent_digest = [34; 32];
        let body = lifecycle_body(operation_id, intent_digest, None);
        let url = format!(
            "https://operator.example/operator/communities/{}/identity-lifecycle/retire",
            domain.as_uuid()
        );
        let expected = hex::encode(<[u8; 32]>::from(Sha256::digest(&body)));
        let conflicting = hex::encode([77_u8; 32]);
        for payloads in [
            Vec::<String>::new(),
            vec![expected.clone(), expected.clone()],
            vec![expected.clone(), conflicting.clone()],
            vec![conflicting.clone(), expected.clone()],
        ] {
            let proof = signed_at_with_payloads(
                &signer,
                &url,
                &body,
                intent_digest,
                Timestamp::now(),
                &payloads,
            );
            assert!(matches!(
                verifier
                    .verify_lifecycle(
                        &proof,
                        None,
                        None,
                        &url,
                        &body,
                        &replay,
                        domain,
                        OperatorLifecycleAction::Retire,
                        operation_id,
                        intent_digest,
                        1,
                        None,
                        None,
                    )
                    .await,
                Err(OperatorLifecycleGrantError::InvalidCredential)
            ));
        }
        assert_eq!(replay.len(), 0);
        verifier
            .verify_lifecycle(
                &signed(&signer, &url, &body, intent_digest),
                None,
                None,
                &url,
                &body,
                &replay,
                domain,
                OperatorLifecycleAction::Retire,
                operation_id,
                intent_digest,
                1,
                None,
                None,
            )
            .await
            .expect("one exact payload tag");
        assert_eq!(replay.len(), 1);
    }

    #[tokio::test]
    async fn replacement_grant_expiry_is_the_exact_exclusive_minimum() {
        let now_seconds = Utc::now().timestamp();
        let now = DateTime::<Utc>::from_timestamp(now_seconds, 0).expect("current timestamp");
        let cases = [
            (0_i64, 0_i64, 0_i64, 10_i64, 10_i64),
            (-20, -10, -5, 55, 40),
            (-10, -20, -5, 55, 40),
            (-10, -5, -20, 55, 40),
        ];
        for (
            signer_offset,
            approver_offset,
            replacement_offset,
            requested_offset,
            expected_offset,
        ) in cases
        {
            let signer = Keys::generate();
            let approver = Keys::generate();
            let replacement = Keys::generate();
            let verifier = OperatorLifecycleVerifier::requiring_independent_approval(vec![
                signer.public_key(),
                approver.public_key(),
            ])
            .expect("configure verifier");
            let domain = CommunityId::from_uuid(Uuid::new_v4());
            let operation_id = Uuid::new_v4();
            let intent_digest = [35; 32];
            let requested_expiry = now + chrono::Duration::seconds(requested_offset);
            let body = lifecycle_body(
                operation_id,
                intent_digest,
                Some((&replacement, requested_expiry)),
            );
            let url = format!(
                "https://operator.example/operator/communities/{}/identity-lifecycle/rotate",
                domain.as_uuid()
            );
            let proof_at = |keys: &Keys, offset: i64| {
                signed_at(
                    keys,
                    &url,
                    &body,
                    intent_digest,
                    Timestamp::from(u64::try_from(now_seconds + offset).expect("positive time")),
                )
            };
            let grant = verifier
                .verify_lifecycle(
                    &proof_at(&signer, signer_offset),
                    Some(&proof_at(&approver, approver_offset)),
                    Some(&proof_at(&replacement, replacement_offset)),
                    &url,
                    &body,
                    &ReplaySet::default(),
                    domain,
                    OperatorLifecycleAction::Rotate,
                    operation_id,
                    intent_digest,
                    1,
                    Some(replacement.public_key().to_bytes()),
                    Some(requested_expiry),
                )
                .await
                .expect("verify bounded replacement grant");
            assert_eq!(
                grant.expires_at(),
                now + chrono::Duration::seconds(expected_offset)
            );
            assert_eq!(grant.replacement_expires_at(), Some(requested_expiry));
        }
    }

    #[tokio::test]
    async fn successor_expiry_at_authoritative_bound_denies_before_replay() {
        let signer = Keys::generate();
        let replacement = Keys::generate();
        let verifier =
            OperatorLifecycleVerifier::new(vec![signer.public_key()]).expect("configure verifier");
        let replay = ReplaySet::default();
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let operation_id = Uuid::new_v4();
        let intent_digest = [36; 32];
        let at_bound =
            DateTime::<Utc>::from_timestamp(Utc::now().timestamp(), 0).expect("current timestamp");
        let body = lifecycle_body(operation_id, intent_digest, Some((&replacement, at_bound)));
        let url = format!(
            "https://operator.example/operator/communities/{}/identity-lifecycle/enable",
            domain.as_uuid()
        );
        assert!(matches!(
            verifier
                .verify_lifecycle(
                    &signed(&signer, &url, &body, intent_digest),
                    None,
                    Some(&signed(&replacement, &url, &body, intent_digest)),
                    &url,
                    &body,
                    &replay,
                    domain,
                    OperatorLifecycleAction::Enable,
                    operation_id,
                    intent_digest,
                    1,
                    Some(replacement.public_key().to_bytes()),
                    Some(at_bound),
                )
                .await,
            Err(OperatorLifecycleGrantError::InvalidRequest)
        ));
        assert_eq!(replay.len(), 0);
    }

    #[tokio::test]
    async fn provision_intent_seals_domain_operation_policy_principal_and_key() {
        let signer = Keys::generate();
        let selected = Keys::generate();
        let verifier =
            OperatorLifecycleVerifier::new(vec![signer.public_key()]).expect("configure verifier");
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let operation_id = Uuid::new_v4();
        let body = serde_json::to_vec(&serde_json::json!({
            "authorization_domain": domain.as_uuid(),
            "operation_id": operation_id,
            "policy_revision": 7,
            "policy_digest": hex::encode([21_u8; 32]),
            "issuer": "https://issuer.example",
            "subject": "opaque-subject",
            "selected_event_author_pubkey": hex::encode(selected.public_key().to_bytes())
        }))
        .expect("serialize provision body");
        let url = "https://operator.example/operator/identity-bindings/provision";
        assert!(matches!(
            verifier
                .verify_provision_binding(
                    &signed(&signer, url, &body, [1; 32]),
                    None,
                    url,
                    &body,
                    &ReplaySet::default(),
                    CommunityId::from_uuid(Uuid::new_v4()),
                )
                .await,
            Err(OperatorLifecycleGrantError::InvalidRequest)
        ));
        let verified = verifier
            .verify_provision_binding(
                &signed(&signer, url, &body, [1; 32]),
                None,
                url,
                &body,
                &ReplaySet::default(),
                domain,
            )
            .await
            .expect("verify provision intent");
        assert_eq!(verified.authorization_domain(), domain);
        assert_eq!(verified.operation_id(), operation_id);
        assert_eq!(verified.policy_revision(), 7);
        assert_eq!(verified.policy_digest(), [21; 32]);
        assert_eq!(
            verified.selected_event_author_pubkey(),
            selected.public_key().to_bytes()
        );
        assert_eq!(
            verified.principal_storage_key().issuer(),
            "https://issuer.example"
        );
        assert_eq!(verified.principal_storage_key().subject(), "opaque-subject");
        assert_eq!(
            format!("{verified:?}"),
            "VerifiedProvisionBindingIntent([REDACTED])"
        );
    }

    #[tokio::test]
    async fn provision_rejects_noncanonical_route_before_replay() {
        let signer = Keys::generate();
        let selected = Keys::generate();
        let verifier =
            OperatorLifecycleVerifier::new(vec![signer.public_key()]).expect("configure verifier");
        let domain = CommunityId::from_uuid(Uuid::new_v4());
        let body = serde_json::to_vec(&serde_json::json!({
            "authorization_domain": domain.as_uuid(),
            "operation_id": Uuid::new_v4(),
            "policy_revision": 7,
            "policy_digest": hex::encode([21_u8; 32]),
            "issuer": "https://issuer.example",
            "subject": "opaque-subject",
            "selected_event_author_pubkey": hex::encode(selected.public_key().to_bytes())
        }))
        .expect("serialize provision body");
        let aliases = [
            "http://operator.example/operator/identity-bindings/provision",
            "https://operator.example/operator/identity-bindings/provision/",
            "https://operator.example/operator/identity-bindings/provision?alias=true",
            "https://operator.example/operator/identity-bindings/provision#alias",
            "https://operator.example/operator/communities/provision",
            "https://operator.example/operator/identity-bindings/other/../provision",
        ];
        let replay = ReplaySet::default();
        for alias in aliases {
            assert!(matches!(
                verifier
                    .verify_provision_binding(
                        &signed(&signer, alias, &body, [1; 32]),
                        None,
                        alias,
                        &body,
                        &replay,
                        domain,
                    )
                    .await,
                Err(OperatorLifecycleGrantError::InvalidRequest)
            ));
        }
        assert_eq!(replay.len(), 0, "route rejection must precede replay I/O");
    }

    #[test]
    fn opaque_authority_has_no_sensitive_debug_output() {
        let first = Keys::generate().public_key();
        let second = Keys::generate().public_key();
        let verifier = OperatorLifecycleVerifier::new(vec![first]);
        assert!(verifier.is_ok());
        assert!(OperatorLifecycleVerifier::new(Vec::new()).is_err());
        assert!(OperatorLifecycleVerifier::requiring_independent_approval(vec![first]).is_err());
        let dual = OperatorLifecycleVerifier::requiring_independent_approval(vec![first, second])
            .expect("independent policy");
        assert!(dual.requires_independent_approval());
        assert_ne!(
            dual.approval_policy_digest(),
            verifier.expect("stock policy").approval_policy_digest()
        );
        assert_eq!(OperatorLifecycleAction::Retire as i16, 3);
        assert_eq!(OperatorLifecycleAction::Disable as i16, 4);
        assert_eq!(OperatorLifecycleAction::Revoke as i16, 5);
        assert_eq!(OperatorLifecycleAction::Rotate as i16, 6);
        assert_eq!(OperatorLifecycleAction::Recover as i16, 7);
        assert_eq!(OperatorLifecycleAction::Enable as i16, 8);
        assert_eq!(
            OperatorLifecycleAction::from_path_segment("admission-loss"),
            None
        );
    }
}
