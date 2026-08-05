//! Fail-closed compatibility seam for protected transport coupling.
//!
//! The route-inventory slice replaces this bounded seam with the complete
//! machine-readable inventory. Unknown surfaces never match, and effects that
//! need the later inventory deny while enforcement is active.

use buzz_auth::{AuthTransport, AuthorizationCapability};

use crate::authorization_runtime::finalization::AuthorizationMode;

/// Closed effect identifiers referenced by the protected-transport slice.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum EffectSurfaceId {
    /// Autonomous or delayed workflow execution.
    WorkflowBackgroundExecution,
    /// Legacy best-effort audit delivery.
    LegacyAuditDelivery,
}

/// Proof that one effect is permitted in a non-enforcing compatibility mode.
pub struct EffectPermit {
    id: EffectSurfaceId,
}

impl EffectPermit {
    /// Registered effect represented by this permit.
    pub const fn id(&self) -> EffectSurfaceId {
        self.id
    }
}

/// Fail-closed compatibility error before the complete inventory is installed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum EffectPermitError {
    /// The effect requires the complete route inventory in enforcing mode.
    #[error("protected effect inventory is not installed")]
    InventoryRequired,
}

/// Deny registered effects in enforcing mode until the complete inventory is installed.
pub fn require_effect_permit(
    mode: Option<AuthorizationMode>,
    id: EffectSurfaceId,
) -> Result<EffectPermit, EffectPermitError> {
    if mode == Some(AuthorizationMode::Enforce) {
        return Err(EffectPermitError::InventoryRequired);
    }
    Ok(EffectPermit { id })
}

/// Recheck the stable handler surface, proof transport, and portable capability.
pub fn protected_operation_matches(
    surface: &str,
    transport: AuthTransport,
    capability: AuthorizationCapability,
) -> bool {
    use AuthorizationCapability as Capability;
    match surface {
        "ws_req" | "ws_count" | "ws_fanout" | "client.status.current" => {
            transport == AuthTransport::RelayWebSocket && capability == Capability::CommunityRead
        }
        "ws_event" => {
            transport == AuthTransport::RelayWebSocket
                && matches!(
                    capability,
                    Capability::CommunityWrite | Capability::Moderate
                )
        }
        "event_ingest" => {
            matches!(
                transport,
                AuthTransport::RelayWebSocket | AuthTransport::HttpBridge
            ) && matches!(
                capability,
                Capability::CommunityWrite | Capability::Moderate
            )
        }
        "http_events" => {
            transport == AuthTransport::HttpBridge
                && matches!(
                    capability,
                    Capability::CommunityWrite | Capability::Moderate
                )
        }
        "http_query" | "http_count" => {
            transport == AuthTransport::HttpBridge && capability == Capability::CommunityRead
        }
        "http_moderation_read" => {
            transport == AuthTransport::HttpBridge && capability == Capability::Moderate
        }
        "media.upload" => {
            transport == AuthTransport::MediaUpload && capability == Capability::MediaWrite
        }
        "media.read" => {
            transport == AuthTransport::MediaDownload && capability == Capability::MediaRead
        }
        "git.info_refs" => {
            transport == AuthTransport::Git
                && matches!(capability, Capability::GitRead | Capability::GitWrite)
        }
        "git.upload_pack" => transport == AuthTransport::Git && capability == Capability::GitRead,
        "git.receive_pack" => transport == AuthTransport::Git && capability == Capability::GitWrite,
        "audio.join" => transport == AuthTransport::Audio && capability == Capability::AudioJoin,
        "invite.mint" => {
            transport == AuthTransport::HttpBridge && capability == Capability::InviteMint
        }
        "invite.claim" => {
            transport == AuthTransport::HttpBridge && capability == Capability::InviteClaim
        }
        _ => false,
    }
}

/// Return the exact HTTP bridge event-ingest capability.
pub const fn event_ingest_capability(kind: u32) -> AuthorizationCapability {
    match kind {
        9040..=9044 => AuthorizationCapability::Moderate,
        _ => AuthorizationCapability::CommunityWrite,
    }
}

/// Resolve Git's `info/refs` capability from the validated service name.
pub fn git_info_refs_capability(service: &str) -> Option<AuthorizationCapability> {
    match service {
        "git-upload-pack" => Some(AuthorizationCapability::GitRead),
        "git-receive-pack" => Some(AuthorizationCapability::GitWrite),
        _ => None,
    }
}
