/// NIP-42 authentication handler.
pub mod auth;
/// Subscription close (CLOSE) handler.
pub mod close;
/// Command executor — transactional processing for command kinds.
pub mod command_executor;
/// Relay-operator community provisioning HTTP support.
pub mod community_provisioning;
/// NIP-45 COUNT handler.
pub mod count;
/// EVENT handler — WS dispatcher → ingest pipeline → fan-out.
pub mod event;
/// NIP-IA identity archive request handler (kinds 9035–9036).
pub mod identity_archive;
/// imeta tag validation helpers.
pub mod imeta;
/// Transport-neutral event ingestion pipeline.
pub mod ingest;
/// Community moderation authorization seam (capability helper).
pub mod moderation_authz;
/// Community moderation command handler (kinds 9040–9044).
pub mod moderation_commands;
/// Relay-signed moderation notice DMs.
pub mod moderation_notices;
/// Product-feedback validation + deployment sidecar persistence.
pub mod product_feedback;
#[allow(dead_code, missing_docs)]
pub mod push_lease;
/// NIP-43 relay membership admin command handler (kinds 9030–9032).
pub mod relay_admin;
/// NIP-56 report (kind:1984) validation + moderation queue persistence.
pub mod report;
/// REQ handler — subscribe, deliver historical events, then EOSE.
pub mod req;
/// NIP-29 and NIP-25 side-effect handlers.
pub mod side_effects;

/// Validate a channel picture/avatar URL. Empty explicitly clears the picture;
/// otherwise only compact http(s) URLs are accepted. Uploaded channel icons flow
/// through relay media URLs, so data URLs are intentionally not accepted here.
pub(crate) fn validate_channel_picture_url(picture: &str) -> Result<(), String> {
    const MAX_CHANNEL_PICTURE_URL_LEN: usize = 2048;

    if picture.is_empty() {
        return Ok(());
    }
    if picture.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("picture contains invalid characters".to_string());
    }
    if !picture.starts_with("https://") && !picture.starts_with("http://") {
        return Err("picture must be an http(s) URL, or empty to clear".to_string());
    }
    if picture.len() > MAX_CHANNEL_PICTURE_URL_LEN {
        return Err(format!(
            "picture URL too long: {} bytes (max {MAX_CHANNEL_PICTURE_URL_LEN})",
            picture.len()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_channel_picture_url;

    #[test]
    fn channel_picture_url_accepts_http_https_and_empty_clear() {
        assert!(validate_channel_picture_url("").is_ok());
        assert!(validate_channel_picture_url("http://example.test/icon.png").is_ok());
        assert!(validate_channel_picture_url("https://example.test/icon.png").is_ok());
    }

    #[test]
    fn channel_picture_url_rejects_data_urls_whitespace_and_oversized_urls() {
        assert!(validate_channel_picture_url("data:image/png;base64,abc").is_err());
        assert!(validate_channel_picture_url(" https://example.test/icon.png").is_err());
        assert!(validate_channel_picture_url("https://example.test/icon.png\n").is_err());

        let oversized = format!("https://example.test/{}", "a".repeat(2048));
        assert!(validate_channel_picture_url(&oversized).is_err());
    }
}

/// Extract an optional TTL (in seconds) from a Nostr event's `ttl` tag,
/// applying the server-side override when configured.
///
/// Returns `None` when the event carries no `ttl` tag — the channel is permanent.
pub fn resolve_ttl(event: &nostr::Event, ephemeral_ttl_override: Option<i32>) -> Option<i32> {
    let from_tag: Option<i32> = event.tags.iter().find_map(|t| {
        if t.kind().to_string() == "ttl" {
            t.content().and_then(|s| s.parse::<i32>().ok())
        } else {
            None
        }
    });

    match (from_tag, ephemeral_ttl_override) {
        (Some(original), Some(ovr)) => {
            tracing::debug!(
                original,
                override_val = ovr,
                "Applying BUZZ_EPHEMERAL_TTL_OVERRIDE"
            );
            Some(ovr)
        }
        (ttl, _) => ttl,
    }
}
