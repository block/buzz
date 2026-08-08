//! Publishing the admin-signed membership and identity events produced by a
//! verified migration claim.
//!
//! This is the one place the service uses the operator's admin key. The Slack
//! OAuth join path first publishes an idempotent member-add event, then a
//! `KIND_IMPORT_IDENTITY_BINDING` mapping the proven subject
//! (`slack:<team>:<user>`) to the claimant's Buzz pubkey. Email recovery
//! publishes only the binding.
//!
//! The attestation is only *half* of a binding: nothing is attributed until the
//! claimant's own `KIND_IMPORT_IDENTITY_CLAIM` (self-signed) also exists. So a
//! stolen admin key can, at worst, publish attestations that stay inert without
//! each subject's separate consent.

use buzz_core::kind::RELAY_ADMIN_ADD_MEMBER;
use nostr::{EventBuilder, Keys, Kind, Tag};

/// Why publishing an admin-signed event failed.
#[derive(Debug, thiserror::Error)]
pub enum AttestError {
    #[error("could not build event: {0}")]
    Build(String),
    #[error("could not sign event: {0}")]
    Sign(String),
    #[error("relay rejected the event: {0}")]
    Rejected(String),
    #[error(transparent)]
    Transport(#[from] buzz_ws_client::WsClientError),
}

/// Add the claimant as a community member (kind 9030 relay-admin add-member).
///
/// The relay derives the community from the connection, checks the signer is an
/// owner/admin, and — crucially — treats an already-present member as a silent
/// no-op, so re-running a claim never duplicates or downgrades membership. This
/// is the "join" half of a Slack-migration sign-in: the OAuth-verified person
/// is registered before their history is attributed.
pub async fn publish_add_member(
    relay_url: &str,
    admin: &Keys,
    member_pubkey_hex: &str,
    auth_tag: Option<&Tag>,
) -> Result<String, AttestError> {
    let p = Tag::parse(["p", member_pubkey_hex]).map_err(|e| AttestError::Build(e.to_string()))?;
    let role = Tag::parse(["role", "member"]).map_err(|e| AttestError::Build(e.to_string()))?;
    let event = EventBuilder::new(Kind::Custom(RELAY_ADMIN_ADD_MEMBER as u16), "")
        .tags([p, role])
        .sign_with_keys(admin)
        .map_err(|e| AttestError::Sign(e.to_string()))?;
    let event_id = event.id.to_hex();
    let ok = buzz_ws_client::publish_event(relay_url, event, admin, auth_tag, 75).await?;
    if !ok.accepted {
        return Err(AttestError::Rejected(ok.message));
    }
    Ok(event_id)
}

/// Sign and publish the attestation `subject → bound_pubkey_hex` with the
/// operator's `admin` key. Returns the published event id on success.
///
/// `subject` is the binding key (`slack:<team>:<user>`); `bound_pubkey_hex` is the
/// claimant's 64-char hex pubkey. `auth_tag` carries the relay's community
/// scope when one is required (same tag the CLI injects).
pub async fn publish_attestation(
    relay_url: &str,
    admin: &Keys,
    subject: &str,
    bound_pubkey_hex: &str,
    auth_tag: Option<&Tag>,
) -> Result<String, AttestError> {
    let builder = buzz_sdk::build_import_identity_binding(subject, bound_pubkey_hex)
        .map_err(|e| AttestError::Build(e.to_string()))?;
    let event = builder
        .sign_with_keys(admin)
        .map_err(|e| AttestError::Sign(e.to_string()))?;
    let event_id = event.id.to_hex();
    let ok = buzz_ws_client::publish_event(relay_url, event, admin, auth_tag, 75).await?;
    if !ok.accepted {
        return Err(AttestError::Rejected(ok.message));
    }
    Ok(event_id)
}
