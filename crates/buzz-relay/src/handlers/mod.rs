/// NIP-42 authentication handler.
pub mod auth;
/// Subscription close (CLOSE) handler.
pub mod close;
/// Command executor — transactional processing for command kinds.
pub mod command_executor;
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
/// Mesh hole-punch signaling: validate membership + emit paired call-me-now.
pub mod mesh_signaling;
/// NIP-43 relay membership admin command handler (kinds 9030–9032).
pub mod relay_admin;
/// REQ handler — subscribe, deliver historical events, then EOSE.
pub mod req;
/// NIP-29 and NIP-25 side-effect handlers.
pub mod side_effects;

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
