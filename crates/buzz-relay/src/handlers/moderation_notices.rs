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

use buzz_core::kind::{event_kind_u32, KIND_MEMBER_ADDED_NOTIFICATION, KIND_STREAM_MESSAGE};
use buzz_core::tenant::TenantContext;

use super::event::dispatch_persistent_event;
use super::side_effects::{
    emit_group_discovery_events, emit_membership_notification_tagged,
    publish_dm_visibility_snapshot,
};
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
        /// The operator-authored public reason (mirrors the tombstone's
        /// `public_reason`); the resolve API documents this text is public.
        public_reason: String,
    },
    /// To a banned/timed-out user: terms of the restriction.
    Restriction {
        /// The audit action row.
        action_id: Uuid,
        /// `ban` | `timeout`.
        kind: String,
        /// The operator-authored public reason; the resolve API documents this
        /// text is public.
        public_reason: String,
        /// For `timeout`: when the restriction lifts. `None` for `ban`
        /// (indefinite) — rendered as "until <RFC3339>" in the timeout body.
        timeout_until: Option<chrono::DateTime<chrono::Utc>>,
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
    // Drop both participants' cached channel sets on every pod, as opening a
    // DM does (`command_executor::handle_dm_open`), so the read the notice
    // below prompts includes this DM. Done on every delivery, not just on
    // creation, so a retry after a failed first delivery still drops them.
    for participant in [recipient_pubkey, relay_pubkey_bytes.as_slice()] {
        state.invalidate_membership(tenant, dm_channel_id, participant);
    }

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
    // Clients filter the sidebar on the NIP-DV snapshot, not `hidden_at`, so
    // republish it before the notice that makes them re-read.
    publish_dm_visibility_snapshot(tenant, state, recipient_pubkey).await?;

    // 2. Ensure the relay's "{host} Moderation" kind:0 profile exists, and 3.
    //    the DM's kind:39000 discovery (with `hidden` / `t=dm` / `p`). Both are
    //    replaceable events, so we emit them on EVERY send rather than gating on
    //    first creation: if discovery failed on the first delivery (it is
    //    `?`-propagated), a `was_created`-gated retry would skip it forever and
    //    leave the thread permanently undiscoverable — a notice delivered into a
    //    channel no client can render. Notices are rare; unconditional re-emit is
    //    cheap and the replaceable-event store makes it idempotent.
    if let Err(e) = publish_moderation_profile(tenant, state, &relay_pubkey_hex).await {
        warn!(error = %e, "moderation profile publish failed (continuing)");
    }
    emit_group_discovery_events(tenant, state, dm_channel_id).await?;

    // The DM is now readable in the recipient's channel list: tell them, as
    // opening a DM does. Sent on every delivery so a new, existing or hidden
    // DM resurfaces live; a repeated notice is a harmless hint. The source tag
    // keeps two deliveries in the same second from sharing one notice.
    let source_id = notice.source_id();
    if let Err(e) = emit_membership_notification_tagged(
        tenant,
        state,
        dm_channel_id,
        recipient_pubkey,
        relay_pubkey_bytes.as_slice(),
        KIND_MEMBER_ADDED_NOTIFICATION,
        &[Tag::parse([MODERATION_SOURCE_TAG, &source_id.to_string()])?],
    )
    .await
    {
        warn!(channel = %dm_channel_id, error = %e, "membership notification emission failed");
    }

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
    // every other user profile so it is resolvable by any client, and through
    // the same seam as ingest: the accepted head projects into `users` in the
    // same transaction, so the relay's identity is searchable like any other
    // publisher and a notice never leaves it behind as an unprojected head
    // for `buzz-admin backfill-user-profiles --precheck`.
    let projection = buzz_db::ProfileProjection::from_kind0_event(&event, tenant.host());
    let outcome = state
        .db
        .replace_profile_event(tenant.community(), &event, &projection)
        .await
        .map_err(|e| anyhow::anyhow!("moderation notice write failed: {}", e.kind()))?;
    if outcome.was_inserted {
        let kind_u32 = event_kind_u32(&outcome.event.event);
        dispatch_persistent_event(
            tenant,
            state,
            &outcome.event,
            kind_u32,
            relay_pubkey_hex,
            None,
        )
        .await;
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
    /// notice's own fields — a report/action status, a summary, and a
    /// `public_reason` that mirrors the tombstone. `public_reason` is the
    /// operator-authored public reason (documented public at the resolve API),
    /// not report-private context: these bodies never carry reporter
    /// identities, other reporters, or raw report notes.
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
                timeout_until,
                ..
            } => {
                let action = match kind.as_str() {
                    "ban" => "You have been banned from",
                    "timeout" => "You have been timed out in",
                    other => other,
                };
                // A timeout tells the user when it lifts; a ban is indefinite.
                let terms = match (kind.as_str(), timeout_until) {
                    ("timeout", Some(until)) => {
                        format!(" until {}", until.to_rfc3339())
                    }
                    _ => String::new(),
                };
                format!("{action} {community}{terms}.\n\nReason: {public_reason}")
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
                timeout_until: None,
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
            timeout_until: None,
        }
        .body(&t);
        assert!(ban.contains("banned from example.org"));
        assert!(ban.contains("Repeated spam."));
        // A ban is indefinite: no "until" clause.
        assert!(!ban.contains("until"));

        let until = chrono::DateTime::parse_from_rfc3339("2026-09-01T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let timeout = ModerationNotice::Restriction {
            action_id: Uuid::new_v4(),
            kind: "timeout".into(),
            public_reason: "Cool off.".into(),
            timeout_until: Some(until),
        }
        .body(&t);
        assert!(timeout.contains("timed out in example.org"));
        // The timeout notice must tell the user for how long (VISION_MODERATION).
        assert!(
            timeout.contains("until 2026-09-01T12:00:00+00:00"),
            "timeout body must carry the expiry term; got: {timeout}"
        );
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

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use nostr::Keys;

    async fn precheck(state: &AppState, tenant: &TenantContext) -> (i64, i64, i64) {
        let row = state
            .db
            .profile_backfill_precheck()
            .await
            .expect("precheck")
            .into_iter()
            .find(|row| row.community_id == tenant.community())
            .expect("community in precheck");
        (
            row.kind0_publishers,
            row.users_rows,
            row.unprojected_publishers,
        )
    }

    async fn users_row(
        state: &AppState,
        tenant: &TenantContext,
        pubkey: &[u8],
    ) -> Option<(Option<String>, Option<Vec<u8>>)> {
        sqlx::query_as(
            "SELECT name, metadata_event_id FROM users WHERE community_id = $1 AND pubkey = $2",
        )
        .bind(tenant.community().as_uuid())
        .bind(pubkey)
        .fetch_optional(state.db.pool())
        .await
        .expect("read users row")
    }

    /// The relay's own kind-0 goes through the projection seam, so publishing
    /// it (on every notice send, and again after a restart — including one
    /// that rotates the relay key) never leaves
    /// `backfill-user-profiles --precheck` with an unprojected publisher.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn moderation_profile_is_projected_so_precheck_reads_zero() {
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to test DB");
        let community_id = Uuid::new_v4();
        let host = format!("moderation-profile-{}.example", community_id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(&host)
            .execute(&pool)
            .await
            .expect("insert test community");
        let tenant = TenantContext::resolved(
            buzz_core::CommunityId::from_uuid(community_id),
            host.clone(),
        );
        let state = crate::state::tests::test_state_with_database_pool(pool.clone()).await;
        let relay_hex = state.relay_keypair.public_key().to_hex();
        let relay_pubkey = state.relay_keypair.public_key().to_bytes();

        publish_moderation_profile(&tenant, &state, &relay_hex)
            .await
            .expect("publish moderation profile");
        assert_eq!(precheck(&state, &tenant).await, (1, 1, 0));
        let head = state
            .db
            .get_current_profile_events(tenant.community(), &[relay_pubkey.to_vec()])
            .await
            .expect("relay head");
        assert_eq!(
            users_row(&state, &tenant, &relay_pubkey).await,
            Some((
                Some(format!("{host} Moderation")),
                Some(head[0].event.id.as_bytes().to_vec()),
            )),
            "the users row is the projection of the published head"
        );

        // Same key again (the next notice, or a restart with the pinned relay
        // key): the newer head replaces the old one and the row follows it.
        publish_moderation_profile(&tenant, &state, &relay_hex)
            .await
            .expect("republish moderation profile");
        assert_eq!(precheck(&state, &tenant).await, (1, 1, 0));

        // A restart without the pinned key publishes under a fresh identity;
        // that publisher is projected too.
        let restarted = crate::state::tests::test_state_with_database_pool(pool).await;
        let restarted_hex = restarted.relay_keypair.public_key().to_hex();
        publish_moderation_profile(&tenant, &restarted, &restarted_hex)
            .await
            .expect("publish after restart");
        assert_eq!(precheck(&restarted, &tenant).await, (2, 2, 0));
    }

    /// One real user plus one moderation notice: the relay's projected
    /// moderation profile gives it a `users` row, but the relay is not a user.
    /// The adoption gauges count exactly what they did before that row
    /// existed — one human, and the relay's own events as `unknown` — while
    /// the precheck still sees every publisher projected.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn moderation_notice_does_not_count_the_relay_as_a_user() {
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to test DB");
        let community_id = Uuid::new_v4();
        let host = format!("moderation-usage-{}.example", community_id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(&host)
            .execute(&pool)
            .await
            .expect("insert test community");
        let tenant = TenantContext::resolved(
            buzz_core::CommunityId::from_uuid(community_id),
            host.clone(),
        );
        let state = crate::state::tests::test_state_with_database_pool(pool).await;
        let relay_pubkey = state.relay_keypair.public_key().to_bytes();

        // One human: an AUTH row plus a kind-0 head published now, so they
        // count in both the stock and the 1-day active gauges.
        let human = Keys::generate();
        let human_pubkey = human.public_key().to_bytes();
        state
            .db
            .ensure_user(tenant.community(), &human_pubkey)
            .await
            .expect("human users row");
        let profile = EventBuilder::new(Kind::Metadata, r#"{"name":"wes"}"#)
            .sign_with_keys(&human)
            .expect("sign kind-0");
        state
            .db
            .replace_profile_event(
                tenant.community(),
                &profile,
                &buzz_db::ProfileProjection::from_kind0_event(&profile, tenant.host()),
            )
            .await
            .expect("human profile");

        send_moderation_notice(
            &tenant,
            &state,
            &human_pubkey,
            ModerationNotice::ReportResolved {
                report_id: Uuid::new_v4(),
                status: "resolved".into(),
                summary: "Thanks for the report.".into(),
            },
            chrono::Utc::now(),
        )
        .await
        .expect("send moderation notice");
        assert_eq!(
            precheck(&state, &tenant).await,
            (2, 2, 0),
            "the human and the relay are both projected"
        );

        let users = state
            .db
            .usage_user_counts(&relay_pubkey)
            .await
            .expect("user counts");
        let row = users
            .iter()
            .find(|row| row.community_id == community_id)
            .expect("community in user counts");
        assert_eq!(
            (row.human, row.agent),
            (1, 0),
            "the relay's identity row is not a user"
        );

        let active = state
            .db
            .usage_active_user_counts("1 day", &relay_pubkey)
            .await
            .expect("active user counts");
        let row = active
            .iter()
            .find(|row| row.community_id == community_id)
            .expect("community in active counts");
        assert_eq!(
            (row.human, row.agent, row.unknown),
            (1, 0, 1),
            "the relay's own events classify as unknown, as they did before it had a row"
        );
    }
}
