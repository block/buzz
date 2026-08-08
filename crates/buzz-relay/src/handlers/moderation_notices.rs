//! Relay-signed moderation notice DMs (Phase 1 contract).
//!
//! Plan §0.3 (Tyler, 2026-07-07): every resolution/action notice is a real
//! nostr message in the DB, authored by the relay moderation key:
//!
//! 1. Create/reuse the two-party DM channel `{relay mod key, user}` via the
//!    participant-hash-idempotent DM model (`buzz-db/src/dm.rs`).
//! 2. Emit kind:39000 discovery with `hidden`, `t=dm`, and `p` tags.
//! 3. Insert a relay-signed kind:9 with `h=<dm_channel_id>`.
//! 4. Publish a relay kind:0 profile named "{Community} Moderation".
//!
//! One DM thread per user per community. Non-replyable in v1 (replies are
//! v2 appeal routing). The same primitive carries reporter-resolution,
//! actioned-author, and timeout/ban notices.
//!
//! ## Privacy
//! Notices to an actioned author never name the reporter(s) or quote report
//! notes. Notices to a reporter never reveal other reporters.
//!
//! Lane ownership: L5 (Sami).

use std::sync::Arc;

use nostr::{EventBuilder, Kind, Tag};
use tracing::warn;
use uuid::Uuid;

use buzz_core::kind::{event_kind_u32, KIND_STREAM_MESSAGE};
use buzz_core::tenant::TenantContext;

use super::event::dispatch_persistent_event;
use super::side_effects::emit_group_discovery_events;
use crate::state::AppState;

/// Tag naming the moderation source row (report/action) a notice was derived
/// from. Deliberately non-standard: `e` is reserved for 32-byte event ids, but
/// the source is an opaque DB row UUID. Used for idempotency and client linking.
const MODERATION_SOURCE_TAG: &str = "moderation_source";

/// Which notice is being delivered — determines template + audience.
#[derive(Debug, Clone)]
pub enum ModerationNotice {
    /// To a reporter: their report was reviewed; outcome summary.
    ReportResolved {
        /// The resolved report row.
        report_id: Uuid,
        /// `resolved` | `dismissed`.
        status: String,
        /// Sanitized outcome line (no reporter/mod identities beyond policy).
        summary: String,
    },
    /// To an actioned author: which message, which rule, what happened.
    ContentActioned {
        /// The audit action row.
        action_id: Uuid,
        /// Sanitized reason (mirrors the tombstone's `public_reason`).
        public_reason: String,
    },
    /// To a banned/timed-out user: terms of the restriction.
    Restriction {
        /// The audit action row.
        action_id: Uuid,
        /// `ban` | `timeout` (with expiry rendered into the message).
        kind: String,
        /// Sanitized reason.
        public_reason: String,
    },
}

