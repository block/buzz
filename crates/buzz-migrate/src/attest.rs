//! Publishing the owner/admin **attestation** (kind 30623) that a verified
//! claim produces.
//!
//! This is the one place the service uses the operator's admin key. It signs a
//! single `KIND_IMPORT_IDENTITY_BINDING` event mapping the proven
//! `subject` (`slack:<id>`) to the claimant's Buzz pubkey, then publishes it to
//! the relay. The event is public-key-only and parameterized-replaceable, so a
//! mistaken or superseded attestation can be overwritten (NIP-33) or revoked by
//! the operator later — the service never mints anything irreversible.
//!
//! The attestation is only *half* of a binding: nothing is attributed until the
//! claimant's own `KIND_IMPORT_IDENTITY_CLAIM` (self-signed) also exists. So a
//! stolen admin key can, at worst, publish attestations that stay inert without
//! each subject's separate consent.

use nostr::{Keys, Tag};

/// Why publishing an attestation failed.
#[derive(Debug, thiserror::Error)]
pub enum AttestError {
    #[error("could not build attestation: {0}")]
    Build(String),
    #[error("could not sign attestation: {0}")]
    Sign(String),
    #[error("relay rejected the attestation: {0}")]
    Rejected(String),
    #[error(transparent)]
    Transport(#[from] buzz_ws_client::WsClientError),
}

/// Sign and publish the attestation `subject → bound_pubkey_hex` with the
/// operator's `admin` key. Returns the published event id on success.
///
/// `subject` is the binding key (`slack:<id>`); `bound_pubkey_hex` is the
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
