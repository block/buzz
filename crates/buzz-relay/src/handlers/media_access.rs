//! Authorize local attachment references before their containing event is stored.

use buzz_core::TenantContext;
use nostr::Event;

use super::ingest::IngestError;
use crate::state::AppState;

/// A reference can grant access to additional readers. Check the author's
/// existing access first so a removed member cannot publish a known private
/// URL into their profile or another channel to regain download access.
pub async fn validate_media_references(
    tenant: &TenantContext,
    state: &AppState,
    event: &Event,
) -> Result<(), IngestError> {
    // The SQL extractor only recognizes literal /media/ URLs (including JSON
    // escaped slashes). Most events have none and need no additional DB read.
    if !event.content.contains("media")
        && !event
            .tags
            .iter()
            .any(|tag| tag.as_slice().iter().any(|part| part.contains("media")))
    {
        return Ok(());
    }
    let tags = serde_json::to_value(&event.tags).map_err(|error| {
        IngestError::Internal(format!(
            "error: media reference serialization failed: {error}"
        ))
    })?;
    let hashes = state
        .db
        .local_media_hashes(&event.content, &tags, tenant.host())
        .await
        .map_err(|error| {
            IngestError::Internal(format!("error: media reference lookup failed: {error}"))
        })?;
    if hashes.len() > 64 {
        return Err(IngestError::Rejected(
            "invalid: too many media references".into(),
        ));
    }
    let author = super::ingest::effective_message_author(event, &state.relay_keypair.public_key());
    for sha256 in hashes {
        let allowed = state
            .db
            .can_reference_media(tenant.community(), &sha256, &author)
            .await
            .map_err(|error| {
                IngestError::Internal(format!("error: media access lookup failed: {error}"))
            })?;
        if !allowed {
            return Err(IngestError::Rejected(
                "restricted: attachment is not accessible".into(),
            ));
        }
    }
    Ok(())
}
