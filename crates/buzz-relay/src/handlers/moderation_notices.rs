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

/// Which notice is being delivered — determines template + audience.
#[derive(Debug, Clone)]
pub enum ModerationNotice {
    /// To a reporter: their report was reviewed; outcome summary.
    ReportResolved {
        /// The resolved report row.
        report_id: Uuid,
        /// `resolved` | `dismissed` | `escalated`.
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
/// Idempotent per (action/report id, recipient): re-delivery is a no-op.
pub async fn send_moderation_notice(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    recipient_pubkey: &[u8],
    notice: ModerationNotice,
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

    // Idempotency: a notice for this source id already exists in this DM ⇒ no-op.
    // The `e` tag carries the source (report/action) id; keyed on it, a retry
    // after a crash between insert and fan-out is a safe no-op.
    let source_hex = notice.source_id().to_string();
    let existing = state
        .db
        .query_events(&buzz_db::event::EventQuery {
            kinds: Some(vec![KIND_STREAM_MESSAGE as i32]),
            channel_id: Some(dm_channel_id),
            e_tags: Some(vec![source_hex.clone()]),
            authors: Some(vec![relay_pubkey_bytes.to_vec()]),
            limit: Some(1),
            ..buzz_db::event::EventQuery::for_community(tenant.community())
        })
        .await?;
    if !existing.is_empty() {
        return Ok(());
    }

    // 2. Ensure the relay's "{host} Moderation" kind:0 profile exists, and 3.
    //    the DM's kind:39000 discovery (with `hidden` / `t=dm` / `p`). Both are
    //    replaceable and cheap to re-emit, but we only need them on first use.
    if was_created {
        if let Err(e) = publish_moderation_profile(tenant, state, &relay_pubkey_hex).await {
            warn!(error = %e, "moderation profile publish failed (continuing)");
        }
        emit_group_discovery_events(tenant, state, dm_channel_id).await?;
    }

    // 4. Insert the relay-signed kind:9 notice with `h=<dm_channel_id>` and an
    //    `e` tag naming the source id for idempotency + client linking.
    let tags = vec![
        Tag::parse(["h", &dm_channel_id.to_string()])?,
        Tag::parse(["e", &source_hex])?,
    ];
    let event = EventBuilder::new(
        Kind::Custom(KIND_STREAM_MESSAGE as u16),
        notice.body(tenant),
    )
    .tags(tags)
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
    /// the `e`-tag target that lets a client link the notice back to its action.
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