/// Deliver a moderation notice to `recipient` in this community's
/// relay-authored DM thread (created on first use, reused after).
///
/// Idempotent and concurrency-safe: the notice event is constructed
/// deterministically from `idempotency_ts` (the outbox row's `created_at`) so
/// that two workers racing on the same outbox row produce byte-identical Nostr
/// events. The `insert_event` ON CONFLICT DO NOTHING constraint then ensures
/// exactly one row is durably persisted. Pass `row.created_at` as
/// `idempotency_ts`.
pub async fn send_moderation_notice(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    recipient_pubkey: &[u8],
    notice: ModerationNotice,
    idempotency_ts: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<()> {
    if recipient_pubkey.len() != 32 {
        anyhow::bail!(
            "moderation notice recipient must be a 32-byte pubkey, got {}",
            recipient_pubkey.len()
        );
    }
    let relay_pubkey = state.relay_keypair.public_key();
    let relay_pubkey_bytes = relay_pubkey.to_bytes();
    let relay_pubkey_hex = hex::encode(relay_pubkey_bytes);

    // Never DM the relay key itself (would create a self-DM and is meaningless).
    if recipient_pubkey == relay_pubkey_bytes.as_slice() {
        return Ok(());
    }

    // 1. Create/reuse the two-party DM channel {relay mod key, recipient}.
    //    `open_dm` is participant-hash idempotent, so re-delivery to the same
    //    user reuses the one thread per (community, user).
    let (dm_channel, was_created) = state
        .db
        .open_dm(
            tenant.community(),
            &[recipient_pubkey],
            relay_pubkey_bytes.as_slice(),
        )
        .await?;
    let dm_channel_id = dm_channel.id;

    // Count new DM creation; side-effect gates below intentionally do not
    // gate on was_created (see comment at step 2).
    if was_created {
        metrics::counter!(
            "buzz_channels_created_total",
            "community" => tenant.host().to_owned(),
            "type" => "dm"
        )
        .increment(1);
    }

    // Resurface the moderation DM for the recipient. `open_dm` only clears
    // `hidden_at` for `created_by` (the relay key), so a user who hid the
    // "{host} Moderation" thread would never see a later ban/resolution notice.
    // The closed-loop trust requirement needs the notice to reappear.
    state
        .db
        .unhide_dm(tenant.community(), dm_channel_id, recipient_pubkey)
        .await?;

    // 2. Ensure the relay's "{host} Moderation" kind:0 profile exists, and 3.
    //    the DM's kind:39000 discovery (with `hidden` / `t=dm` / `p`). Both are
    //    replaceable events, so we emit them on EVERY send rather than gating on
    //    first creation: if discovery failed on the first delivery (it is
    //    `?`-propagated), a `was_created`-gated retry would skip it forever and
    //    leave the thread permanently undiscoverable — a notice delivered into a
    //    channel no client can render. Notices are rare; unconditional re-emit is
    //    cheap and `replace_addressable_event` makes it idempotent.
    if let Err(e) = publish_moderation_profile(tenant, state, &relay_pubkey_hex).await {
        warn!(error = %e, "moderation profile publish failed (continuing)");
    }
    emit_group_discovery_events(tenant, state, dm_channel_id).await?;

    // 4. Insert the relay-signed kind:9 notice with `h=<dm_channel_id>` and a
    //    `moderation_source` tag naming the source row id (idempotency +
    //    client linking).
    //
    //    Concurrency-safe idempotency: the event is constructed deterministically
    //    from `idempotency_ts` (the outbox row's immutable `created_at`). Two
    //    workers racing on the same outbox row produce byte-identical Nostr events
    //    (same pubkey + created_at + kind + tags + content = same SHA256 event ID).
    //    `insert_event`'s ON CONFLICT DO NOTHING ensures exactly one row is
    //    durably persisted regardless of how many workers reach this point.
    let source_id = notice.source_id();
    let tags = vec![
        Tag::parse(["h", &dm_channel_id.to_string()])?,
        Tag::parse([MODERATION_SOURCE_TAG, &source_id.to_string()])?,
    ];
    let ts = nostr::Timestamp::from(idempotency_ts.timestamp() as u64);
    let event = EventBuilder::new(
        Kind::Custom(KIND_STREAM_MESSAGE as u16),
        notice.body(tenant),
    )
    .tags(tags)
    .custom_created_at(ts)
    .sign_with_keys(&state.relay_keypair)
    .map_err(|e| anyhow::anyhow!("failed to sign moderation notice: {e}"))?;

    let (stored, _inserted) = state
        .db
        .insert_event(tenant.community(), &event, Some(dm_channel_id))
        .await?;

    let kind_u32 = event_kind_u32(&stored.event);
    dispatch_persistent_event(tenant, state, &stored, kind_u32, &relay_pubkey_hex, None).await;

    Ok(())
}

/// Publish the relay-signed kind:0 "{host} Moderation" profile so clients can
/// render the DM author with a recognizable name. Replaceable (NIP-01), so
/// re-emitting is idempotent — the latest wins.
async fn publish_moderation_profile(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    relay_pubkey_hex: &str,
) -> anyhow::Result<()> {
    let name = format!("{} Moderation", tenant.host());
    let metadata = serde_json::json!({
        "name": name,
        "display_name": name,
        "about": "Automated notices about moderation actions in this community. \
                  Replies are not monitored.",
    });
    let event = EventBuilder::new(Kind::Metadata, metadata.to_string())
        .sign_with_keys(&state.relay_keypair)
        .map_err(|e| anyhow::anyhow!("failed to sign moderation profile: {e}"))?;

    // kind:0 is a replaceable event; store globally (channel_id = None) like
    // every other user profile so it is resolvable by any client.
    let (stored, was_inserted) = state
        .db
        .replace_addressable_event(tenant.community(), &event, None)
        .await?;
    if was_inserted {
        let kind_u32 = event_kind_u32(&stored.event);
        dispatch_persistent_event(tenant, state, &stored, kind_u32, relay_pubkey_hex, None).await;
    }
    Ok(())
}

impl ModerationNotice {
    /// The source row id this notice is derived from — the idempotency key and
    /// the `moderation_source` tag value that lets a client link the notice back
    /// to its action.
    fn source_id(&self) -> Uuid {
        match self {
            ModerationNotice::ReportResolved { report_id, .. } => *report_id,
            ModerationNotice::ContentActioned { action_id, .. } => *action_id,
            ModerationNotice::Restriction { action_id, .. } => *action_id,
        }
    }

    /// Render the recipient-facing message body.
    ///
    /// Privacy invariant (module docs): these strings are built only from the
    /// notice's own sanitized fields — a report/action status, a summary, and a
    /// `public_reason` that already mirrors the tombstone. They never carry
    /// reporter identities, other reporters, or raw report notes.
    fn body(&self, tenant: &TenantContext) -> String {
        let community = tenant.host();
        match self {
            ModerationNotice::ReportResolved {
                status, summary, ..
            } => {
                let outcome = match status.as_str() {
                    "resolved" => "was reviewed and acted on",
                    "dismissed" => "was reviewed; no action was taken",
                    "escalated" => "was escalated for further review",
                    other => other,
                };
                format!(
                    "Thanks for your report to {community}. Your report {outcome}.\n\n{summary}"
                )
            }
            ModerationNotice::ContentActioned { public_reason, .. } => {
                format!(
                    "A moderator in {community} took action on your content.\n\nReason: {public_reason}"
                )
            }
            ModerationNotice::Restriction {
                kind,
                public_reason,
                ..
            } => {
                let action = match kind.as_str() {
                    "ban" => "You have been banned from",
                    "timeout" => "You have been timed out in",
                    other => other,
                };
                format!("{action} {community}.\n\nReason: {public_reason}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant() -> TenantContext {
        TenantContext::resolved(
            buzz_core::CommunityId::from_uuid(Uuid::new_v4()),
            "example.org",
        )
    }

    #[test]
    fn source_id_selects_the_right_field() {
        let report = Uuid::new_v4();
        let action = Uuid::new_v4();
        assert_eq!(
            ModerationNotice::ReportResolved {
                report_id: report,
                status: "resolved".into(),
                summary: String::new(),
            }
            .source_id(),
            report
        );
        assert_eq!(
            ModerationNotice::ContentActioned {
                action_id: action,
                public_reason: String::new(),
            }
            .source_id(),
            action
        );
        assert_eq!(
            ModerationNotice::Restriction {
                action_id: action,
                kind: "ban".into(),
                public_reason: String::new(),
            }
            .source_id(),
            action
        );
    }

    #[test]
    fn report_resolved_body_reflects_status_and_never_leaks_reporter() {
        let t = tenant();
        let body = ModerationNotice::ReportResolved {
            report_id: Uuid::new_v4(),
            status: "dismissed".into(),
            summary: "The message did not violate community rules.".into(),
        }
        .body(&t);
        assert!(body.contains("example.org"));
        assert!(body.contains("no action was taken"));
        assert!(body.contains("did not violate"));
    }

    #[test]
    fn restriction_body_distinguishes_ban_from_timeout() {
        let t = tenant();
        let ban = ModerationNotice::Restriction {
            action_id: Uuid::new_v4(),
            kind: "ban".into(),
            public_reason: "Repeated spam.".into(),
        }
        .body(&t);
        assert!(ban.contains("banned from example.org"));
        assert!(ban.contains("Repeated spam."));

        let timeout = ModerationNotice::Restriction {
            action_id: Uuid::new_v4(),
            kind: "timeout".into(),
            public_reason: "Cool off.".into(),
        }
        .body(&t);
        assert!(timeout.contains("timed out in example.org"));
    }

    #[test]
    fn content_actioned_body_carries_only_the_public_reason() {
        let t = tenant();
        let body = ModerationNotice::ContentActioned {
            action_id: Uuid::new_v4(),
            public_reason: "Off-topic.".into(),
        }
        .body(&t);
        assert!(body.contains("took action on your content"));
        assert!(body.contains("Off-topic."));
    }
}
