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
#[cfg(test)]
use super::side_effects::publish_dm_visibility_snapshot;
use super::side_effects::{
    emit_group_discovery_events, publish_dm_visibility_snapshots_for_recipients,
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
/// Crash-retry safe per (action/report id, recipient): a retry after a
/// committed insert is a no-op; concurrent duplicate sends are not serialized
/// in v1.
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

    // A crash retry for an already-delivered source must be a complete no-op,
    // including preserving a later user hide. `open_dm` deliberately unhides
    // an existing thread, so perform this check before calling it.
    let source_id = notice.source_id();
    let participant_hash =
        buzz_db::dm::compute_participant_hash(&[recipient_pubkey, relay_pubkey_bytes.as_slice()]);
    if let Some(existing_dm) = state
        .db
        .find_dm_by_participants(tenant.community(), &participant_hash)
        .await?
    {
        if notice_already_sent(
            state,
            tenant,
            existing_dm.id,
            &relay_pubkey_bytes,
            source_id,
        )
        .await?
        {
            // A prior attempt may have committed the notice before a
            // transient discovery failure. Duplicate delivery is still a
            // content no-op, but retry the idempotent discovery generation so
            // the channel eventually has a complete 39000/39001/39002 set.
            emit_group_discovery_events(tenant, state, existing_dm.id).await?;
            return Ok(());
        }
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

    // 2. Ensure the relay's "{host} Moderation" kind:0 profile exists.
    if let Err(e) = publish_moderation_profile(tenant, state, &relay_pubkey_hex).await {
        warn!(error = %e, "moderation profile publish failed (continuing)");
    }

    // 3. Insert the relay-signed kind:9 notice with `h=<dm_channel_id>` and a
    //    `moderation_source` tag naming the source row id (idempotency +
    //    client linking).
    let tags = vec![
        Tag::parse(["h", &dm_channel_id.to_string()])?,
        Tag::parse([MODERATION_SOURCE_TAG, &source_id.to_string()])?,
    ];
    let event = EventBuilder::new(
        Kind::Custom(KIND_STREAM_MESSAGE as u16),
        notice.body(tenant),
    )
    .tags(tags)
    .sign_with_keys(&state.relay_keypair)
    .map_err(|e| anyhow::anyhow!("failed to sign moderation notice: {e}"))?;

    // Commit the accepted notice and recipient resurface together. Discovery
    // runs first, so any failure before this transaction leaves a previously
    // hidden moderation conversation hidden rather than exposing stale content.
    let (stored, _inserted, resurfaced_viewers) = state
        .db
        .insert_dm_event_with_thread_metadata(
            tenant.community(),
            &event,
            dm_channel_id,
            None,
            relay_pubkey_bytes.as_slice(),
        )
        .await?;

    if let Err(e) =
        publish_dm_visibility_snapshots_for_recipients(tenant, state, &resurfaced_viewers).await
    {
        warn!(error = %e, "moderation DM visibility snapshot failed (continuing)");
    }

    let kind_u32 = event_kind_u32(&stored.event);
    dispatch_persistent_event(tenant, state, &stored, kind_u32, &relay_pubkey_hex, None).await;

    // 4. Publish the idempotent discovery generation after the notice is
    // durable. The roster is established first inside the helper, so a roster
    // failure cannot leave metadata/admin heads without kind 39002. Propagate
    // any remaining failure after the notice commit; an idempotent caller retry
    // takes the duplicate path above and repairs discovery without reinserting
    // or resurfacing the notice.
    emit_group_discovery_events(tenant, state, dm_channel_id).await?;

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

/// True if a relay-authored notice for `source_id` already exists in this DM.
///
/// Idempotency scan scoped to the recipient's single moderation DM thread
/// (kind:9, relay-authored) — bounded by that user's own notice history, so no
/// unbounded read. Matches the opaque `moderation_source` tag in Rust because
/// `EventQuery` only pushes down standardized `e`/`d`/`p` tags and this row id
/// is intentionally not an `e` tag (see `MODERATION_SOURCE_TAG`).
///
/// `limit` is set to the query clamp (1000): matching is post-query in Rust so
/// `Some(1)` would be wrong, and the default 100-row window could let an old
/// source id fall out of view and re-send a duplicate on crash-retry. 1000
/// moderation notices to one user in one community is a practical ceiling.
async fn notice_already_sent(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    dm_channel_id: Uuid,
    relay_pubkey_bytes: &[u8],
    source_id: Uuid,
) -> anyhow::Result<bool> {
    let existing = state
        .db
        .query_events(&buzz_db::event::EventQuery {
            kinds: Some(vec![KIND_STREAM_MESSAGE as i32]),
            channel_id: Some(dm_channel_id),
            authors: Some(vec![relay_pubkey_bytes.to_vec()]),
            limit: Some(1000),
            ..buzz_db::event::EventQuery::for_community(tenant.community())
        })
        .await?;

    let source_str = source_id.to_string();
    Ok(existing.iter().any(|stored| {
        stored.event.tags.iter().any(|t| {
            let parts = t.as_slice();
            parts.len() >= 2 && parts[0] == MODERATION_SOURCE_TAG && parts[1] == source_str
        })
    }))
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

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn delivered_notice_retry_preserves_a_later_user_hide() {
        use nostr::Keys;
        use sqlx::{PgPool, Row};

        let admin_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".into());
        let admin = PgPool::connect(&admin_url).await.expect("connect admin");
        let scratch_name = format!("moderation_notice_{}", Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE {scratch_name}"
        )))
        .execute(&admin)
        .await
        .expect("create scratch database");
        let slash = admin_url.rfind('/').expect("database URL path");
        let scratch_url = format!("{}/{}", &admin_url[..slash], scratch_name);
        let pool = PgPool::connect(&scratch_url)
            .await
            .expect("connect scratch database");
        buzz_db::migration::run_migrations(&pool)
            .await
            .expect("migrate scratch database");

        let community_uuid = Uuid::new_v4();
        let host = format!("moderation-{}.example", community_uuid.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_uuid)
            .bind(&host)
            .execute(&pool)
            .await
            .expect("insert community");
        let tenant =
            TenantContext::resolved(buzz_core::CommunityId::from_uuid(community_uuid), host);
        let relay_keys = Keys::generate();
        let recipient = Keys::generate();
        let recipient_bytes = recipient.public_key().to_bytes();

        let mut config = crate::config::Config::from_env().expect("default config");
        config.database_url = scratch_url;
        config.read_database_url = None;
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".into();
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            relay_keys.clone(),
            media_storage,
        );
        let state = Arc::new(state);

        let notice = ModerationNotice::Restriction {
            action_id: Uuid::new_v4(),
            kind: "timeout".into(),
            public_reason: "Cool off.".into(),
        };

        let relay_pubkey_bytes = relay_keys.public_key().to_bytes();
        let (preexisting_dm, _) = state
            .db
            .open_dm(
                tenant.community(),
                &[recipient_bytes.as_slice()],
                relay_pubkey_bytes.as_slice(),
            )
            .await
            .expect("open moderation DM before injected failure");
        state
            .db
            .hide_dm(
                tenant.community(),
                preexisting_dm.id,
                recipient_bytes.as_slice(),
            )
            .await
            .expect("hide moderation DM before injected failure");
        sqlx::raw_sql(
            "CREATE FUNCTION fail_moderation_notice_insert() RETURNS trigger \
             LANGUAGE plpgsql AS $$ BEGIN \
               IF NEW.kind = 9 THEN RAISE EXCEPTION 'injected notice failure'; END IF; \
               RETURN NEW; \
             END $$; \
             CREATE TRIGGER fail_moderation_notice_insert \
             BEFORE INSERT ON events FOR EACH ROW \
             EXECUTE FUNCTION fail_moderation_notice_insert();",
        )
        .execute(&pool)
        .await
        .expect("install notice failure injection");
        send_moderation_notice(&tenant, &state, recipient_bytes.as_slice(), notice.clone())
            .await
            .expect_err("notice insertion failure must be returned");
        let hidden_after_failure: bool = sqlx::query_scalar(
            "SELECT hidden_at IS NOT NULL FROM channel_members \
             WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3",
        )
        .bind(tenant.community().as_uuid())
        .bind(preexisting_dm.id)
        .bind(recipient_bytes.as_slice())
        .fetch_one(&pool)
        .await
        .expect("read hidden state after notice failure");
        assert!(
            hidden_after_failure,
            "failed notice insertion must not expose stale moderation content"
        );
        let discovery_after_failure: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events \
             WHERE community_id = $1 AND channel_id = $2 \
               AND kind IN (39000, 39001, 39002) AND deleted_at IS NULL",
        )
        .bind(tenant.community().as_uuid())
        .bind(preexisting_dm.id)
        .fetch_one(&pool)
        .await
        .expect("count discovery after failed notice");
        assert_eq!(
            discovery_after_failure, 0,
            "a rejected notice must not publish a partial discovery generation"
        );
        sqlx::raw_sql(
            "DROP TRIGGER fail_moderation_notice_insert ON events; \
             DROP FUNCTION fail_moderation_notice_insert();",
        )
        .execute(&pool)
        .await
        .expect("remove notice failure injection");

        sqlx::raw_sql(
            "CREATE FUNCTION fail_moderation_metadata_insert() RETURNS trigger \
             LANGUAGE plpgsql AS $$ BEGIN \
               IF NEW.kind = 39000 THEN RAISE EXCEPTION 'injected discovery failure'; END IF; \
               RETURN NEW; \
             END $$; \
             CREATE TRIGGER fail_moderation_metadata_insert \
             BEFORE INSERT ON events FOR EACH ROW \
             EXECUTE FUNCTION fail_moderation_metadata_insert();",
        )
        .execute(&pool)
        .await
        .expect("install discovery failure injection");
        send_moderation_notice(&tenant, &state, recipient_bytes.as_slice(), notice.clone())
            .await
            .expect_err("post-commit discovery failure must request a retry");
        let partial_discovery_kinds: Vec<i32> = sqlx::query_scalar(
            "SELECT kind FROM events \
             WHERE community_id = $1 AND channel_id = $2 \
               AND kind IN (39000, 39001, 39002) AND deleted_at IS NULL \
             ORDER BY kind",
        )
        .bind(tenant.community().as_uuid())
        .bind(preexisting_dm.id)
        .fetch_all(&pool)
        .await
        .expect("read roster-first discovery state");
        assert_eq!(
            partial_discovery_kinds,
            vec![39002],
            "metadata failure must not strand 39000/39001 without a roster"
        );
        let notice_count_after_discovery_failure: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events \
             WHERE community_id = $1 AND channel_id = $2 AND kind = 9 \
               AND deleted_at IS NULL",
        )
        .bind(tenant.community().as_uuid())
        .bind(preexisting_dm.id)
        .fetch_one(&pool)
        .await
        .expect("count durable notice after discovery failure");
        assert_eq!(notice_count_after_discovery_failure, 1);
        sqlx::raw_sql(
            "DROP TRIGGER fail_moderation_metadata_insert ON events; \
             DROP FUNCTION fail_moderation_metadata_insert();",
        )
        .execute(&pool)
        .await
        .expect("remove discovery failure injection");

        send_moderation_notice(&tenant, &state, recipient_bytes.as_slice(), notice.clone())
            .await
            .expect("duplicate retry repairs discovery");
        let discovery_kinds: Vec<i32> = sqlx::query_scalar(
            "SELECT kind FROM events \
             WHERE community_id = $1 AND channel_id = $2 \
               AND kind IN (39000, 39001, 39002) AND deleted_at IS NULL \
             ORDER BY kind",
        )
        .bind(tenant.community().as_uuid())
        .bind(preexisting_dm.id)
        .fetch_all(&pool)
        .await
        .expect("read converged moderation discovery");
        assert_eq!(discovery_kinds, vec![39000, 39001, 39002]);

        let participant_hash = buzz_db::dm::compute_participant_hash(&[
            recipient_bytes.as_slice(),
            relay_keys.public_key().to_bytes().as_slice(),
        ]);
        let dm = state
            .db
            .find_dm_by_participants(tenant.community(), &participant_hash)
            .await
            .expect("find moderation DM")
            .expect("moderation DM exists");
        state
            .db
            .hide_dm(tenant.community(), dm.id, recipient_bytes.as_slice())
            .await
            .expect("hide moderation DM");
        publish_dm_visibility_snapshot(&tenant, &state, recipient_bytes.as_slice())
            .await
            .expect("publish hidden snapshot");

        let notice_count_before: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events \
             WHERE community_id = $1 AND channel_id = $2 AND kind = $3 \
               AND pubkey = $4 AND deleted_at IS NULL",
        )
        .bind(tenant.community().as_uuid())
        .bind(dm.id)
        .bind(KIND_STREAM_MESSAGE as i32)
        .bind(relay_keys.public_key().to_bytes().as_slice())
        .fetch_one(&pool)
        .await
        .expect("count notices before retry");

        send_moderation_notice(&tenant, &state, recipient_bytes.as_slice(), notice)
            .await
            .expect("retry delivered notice");

        let hidden: bool = sqlx::query(
            "SELECT hidden_at IS NOT NULL AS hidden FROM channel_members \
             WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3",
        )
        .bind(tenant.community().as_uuid())
        .bind(dm.id)
        .bind(recipient_bytes.as_slice())
        .fetch_one(&pool)
        .await
        .expect("read hidden state")
        .try_get("hidden")
        .expect("decode hidden state");
        assert!(hidden, "duplicate notice must preserve the user's hide");

        let snapshot = state
            .db
            .query_events(&buzz_db::event::EventQuery {
                kinds: Some(vec![buzz_core::kind::KIND_DM_VISIBILITY as i32]),
                pubkey: Some(relay_keys.public_key().to_bytes().to_vec()),
                d_tag: Some(recipient.public_key().to_hex()),
                limit: Some(1),
                ..buzz_db::event::EventQuery::for_community(tenant.community())
            })
            .await
            .expect("read visibility snapshot")
            .into_iter()
            .next()
            .expect("visibility snapshot exists");
        assert!(snapshot.event.tags.iter().any(|tag| {
            let parts = tag.as_slice();
            parts.len() >= 2 && parts[0] == "h" && parts[1] == dm.id.to_string()
        }));

        let notice_count_after: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events \
             WHERE community_id = $1 AND channel_id = $2 AND kind = $3 \
               AND pubkey = $4 AND deleted_at IS NULL",
        )
        .bind(tenant.community().as_uuid())
        .bind(dm.id)
        .bind(KIND_STREAM_MESSAGE as i32)
        .bind(relay_keys.public_key().to_bytes().as_slice())
        .fetch_one(&pool)
        .await
        .expect("count notices after retry");
        assert_eq!(notice_count_after, notice_count_before);

        audit_shutdown
            .drain(std::time::Duration::from_secs(5))
            .await;
        drop(state);
        pool.close().await;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {scratch_name} WITH (FORCE)"
        )))
        .execute(&admin)
        .await;
    }
}
